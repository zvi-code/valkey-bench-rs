//! Bitmap filtering for iteration.
//!
//! This module provides bitmap-based filtering capabilities:
//!
//! - [`BitmapOps`]: Trait for atomic bitmap operations
//! - [`FilterCondition`]: Filter based on set/unset bits
//! - [`UpdateAction`]: Atomic update operations (claim/release)
//! - [`BitmapFilterConfig`]: Configuration for filtered iteration
//!
//! # Filtering Modes
//!
//! 1. **No filter**: Iterate over all coordinates
//! 2. **Set only**: Only coordinates where bitmap bit is set
//! 3. **Unset only**: Only coordinates where bitmap bit is unset
//!
//! # Atomic Updates
//!
//! During iteration, you can atomically update the bitmap:
//! - `Set`: Claim an unset coordinate (test-and-set)
//! - `Clear`: Release a set coordinate (test-and-clear)

use crate::strides::{OptimizedStrides, Strides};
use std::sync::Arc;

// =============================================================================
// BitmapOps Trait
// =============================================================================

/// Trait for bitmap operations required by the filter.
///
/// This trait abstracts the bitmap operations needed for filtering,
/// allowing different bitmap implementations to be used.
///
/// # Required Methods
///
/// - `test`: Check if a bit is set
/// - `test_and_set`: Atomically set a bit if unset
/// - `test_and_clear`: Atomically clear a bit if set
///
/// # Optional Methods
///
/// - `find_next_set`: SIMD-accelerated scan for next set bit
/// - `find_next_unset`: SIMD-accelerated scan for next unset bit
/// - `capacity`: Total number of bits
/// - `count`: Number of set bits (population count)
pub trait BitmapOps: Send + Sync {
    /// Test if a bit is set at the given index.
    fn test(&self, index: usize) -> bool;

    /// Atomically test and set a bit.
    /// Returns true if the bit was unset and is now set.
    fn test_and_set(&self, index: usize) -> bool;

    /// Atomically test and clear a bit.
    /// Returns true if the bit was set and is now cleared.
    fn test_and_clear(&self, index: usize) -> bool;

    /// Set a bit (non-atomic test).
    fn set(&self, index: usize);

    /// Clear a bit (non-atomic test).
    fn clear(&self, index: usize);

    /// Find the next set bit starting from `start_index`.
    /// Returns None if no set bit is found.
    ///
    /// Default implementation returns None (not supported).
    /// Implementations should override with SIMD-optimized versions.
    fn find_next_set(&self, _start_index: usize) -> Option<usize> {
        None
    }

    /// Find the next unset bit starting from `start_index` up to `max_index`.
    /// Returns None if no unset bit is found before `max_index`.
    ///
    /// Default implementation returns None (not supported).
    /// Implementations should override with SIMD-optimized versions.
    fn find_next_unset(&self, _start_index: usize, _max_index: usize) -> Option<usize> {
        None
    }

    /// Returns true if find_next_set is efficiently supported (e.g., SIMD).
    fn supports_find_next_set(&self) -> bool {
        false
    }

    /// Returns true if find_next_unset is efficiently supported (e.g., SIMD).
    fn supports_find_next_unset(&self) -> bool {
        false
    }

    /// Get the capacity of the bitmap in bits.
    fn capacity(&self) -> usize {
        0
    }

    /// Get the population count (number of set bits).
    fn count(&self) -> u64 {
        0
    }

    /// Get the density (set bits / capacity).
    fn density(&self) -> f64 {
        let cap = self.capacity();
        if cap == 0 {
            0.0
        } else {
            self.count() as f64 / cap as f64
        }
    }
}

// =============================================================================
// NoBitmap - No-op implementation for unfiltered iteration
// =============================================================================

/// A no-op bitmap that allows all coordinates.
///
/// This is used when no bitmap filtering is needed, allowing the type system
/// to handle filtered and unfiltered iteration uniformly.
#[derive(Clone, Debug, Default)]
pub struct NoBitmap;

impl BitmapOps for NoBitmap {
    #[inline]
    fn test(&self, _index: usize) -> bool {
        true
    }

