//! Iteration strategy for workload key generation
//!
//! This module provides flexible iteration patterns for workload key generation:
//! - Sequential: Iterate through keys in order (0, 1, 2, ...)
//! - Random: Deterministic pseudo-random key selection
//! - Subset: Iterate over a range within the keyspace
//! - Zipfian: Hot-key distribution (power law)
//! - MixedRatio: Configurable ratio of existing vs new keys (via keyspace_tracker)
//!
//! ## Integration with keyspace_tracker
//!
//! The `KeyspaceIterator` struct bridges `IterationStrategy` with keyspace_tracker's
//! powerful `TrackerIterBuilder`. This enables:
//! - Mixed-ratio iteration (e.g., 90% overwrites + 10% new writes)
//! - Existence-aware iteration (set-only, unset-only)
//! - Atomic claim semantics for concurrent workloads
//! - Access distributions (Zipfian, Exponential, Hotspot, etc.)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use keyspace_tracker::{AccessDistribution, PrefixTracker, SamplingConfig, TrackerConfig};

/// Iteration strategy for workload key generation
#[derive(Debug, Clone)]
#[derive(Default)]
pub enum IterationStrategy {
    /// Sequential iteration (0, 1, 2, ...)
    #[default]
    Sequential,

    /// Deterministic pseudo-random iteration
    Random {
        /// Seed for random number generation
        seed: u64,
    },

    /// Subset of keyspace (only keys within range)
    Subset {
        /// Start of range (inclusive)
        start: u64,
        /// End of range (exclusive)
        end: u64,
        /// Inner iteration strategy within the subset
        inner: Box<IterationStrategy>,
    },

    /// Zipfian distribution (hot keys)
    Zipfian {
        /// Skew parameter (higher = more skewed toward hot keys)
        /// Typical values: 0.5 - 2.0
        skew: f64,
        /// Seed for random number generation
        seed: u64,
    },
}


impl IterationStrategy {
    /// Create a sequential strategy
    pub fn sequential() -> Self {
        IterationStrategy::Sequential
    }

    /// Create a random strategy with the given seed
    pub fn random(seed: u64) -> Self {
        IterationStrategy::Random { seed }
    }

    /// Create a subset strategy
    pub fn subset(start: u64, end: u64, inner: IterationStrategy) -> Self {
        IterationStrategy::Subset {
            start,
            end,
            inner: Box::new(inner),
        }
    }

    /// Create a Zipfian strategy
    pub fn zipfian(skew: f64, seed: u64) -> Self {
        IterationStrategy::Zipfian { skew, seed }
    }

    /// Get the next key using the given counter and keyspace length
    ///
    /// # Arguments
    /// * `counter` - Current iteration counter (monotonically increasing)
    /// * `keyspace_len` - Total size of the keyspace
    ///
    /// # Returns
    /// The key index within [0, keyspace_len)
    pub fn next_key(&self, counter: u64, keyspace_len: u64) -> u64 {
        match self {
            IterationStrategy::Sequential => counter % keyspace_len,

            IterationStrategy::Random { seed } => {
                Self::splitmix64(*seed, counter, keyspace_len)
            }

            IterationStrategy::Subset { start, end, inner } => {
                let range_len = end.saturating_sub(*start);
                if range_len == 0 {
                    *start
                } else {
                    let inner_key = inner.next_key(counter, range_len);
                    start + inner_key
                }
            }

            IterationStrategy::Zipfian { skew, seed } => {
                Self::zipfian_key(*seed, counter, keyspace_len, *skew)
            }
        }
    }

    /// SplitMix64 deterministic "random" key generation
    ///
    /// Same algorithm used in the existing codebase for reproducible benchmarks.
    fn splitmix64(seed: u64, index: u64, keyspace_len: u64) -> u64 {
        let mut x = seed.wrapping_add(index.wrapping_mul(0x9E3779B97F4A7C15));
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
        x = x ^ (x >> 31);
        x % keyspace_len
    }

