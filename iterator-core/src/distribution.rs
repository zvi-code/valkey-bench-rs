//! Distribution types for controlling iteration access patterns.
//!
//! The [`Dist`] enum represents all supported access patterns for iteration.
//! Distributions are either ordered (deterministic, visit each element once)
//! or unordered (random sampling with configurable probability).
//!
//! [`DistState`] provides the runtime state for sampling from distributions.
//!
//! The [`Distribution`] trait provides a standardized interface for sampling,
//! enabling trait-based polymorphism and reducing code duplication.

use fastrand::Rng;
use num_integer::gcd;

/// Trait for distribution sampling.
///
/// This trait standardizes the sampling interface across all distribution types,
/// enabling trait-based polymorphism and cleaner code organization.
///
/// # Examples
///
/// ```rust
/// use iterator_core::{Distribution, SequentialDist};
/// use fastrand::Rng;
///
/// let mut dist = SequentialDist::new(10);
/// let mut rng = Rng::with_seed(42);
///
/// assert_eq!(dist.sample(&mut rng), 0);
/// assert_eq!(dist.sample(&mut rng), 1);
/// assert_eq!(dist.sample(&mut rng), 2);
///
/// dist.reset();
/// assert_eq!(dist.sample(&mut rng), 0);
/// ```
pub trait Distribution {
    /// Sample the next value from the distribution.
    ///
    /// # Arguments
    ///
    /// * `rng` - Random number generator for stochastic distributions
    ///
    /// # Returns
    ///
    /// An index in range [0, range_size).
    fn sample(&mut self, rng: &mut Rng) -> u64;

    /// Reset the distribution to its initial state.
    ///
    /// For ordered distributions, this resets the position to 0.
    /// For unordered distributions, this may be a no-op.
    fn reset(&mut self);

    /// Returns true if the distribution visits elements in deterministic order.
    ///
    /// Ordered distributions visit each element exactly once when iterated
    /// to exhaustion and have natural termination.
    fn is_ordered(&self) -> bool;

    /// Check if ordered iteration completed one full cycle.
    ///
    /// Only meaningful for ordered distributions. Returns false for
    /// unordered distributions (they never exhaust).
    fn is_exhausted(&self) -> bool;

    /// Get the range size for this distribution.
    fn range_size(&self) -> u64;
}

/// Distribution enum representing all supported access patterns.
///
/// Distributions fall into two categories:
/// - **Ordered**: Visit elements in deterministic order, natural termination
/// - **Unordered**: Random sampling, requires explicit iteration limit
///
/// # Composition
///
/// Distributions can be composed using [`Dist::Permute`] and [`Dist::Reverse`]
/// wrappers to create new access patterns while preserving ordering properties.
///
/// # Examples
///
/// ```rust
/// use iterator_core::Dist;
///
/// // Ordered distributions
/// let seq = Dist::Sequential;
/// let semi = Dist::SemiSequential { multiplier: 7 };
///
/// assert!(seq.is_ordered());
/// assert!(semi.is_ordered());
///
/// // Unordered distributions
/// let uniform = Dist::Uniform;
/// let zipf = Dist::Zipfian { skew: 0.99 };
///
/// assert!(!uniform.is_ordered());
/// assert!(!zipf.is_ordered());
///
/// // Composition preserves ordering
/// let permuted = Dist::Permute {
///     inner: Box::new(Dist::Sequential),
///     multiplier: 31,
/// };
/// assert!(permuted.is_ordered());
/// ```
#[derive(Clone, Debug)]
pub enum Dist {
    /// Sequential: 0, 1, 2, ..., N-1 (ordered)
    ///
    /// Visits each element exactly once in ascending order.
    Sequential,

    /// SemiSequential: permuted sequential (ordered)
    ///
    /// Implemented as Sequential composed with Permute.
    /// Scatters sequential access using multiplication modulo range size.
    /// The multiplier should be coprime with the range size for full coverage.
    SemiSequential {
        /// Multiplier for permutation. Should be coprime with range size.
        multiplier: u64,
    },

    /// Uniform random (unordered)
    ///
    /// Each element has equal probability of being sampled.
    Uniform,

    /// Zipfian distribution: P(k) ∝ 1/k^skew (unordered)
    ///
    /// Models popularity distributions where a few items are very popular.
    /// Higher skew values concentrate probability on lower indices.
    Zipfian {
        /// Skew parameter. Common values: 0.99 (moderate), 1.2 (high)
        skew: f64,
    },

    /// Exponential decay: P(k) ∝ e^(-λk) (unordered)
    ///
    /// Probability decreases exponentially with index.
    Exponential {
        /// Decay rate. Higher values concentrate probability on lower indices.
        lambda: f64,
    },

    /// Normal/Gaussian distribution (unordered)
    ///
    /// Samples centered around a mean with configurable spread.
    Normal {
        /// Mean as percentage of range (0.0 to 1.0)
        mean_pct: f64,
        /// Standard deviation as percentage of range
        std_pct: f64,
    },

    /// Hotspot distribution (unordered)
    ///
    /// A fraction of keys receive a disproportionate fraction of accesses.
    Hotspot {
        /// Fraction of keys that are "hot" (0.0 to 1.0)
        hot_pct: f64,
        /// Probability of accessing a hot key (0.0 to 1.0)
        hot_prob: f64,
    },

