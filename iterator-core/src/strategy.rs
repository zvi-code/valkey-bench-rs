//! Iteration strategies for traversing coordinate spaces.
//!
//! This module provides the [`IterationStrategy`] trait and implementations:
//!
//! - [`ScanStrategy`]: SIMD-accelerated bitmap scanning (find_next_set/unset)
//! - [`SampleStrategy`]: Distribution-based sampling with retry on filter mismatch
//! - [`AdaptiveStrategy`]: Automatically selects scan or sample based on density
//!
//! # When to Use Each Strategy
//!
//! | Strategy | Best For | Density | Distribution |
//! |----------|----------|---------|--------------|
//! | Scan | Sequential access, write/delete ops | Any | Sequential only |
//! | Sample | Random access patterns | >10% | Any distribution |
//! | Adaptive | Unknown workloads | Auto | Any distribution |

use crate::distribution::{Dist, DistState};
use crate::filter::{BitmapOps, FilterCondition};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// =============================================================================
// IterationStrategy Trait
// =============================================================================

/// Strategy for finding the next valid coordinate in a space.
///
/// Different strategies trade off between:
/// - **Scan**: Sequential, efficient for dense/sparse bitmaps with SIMD
/// - **Sample**: Random access, efficient for various distributions
/// - **Adaptive**: Switches between scan and sample based on density
///
/// # Type Parameters
///
/// - `B`: The bitmap type implementing [`BitmapOps`]
pub trait IterationStrategy<B: BitmapOps>: Clone + Send + Sync {
    /// State maintained by this strategy across iterations.
    type State: StrategyState;

    /// Create initial state for iteration.
    ///
    /// # Arguments
    ///
    /// * `max_index` - Maximum valid index (exclusive)
    /// * `seed` - Random seed for stochastic strategies
    fn init_state(&self, max_index: u64, seed: u64) -> Self::State;

    /// Find the next valid index.
    ///
    /// # Arguments
    ///
    /// * `state` - Mutable strategy state
    /// * `bitmap` - The bitmap to check
    /// * `filter` - Filter condition to apply
    /// * `max_index` - Maximum valid index (exclusive)
    ///
    /// # Returns
    ///
    /// The next valid index, or None if exhausted.
    fn next_index(
        &self,
        state: &mut Self::State,
        bitmap: &B,
        filter: FilterCondition,
        max_index: u64,
    ) -> Option<u64>;

    /// Check if this strategy benefits from SIMD scanning.
    fn benefits_from_scan(&self) -> bool {
        false
    }

    /// Check if this strategy is ordered (deterministic sequence).
    fn is_ordered(&self) -> bool;

    /// Reset strategy state for a new iteration cycle.
    fn reset(&self, state: &mut Self::State, seed: u64);
}

/// Marker trait for strategy state types.
pub trait StrategyState: Send {}

// =============================================================================
// ScanStrategy - SIMD-accelerated bitmap scanning
// =============================================================================

/// Strategy that scans the bitmap using SIMD-accelerated find_next operations.
///
/// This is the most efficient strategy when:
/// - You need sequential or write-cursor-based iteration
/// - The bitmap supports efficient find_next_set/find_next_unset
/// - You want to iterate over all matching entries without distribution bias
///
/// # Cursor Modes
///
/// - **Local cursor**: Each iterator has its own position (default)
/// - **Shared cursor**: Multiple iterators share a position (for concurrent claiming)
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{ScanStrategy, AtomicBitmap};
///
/// let strategy = ScanStrategy::new();
/// // or with shared cursor for concurrent claiming:
/// let shared_cursor = Arc::new(AtomicU64::new(0));
/// let strategy = ScanStrategy::with_shared_cursor(shared_cursor);
/// ```
#[derive(Clone, Debug)]
pub struct ScanStrategy {
    /// Optional shared cursor for concurrent claiming
    shared_cursor: Option<Arc<AtomicU64>>,
    /// Enable wrap-around when reaching max_index
    wrap_around: bool,
}

