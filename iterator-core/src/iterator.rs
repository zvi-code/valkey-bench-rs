//! Unified generic iterator.
//!
//! This module provides [`GenericIter`], a unified iterator that combines:
//! - A coordinate space (what coordinates to iterate over)
//! - An iteration strategy (how to traverse the space)
//! - Optional bitmap filtering (which coordinates to include)
//!
//! # Builder Pattern
//!
//! Use [`IterBuilder`] for fluent iterator construction:
//!
//! ```rust,ignore
//! use iterator_core::{IterBuilder, SingleDim, AtomicBitmap};
//! use std::sync::Arc;
//!
//! let bitmap = Arc::new(AtomicBitmap::with_capacity(1000));
//!
//! // Scan-based iteration over set bits
//! let iter = IterBuilder::new(SingleDim::new(1000))
//!     .with_bitmap(bitmap)
//!     .set_only()
//!     .scan();
//!
//! for id in iter {
//!     println!("Found: {}", id);
//! }
//! ```

use crate::coords::CoordinateSpace;
use crate::distribution::Dist;
use crate::filter::{BitmapFilterConfig, BitmapOps, FilterCondition, NoBitmap, UpdateAction};
use crate::strategy::{
    AdaptiveStrategy, IterationStrategy, SampleStrategy, ScanStrategy, SequentialStrategy,
};
use std::marker::PhantomData;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

// =============================================================================
// GenericIter - Unified iterator
// =============================================================================

/// A unified iterator over coordinate spaces with bitmap filtering.
///
/// `GenericIter` combines three components:
/// - **Space** (`C`): Defines the coordinate space (1D, 2D, N-D)
/// - **Strategy** (`S`): How to traverse the space (scan, sample, adaptive)
/// - **Bitmap** (`B`): Optional bitmap for filtering
///
/// # Type Parameters
///
/// - `C`: Coordinate space implementing [`CoordinateSpace`]
/// - `S`: Iteration strategy implementing [`IterationStrategy`]
/// - `B`: Bitmap type implementing [`BitmapOps`]
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{GenericIter, SingleDim, ScanStrategy, NoBitmap};
///
/// // Simple iteration without bitmap
/// let iter: GenericIter<SingleDim, ScanStrategy, NoBitmap> = GenericIter::new(
///     SingleDim::new(100),
///     ScanStrategy::new(),
///     None,
///     Some(50), // limit
///     42,       // seed
/// );
///
/// for id in iter {
///     println!("{}", id);
/// }
/// ```
#[derive(Debug)]
pub struct GenericIter<C, S, B = NoBitmap>
where
    C: CoordinateSpace,
    S: IterationStrategy<B>,
    B: BitmapOps,
{
    /// The coordinate space
    space: C,
    /// The iteration strategy
    strategy: S,
    /// Strategy state
    strategy_state: S::State,
    /// Bitmap filter configuration (optional)
    filter_config: Option<BitmapFilterConfig<B>>,
    /// Filter condition (cached from filter_config or default)
    filter: FilterCondition,
    /// Maximum number of items to yield
    limit: Option<u64>,
    /// Number of items yielded so far
    count: u64,
    /// Whether iteration has been exhausted
    exhausted: bool,
    /// Phantom for bitmap type
    _bitmap: PhantomData<B>,
}