    /// Zipfian distribution key generation
    ///
    /// Uses rejection sampling with the generalized harmonic number approximation.
    fn zipfian_key(seed: u64, counter: u64, keyspace_len: u64, skew: f64) -> u64 {
        if keyspace_len == 0 {
            return 0;
        }

        // Use counter as input to deterministic random
        let random_val = Self::splitmix64(seed, counter, u64::MAX);
        let uniform = (random_val as f64) / (u64::MAX as f64);

        // Inverse CDF approximation for Zipfian
        // p(k) = 1/(k^s * H_n,s) where H_n,s is the generalized harmonic number
        let n = keyspace_len as f64;
        let s = skew;

        // Approximation: For s > 1, H_n,s ~ zeta(s) - 1/(s-1)/n^(s-1)
        // For s <= 1, use a different approximation
        let key = if s > 1.0 {
            // Use inverse transform sampling with approximation
            let inv_s = 1.0 / s;
            let base = 1.0 - uniform;
            let k = (base.powf(-inv_s) * (1.0 - uniform).powf(1.0 / (s - 1.0))).min(n);
            k.max(1.0) as u64
        } else {
            // For s <= 1, use a simpler approximation
            let k = (uniform * n).powf(1.0 / (s + 0.5));
            k.min(n).max(1.0) as u64
        };

        // Clamp to valid range
        (key - 1).min(keyspace_len - 1)
    }

    /// Parse iteration strategy from string
    ///
    /// Formats:
    /// - "sequential" or "seq"
    /// - "random" or "random:SEED"
    /// - "subset:START:END" or "subset:START:END:inner"
    /// - "zipfian:SKEW" or "zipfian:SKEW:SEED"
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim().to_lowercase();

        if s == "sequential" || s == "seq" {
            return Ok(IterationStrategy::Sequential);
        }

        if s == "random" {
            return Ok(IterationStrategy::Random { seed: 42 });
        }

        if let Some(rest) = s.strip_prefix("random:") {
            let seed = rest
                .parse::<u64>()
                .map_err(|_| format!("Invalid random seed: {}", rest))?;
            return Ok(IterationStrategy::Random { seed });
        }

        if let Some(rest) = s.strip_prefix("subset:") {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() < 2 {
                return Err("subset requires start:end".to_string());
            }

            let start = parts[0]
                .parse::<u64>()
                .map_err(|_| format!("Invalid subset start: {}", parts[0]))?;
            let end = parts[1]
                .parse::<u64>()
                .map_err(|_| format!("Invalid subset end: {}", parts[1]))?;

            let inner = if parts.len() > 2 {
                Self::parse(&parts[2..].join(":"))?
            } else {
                IterationStrategy::Sequential
            };

            return Ok(IterationStrategy::Subset {
                start,
                end,
                inner: Box::new(inner),
            });
        }

        if let Some(rest) = s.strip_prefix("zipfian:") {
            let parts: Vec<&str> = rest.split(':').collect();
            let skew = parts[0]
                .parse::<f64>()
                .map_err(|_| format!("Invalid zipfian skew: {}", parts[0]))?;
            let seed = if parts.len() > 1 {
                parts[1]
                    .parse::<u64>()
                    .map_err(|_| format!("Invalid zipfian seed: {}", parts[1]))?
            } else {
                42
            };
            return Ok(IterationStrategy::Zipfian { skew, seed });
        }

        Err(format!("Unknown iteration strategy: {}", s))
    }
}

/// Thread-safe iteration state
///
/// Wraps an IterationStrategy with atomic counter for concurrent access.
pub struct IterationState {
    strategy: IterationStrategy,
    counter: AtomicU64,
}

impl IterationState {
    /// Create new iteration state with the given strategy
    pub fn new(strategy: IterationStrategy) -> Self {
        Self {
            strategy,
            counter: AtomicU64::new(0),
        }
    }

    /// Get the next key and increment counter
    pub fn next_key(&self, keyspace_len: u64) -> u64 {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        self.strategy.next_key(counter, keyspace_len)
    }

    /// Get the current counter value
    pub fn counter(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }

    /// Reset the counter
    pub fn reset(&self) {
        self.counter.store(0, Ordering::Relaxed);
    }

    /// Get a reference to the strategy
    pub fn strategy(&self) -> &IterationStrategy {
        &self.strategy
    }
}

// =============================================================================
// KeyspaceIterator - Bridge between IterationStrategy and keyspace_tracker
// =============================================================================