impl Default for ScanStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanStrategy {
    /// Create a new scan strategy with local cursor.
    pub fn new() -> Self {
        Self {
            shared_cursor: None,
            wrap_around: false,
        }
    }

    /// Create with a shared cursor for concurrent access.
    ///
    /// Multiple iterators sharing the same cursor will claim disjoint entries.
    pub fn with_shared_cursor(cursor: Arc<AtomicU64>) -> Self {
        Self {
            shared_cursor: Some(cursor),
            wrap_around: false,
        }
    }

    /// Enable wrap-around mode for continuous iteration.
    pub fn wrap_around(mut self) -> Self {
        self.wrap_around = true;
        self
    }
}

/// State for scan strategy.
#[derive(Debug)]
pub struct ScanState {
    /// Current position in the bitmap
    local_cursor: u64,
    /// Whether we've wrapped around (to detect exhaustion)
    wrapped: bool,
    /// Starting position (for wrap-around detection)
    start_pos: u64,
    /// Number of items yielded
    count: u64,
}

impl StrategyState for ScanState {}

impl<B: BitmapOps> IterationStrategy<B> for ScanStrategy {
    type State = ScanState;

    fn init_state(&self, _max_index: u64, _seed: u64) -> ScanState {
        let start = self
            .shared_cursor
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0);

        ScanState {
            local_cursor: start,
            wrapped: false,
            start_pos: start,
            count: 0,
        }
    }

    fn next_index(
        &self,
        state: &mut ScanState,
        bitmap: &B,
        filter: FilterCondition,
        max_index: u64,
    ) -> Option<u64> {
        if max_index == 0 {
            return None;
        }

        // Get current cursor position
        let cursor = if let Some(shared) = &self.shared_cursor {
            shared.load(Ordering::Relaxed)
        } else {
            state.local_cursor
        };

        let start = cursor;
        let mut pos = start;

        loop {
            // Try to find next matching index
            let found = match filter {
                FilterCondition::None => {
                    if pos < max_index {
                        Some(pos as usize)
                    } else {
                        None
                    }
                }
                FilterCondition::Set => bitmap.find_next_set(pos as usize),
                FilterCondition::Unset => bitmap.find_next_unset(pos as usize, max_index as usize),
            };

            match found {
                Some(idx) if (idx as u64) < max_index => {
                    let idx = idx as u64;

                    // Update cursor
                    let next_pos = idx + 1;
                    if let Some(shared) = &self.shared_cursor {
                        // Try to claim this position atomically
                        match shared.compare_exchange_weak(
                            pos,
                            next_pos,
                            Ordering::AcqRel,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => {
                                state.local_cursor = next_pos;
                                state.count += 1;
                                return Some(idx);
                            }
                            Err(actual) => {
                                // Another thread moved the cursor, retry from new position
                                pos = actual;
                                continue;
                            }
                        }
                    } else {
                        state.local_cursor = next_pos;
                        state.count += 1;
                        return Some(idx);
                    }
                }
                _ => {
                    // No more matches found
                    if self.wrap_around && !state.wrapped {
                        // Wrap around to start
                        state.wrapped = true;
                        pos = 0;
                        if let Some(shared) = &self.shared_cursor {
                            shared.store(0, Ordering::Relaxed);
                        }
                        state.local_cursor = 0;

                        // If we started at 0, we're done
                        if start == 0 {
                            return None;
                        }
                        continue;
                    }
                    return None;
                }
            }
        }
    }

    fn benefits_from_scan(&self) -> bool {
        true
    }

    fn is_ordered(&self) -> bool {
        true
    }

    fn reset(&self, state: &mut ScanState, _seed: u64) {
        state.local_cursor = 0;
        state.wrapped = false;
        state.start_pos = 0;
        state.count = 0;
        if let Some(shared) = &self.shared_cursor {
            shared.store(0, Ordering::Relaxed);
        }
    }
}

// =============================================================================
// SampleStrategy - Distribution-based sampling
// =============================================================================