impl<C, S, B> GenericIter<C, S, B>
where
    C: CoordinateSpace,
    S: IterationStrategy<B>,
    B: BitmapOps,
{
    /// Create a new generic iterator.
    ///
    /// # Arguments
    ///
    /// * `space` - The coordinate space to iterate over
    /// * `strategy` - The iteration strategy to use
    /// * `filter_config` - Optional bitmap filter configuration
    /// * `limit` - Optional limit on number of items to yield
    /// * `seed` - Random seed for stochastic strategies
    pub fn new(
        space: C,
        strategy: S,
        filter_config: Option<BitmapFilterConfig<B>>,
        limit: Option<u64>,
        seed: u64,
    ) -> Self {
        let max_index = space.total_size();
        let filter = filter_config
            .as_ref()
            .map(|c| c.condition())
            .unwrap_or(FilterCondition::None);
        let strategy_state = strategy.init_state(max_index, seed);

        Self {
            space,
            strategy,
            strategy_state,
            filter_config,
            filter,
            limit,
            count: 0,
            exhausted: false,
            _bitmap: PhantomData,
        }
    }

    /// Get a reference to the coordinate space.
    #[inline]
    pub fn space(&self) -> &C {
        &self.space
    }

    /// Get a reference to the strategy.
    #[inline]
    pub fn strategy(&self) -> &S {
        &self.strategy
    }

    /// Get the current count of yielded items.
    #[inline]
    pub fn items_yielded(&self) -> u64 {
        self.count
    }

    /// Get the limit, if set.
    #[inline]
    pub fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Set the limit.
    pub fn set_limit(&mut self, limit: Option<u64>) {
        self.limit = limit;
    }

    /// Check if iteration is exhausted.
    #[inline]
    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Get the filter condition.
    #[inline]
    pub fn filter_condition(&self) -> FilterCondition {
        self.filter
    }

    /// Set the filter condition.
    pub fn set_filter_condition(&mut self, condition: FilterCondition) {
        self.filter = condition;
        if let Some(config) = &mut self.filter_config {
            config.set_condition(condition);
        }
    }

    /// Get a reference to the filter config.
    #[inline]
    pub fn filter_config(&self) -> Option<&BitmapFilterConfig<B>> {
        self.filter_config.as_ref()
    }

    /// Reset the iterator for a new cycle.
    pub fn reset(&mut self, seed: u64) {
        self.strategy.reset(&mut self.strategy_state, seed);
        self.count = 0;
        self.exhausted = false;
    }

    /// Get the next coordinate and atomically update the bitmap.
    ///
    /// This is useful for claiming or releasing coordinates atomically.
    ///
    /// # Arguments
    ///
    /// * `action` - The update action to perform
    ///
    /// # Returns
    ///
    /// The next matching coordinate if found and update succeeded, None otherwise.
    pub fn next_and_update(&mut self, action: UpdateAction) -> Option<C::Output> {
        let config = self.filter_config.as_ref()?;

        loop {
            // Check limit
            if let Some(limit) = self.limit {
                if self.count >= limit {
                    self.exhausted = true;
                    return None;
                }
            }

            // Get next index from strategy
            let max_index = self.space.total_size();
            let index = self.strategy.next_index(
                &mut self.strategy_state,
                config.bitmap().as_ref(),
                self.filter,
                max_index,
            )?;

            // Try atomic update
            if config.test_and_update(index as usize, action) {
                self.count += 1;
                return Some(self.space.index_to_coords(index));
            }

            // Update failed, retry (another thread claimed it)
        }
    }
}

impl<C, S, B> Iterator for GenericIter<C, S, B>
where
    C: CoordinateSpace,
    S: IterationStrategy<B>,
    B: BitmapOps,
{
    type Item = C::Output;

    fn next(&mut self) -> Option<Self::Item> {
        // Check if exhausted
        if self.exhausted {
            return None;
        }

        // Check limit
        if let Some(limit) = self.limit {
            if self.count >= limit {
                self.exhausted = true;
                return None;
            }
        }

        // Get bitmap reference for strategy
        let max_index = self.space.total_size();

        let index = if let Some(config) = &self.filter_config {
            self.strategy.next_index(
                &mut self.strategy_state,
                config.bitmap().as_ref(),
                self.filter,
                max_index,
            )
        } else {
            // No bitmap, use NoBitmap
            let no_bitmap = NoBitmap;
            // This is a bit awkward but needed for type safety
            // In practice, strategies should handle NoBitmap efficiently
            self.strategy.next_index(
                &mut self.strategy_state,
                // Safety: NoBitmap is zero-sized and has no state
                unsafe { &*(&no_bitmap as *const NoBitmap as *const B) },
                self.filter,
                max_index,
            )
        }?;

        self.count += 1;
        Some(self.space.index_to_coords(index))
    }
}