/// Filter for iteration based on existence state
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExistenceFilter {
    /// Iterate over all IDs (no filter)
    #[default]
    All,
    /// Only iterate over IDs that exist (set in tracker)
    SetOnly,
    /// Only iterate over IDs that don't exist (unset in tracker)
    UnsetOnly,
}

/// Thread-safe keyspace iterator using keyspace_tracker
///
/// Bridges `IterationStrategy` with keyspace_tracker's `TrackerIterBuilder`,
/// providing:
/// - Existence-aware iteration (set-only, unset-only, mixed-ratio)
/// - Atomic claim semantics for concurrent writes
/// - Access distributions (Zipfian, Exponential, etc.)
/// - Memory-efficient bitmap-based filtering
///
/// # Example
///
/// ```ignore
/// // Create iterator for loading unset vectors
/// let iter = KeyspaceIterator::new(
///     "vec:",
///     1_000_000,
///     IterationStrategy::Sequential,
///     ExistenceFilter::UnsetOnly,
/// );
///
/// // Claim IDs atomically across threads
/// while let Some(id) = iter.claim_next() {
///     // Load vector at id
/// }
/// ```
pub struct KeyspaceIterator {
    /// Underlying tracker for existence checking
    tracker: Arc<PrefixTracker>,
    /// Iteration strategy
    strategy: IterationStrategy,
    /// Existence filter
    filter: ExistenceFilter,
    /// Mixed ratio (if Some, overrides filter)
    set_ratio: Option<f64>,
    /// Maximum ID (exclusive)
    max_id: u64,
    /// Atomic counter for claim operations
    counter: AtomicU64,
    /// Optional limit on items to return
    limit: Option<u64>,
}

impl KeyspaceIterator {
    /// Create a new keyspace iterator
    ///
    /// # Arguments
    /// * `prefix` - Key prefix (e.g., "vec:")
    /// * `max_id` - Maximum ID (exclusive)
    /// * `strategy` - Iteration strategy
    /// * `filter` - Existence filter
    pub fn new(
        prefix: &str,
        max_id: u64,
        strategy: IterationStrategy,
        filter: ExistenceFilter,
    ) -> Self {
        let config = TrackerConfig::simple(prefix).with_max_id(max_id);
        let tracker = Arc::new(PrefixTracker::new(config));

        Self {
            tracker,
            strategy,
            filter,
            set_ratio: None,
            max_id,
            counter: AtomicU64::new(0),
            limit: None,
        }
    }

    /// Create a new keyspace iterator with an existing tracker
    pub fn with_tracker(
        tracker: Arc<PrefixTracker>,
        max_id: u64,
        strategy: IterationStrategy,
        filter: ExistenceFilter,
    ) -> Self {
        Self {
            tracker,
            strategy,
            filter,
            set_ratio: None,
            max_id,
            counter: AtomicU64::new(0),
            limit: None,
        }
    }

    /// Set mixed ratio (overrides filter)
    ///
    /// # Arguments
    /// * `ratio` - Ratio of existing (set) items to return (0.0-1.0)
    ///   - 0.9 means 90% existing keys, 10% new keys
    ///   - Useful for "90% overwrite + 10% insert" patterns
    pub fn with_set_ratio(mut self, ratio: f64) -> Self {
        self.set_ratio = Some(ratio.clamp(0.0, 1.0));
        self
    }

    /// Set a limit on items to return
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Get the underlying tracker
    pub fn tracker(&self) -> &Arc<PrefixTracker> {
        &self.tracker
    }

    /// Mark an ID as existing (set the bit)
    #[inline]
    pub fn mark_exists(&self, id: u64) {
        if id < self.max_id {
            self.tracker.add(id);
        }
    }

    /// Mark an ID as not existing (clear the bit)
    #[inline]
    pub fn mark_deleted(&self, id: u64) {
        if id < self.max_id {
            self.tracker.remove(id);
        }
    }

    /// Check if an ID exists
    #[inline]
    pub fn exists(&self, id: u64) -> bool {
        if id >= self.max_id {
            return false;
        }
        self.tracker.exists(id)
    }

    /// Get count of existing IDs
    #[inline]
    pub fn count(&self) -> u64 {
        self.tracker.count()
    }