/// Strategy that samples indices from a statistical distribution.
///
/// On filter mismatch, it retries up to `max_retries` times before giving up.
/// This is efficient when the filter density is reasonable (>10%).
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{SampleStrategy, Dist};
///
/// // Uniform random sampling
/// let strategy = SampleStrategy::new(Dist::Uniform);
///
/// // Zipfian with custom retry limit
/// let strategy = SampleStrategy::new(Dist::Zipfian { skew: 0.99 })
///     .with_max_retries(50);
/// ```
#[derive(Clone, Debug)]
pub struct SampleStrategy {
    /// The distribution to sample from
    distribution: Dist,
    /// Maximum retries on filter mismatch
    max_retries: u32,
}

impl SampleStrategy {
    /// Create a new sample strategy with the given distribution.
    pub fn new(distribution: Dist) -> Self {
        Self {
            distribution,
            max_retries: 100,
        }
    }

    /// Set the maximum number of retries on filter mismatch.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Get the distribution.
    pub fn distribution(&self) -> &Dist {
        &self.distribution
    }
}

/// State for sample strategy.
#[derive(Debug)]
pub struct SampleState {
    /// Distribution state for sampling
    dist_state: DistState,
    /// Number of samples taken
    count: u64,
    /// Total retries across all samples
    total_retries: u64,
}

impl StrategyState for SampleState {}

impl<B: BitmapOps> IterationStrategy<B> for SampleStrategy {
    type State = SampleState;

    fn init_state(&self, max_index: u64, seed: u64) -> SampleState {
        SampleState {
            dist_state: DistState::new(self.distribution.clone(), max_index, seed),
            count: 0,
            total_retries: 0,
        }
    }

    fn next_index(
        &self,
        state: &mut SampleState,
        bitmap: &B,
        filter: FilterCondition,
        _max_index: u64,
    ) -> Option<u64> {
        for retry in 0..self.max_retries {
            let idx = state.dist_state.sample();

            let matches = match filter {
                FilterCondition::None => true,
                FilterCondition::Set => bitmap.test(idx as usize),
                FilterCondition::Unset => !bitmap.test(idx as usize),
            };

            if matches {
                state.count += 1;
                state.total_retries += retry as u64;
                return Some(idx);
            }
        }

        // Max retries exceeded
        state.total_retries += self.max_retries as u64;
        None
    }

    fn benefits_from_scan(&self) -> bool {
        false
    }

    fn is_ordered(&self) -> bool {
        self.distribution.is_ordered()
    }

    fn reset(&self, state: &mut SampleState, seed: u64) {
        state.dist_state.reset(seed);
        state.count = 0;
        state.total_retries = 0;
    }
}

// =============================================================================
// AdaptiveStrategy - Auto-selects scan or sample
// =============================================================================

/// Strategy that automatically selects between scan and sample based on density.
///
/// - **High density (>threshold)**: Uses sample strategy (faster random access)
/// - **Low density (<threshold)**: Uses scan strategy (efficient SIMD scanning)
///
/// This is useful when the bitmap density is unknown or varies over time.
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{AdaptiveStrategy, Dist};
///
/// // Default threshold (0.1 = 10%)
/// let strategy = AdaptiveStrategy::new(Dist::Uniform);
///
/// // Custom threshold
/// let strategy = AdaptiveStrategy::new(Dist::Zipfian { skew: 0.99 })
///     .with_density_threshold(0.2);
/// ```
#[derive(Clone, Debug)]
pub struct AdaptiveStrategy {
    /// Distribution for sample mode
    distribution: Dist,
    /// Density threshold for switching modes
    density_threshold: f64,
    /// Maximum retries for sample mode
    max_retries: u32,
}

impl AdaptiveStrategy {
    /// Create a new adaptive strategy with the given distribution.
    pub fn new(distribution: Dist) -> Self {
        Self {
            distribution,
            density_threshold: 0.1, // 10% default
            max_retries: 100,
        }
    }