// =============================================================================
// IterBuilder - Fluent builder for GenericIter
// =============================================================================

/// Builder for constructing [`GenericIter`] with a fluent API.
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{IterBuilder, SingleDim, MultiDim, AtomicBitmap, Dist};
/// use std::sync::Arc;
///
/// // Simple 1D iteration without bitmap
/// let iter = IterBuilder::new(SingleDim::new(100))
///     .with_limit(50)
///     .sequential();
///
/// // 1D with bitmap filtering
/// let bitmap = Arc::new(AtomicBitmap::with_capacity(1000));
/// let iter = IterBuilder::new(SingleDim::new(1000))
///     .with_bitmap(bitmap)
///     .set_only()
///     .scan();
///
/// // Multi-dimensional with distribution
/// let iter = IterBuilder::new(MultiDim::new(&[100, 200]))
///     .with_limit(5000)
///     .sample(Dist::Zipfian { skew: 0.99 });
/// ```
#[derive(Debug)]
pub struct IterBuilder<C: CoordinateSpace, B: BitmapOps = NoBitmap> {
    space: C,
    bitmap: Option<Arc<B>>,
    filter: FilterCondition,
    limit: Option<u64>,
    seed: u64,
    secondary_filters: Vec<(Arc<B>, FilterCondition)>,
}

impl<C: CoordinateSpace> IterBuilder<C, NoBitmap> {
    /// Create a new builder for the given coordinate space.
    pub fn new(space: C) -> Self {
        Self {
            space,
            bitmap: None,
            filter: FilterCondition::None,
            limit: None,
            seed: fastrand::u64(..),
            secondary_filters: Vec::new(),
        }
    }
}

impl<C: CoordinateSpace, B: BitmapOps + 'static> IterBuilder<C, B> {
    /// Add a bitmap for filtering.
    pub fn with_bitmap<B2: BitmapOps + 'static>(self, bitmap: Arc<B2>) -> IterBuilder<C, B2> {
        IterBuilder {
            space: self.space,
            bitmap: Some(bitmap),
            filter: self.filter,
            limit: self.limit,
            seed: self.seed,
            secondary_filters: Vec::new(),
        }
    }

    /// Set the filter condition to only return set bits.
    pub fn set_only(mut self) -> Self {
        self.filter = FilterCondition::Set;
        self
    }

    /// Set the filter condition to only return unset bits.
    pub fn unset_only(mut self) -> Self {
        self.filter = FilterCondition::Unset;
        self
    }

    /// Set the filter condition.
    pub fn with_filter(mut self, filter: FilterCondition) -> Self {
        self.filter = filter;
        self
    }

    /// Set the iteration limit.
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set the random seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Add a secondary filter (read-only).
    pub fn with_secondary_filter(mut self, bitmap: Arc<B>, condition: FilterCondition) -> Self {
        self.secondary_filters.push((bitmap, condition));
        self
    }

    /// Build with sequential strategy (simple 0, 1, 2, ...).
    pub fn sequential(self) -> GenericIter<C, SequentialStrategy, B> {
        let filter_config = self.build_filter_config();
        GenericIter::new(
            self.space,
            SequentialStrategy::new(),
            filter_config,
            self.limit,
            self.seed,
        )
    }

    /// Build with sequential wrap-around strategy.
    pub fn sequential_wrap(self) -> GenericIter<C, SequentialStrategy, B> {
        let filter_config = self.build_filter_config();
        GenericIter::new(
            self.space,
            SequentialStrategy::new().wrap_around(),
            filter_config,
            self.limit,
            self.seed,
        )
    }

    /// Build with scan strategy (SIMD-accelerated bitmap scanning).
    pub fn scan(self) -> GenericIter<C, ScanStrategy, B> {
        let filter_config = self.build_filter_config();
        GenericIter::new(
            self.space,
            ScanStrategy::new(),
            filter_config,
            self.limit,
            self.seed,
        )
    }

    /// Build with scan strategy and wrap-around.
    pub fn scan_wrap(self) -> GenericIter<C, ScanStrategy, B> {
        let filter_config = self.build_filter_config();
        GenericIter::new(
            self.space,
            ScanStrategy::new().wrap_around(),
            filter_config,
            self.limit,
            self.seed,
        )
    }

    /// Build with scan strategy using a shared cursor (for concurrent claiming).
    pub fn scan_shared(self, cursor: Arc<AtomicU64>) -> GenericIter<C, ScanStrategy, B> {
        let filter_config = self.build_filter_config();
        GenericIter::new(
            self.space,
            ScanStrategy::with_shared_cursor(cursor),
            filter_config,
            self.limit,
            self.seed,
        )
    }

    /// Build with sample strategy using the given distribution.
    pub fn sample(self, distribution: Dist) -> GenericIter<C, SampleStrategy, B> {
        let filter_config = self.build_filter_config();
        GenericIter::new(
            self.space,
            SampleStrategy::new(distribution),
            filter_config,
            self.limit,
            self.seed,
        )
    }

    /// Build with uniform random sampling.
    pub fn uniform(self) -> GenericIter<C, SampleStrategy, B> {
        self.sample(Dist::Uniform)
    }

    /// Build with Zipfian sampling.
    pub fn zipfian(self, skew: f64) -> GenericIter<C, SampleStrategy, B> {
        self.sample(Dist::Zipfian { skew })
    }

    /// Build with adaptive strategy (auto-selects scan/sample).
    pub fn adaptive(self, distribution: Dist) -> GenericIter<C, AdaptiveStrategy, B> {
        let filter_config = self.build_filter_config();
        GenericIter::new(
            self.space,
            AdaptiveStrategy::new(distribution),
            filter_config,
            self.limit,
            self.seed,
        )
    }

    /// Build the filter configuration.
    fn build_filter_config(&self) -> Option<BitmapFilterConfig<B>> {
        let bitmap = self.bitmap.clone()?;
        let dim_sizes = self.space.dim_sizes();

        let mut config = BitmapFilterConfig::new(bitmap, self.filter, dim_sizes);

        for (secondary_bitmap, condition) in &self.secondary_filters {
            config.add_secondary_filter(secondary_bitmap.clone(), *condition);
        }

        Some(config)
    }
}