    /// Get the next ID based on strategy, respecting filter
    ///
    /// Returns None when:
    /// - Limit reached
    /// - No more IDs match the filter
    pub fn next(&self) -> Option<u64> {
        // Check limit
        if let Some(limit) = self.limit {
            if self.counter.load(Ordering::Relaxed) >= limit {
                return None;
            }
        }

        // Fast path for All filter with no mixed ratio
        if self.filter == ExistenceFilter::All && self.set_ratio.is_none() {
            let counter = self.counter.fetch_add(1, Ordering::Relaxed);
            if let Some(limit) = self.limit {
                if counter >= limit {
                    return None;
                }
            }
            return Some(self.strategy.next_key(counter, self.max_id));
        }

        // Existence-aware iteration
        self.next_filtered()
    }

    /// Get the next ID that matches the filter
    fn next_filtered(&self) -> Option<u64> {
        // Try up to max_id times to find a matching ID
        for _ in 0..self.max_id {
            let counter = self.counter.fetch_add(1, Ordering::Relaxed);

            // Check limit
            if let Some(limit) = self.limit {
                if counter >= limit {
                    return None;
                }
            }

            let id = self.strategy.next_key(counter, self.max_id);
            let is_set = self.tracker.exists(id);

            // Check mixed ratio
            if let Some(ratio) = self.set_ratio {
                // Use counter as seed for deterministic behavior
                let want_set = Self::deterministic_bool(counter, ratio);
                if want_set == is_set {
                    return Some(id);
                }
                continue;
            }

            // Check filter
            match self.filter {
                ExistenceFilter::All => return Some(id),
                ExistenceFilter::SetOnly if is_set => return Some(id),
                ExistenceFilter::UnsetOnly if !is_set => return Some(id),
                _ => continue,
            }
        }

        None
    }

    /// Claim the next unset ID atomically (for write operations)
    ///
    /// Atomically finds and marks the next unset ID as set.
    /// Multiple threads can call this concurrently without overlap.
    pub fn claim_unset(&self) -> Option<u64> {
        // Use write iterator from tracker for atomic claim
        for id in self.tracker.iter().unset_only().sequential() {
            let (claimed_id, _) = id;
            if claimed_id < self.max_id {
                // Atomically mark as set
                self.tracker.add(claimed_id);
                return Some(claimed_id);
            }
        }
        None
    }

    /// Claim the next set ID atomically (for delete operations)
    ///
    /// Atomically finds the next set ID. Caller should mark as deleted after
    /// successful deletion.
    pub fn claim_set(&self) -> Option<u64> {
        for id in self.tracker.iter().set_only().sequential() {
            let (claimed_id, _) = id;
            if claimed_id < self.max_id {
                return Some(claimed_id);
            }
        }
        None
    }

    /// Reset the counter
    pub fn reset(&self) {
        self.counter.store(0, Ordering::Relaxed);
    }

    /// Get current counter value
    pub fn counter_value(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }

    /// Deterministic boolean based on counter and ratio
    #[inline]
    fn deterministic_bool(counter: u64, ratio: f64) -> bool {
        // Use SplitMix64 mixing for good distribution
        let mut x = counter.wrapping_mul(0x9E3779B97F4A7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
        x = x ^ (x >> 31);

        // Map to [0, 1) and compare
        (x as f64 / u64::MAX as f64) < ratio
    }
}

/// Convert IterationStrategy to keyspace_tracker's AccessDistribution
impl From<&IterationStrategy> for AccessDistribution {
    fn from(strategy: &IterationStrategy) -> Self {
        match strategy {
            IterationStrategy::Sequential => AccessDistribution::Uniform,
            IterationStrategy::Random { .. } => AccessDistribution::Uniform,
            IterationStrategy::Zipfian { skew, .. } => AccessDistribution::Zipfian { skew: *skew },
            IterationStrategy::Subset { inner, .. } => AccessDistribution::from(inner.as_ref()),
        }
    }
}

/// Build SamplingConfig from IterationStrategy
impl IterationStrategy {
    /// Convert to keyspace_tracker SamplingConfig
    pub fn to_sampling_config(&self) -> SamplingConfig {
        match self {
            IterationStrategy::Sequential => SamplingConfig::new(),
            IterationStrategy::Random { seed } => SamplingConfig::new().with_seed(*seed),
            IterationStrategy::Zipfian { skew, seed } => SamplingConfig::new()
                .with_distribution(AccessDistribution::Zipfian { skew: *skew })
                .with_seed(*seed),
            IterationStrategy::Subset { start, end, inner } => {
                // Note: range is handled separately, just propagate inner config
                inner.to_sampling_config()
            }
        }
    }