    /// Set the density threshold for switching modes.
    ///
    /// - Below threshold: use scan
    /// - Above threshold: use sample
    pub fn with_density_threshold(mut self, threshold: f64) -> Self {
        self.density_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Set the maximum retries for sample mode.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }
}

/// State for adaptive strategy.
#[derive(Debug)]
pub struct AdaptiveState {
    /// Scan state
    scan_cursor: u64,
    /// Sample distribution state
    dist_state: DistState,
    /// Current mode (true = sample, false = scan)
    use_sample: bool,
    /// Number of items yielded
    count: u64,
}

impl StrategyState for AdaptiveState {}

impl<B: BitmapOps> IterationStrategy<B> for AdaptiveStrategy {
    type State = AdaptiveState;

    fn init_state(&self, max_index: u64, seed: u64) -> AdaptiveState {
        AdaptiveState {
            scan_cursor: 0,
            dist_state: DistState::new(self.distribution.clone(), max_index, seed),
            use_sample: false, // Will be determined on first call
            count: 0,
        }
    }

    fn next_index(
        &self,
        state: &mut AdaptiveState,
        bitmap: &B,
        filter: FilterCondition,
        max_index: u64,
    ) -> Option<u64> {
        // Determine mode based on current density
        let density = bitmap.density();
        let effective_density = match filter {
            FilterCondition::None => 1.0,
            FilterCondition::Set => density,
            FilterCondition::Unset => 1.0 - density,
        };

        state.use_sample =
            effective_density > self.density_threshold && !self.distribution.is_ordered();

        if state.use_sample {
            // Sample mode
            for _ in 0..self.max_retries {
                let idx = state.dist_state.sample();

                let matches = match filter {
                    FilterCondition::None => true,
                    FilterCondition::Set => bitmap.test(idx as usize),
                    FilterCondition::Unset => !bitmap.test(idx as usize),
                };

                if matches {
                    state.count += 1;
                    return Some(idx);
                }
            }
            None
        } else {
            // Scan mode
            let found = match filter {
                FilterCondition::None => {
                    if state.scan_cursor < max_index {
                        Some(state.scan_cursor as usize)
                    } else {
                        None
                    }
                }
                FilterCondition::Set => bitmap.find_next_set(state.scan_cursor as usize),
                FilterCondition::Unset => {
                    bitmap.find_next_unset(state.scan_cursor as usize, max_index as usize)
                }
            };

            match found {
                Some(idx) if (idx as u64) < max_index => {
                    state.scan_cursor = idx as u64 + 1;
                    state.count += 1;
                    Some(idx as u64)
                }
                _ => None,
            }
        }
    }

    fn benefits_from_scan(&self) -> bool {
        true // Can use scan in low-density mode
    }

    fn is_ordered(&self) -> bool {
        false // Switches modes so not strictly ordered
    }

    fn reset(&self, state: &mut AdaptiveState, seed: u64) {
        state.scan_cursor = 0;
        state.dist_state.reset(seed);
        state.use_sample = false;
        state.count = 0;
    }
}

// =============================================================================
// SequentialStrategy - Simple sequential iteration
// =============================================================================

/// Simple sequential strategy without bitmap dependency.
///
/// This is the most basic strategy: iterates 0, 1, 2, ... up to limit.
/// Useful when no filtering is needed.
#[derive(Clone, Debug, Default)]
pub struct SequentialStrategy {
    wrap_around: bool,
}

impl SequentialStrategy {
    /// Create a new sequential strategy.
    pub fn new() -> Self {
        Self { wrap_around: false }
    }

    /// Enable wrap-around mode.
    pub fn wrap_around(mut self) -> Self {
        self.wrap_around = true;
        self
    }
}

/// State for sequential strategy.
#[derive(Debug)]
pub struct SequentialState {
    position: u64,
    wrapped: bool,
}

impl StrategyState for SequentialState {}

impl<B: BitmapOps> IterationStrategy<B> for SequentialStrategy {
    type State = SequentialState;