// =============================================================================
// Type Aliases for Common Patterns
// =============================================================================

/// 1D iterator with scan strategy (like keyspace_tracker's SequentialIter).
#[allow(dead_code)]
pub type ScanIter1D<B> = GenericIter<crate::coords::SingleDim, ScanStrategy, B>;

/// 1D iterator with sample strategy.
#[allow(dead_code)]
pub type SampleIter1D<B> = GenericIter<crate::coords::SingleDim, SampleStrategy, B>;

/// 2D iterator with scan strategy (like keyspace_tracker's hierarchical iter).
#[allow(dead_code)]
pub type ScanIter2D<B> = GenericIter<crate::coords::TwoDim, ScanStrategy, B>;

/// 2D iterator with sample strategy.
#[allow(dead_code)]
pub type SampleIter2D<B> = GenericIter<crate::coords::TwoDim, SampleStrategy, B>;

/// N-D iterator with scan strategy.
#[allow(dead_code)]
pub type ScanIterND<B> = GenericIter<crate::coords::MultiDim, ScanStrategy, B>;

/// N-D iterator with sample strategy (like iterators-rs's MultiDimIterator).
#[allow(dead_code)]
pub type SampleIterND<B> = GenericIter<crate::coords::MultiDim, SampleStrategy, B>;