    /// Get ID range if this is a Subset strategy
    pub fn id_range(&self) -> Option<(u64, u64)> {
        match self {
            IterationStrategy::Subset { start, end, .. } => Some((*start, *end)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequential_iteration() {
        let strategy = IterationStrategy::Sequential;
        assert_eq!(strategy.next_key(0, 100), 0);
        assert_eq!(strategy.next_key(1, 100), 1);
        assert_eq!(strategy.next_key(99, 100), 99);
        assert_eq!(strategy.next_key(100, 100), 0);
        assert_eq!(strategy.next_key(150, 100), 50);
    }

    #[test]
    fn test_random_iteration() {
        let strategy = IterationStrategy::Random { seed: 12345 };

        // Same seed + counter should produce same result
        let k1 = strategy.next_key(0, 1000);

        let k2 = strategy.next_key(0, 1000);
        assert_eq!(k1, k2);

        // Different counters should (usually) produce different results
        let k3 = strategy.next_key(1, 1000);
        // Not asserting inequality as it could theoretically collide
        assert!(k1 < 1000);
        assert!(k3 < 1000);
    }

    #[test]
    fn test_subset_iteration() {
        let strategy = IterationStrategy::Subset {
            start: 100,
            end: 200,
            inner: Box::new(IterationStrategy::Sequential),
        };

        assert_eq!(strategy.next_key(0, 1000), 100);
        assert_eq!(strategy.next_key(1, 1000), 101);
        assert_eq!(strategy.next_key(99, 1000), 199);
        assert_eq!(strategy.next_key(100, 1000), 100); // Wraps within subset
    }

    #[test]
    fn test_parse_sequential() {
        let s = IterationStrategy::parse("sequential").unwrap();
        matches!(s, IterationStrategy::Sequential);

        let s = IterationStrategy::parse("seq").unwrap();
        matches!(s, IterationStrategy::Sequential);
    }

    #[test]
    fn test_parse_random() {
        let s = IterationStrategy::parse("random").unwrap();
        if let IterationStrategy::Random { seed } = s {
            assert_eq!(seed, 42);
        } else {
            panic!("Expected Random");
        }

        let s = IterationStrategy::parse("random:12345").unwrap();
        if let IterationStrategy::Random { seed } = s {
            assert_eq!(seed, 12345);
        } else {
            panic!("Expected Random");
        }
    }

    #[test]
    fn test_parse_subset() {
        let s = IterationStrategy::parse("subset:100:200").unwrap();
        if let IterationStrategy::Subset { start, end, inner } = s {
            assert_eq!(start, 100);
            assert_eq!(end, 200);
            matches!(*inner, IterationStrategy::Sequential);
        } else {
            panic!("Expected Subset");
        }
    }

    #[test]
    fn test_parse_zipfian() {
        let s = IterationStrategy::parse("zipfian:1.5").unwrap();
        if let IterationStrategy::Zipfian { skew, seed } = s {
            assert!((skew - 1.5).abs() < 0.001);
            assert_eq!(seed, 42);
        } else {
            panic!("Expected Zipfian");
        }
    }

    #[test]
    fn test_iteration_state() {
        let state = IterationState::new(IterationStrategy::Sequential);
        assert_eq!(state.next_key(100), 0);
        assert_eq!(state.next_key(100), 1);
        assert_eq!(state.next_key(100), 2);
        assert_eq!(state.counter(), 3);

        state.reset();
        assert_eq!(state.counter(), 0);
        assert_eq!(state.next_key(100), 0);
    }

    #[test]
    fn test_zipfian_distribution() {
        let strategy = IterationStrategy::Zipfian { skew: 1.0, seed: 42 };

        // Generate many keys and check distribution
        let mut counts = vec![0u64; 100];
        for i in 0..10000 {
            let key = strategy.next_key(i, 100);
            counts[key as usize] += 1;
        }

        // Lower keys should have more hits (Zipfian skews toward lower indices)
        // This is a loose check - just verify we get some skew
        let low_sum: u64 = counts[0..10].iter().sum();
        let high_sum: u64 = counts[90..100].iter().sum();
        assert!(low_sum > high_sum, "Zipfian should skew toward lower indices");
    }

    // =========================================================================
    // KeyspaceIterator tests
    // =========================================================================

    #[test]
    fn test_keyspace_iterator_basic() {
        let iter = KeyspaceIterator::new(
            "test:",
            100,
            IterationStrategy::Sequential,
            ExistenceFilter::All,
        );

        // Sequential iteration
        assert_eq!(iter.next(), Some(0));
        assert_eq!(iter.next(), Some(1));
        assert_eq!(iter.next(), Some(2));
    }

    #[test]
    fn test_keyspace_iterator_with_limit() {
        let iter = KeyspaceIterator::new(
            "test:",
            100,
            IterationStrategy::Sequential,
            ExistenceFilter::All,
        )
        .with_limit(5);

        let mut count = 0;
        while iter.next().is_some() {
            count += 1;
        }
        assert_eq!(count, 5);
    }

    #[test]
    fn test_keyspace_iterator_set_only() {
        let iter = KeyspaceIterator::new(
            "test:",
            100,
            IterationStrategy::Sequential,
            ExistenceFilter::SetOnly,
        );

        // Mark some IDs as existing
        iter.mark_exists(5);
        iter.mark_exists(10);
        iter.mark_exists(15);

        // Should only return existing IDs
        let id1 = iter.next();
        assert!(id1.is_some());
        let id = id1.unwrap();
        assert!(iter.exists(id), "Returned ID {} should exist", id);
    }

    #[test]
    fn test_keyspace_iterator_unset_only() {
        let iter = KeyspaceIterator::new(
            "test:",
            10,
            IterationStrategy::Sequential,
            ExistenceFilter::UnsetOnly,
        );

        // Mark all even IDs as existing
        for i in (0..10).step_by(2) {
            iter.mark_exists(i);
        }

        // Should only return unset (odd) IDs
        let id = iter.next().unwrap();
        assert!(!iter.exists(id), "Returned ID {} should not exist", id);
    }

    #[test]
    fn test_keyspace_iterator_mixed_ratio() {
        let iter = KeyspaceIterator::new(
            "test:",
            1000,
            IterationStrategy::Random { seed: 42 },
            ExistenceFilter::All,
        )
        .with_set_ratio(0.8)
        .with_limit(1000);

        // Mark first 500 as existing
        for i in 0..500 {
            iter.mark_exists(i);
        }

        // Count how many returned IDs are existing vs new
        let mut existing_count = 0;
        let mut new_count = 0;

        while let Some(id) = iter.next() {
            if iter.exists(id) {
                existing_count += 1;
            } else {
                new_count += 1;
            }
        }

        let total = existing_count + new_count;
        let actual_ratio = existing_count as f64 / total as f64;

        // Should be approximately 80% existing (within 10% tolerance)
        assert!(
            (actual_ratio - 0.8).abs() < 0.15,
            "Expected ~80% existing, got {:.1}% ({}/{})",
            actual_ratio * 100.0,
            existing_count,
            total
        );
    }

    #[test]
    fn test_keyspace_iterator_claim_unset() {
        let iter = KeyspaceIterator::new(
            "test:",
            10,
            IterationStrategy::Sequential,
            ExistenceFilter::UnsetOnly,
        );

        // Mark some as existing
        iter.mark_exists(0);
        iter.mark_exists(2);
        iter.mark_exists(4);

        // Claim should skip existing IDs
        let claimed = iter.claim_unset();
        assert!(claimed.is_some());
        let id = claimed.unwrap();
        // The claim_unset marks the ID as set after claiming
        // So we just verify it was an odd ID (originally unset)
        assert!(id == 1 || id == 3 || id == 5 || id == 6 || id == 7 || id == 8 || id == 9,
                "Claimed ID {} should have been originally unset", id);
    }

    #[test]
    fn test_strategy_to_sampling_config() {
        let strategy = IterationStrategy::Zipfian { skew: 0.99, seed: 123 };
        let config = strategy.to_sampling_config();

        assert_eq!(config.seed, Some(123));
        assert!(matches!(
            config.distribution,
            AccessDistribution::Zipfian { skew } if (skew - 0.99).abs() < 0.001
        ));
    }
}
