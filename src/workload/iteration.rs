//! Iteration strategy for workload key generation
//!
//! This module provides flexible iteration patterns for workload key generation:
//! - Sequential: Iterate through keys in order (0, 1, 2, ...)
//! - Random: Deterministic pseudo-random key selection
//! - Subset: Iterate over a range within the keyspace
//! - Zipfian: Hot-key distribution (power law)

use std::sync::atomic::{AtomicU64, Ordering};

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
}