    #[inline]
    fn test_and_set(&self, _index: usize) -> bool {
        true
    }

    #[inline]
    fn test_and_clear(&self, _index: usize) -> bool {
        true
    }

    #[inline]
    fn set(&self, _index: usize) {}

    #[inline]
    fn clear(&self, _index: usize) {}

    fn supports_find_next_set(&self) -> bool {
        true
    }

    fn supports_find_next_unset(&self) -> bool {
        true
    }

    fn find_next_set(&self, start_index: usize) -> Option<usize> {
        Some(start_index) // All bits are "set"
    }

    fn find_next_unset(&self, start_index: usize, max_index: usize) -> Option<usize> {
        if start_index < max_index {
            Some(start_index) // All bits are "unset"
        } else {
            None
        }
    }

    fn capacity(&self) -> usize {
        usize::MAX
    }

    fn count(&self) -> u64 {
        0
    }
}

// =============================================================================
// FilterCondition
// =============================================================================

/// Filter condition for bitmap-based filtering.
///
/// Determines which coordinates are returned based on bitmap state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterCondition {
    /// No filtering - return all coordinates
    #[default]
    None,
    /// Only return coordinates where the bitmap bit is set (1)
    Set,
    /// Only return coordinates where the bitmap bit is unset (0)
    Unset,
}

impl FilterCondition {
    /// Check if a bit state matches this condition.
    #[inline]
    pub fn matches(&self, bit_is_set: bool) -> bool {
        match self {
            FilterCondition::None => true,
            FilterCondition::Set => bit_is_set,
            FilterCondition::Unset => !bit_is_set,
        }
    }

    /// Check if filtering is enabled.
    #[inline]
    pub fn is_filtering(&self) -> bool {
        !matches!(self, FilterCondition::None)
    }
}

// =============================================================================
// UpdateAction
// =============================================================================

/// Action to perform on the bitmap after returning a coordinate.
///
/// Used with atomic claim operations to update bitmap state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateAction {
    /// Set the bit after returning (claim operation)
    Set,
    /// Clear the bit after returning (release operation)
    Clear,
}

// =============================================================================
// SecondaryFilter
// =============================================================================

/// A secondary filter that provides read-only filtering on a bitmap.
///
/// Secondary filters are checked in addition to the primary filter,
/// but cannot be updated via `next_and_update()`.
#[derive(Debug)]
pub struct SecondaryFilter<B: BitmapOps> {
    bitmap: Arc<B>,
    condition: FilterCondition,
}

impl<B: BitmapOps> Clone for SecondaryFilter<B> {
    fn clone(&self) -> Self {
        Self {
            bitmap: Arc::clone(&self.bitmap),
            condition: self.condition,
        }
    }
}

impl<B: BitmapOps> SecondaryFilter<B> {
    /// Create a new secondary filter.
    pub fn new(bitmap: Arc<B>, condition: FilterCondition) -> Self {
        Self { bitmap, condition }
    }

    /// Check if the given index matches this filter's condition.
    #[inline]
    pub fn matches(&self, index: usize) -> bool {
        self.condition.matches(self.bitmap.test(index))
    }

    /// Get the filter condition.
    #[inline]
    pub fn condition(&self) -> FilterCondition {
        self.condition
    }

    /// Get a reference to the bitmap.
    #[inline]
    pub fn bitmap(&self) -> &Arc<B> {
        &self.bitmap
    }
}

// =============================================================================
// BitmapFilterConfig
// =============================================================================