    /// Latest-N bias (unordered)
    ///
    /// Recent keys (high indices) receive more accesses.
    Latest {
        /// Fraction of keys considered "recent" (0.0 to 1.0)
        recent_pct: f64,
        /// Probability of accessing a recent key (0.0 to 1.0)
        recent_prob: f64,
    },

    /// Permutation wrapper - bijective scatter
    ///
    /// Applies `(inner_sample * multiplier) % range_size` transformation.
    /// Preserves ordering property of inner distribution.
    Permute {
        /// Inner distribution to transform
        inner: Box<Dist>,
        /// Multiplier for permutation. Should be coprime with range size.
        multiplier: u64,
    },

    /// Reverse wrapper
    ///
    /// Applies `range_size - 1 - inner_sample` transformation.
    /// Preserves ordering property of inner distribution.
    Reverse {
        /// Inner distribution to reverse
        inner: Box<Dist>,
    },
}

impl Dist {
    /// Returns true if distribution visits elements in deterministic order.
    ///
    /// Ordered distributions:
    /// - Visit each element exactly once (when iterated to exhaustion)
    /// - Have natural termination after visiting all elements
    /// - Include: Sequential, SemiSequential, and any Permute/Reverse of ordered
    ///
    /// Unordered distributions:
    /// - Sample randomly without deterministic progression
    /// - Require explicit iteration limit
    /// - Include: Uniform, Zipfian, Exponential, Normal, Hotspot, Latest
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::Dist;
    ///
    /// assert!(Dist::Sequential.is_ordered());
    /// assert!(Dist::SemiSequential { multiplier: 7 }.is_ordered());
    /// assert!(!Dist::Uniform.is_ordered());
    /// assert!(!Dist::Zipfian { skew: 0.99 }.is_ordered());
    ///
    /// // Wrappers preserve ordering
    /// let permuted = Dist::Permute {
    ///     inner: Box::new(Dist::Sequential),
    ///     multiplier: 31,
    /// };
    /// assert!(permuted.is_ordered());
    ///
    /// let reversed_uniform = Dist::Reverse {
    ///     inner: Box::new(Dist::Uniform),
    /// };
    /// assert!(!reversed_uniform.is_ordered());
    /// ```
    pub fn is_ordered(&self) -> bool {
        match self {
            Dist::Sequential | Dist::SemiSequential { .. } => true,
            Dist::Permute { inner, .. } | Dist::Reverse { inner } => inner.is_ordered(),
            _ => false,
        }
    }

    /// Create a SemiSequential distribution with an automatically chosen coprime multiplier.
    ///
    /// The multiplier is chosen to be coprime with the range size, ensuring that
    /// the permutation is bijective and covers all elements exactly once.
    ///
    /// This is equivalent to `Permute { inner: Sequential, multiplier }` where
    /// the multiplier is guaranteed to be coprime with `range_size`.
    ///
    /// # Arguments
    ///
    /// * `range_size` - The size of the range to iterate over
    ///
    /// # Returns
    ///
    /// A `SemiSequential` distribution with a coprime multiplier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::Dist;
    ///
    /// let dist = Dist::semi_sequential(100);
    /// assert!(dist.is_ordered());
    ///
    /// // The multiplier is automatically chosen to be coprime with 100
    /// if let Dist::SemiSequential { multiplier } = dist {
    ///     // GCD(multiplier, 100) == 1
    ///     assert!(gcd(multiplier, 100) == 1);
    /// }
    ///
    /// fn gcd(a: u64, b: u64) -> u64 {
    ///     if b == 0 { a } else { gcd(b, a % b) }
    /// }
    /// ```
    pub fn semi_sequential(range_size: u64) -> Self {
        let multiplier = Self::find_coprime_multiplier(range_size);
        Dist::SemiSequential { multiplier }
    }

    /// Create a Permute distribution wrapping an inner distribution with an
    /// automatically chosen coprime multiplier.
    ///
    /// # Arguments
    ///
    /// * `inner` - The inner distribution to wrap
    /// * `range_size` - The size of the range (needed to find coprime multiplier)
    ///
    /// # Returns
    ///
    /// A `Permute` distribution with a coprime multiplier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::Dist;
    ///
    /// let dist = Dist::permute_coprime(Dist::Sequential, 100);
    /// assert!(dist.is_ordered());
    /// ```
    pub fn permute_coprime(inner: Dist, range_size: u64) -> Self {
        let multiplier = Self::find_coprime_multiplier(range_size);
        Dist::Permute {
            inner: Box::new(inner),
            multiplier,
        }
    }

    /// Find a multiplier that is coprime with the given range size.
    ///
    /// Uses a simple heuristic: tries small primes and odd numbers until
    /// finding one that is coprime with range_size.
    ///
    /// # Arguments
    ///
    /// * `range_size` - The range size to find a coprime for
    ///
    /// # Returns
    ///
    /// A value that is coprime with `range_size`.
    fn find_coprime_multiplier(range_size: u64) -> u64 {
        if range_size <= 1 {
            return 1;
        }

        // Try some good multipliers that scatter well
        // These are chosen to be likely coprime with common range sizes
        const CANDIDATES: [u64; 10] = [
            2654435761, // Golden ratio prime (good for hashing)
            1103515245, // LCG multiplier
            6364136223846793005, // PCG multiplier
            31,         // Small prime
            37,         // Small prime
            41,         // Small prime
            43,         // Small prime
            47,         // Small prime
            53,         // Small prime
            59,         // Small prime
        ];

        for &candidate in &CANDIDATES {
            if gcd(candidate, range_size) == 1 {
                return candidate;
            }
        }

        // Fallback: find the smallest odd number >= 3 that is coprime
        let mut m = 3u64;
        while gcd(m, range_size) != 1 {
            m += 2;
            if m >= range_size {
                m = 1; // Fallback to 1 (identity permutation)
                break;
            }
        }
        m
    }
}