/// N-D iterator with adaptive strategy.
#[allow(dead_code)]
pub type AdaptiveIterND<B> = GenericIter<crate::coords::MultiDim, AdaptiveStrategy, B>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::{MultiDim, SingleDim, TwoDim};
    use crate::AtomicBitmap;

    #[test]
    fn test_builder_sequential_no_bitmap() {
        let iter = IterBuilder::new(SingleDim::new(10)).sequential();

        let results: Vec<u64> = iter.collect();
        assert_eq!(results, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn test_builder_sequential_with_limit() {
        let iter = IterBuilder::new(SingleDim::new(100)).with_limit(5).sequential();

        let results: Vec<u64> = iter.collect();
        assert_eq!(results, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_builder_with_bitmap_set_only() {
        let bitmap = Arc::new(AtomicBitmap::with_capacity(100));
        bitmap.set(10);
        bitmap.set(20);
        bitmap.set(30);

        let iter = IterBuilder::new(SingleDim::new(100))
            .with_bitmap(bitmap)
            .set_only()
            .sequential();

        let results: Vec<u64> = iter.collect();
        assert_eq!(results, vec![10, 20, 30]);
    }

    #[test]
    fn test_builder_scan_with_bitmap() {
        let bitmap = Arc::new(AtomicBitmap::with_capacity(100));
        bitmap.set(5);
        bitmap.set(15);
        bitmap.set(25);

        let iter = IterBuilder::new(SingleDim::new(100))
            .with_bitmap(bitmap)
            .set_only()
            .scan();

        let results: Vec<u64> = iter.collect();
        assert_eq!(results, vec![5, 15, 25]);
    }

    #[test]
    fn test_builder_2d() {
        let iter = IterBuilder::new(TwoDim::new(3, 2)).sequential();

        let results: Vec<(u64, u64)> = iter.collect();
        assert_eq!(
            results,
            vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)]
        );
    }

    #[test]
    fn test_builder_multi_dim() {
        let iter = IterBuilder::new(MultiDim::new(&[2, 2, 2])).sequential();

        let results: Vec<_> = iter.collect();
        assert_eq!(results.len(), 8);
        assert_eq!(&results[0][..], &[0, 0, 0]);
        assert_eq!(&results[7][..], &[1, 1, 1]);
    }

    #[test]
    fn test_builder_uniform_sampling() {
        let iter = IterBuilder::new(SingleDim::new(1000))
            .with_limit(100)
            .with_seed(42)
            .uniform();

        let results: Vec<u64> = iter.collect();
        assert_eq!(results.len(), 100);
        assert!(results.iter().all(|&x| x < 1000));
    }

    #[test]
    fn test_next_and_update() {
        let bitmap = Arc::new(AtomicBitmap::with_capacity(100));

        let mut iter = IterBuilder::new(SingleDim::new(100))
            .with_bitmap(bitmap.clone())
            .unset_only()
            .sequential();

        // Claim first 3 slots
        let c1 = iter.next_and_update(UpdateAction::Set).unwrap();
        let c2 = iter.next_and_update(UpdateAction::Set).unwrap();
        let c3 = iter.next_and_update(UpdateAction::Set).unwrap();

        assert_eq!(c1, 0);
        assert_eq!(c2, 1);
        assert_eq!(c3, 2);

        // Verify bitmap was updated
        assert!(bitmap.test(0));
        assert!(bitmap.test(1));
        assert!(bitmap.test(2));
        assert!(!bitmap.test(3));
    }

    #[test]
    fn test_iterator_items_yielded() {
        let mut iter = IterBuilder::new(SingleDim::new(100))
            .with_limit(10)
            .sequential();

        assert_eq!(iter.items_yielded(), 0);

        iter.next();
        assert_eq!(iter.items_yielded(), 1);

        iter.next();
        iter.next();
        assert_eq!(iter.items_yielded(), 3);

        // Exhaust
        while iter.next().is_some() {}
        assert_eq!(iter.items_yielded(), 10);
    }

    #[test]
    fn test_reset() {
        let mut iter = IterBuilder::new(SingleDim::new(10))
            .with_limit(5)
            .with_seed(42)
            .sequential();

        // Consume some
        iter.next();
        iter.next();
        iter.next();
        assert_eq!(iter.items_yielded(), 3);

        // Reset
        iter.reset(42);
        assert_eq!(iter.items_yielded(), 0);
        assert!(!iter.is_exhausted());

        // Should start from beginning
        assert_eq!(iter.next(), Some(0));
        assert_eq!(iter.next(), Some(1));
    }
}