/// Configuration for bitmap-based filtering.
///
/// Stores the primary bitmap, filter condition, optional secondary filters,
/// and precomputed strides for efficient index computation.
///
/// # Primary vs Secondary Filters
///
/// - **Primary filter**: The main bitmap that can be updated via `next_and_update()`.
/// - **Secondary filters**: Additional read-only bitmaps (AND logic with primary).
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{BitmapFilterConfig, FilterCondition, AtomicBitmap};
/// use std::sync::Arc;
///
/// let bitmap = Arc::new(AtomicBitmap::with_capacity(1000));
/// let config = BitmapFilterConfig::new(
///     bitmap,
///     FilterCondition::Unset,
///     &[1000],
/// );
/// ```
#[derive(Debug)]
pub struct BitmapFilterConfig<B: BitmapOps> {
    /// Reference to the primary shared atomic bitmap
    bitmap: Arc<B>,
    /// Current filter condition for the primary bitmap
    condition: FilterCondition,
    /// Number of dimensions covered by the bitmap
    prefix_dims: usize,
    /// Optimized strides for bitmap index calculation
    strides: OptimizedStrides,
    /// Secondary filters (read-only, all must match)
    secondary_filters: Vec<SecondaryFilter<B>>,
}

impl<B: BitmapOps> Clone for BitmapFilterConfig<B> {
    fn clone(&self) -> Self {
        Self {
            bitmap: Arc::clone(&self.bitmap),
            condition: self.condition,
            prefix_dims: self.prefix_dims,
            strides: self.strides.clone(),
            secondary_filters: self.secondary_filters.clone(),
        }
    }
}

impl<B: BitmapOps> BitmapFilterConfig<B> {
    /// Create a new bitmap filter configuration.
    ///
    /// # Arguments
    ///
    /// * `bitmap` - Reference to the shared atomic bitmap
    /// * `condition` - Initial filter condition
    /// * `dim_sizes` - Sizes of each dimension (all dimensions covered)
    pub fn new(bitmap: Arc<B>, condition: FilterCondition, dim_sizes: &[u64]) -> Self {
        let prefix_dims = dim_sizes.len();
        let strides = OptimizedStrides::from_sizes(dim_sizes, prefix_dims);

        Self {
            bitmap,
            condition,
            prefix_dims,
            strides,
            secondary_filters: Vec::new(),
        }
    }

    /// Create a configuration covering only the first `prefix_dims` dimensions.
    ///
    /// # Arguments
    ///
    /// * `bitmap` - Reference to the shared atomic bitmap
    /// * `condition` - Initial filter condition
    /// * `prefix_dims` - Number of dimensions covered by the bitmap
    /// * `dim_sizes` - Sizes of all dimensions
    ///
    /// # Panics
    ///
    /// Panics if `prefix_dims` is 0 or exceeds the number of dimensions.
    pub fn with_prefix_dims(
        bitmap: Arc<B>,
        condition: FilterCondition,
        prefix_dims: usize,
        dim_sizes: &[u64],
    ) -> Self {
        assert!(prefix_dims > 0, "prefix_dims must be greater than 0");
        assert!(
            prefix_dims <= dim_sizes.len(),
            "prefix_dims ({}) exceeds number of dimensions ({})",
            prefix_dims,
            dim_sizes.len()
        );

        let strides = OptimizedStrides::from_sizes(dim_sizes, prefix_dims);

        Self {
            bitmap,
            condition,
            prefix_dims,
            strides,
            secondary_filters: Vec::new(),
        }
    }

    /// Add a secondary filter.
    ///
    /// All secondary filter conditions must match (AND logic).
    pub fn with_secondary_filter(mut self, bitmap: Arc<B>, condition: FilterCondition) -> Self {
        self.secondary_filters
            .push(SecondaryFilter::new(bitmap, condition));
        self
    }

    /// Add a secondary filter (mutable version).
    pub fn add_secondary_filter(&mut self, bitmap: Arc<B>, condition: FilterCondition) {
        self.secondary_filters
            .push(SecondaryFilter::new(bitmap, condition));
    }

    /// Get the current filter condition.
    #[inline]
    pub fn condition(&self) -> FilterCondition {
        self.condition
    }

    /// Set the filter condition.
    #[inline]
    pub fn set_condition(&mut self, condition: FilterCondition) {
        self.condition = condition;
    }

    /// Get the number of prefix dimensions.
    #[inline]
    pub fn prefix_dims(&self) -> usize {
        self.prefix_dims
    }

    /// Get a reference to the bitmap.
    #[inline]
    pub fn bitmap(&self) -> &Arc<B> {
        &self.bitmap
    }