// =============================================================================
// Distribution Trait Implementations
// =============================================================================

/// Sequential distribution: 0, 1, 2, ..., N-1 (ordered)
///
/// Visits each element exactly once in ascending order.
#[derive(Clone, Debug)]
pub struct SequentialDist {
    range_size: u64,
    position: u64,
}

impl SequentialDist {
    /// Create a new sequential distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    pub fn new(range_size: u64) -> Self {
        Self {
            range_size,
            position: 0,
        }
    }
}

impl Distribution for SequentialDist {
    fn sample(&mut self, _rng: &mut Rng) -> u64 {
        let idx = self.position % self.range_size;
        self.position += 1;
        idx
    }

    fn reset(&mut self) {
        self.position = 0;
    }

    fn is_ordered(&self) -> bool {
        true
    }

    fn is_exhausted(&self) -> bool {
        self.position >= self.range_size
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// Uniform random distribution (unordered)
///
/// Each element has equal probability of being sampled.
#[derive(Clone, Debug)]
pub struct UniformDist {
    range_size: u64,
}

impl UniformDist {
    /// Create a new uniform distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    pub fn new(range_size: u64) -> Self {
        Self { range_size }
    }
}

impl Distribution for UniformDist {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        rng.u64(0..self.range_size)
    }

    fn reset(&mut self) {
        // No-op for uniform distribution
    }

    fn is_ordered(&self) -> bool {
        false
    }