    fn init_state(&self, _max_index: u64, _seed: u64) -> SequentialState {
        SequentialState {
            position: 0,
            wrapped: false,
        }
    }

    fn next_index(
        &self,
        state: &mut SequentialState,
        bitmap: &B,
        filter: FilterCondition,
        max_index: u64,
    ) -> Option<u64> {
        loop {
            if state.position >= max_index {
                if self.wrap_around && !state.wrapped {
                    state.position = 0;
                    state.wrapped = true;
                    continue;
                }
                return None;
            }

            let pos = state.position;
            state.position += 1;

            let matches = match filter {
                FilterCondition::None => true,
                FilterCondition::Set => bitmap.test(pos as usize),
                FilterCondition::Unset => !bitmap.test(pos as usize),
            };

            if matches {
                return Some(pos);
            }
        }
    }

    fn benefits_from_scan(&self) -> bool {
        true
    }

    fn is_ordered(&self) -> bool {
        true
    }

    fn reset(&self, state: &mut SequentialState, _seed: u64) {
        state.position = 0;
        state.wrapped = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::NoBitmap;

    #[test]
    fn test_sequential_strategy() {
        let strategy = SequentialStrategy::new();
        let bitmap = NoBitmap;
        let mut state =
            <SequentialStrategy as IterationStrategy<NoBitmap>>::init_state(&strategy, 10, 0);

        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10),
            Some(0)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10),
            Some(1)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10),
            Some(2)
        );

        // Exhaust
        for _ in 3..10 {
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10);
        }
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10),
            None
        );
    }

    #[test]
    fn test_sequential_wrap_around() {
        let strategy = SequentialStrategy::new().wrap_around();
        let bitmap = NoBitmap;
        let mut state =
            <SequentialStrategy as IterationStrategy<NoBitmap>>::init_state(&strategy, 3, 0);

        // First cycle
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 3),
            Some(0)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 3),
            Some(1)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 3),
            Some(2)
        );

        // Wrap around
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 3),
            Some(0)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 3),
            Some(1)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 3),
            Some(2)
        );

        // Second wrap not allowed
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 3),
            None
        );
    }

    #[test]
    fn test_sample_strategy() {
        let strategy = SampleStrategy::new(Dist::Sequential);
        let bitmap = NoBitmap;
        let mut state =
            <SampleStrategy as IterationStrategy<NoBitmap>>::init_state(&strategy, 10, 42);

        // Sequential distribution should give 0, 1, 2, ...
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10),
            Some(0)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10),
            Some(1)
        );
        assert_eq!(
            strategy.next_index(&mut state, &bitmap, FilterCondition::None, 10),
            Some(2)
        );
    }

    #[test]
    fn test_sample_strategy_uniform() {
        let strategy = SampleStrategy::new(Dist::Uniform);
        let bitmap = NoBitmap;
        let mut state =
            <SampleStrategy as IterationStrategy<NoBitmap>>::init_state(&strategy, 1000, 42);

        // Should return valid indices
        for _ in 0..100 {
            let idx = strategy
                .next_index(&mut state, &bitmap, FilterCondition::None, 1000)
                .unwrap();
            assert!(idx < 1000);
        }
    }

    #[test]
    fn test_scan_strategy_is_ordered() {
        let strategy = ScanStrategy::new();
        assert!(<ScanStrategy as IterationStrategy<NoBitmap>>::is_ordered(&strategy));
    }

    #[test]
    fn test_sample_strategy_ordering() {
        let seq = SampleStrategy::new(Dist::Sequential);
        let uniform = SampleStrategy::new(Dist::Uniform);
        let zipf = SampleStrategy::new(Dist::Zipfian { skew: 0.99 });

        assert!(<SampleStrategy as IterationStrategy<NoBitmap>>::is_ordered(&seq));
        assert!(!<SampleStrategy as IterationStrategy<NoBitmap>>::is_ordered(&uniform));
        assert!(!<SampleStrategy as IterationStrategy<NoBitmap>>::is_ordered(&zipf));
    }
}