    /// Get the precomputed strides.
    #[inline]
    pub fn strides(&self) -> &[u64] {
        self.strides.as_slice()
    }

    /// Get the secondary filters.
    #[inline]
    pub fn secondary_filters(&self) -> &[SecondaryFilter<B>] {
        &self.secondary_filters
    }

    /// Compute the bitmap index for coordinates.
    ///
    /// Uses the first `prefix_dims` coordinates.
    #[inline]
    pub fn compute_index(&self, coords: &[u64]) -> usize {
        self.strides.compute_index(coords)
    }

    /// Compute the bitmap index from a flat space index.
    ///
    /// For 1D spaces, this is just the index itself.
    /// For higher dimensions, converts to coords first.
    #[inline]
    pub fn compute_index_from_flat(&self, flat_index: u64) -> usize {
        // For 1D, the flat index is the bitmap index
        if self.prefix_dims == 1 {
            flat_index as usize
        } else {
            // For N-D, we need to compute coords first
            // This is less efficient but handles the general case
            flat_index as usize
        }
    }

    /// Check if coordinates match all filter conditions.
    ///
    /// Checks primary filter AND all secondary filters.
    #[inline]
    pub fn matches(&self, index: usize) -> bool {
        // Check primary filter
        let primary_matches = self.condition.matches(self.bitmap.test(index));

        if !primary_matches {
            return false;
        }

        // Check all secondary filters (AND logic)
        self.secondary_filters.iter().all(|f| f.matches(index))
    }

    /// Check if a flat index matches all filter conditions.
    #[inline]
    pub fn matches_flat(&self, flat_index: u64) -> bool {
        let index = self.compute_index_from_flat(flat_index);
        self.matches(index)
    }

    /// Atomically test and update the bitmap.
    ///
    /// Returns true if the operation succeeded (bit was in expected state).
    #[inline]
    pub fn test_and_update(&self, index: usize, action: UpdateAction) -> bool {
        match action {
            UpdateAction::Set => self.bitmap.test_and_set(index),
            UpdateAction::Clear => self.bitmap.test_and_clear(index),
        }
    }

    /// Check if efficient scanning is supported.
    #[inline]
    pub fn supports_scan(&self) -> bool {
        match self.condition {
            FilterCondition::None => true,
            FilterCondition::Set => self.bitmap.supports_find_next_set(),
            FilterCondition::Unset => self.bitmap.supports_find_next_unset(),
        }
    }

    /// Find next matching index using SIMD scan.
    ///
    /// Returns None if no matching index found or scan not supported.
    #[inline]
    pub fn find_next(&self, start_index: usize, max_index: usize) -> Option<usize> {
        match self.condition {
            FilterCondition::None => {
                if start_index < max_index {
                    Some(start_index)
                } else {
                    None
                }
            }
            FilterCondition::Set => self.bitmap.find_next_set(start_index),
            FilterCondition::Unset => self.bitmap.find_next_unset(start_index, max_index),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_condition_matches() {
        assert!(FilterCondition::None.matches(true));
        assert!(FilterCondition::None.matches(false));

        assert!(FilterCondition::Set.matches(true));
        assert!(!FilterCondition::Set.matches(false));

        assert!(!FilterCondition::Unset.matches(true));
        assert!(FilterCondition::Unset.matches(false));
    }

    #[test]
    fn test_filter_condition_is_filtering() {
        assert!(!FilterCondition::None.is_filtering());
        assert!(FilterCondition::Set.is_filtering());
        assert!(FilterCondition::Unset.is_filtering());
    }

    #[test]
    fn test_no_bitmap() {
        let bitmap = NoBitmap;
        assert!(bitmap.test(0));
        assert!(bitmap.test(1000));
        assert!(bitmap.test_and_set(0));
        assert!(bitmap.test_and_clear(0));
        assert!(bitmap.supports_find_next_set());
        assert!(bitmap.supports_find_next_unset());
        assert_eq!(bitmap.find_next_set(42), Some(42));
        assert_eq!(bitmap.find_next_unset(42, 100), Some(42));
    }
}