    fn is_exhausted(&self) -> bool {
        false // Unordered distributions never exhaust
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// Zipfian distribution: P(k) ∝ 1/k^skew (unordered)
///
/// Models popularity distributions where a few items are very popular.
/// Higher skew values concentrate probability on lower indices.
#[derive(Clone, Debug)]
pub struct ZipfianDist {
    range_size: u64,
    skew: f64,
    /// Precomputed CDF table for inverse transform sampling
    cdf: Vec<f64>,
}

impl ZipfianDist {
    /// Maximum size for Zipfian CDF table
    const TABLE_SIZE: usize = 1024;

    /// Create a new Zipfian distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    /// * `skew` - Skew parameter. Common values: 0.99 (moderate), 1.2 (high)
    pub fn new(range_size: u64, skew: f64) -> Self {
        let cdf = Self::precompute_cdf(range_size, skew);
        Self {
            range_size,
            skew,
            cdf,
        }
    }

    /// Precompute CDF for inverse transform sampling
    fn precompute_cdf(range_size: u64, skew: f64) -> Vec<f64> {
        let table_size = (range_size as usize).min(Self::TABLE_SIZE);
        let mut cdf = Vec::with_capacity(table_size);

        // Compute normalization constant (zeta)
        let mut zeta = 0.0f64;
        for k in 1..=range_size {
            zeta += 1.0 / (k as f64).powf(skew);
        }

        // Compute CDF
        let mut cumulative = 0.0f64;
        for k in 1..=table_size as u64 {
            cumulative += 1.0 / ((k as f64).powf(skew) * zeta);
            cdf.push(cumulative);
        }

        // Ensure last entry is 1.0 (handle floating point errors)
        if let Some(last) = cdf.last_mut() {
            *last = 1.0;
        }

        cdf
    }

    /// Sample from Zipfian tail (for large ranges beyond CDF table)
    fn sample_tail(&self, rng: &mut Rng) -> u64 {
        let table_size = Self::TABLE_SIZE as u64;
        if self.range_size <= table_size {
            return (self.range_size - 1).min(table_size - 1);
        }

        // For the tail, use uniform sampling in the remaining range
        let tail_size = self.range_size - table_size;
        let tail_idx = rng.u64(0..tail_size);
        table_size + tail_idx
    }
}

impl Distribution for ZipfianDist {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        let u = rng.f64();

        // Binary search for the index where CDF >= u
        let idx = self.cdf.partition_point(|&c| c < u);

        // If range_size > table size, we need to handle the tail
        if idx >= self.cdf.len() && self.range_size > self.cdf.len() as u64 {
            return self.sample_tail(rng);
        }

        idx as u64
    }

    fn reset(&mut self) {
        // No-op for Zipfian distribution
    }

    fn is_ordered(&self) -> bool {
        false
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// Latest-N bias distribution (unordered)
///
/// Recent keys (high indices) receive more accesses.
#[derive(Clone, Debug)]
pub struct LatestDist {
    range_size: u64,
    /// Fraction of keys considered "recent" (0.0 to 1.0)
    recent_pct: f64,
    /// Probability of accessing a recent key (0.0 to 1.0)
    recent_prob: f64,
}

impl LatestDist {
    /// Create a new Latest distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    /// * `recent_pct` - Fraction of keys considered "recent" (0.0 to 1.0)
    /// * `recent_prob` - Probability of accessing a recent key (0.0 to 1.0)
    pub fn new(range_size: u64, recent_pct: f64, recent_prob: f64) -> Self {
        Self {
            range_size,
            recent_pct,
            recent_prob,
        }
    }
}

impl Distribution for LatestDist {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        let recent_count = ((self.recent_pct * self.range_size as f64).ceil() as u64).max(1);
        let old_count = self.range_size.saturating_sub(recent_count);

        let u = rng.f64();

        if u < self.recent_prob && recent_count > 0 {
            // Sample from recent keys (last recent_count indices)
            let recent_start = self.range_size - recent_count;
            recent_start + rng.u64(0..recent_count)
        } else if old_count > 0 {
            // Sample from old keys (first old_count indices)
            rng.u64(0..old_count)
        } else {
            // All keys are recent
            rng.u64(0..self.range_size)
        }
    }

    fn reset(&mut self) {
        // No-op for Latest distribution
    }

    fn is_ordered(&self) -> bool {
        false
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// Exponential decay distribution: P(k) ∝ e^(-λk) (unordered)
///
/// Probability decreases exponentially with index.
#[derive(Clone, Debug)]
pub struct ExponentialDist {
    range_size: u64,
    /// Decay rate. Higher values concentrate probability on lower indices.
    lambda: f64,
}

impl ExponentialDist {
    /// Create a new Exponential distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    /// * `lambda` - Decay rate
    pub fn new(range_size: u64, lambda: f64) -> Self {
        Self { range_size, lambda }
    }
}

impl Distribution for ExponentialDist {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        let u = rng.f64();
        // Avoid log(0) by clamping u away from 1
        let u_clamped = u.min(1.0 - f64::EPSILON);
        let k = (-((1.0 - u_clamped).ln()) / self.lambda).floor() as u64;
        k.min(self.range_size - 1)
    }

    fn reset(&mut self) {
        // No-op
    }

    fn is_ordered(&self) -> bool {
        false
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// Normal/Gaussian distribution (unordered)
///
/// Samples centered around a mean with configurable spread.
#[derive(Clone, Debug)]
pub struct NormalDist {
    range_size: u64,
    /// Mean as percentage of range (0.0 to 1.0)
    mean_pct: f64,
    /// Standard deviation as percentage of range
    std_pct: f64,
}

impl NormalDist {
    /// Create a new Normal distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    /// * `mean_pct` - Mean as percentage of range (0.0 to 1.0)
    /// * `std_pct` - Standard deviation as percentage of range
    pub fn new(range_size: u64, mean_pct: f64, std_pct: f64) -> Self {
        Self {
            range_size,
            mean_pct,
            std_pct,
        }
    }
}

impl Distribution for NormalDist {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        // Box-Muller transform to generate normal samples
        let u1 = rng.f64().max(f64::EPSILON); // Avoid log(0)
        let u2 = rng.f64();

        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

        // Convert to range index
        let mean = self.mean_pct * self.range_size as f64;
        let std = self.std_pct * self.range_size as f64;
        let sample = mean + z * std;

        // Clamp to valid range
        let clamped = sample.round().max(0.0).min((self.range_size - 1) as f64);
        clamped as u64
    }

    fn reset(&mut self) {
        // No-op
    }

    fn is_ordered(&self) -> bool {
        false
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// Hotspot distribution (unordered)
///
/// A fraction of keys receive a disproportionate fraction of accesses.
#[derive(Clone, Debug)]
pub struct HotspotDist {
    range_size: u64,
    /// Fraction of keys that are "hot" (0.0 to 1.0)
    hot_pct: f64,
    /// Probability of accessing a hot key (0.0 to 1.0)
    hot_prob: f64,
}

impl HotspotDist {
    /// Create a new Hotspot distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    /// * `hot_pct` - Fraction of keys that are "hot" (0.0 to 1.0)
    /// * `hot_prob` - Probability of accessing a hot key (0.0 to 1.0)
    pub fn new(range_size: u64, hot_pct: f64, hot_prob: f64) -> Self {
        Self {
            range_size,
            hot_pct,
            hot_prob,
        }
    }
}

impl Distribution for HotspotDist {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        let hot_count = ((self.hot_pct * self.range_size as f64).ceil() as u64).max(1);
        let cold_count = self.range_size.saturating_sub(hot_count);

        let u = rng.f64();

        if u < self.hot_prob && hot_count > 0 {
            // Sample from hot keys (first hot_count indices)
            rng.u64(0..hot_count)
        } else if cold_count > 0 {
            // Sample from cold keys (remaining indices)
            hot_count + rng.u64(0..cold_count)
        } else {
            // All keys are hot
            rng.u64(0..self.range_size)
        }
    }

    fn reset(&mut self) {
        // No-op
    }

    fn is_ordered(&self) -> bool {
        false
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// SemiSequential distribution: permuted sequential (ordered)
///
/// Scatters sequential access using multiplication modulo range size.
#[derive(Clone, Debug)]
pub struct SemiSequentialDist {
    range_size: u64,
    multiplier: u64,
    position: u64,
}

impl SemiSequentialDist {
    /// Create a new SemiSequential distribution.
    ///
    /// # Arguments
    ///
    /// * `range_size` - Size of the range [0, range_size)
    /// * `multiplier` - Multiplier for permutation. Should be coprime with range size.
    pub fn new(range_size: u64, multiplier: u64) -> Self {
        Self {
            range_size,
            multiplier,
            position: 0,
        }
    }

    /// Create with an automatically chosen coprime multiplier.
    pub fn with_coprime(range_size: u64) -> Self {
        let multiplier = Dist::find_coprime_multiplier(range_size);
        Self::new(range_size, multiplier)
    }
}

impl Distribution for SemiSequentialDist {
    fn sample(&mut self, _rng: &mut Rng) -> u64 {
        let seq_idx = self.position % self.range_size;
        self.position += 1;
        (seq_idx.wrapping_mul(self.multiplier)) % self.range_size
    }

    fn reset(&mut self) {
        self.position = 0;
    }

    fn is_ordered(&self) -> bool {
        true
    }

    fn is_exhausted(&self) -> bool {
        self.position >= self.range_size
    }

    fn range_size(&self) -> u64 {
        self.range_size
    }
}

/// Permutation wrapper - bijective scatter
///
/// Applies `(inner_sample * multiplier) % range_size` transformation.
/// Preserves ordering property of inner distribution.
#[derive(Clone, Debug)]
pub struct PermuteDist<D: Distribution> {
    inner: D,
    multiplier: u64,
}

impl<D: Distribution> PermuteDist<D> {
    /// Create a new Permute distribution wrapping an inner distribution.
    ///
    /// # Arguments
    ///
    /// * `inner` - The inner distribution to transform
    /// * `multiplier` - Multiplier for permutation. Should be coprime with range size.
    pub fn new(inner: D, multiplier: u64) -> Self {
        Self { inner, multiplier }
    }

    /// Create with an automatically chosen coprime multiplier.
    pub fn with_coprime(inner: D) -> Self {
        let multiplier = Dist::find_coprime_multiplier(inner.range_size());
        Self::new(inner, multiplier)
    }

    /// Get a reference to the inner distribution.
    pub fn inner(&self) -> &D {
        &self.inner
    }

    /// Get a mutable reference to the inner distribution.
    pub fn inner_mut(&mut self) -> &mut D {
        &mut self.inner
    }
}

impl<D: Distribution> Distribution for PermuteDist<D> {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        let inner_sample = self.inner.sample(rng);
        let range_size = self.inner.range_size();
        (inner_sample.wrapping_mul(self.multiplier)) % range_size
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    fn is_ordered(&self) -> bool {
        self.inner.is_ordered()
    }

    fn is_exhausted(&self) -> bool {
        self.inner.is_exhausted()
    }

    fn range_size(&self) -> u64 {
        self.inner.range_size()
    }
}

/// Reverse wrapper
///
/// Applies `range_size - 1 - inner_sample` transformation.
/// Preserves ordering property of inner distribution.
#[derive(Clone, Debug)]
pub struct ReverseDist<D: Distribution> {
    inner: D,
}

impl<D: Distribution> ReverseDist<D> {
    /// Create a new Reverse distribution wrapping an inner distribution.
    ///
    /// # Arguments
    ///
    /// * `inner` - The inner distribution to reverse
    pub fn new(inner: D) -> Self {
        Self { inner }
    }

    /// Get a reference to the inner distribution.
    pub fn inner(&self) -> &D {
        &self.inner
    }

    /// Get a mutable reference to the inner distribution.
    pub fn inner_mut(&mut self) -> &mut D {
        &mut self.inner
    }
}

impl<D: Distribution> Distribution for ReverseDist<D> {
    fn sample(&mut self, rng: &mut Rng) -> u64 {
        let inner_sample = self.inner.sample(rng);
        let range_size = self.inner.range_size();
        range_size - 1 - inner_sample
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    fn is_ordered(&self) -> bool {
        self.inner.is_ordered()
    }

    fn is_exhausted(&self) -> bool {
        self.inner.is_exhausted()
    }

    fn range_size(&self) -> u64 {
        self.inner.range_size()
    }
}

/// Runtime state for sampling from a distribution.
///
/// Separates the distribution configuration ([`Dist`]) from mutable sampling state.
/// Each `DistState` maintains its own position (for ordered distributions) and
/// RNG state (for unordered distributions).
///
/// # Examples
///
/// ```rust
/// use iterator_core::{Dist, DistState};
///
/// // Sequential distribution
/// let mut state = DistState::new(Dist::Sequential, 10, 12345);
/// assert_eq!(state.sample(), 0);
/// assert_eq!(state.sample(), 1);
/// assert_eq!(state.sample(), 2);
///
/// // Uniform random distribution
/// let mut uniform = DistState::new(Dist::Uniform, 100, 42);
/// let sample = uniform.sample();
/// assert!(sample < 100);
/// ```
#[derive(Clone, Debug)]
pub struct DistState {
    dist: Dist,
    range_size: u64,
    position: u64,
    rng: Rng,
    seed: u64,
    /// Precomputed CDF table for Zipfian distribution
    zipf_cdf: Option<Vec<f64>>,
}

impl DistState {
    /// Maximum size for Zipfian CDF table
    const ZIPF_TABLE_SIZE: usize = 1024;

    /// Create a new distribution state.
    ///
    /// # Arguments
    ///
    /// * `dist` - The distribution configuration
    /// * `range_size` - Size of the range [0, range_size)
    /// * `seed` - Seed for the RNG (used by unordered distributions)
    ///
    /// # Panics
    ///
    /// Panics if `range_size` is 0.
    pub fn new(dist: Dist, range_size: u64, seed: u64) -> Self {
        assert!(range_size > 0, "range_size must be greater than 0");

        // Precompute Zipfian CDF if needed
        let zipf_cdf = if let Dist::Zipfian { skew } = &dist {
            Some(Self::precompute_zipf_cdf(range_size, *skew))
        } else {
            None
        };

        Self {
            dist,
            range_size,
            position: 0,
            rng: Rng::with_seed(seed),
            seed,
            zipf_cdf,
        }
    }

    /// Precompute CDF for Zipfian distribution
    fn precompute_zipf_cdf(range_size: u64, skew: f64) -> Vec<f64> {
        let table_size = (range_size as usize).min(Self::ZIPF_TABLE_SIZE);
        let mut cdf = Vec::with_capacity(table_size);

        // Compute normalization constant (zeta)
        let mut zeta = 0.0f64;
        for k in 1..=range_size {
            zeta += 1.0 / (k as f64).powf(skew);
        }

        // Compute CDF
        let mut cumulative = 0.0f64;
        for k in 1..=table_size as u64 {
            cumulative += 1.0 / ((k as f64).powf(skew) * zeta);
            cdf.push(cumulative);
        }

        // Ensure last entry is 1.0 (handle floating point errors)
        if let Some(last) = cdf.last_mut() {
            *last = 1.0;
        }

        cdf
    }

    /// Sample the next index from the distribution.
    ///
    /// For ordered distributions, wraps around when position >= range_size.
    /// For unordered distributions, samples according to the distribution's
    /// probability function.
    ///
    /// # Returns
    ///
    /// An index in range [0, range_size).
    pub fn sample(&mut self) -> u64 {
        // First, determine what operation we need to perform without holding a mutable borrow
        enum SampleOp {
            Sequential,
            SemiSequential(u64),
            Uniform,
            Zipfian(f64),
            Exponential(f64),
            Normal(f64, f64),
            Hotspot(f64, f64),
            Latest(f64, f64),
            Permute(u64),
            Reverse,
        }

        let op = match &self.dist {
            Dist::Sequential => SampleOp::Sequential,
            Dist::SemiSequential { multiplier } => SampleOp::SemiSequential(*multiplier),
            Dist::Uniform => SampleOp::Uniform,
            Dist::Zipfian { skew } => SampleOp::Zipfian(*skew),
            Dist::Exponential { lambda } => SampleOp::Exponential(*lambda),
            Dist::Normal { mean_pct, std_pct } => SampleOp::Normal(*mean_pct, *std_pct),
            Dist::Hotspot { hot_pct, hot_prob } => SampleOp::Hotspot(*hot_pct, *hot_prob),
            Dist::Latest { recent_pct, recent_prob } => SampleOp::Latest(*recent_pct, *recent_prob),
            Dist::Permute { multiplier, .. } => SampleOp::Permute(*multiplier),
            Dist::Reverse { .. } => SampleOp::Reverse,
        };

        // Now perform the operation - the borrow of self.dist is released
        match op {
            SampleOp::Sequential => self.sample_sequential(),
            SampleOp::SemiSequential(multiplier) => self.sample_semi_sequential(multiplier),
            SampleOp::Uniform => self.sample_uniform(),
            SampleOp::Zipfian(skew) => self.sample_zipfian(skew),
            SampleOp::Exponential(lambda) => self.sample_exponential(lambda),
            SampleOp::Normal(mean_pct, std_pct) => self.sample_normal(mean_pct, std_pct),
            SampleOp::Hotspot(hot_pct, hot_prob) => self.sample_hotspot(hot_pct, hot_prob),
            SampleOp::Latest(recent_pct, recent_prob) => self.sample_latest(recent_pct, recent_prob),
            SampleOp::Permute(multiplier) => self.sample_permute_from_dist(multiplier),
            SampleOp::Reverse => self.sample_reverse_from_dist(),
        }
    }

    /// Check if ordered iteration completed one full cycle.
    ///
    /// Only meaningful for ordered distributions. Returns false for
    /// unordered distributions (they never exhaust).
    pub fn is_exhausted(&self) -> bool {
        if self.dist.is_ordered() {
            self.position >= self.range_size
        } else {
            false
        }
    }

    /// Reset to initial state.
    ///
    /// Resets position to 0 and reinitializes RNG with the given seed.
    /// Preserves precomputed values (like Zipfian CDF).
    pub fn reset(&mut self, seed: u64) {
        self.position = 0;
        self.seed = seed;
        self.rng = Rng::with_seed(seed);
    }

    /// Get the current position (for ordered distributions).
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Get the range size.
    pub fn range_size(&self) -> u64 {
        self.range_size
    }

    /// Get a reference to the distribution.
    pub fn dist(&self) -> &Dist {
        &self.dist
    }

    // --- Private sampling methods ---

    /// Sequential: 0, 1, 2, ..., N-1 with wrap-around
    fn sample_sequential(&mut self) -> u64 {
        let idx = self.position % self.range_size;
        self.position += 1;
        idx
    }

    /// SemiSequential: permuted sequential using multiplication
    fn sample_semi_sequential(&mut self, multiplier: u64) -> u64 {
        let seq_idx = self.position % self.range_size;
        self.position += 1;
        (seq_idx.wrapping_mul(multiplier)) % self.range_size
    }

    /// Uniform: random sampling with equal probability
    fn sample_uniform(&mut self) -> u64 {
        self.rng.u64(0..self.range_size)
    }

    /// Zipfian: P(k) ∝ 1/k^skew using inverse transform sampling
    fn sample_zipfian(&mut self, skew: f64) -> u64 {
        let u = self.rng.f64();

        // Use precomputed CDF table for inverse transform sampling
        if let Some(cdf) = &self.zipf_cdf {
            // Binary search for the index where CDF >= u
            let idx = cdf.partition_point(|&c| c < u);

            // If range_size > table size, we need to handle the tail
            if idx >= cdf.len() && self.range_size > cdf.len() as u64 {
                // For values beyond the table, use approximation
                // The tail probability is small for typical skew values
                return self.sample_zipfian_tail(skew, u);
            }

            return idx as u64;
        }

        // Fallback: direct computation (slower)
        self.sample_zipfian_direct(skew, u)
    }

    /// Sample from Zipfian tail (for large ranges beyond CDF table)
    fn sample_zipfian_tail(&mut self, _skew: f64, _u: f64) -> u64 {
        // Use rejection sampling for the tail
        // This is rare for typical skew values (0.99, 1.2, etc.)
        let table_size = Self::ZIPF_TABLE_SIZE as u64;
        if self.range_size <= table_size {
            return (self.range_size - 1).min(table_size - 1);
        }

        // For the tail, use uniform sampling in the remaining range
        // This is an approximation but acceptable for the rare tail samples
        let tail_size = self.range_size - table_size;
        let tail_idx = self.rng.u64(0..tail_size);
        table_size + tail_idx
    }

    /// Direct Zipfian sampling without precomputed table
    fn sample_zipfian_direct(&mut self, skew: f64, u: f64) -> u64 {
        // Compute CDF on the fly (slower but works for any range)
        let mut zeta = 0.0f64;
        for k in 1..=self.range_size {
            zeta += 1.0 / (k as f64).powf(skew);
        }

        let mut cumulative = 0.0f64;
        for k in 1..=self.range_size {
            cumulative += 1.0 / ((k as f64).powf(skew) * zeta);
            if cumulative >= u {
                return k - 1; // Convert to 0-indexed
            }
        }

        self.range_size - 1
    }

    /// Exponential: P(k) ∝ e^(-λk) using inverse transform sampling
    fn sample_exponential(&mut self, lambda: f64) -> u64 {
        // Inverse CDF: k = -ln(1-u) / λ
        // Clamp to range [0, range_size)
        let u = self.rng.f64();

        // Avoid log(0) by clamping u away from 1
        let u_clamped = u.min(1.0 - f64::EPSILON);

        let k = (-((1.0 - u_clamped).ln()) / lambda).floor() as u64;
        k.min(self.range_size - 1)
    }

    /// Normal: Gaussian distribution using Box-Muller transform
    fn sample_normal(&mut self, mean_pct: f64, std_pct: f64) -> u64 {
        // Box-Muller transform to generate normal samples
        let u1 = self.rng.f64().max(f64::EPSILON); // Avoid log(0)
        let u2 = self.rng.f64();

        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

        // Convert to range index
        let mean = mean_pct * self.range_size as f64;
        let std = std_pct * self.range_size as f64;
        let sample = mean + z * std;

        // Clamp to valid range
        let clamped = sample.round().max(0.0).min((self.range_size - 1) as f64);
        clamped as u64
    }

    /// Hotspot: hot_pct keys get hot_prob of accesses
    fn sample_hotspot(&mut self, hot_pct: f64, hot_prob: f64) -> u64 {
        let hot_count = ((hot_pct * self.range_size as f64).ceil() as u64).max(1);
        let cold_count = self.range_size.saturating_sub(hot_count);

        let u = self.rng.f64();

        if u < hot_prob && hot_count > 0 {
            // Sample from hot keys (first hot_count indices)
            self.rng.u64(0..hot_count)
        } else if cold_count > 0 {
            // Sample from cold keys (remaining indices)
            hot_count + self.rng.u64(0..cold_count)
        } else {
            // All keys are hot
            self.rng.u64(0..self.range_size)
        }
    }

    /// Latest: recent_pct keys (high indices) get recent_prob of accesses
    fn sample_latest(&mut self, recent_pct: f64, recent_prob: f64) -> u64 {
        let recent_count = ((recent_pct * self.range_size as f64).ceil() as u64).max(1);
        let old_count = self.range_size.saturating_sub(recent_count);

        let u = self.rng.f64();

        if u < recent_prob && recent_count > 0 {
            // Sample from recent keys (last recent_count indices)
            let recent_start = self.range_size - recent_count;
            recent_start + self.rng.u64(0..recent_count)
        } else if old_count > 0 {
            // Sample from old keys (first old_count indices)
            self.rng.u64(0..old_count)
        } else {
            // All keys are recent
            self.rng.u64(0..self.range_size)
        }
    }

    /// Permute: apply bijective transformation to inner distribution
    fn sample_permute(&mut self, inner: &Dist, multiplier: u64) -> u64 {
        let inner_sample = self.sample_inner(inner);
        (inner_sample.wrapping_mul(multiplier)) % self.range_size
    }

    /// Permute: sample from self.dist's inner distribution (avoids cloning)
    fn sample_permute_from_dist(&mut self, multiplier: u64) -> u64 {
        // Get a raw pointer to the inner distribution to avoid borrow conflicts
        // SAFETY: We only read from the inner distribution, and self.dist is not modified
        // during sampling. The pointer is valid for the duration of this method.
        let inner_ptr = match &self.dist {
            Dist::Permute { inner, .. } => inner.as_ref() as *const Dist,
            _ => unreachable!("sample_permute_from_dist called on non-Permute distribution"),
        };
        // SAFETY: The pointer is valid and points to data owned by self.dist
        let inner_sample = self.sample_inner(unsafe { &*inner_ptr });
        (inner_sample.wrapping_mul(multiplier)) % self.range_size
    }

    /// Reverse: apply reversal transformation to inner distribution
    fn sample_reverse(&mut self, inner: &Dist) -> u64 {
        let inner_sample = self.sample_inner(inner);
        self.range_size - 1 - inner_sample
    }

    /// Reverse: sample from self.dist's inner distribution (avoids cloning)
    fn sample_reverse_from_dist(&mut self) -> u64 {
        // Get a raw pointer to the inner distribution to avoid borrow conflicts
        // SAFETY: We only read from the inner distribution, and self.dist is not modified
        // during sampling. The pointer is valid for the duration of this method.
        let inner_ptr = match &self.dist {
            Dist::Reverse { inner } => inner.as_ref() as *const Dist,
            _ => unreachable!("sample_reverse_from_dist called on non-Reverse distribution"),
        };
        // SAFETY: The pointer is valid and points to data owned by self.dist
        let inner_sample = self.sample_inner(unsafe { &*inner_ptr });
        self.range_size - 1 - inner_sample
    }

    /// Sample from an inner distribution (for wrappers)
    fn sample_inner(&mut self, inner: &Dist) -> u64 {
        match inner {
            Dist::Sequential => self.sample_sequential(),
            Dist::SemiSequential { multiplier } => self.sample_semi_sequential(*multiplier),
            Dist::Uniform => self.sample_uniform(),
            Dist::Zipfian { skew } => self.sample_zipfian(*skew),
            Dist::Exponential { lambda } => self.sample_exponential(*lambda),
            Dist::Normal { mean_pct, std_pct } => self.sample_normal(*mean_pct, *std_pct),
            Dist::Hotspot { hot_pct, hot_prob } => self.sample_hotspot(*hot_pct, *hot_prob),
            Dist::Latest {
                recent_pct,
                recent_prob,
            } => self.sample_latest(*recent_pct, *recent_prob),
            Dist::Permute {
                inner: nested,
                multiplier,
            } => self.sample_permute(nested, *multiplier),
            Dist::Reverse { inner: nested } => self.sample_reverse(nested),
        }
    }
}

/// Sanity tests for distribution module.
/// Complex tests (bias verification, trait tests, proptests) are in tests/dist_tests.rs.
#[cfg(test)]
mod tests {
    use super::*;
    use num_integer::gcd;

    // =========================================================================
    // Basic Ordering Property Tests
    // =========================================================================

    #[test]
    fn test_sequential_is_ordered() {
        assert!(Dist::Sequential.is_ordered());
    }

    #[test]
    fn test_semi_sequential_is_ordered() {
        assert!(Dist::SemiSequential { multiplier: 7 }.is_ordered());
    }

    #[test]
    fn test_uniform_is_unordered() {
        assert!(!Dist::Uniform.is_ordered());
    }

    #[test]
    fn test_zipfian_is_unordered() {
        assert!(!Dist::Zipfian { skew: 0.99 }.is_ordered());
    }

    #[test]
    fn test_exponential_is_unordered() {
        assert!(!Dist::Exponential { lambda: 0.1 }.is_ordered());
    }

    #[test]
    fn test_normal_is_unordered() {
        assert!(!Dist::Normal {
            mean_pct: 0.5,
            std_pct: 0.1
        }
        .is_ordered());
    }

    #[test]
    fn test_hotspot_is_unordered() {
        assert!(!Dist::Hotspot {
            hot_pct: 0.2,
            hot_prob: 0.8
        }
        .is_ordered());
    }

    #[test]
    fn test_latest_is_unordered() {
        assert!(!Dist::Latest {
            recent_pct: 0.1,
            recent_prob: 0.9
        }
        .is_ordered());
    }

    // =========================================================================
    // DistState Basic Tests
    // =========================================================================

    #[test]
    fn test_dist_state_new() {
        let state = DistState::new(Dist::Sequential, 100, 12345);
        assert_eq!(state.range_size(), 100);
        assert_eq!(state.position(), 0);
    }

    #[test]
    #[should_panic(expected = "range_size must be greater than 0")]
    fn test_dist_state_zero_range_panics() {
        DistState::new(Dist::Sequential, 0, 12345);
    }

    #[test]
    fn test_sequential_sampling() {
        let mut state = DistState::new(Dist::Sequential, 5, 12345);
        assert_eq!(state.sample(), 0);
        assert_eq!(state.sample(), 1);
        assert_eq!(state.sample(), 2);
        assert_eq!(state.sample(), 3);
        assert_eq!(state.sample(), 4);
        // Wrap around
        assert_eq!(state.sample(), 0);
        assert_eq!(state.sample(), 1);
    }

    #[test]
    fn test_uniform_bounds() {
        let mut state = DistState::new(Dist::Uniform, 100, 12345);
        for _ in 0..100 {
            let sample = state.sample();
            assert!(sample < 100);
        }
    }

    #[test]
    fn test_reset() {
        let mut state = DistState::new(Dist::Sequential, 10, 12345);
        state.sample();
        state.sample();
        state.sample();
        assert_eq!(state.position(), 3);

        state.reset(12345);
        assert_eq!(state.position(), 0);
        assert_eq!(state.sample(), 0);
    }

    #[test]
    fn test_gcd() {
        // Test using num_integer::gcd directly
        assert_eq!(gcd(12u64, 8u64), 4);
        assert_eq!(gcd(17u64, 13u64), 1);
        assert_eq!(gcd(100u64, 25u64), 25);
        assert_eq!(gcd(7u64, 1u64), 1);
        assert_eq!(gcd(1u64, 7u64), 1);
    }
}
