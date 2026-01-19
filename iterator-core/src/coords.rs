//! Coordinate space abstractions.
//!
//! This module provides the [`CoordinateSpace`] trait and implementations for
//! various dimensionalities:
//!
//! - [`SingleDim`]: 1-dimensional space (simple ID tracking)
//! - [`TwoDim`]: 2-dimensional space (id, sub_id pairs)
//! - [`MultiDim`]: N-dimensional space (arbitrary Cartesian products)
//!
//! # Design Philosophy
//!
//! The trait uses associated types for coordinates, enabling compile-time
//! specialization for common cases (1D uses `u64`, 2D uses `(u64, u64)`)
//! while supporting arbitrary dimensions via `SmallVec`.

use smallvec::SmallVec;
use std::fmt::Debug;

/// Stack-allocated coordinates for up to 4 dimensions, heap for more.
pub type Coords = SmallVec<[u64; 4]>;

/// Defines a coordinate space for iteration.
///
/// A coordinate space represents the domain over which iteration occurs.
/// It provides methods to convert between flat indices and coordinates,
/// compute sizes, and manage dimension information.
///
/// # Associated Types
///
/// - `Output`: The coordinate type returned by iteration (e.g., `u64`, `(u64, u64)`, `Coords`)
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{CoordinateSpace, SingleDim};
///
/// let space = SingleDim::new(100);
/// assert_eq!(space.total_size(), 100);
/// assert_eq!(space.index_to_coords(42), 42);
/// ```
pub trait CoordinateSpace: Clone + Debug + Send + Sync {
    /// The coordinate type produced by this space.
    type Output: Clone + Debug + Send;

    /// Number of dimensions in this space.
    fn num_dimensions(&self) -> usize;

    /// Total number of coordinates (product of all dimension sizes).
    fn total_size(&self) -> u64;

    /// Size of a specific dimension.
    ///
    /// # Panics
    ///
    /// May panic if `dim >= num_dimensions()`.
    fn dim_size(&self, dim: usize) -> u64;

    /// Convert a flat index to coordinates.
    ///
    /// The index should be in range `[0, total_size())`.
    fn index_to_coords(&self, index: u64) -> Self::Output;

    /// Convert coordinates to a flat index.
    fn coords_to_index(&self, coords: &Self::Output) -> u64;

    /// Get all dimension sizes as a slice.
    fn dim_sizes(&self) -> &[u64];

    /// Get precomputed strides for index calculation.
    fn strides(&self) -> &[u64];
}

// =============================================================================
// SingleDim - 1-dimensional coordinate space
// =============================================================================

/// A 1-dimensional coordinate space.
///
/// This is the simplest coordinate space, representing a range `[0, size)`.
/// Coordinates are just `u64` values.
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{CoordinateSpace, SingleDim};
///
/// let space = SingleDim::new(1000);
/// assert_eq!(space.num_dimensions(), 1);
/// assert_eq!(space.total_size(), 1000);
/// assert_eq!(space.index_to_coords(42), 42);
/// assert_eq!(space.coords_to_index(&42), 42);
/// ```
#[derive(Clone, Debug)]
pub struct SingleDim {
    size: u64,
    sizes: [u64; 1],
    strides: [u64; 1],
}

impl SingleDim {
    /// Create a new 1-dimensional space with the given size.
    ///
    /// # Arguments
    ///
    /// * `size` - The size of the dimension (range is `[0, size)`)
    ///
    /// # Panics
    ///
    /// Panics if `size` is 0.
    pub fn new(size: u64) -> Self {
        assert!(size > 0, "size must be greater than 0");
        Self {
            size,
            sizes: [size],
            strides: [1],
        }
    }

    /// Get the size of this dimension.
    #[inline]
    pub fn size(&self) -> u64 {
        self.size
    }
}

impl CoordinateSpace for SingleDim {
    type Output = u64;

    #[inline]
    fn num_dimensions(&self) -> usize {
        1
    }

    #[inline]
    fn total_size(&self) -> u64 {
        self.size
    }

    #[inline]
    fn dim_size(&self, dim: usize) -> u64 {
        assert_eq!(dim, 0, "SingleDim only has dimension 0");
        self.size
    }

    #[inline]
    fn index_to_coords(&self, index: u64) -> u64 {
        debug_assert!(index < self.size, "index out of bounds");
        index
    }

    #[inline]
    fn coords_to_index(&self, coords: &u64) -> u64 {
        debug_assert!(*coords < self.size, "coords out of bounds");
        *coords
    }

    #[inline]
    fn dim_sizes(&self) -> &[u64] {
        &self.sizes
    }

    #[inline]
    fn strides(&self) -> &[u64] {
        &self.strides
    }
}

// =============================================================================
// TwoDim - 2-dimensional coordinate space
// =============================================================================

/// A 2-dimensional coordinate space.
///
/// Represents a Cartesian product of two ranges: `[0, primary_size) × [0, secondary_size)`.
/// Coordinates are `(u64, u64)` tuples representing `(primary, secondary)`.
///
/// This is useful for tracking (id, sub_id) pairs like hash fields or nested keys.
///
/// # Index Layout
///
/// Indices are laid out in row-major order:
/// - Index 0 → (0, 0)
/// - Index 1 → (0, 1)
/// - Index `secondary_size` → (1, 0)
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{CoordinateSpace, TwoDim};
///
/// let space = TwoDim::new(100, 50); // 100 primary IDs, 50 sub-IDs each
/// assert_eq!(space.num_dimensions(), 2);
/// assert_eq!(space.total_size(), 5000);
///
/// // Index to coords
/// assert_eq!(space.index_to_coords(0), (0, 0));
/// assert_eq!(space.index_to_coords(50), (1, 0));
/// assert_eq!(space.index_to_coords(51), (1, 1));
///
/// // Coords to index
/// assert_eq!(space.coords_to_index(&(1, 5)), 55);
/// ```
#[derive(Clone, Debug)]
pub struct TwoDim {
    primary_size: u64,
    secondary_size: u64,
    sizes: [u64; 2],
    strides: [u64; 2],
}

impl TwoDim {
    /// Create a new 2-dimensional space.
    ///
    /// # Arguments
    ///
    /// * `primary_size` - Size of the first dimension (e.g., number of keys)
    /// * `secondary_size` - Size of the second dimension (e.g., fields per key)
    ///
    /// # Panics
    ///
    /// Panics if either size is 0.
    pub fn new(primary_size: u64, secondary_size: u64) -> Self {
        assert!(primary_size > 0, "primary_size must be greater than 0");
        assert!(secondary_size > 0, "secondary_size must be greater than 0");
        Self {
            primary_size,
            secondary_size,
            sizes: [primary_size, secondary_size],
            strides: [secondary_size, 1],
        }
    }

    /// Get the primary dimension size.
    #[inline]
    pub fn primary_size(&self) -> u64 {
        self.primary_size
    }

    /// Get the secondary dimension size.
    #[inline]
    pub fn secondary_size(&self) -> u64 {
        self.secondary_size
    }
}

impl CoordinateSpace for TwoDim {
    type Output = (u64, u64);

    #[inline]
    fn num_dimensions(&self) -> usize {
        2
    }

    #[inline]
    fn total_size(&self) -> u64 {
        self.primary_size * self.secondary_size
    }

    #[inline]
    fn dim_size(&self, dim: usize) -> u64 {
        match dim {
            0 => self.primary_size,
            1 => self.secondary_size,
            _ => panic!("TwoDim only has dimensions 0 and 1"),
        }
    }

    #[inline]
    fn index_to_coords(&self, index: u64) -> (u64, u64) {
        debug_assert!(index < self.total_size(), "index out of bounds");
        let primary = index / self.secondary_size;
        let secondary = index % self.secondary_size;
        (primary, secondary)
    }

    #[inline]
    fn coords_to_index(&self, coords: &(u64, u64)) -> u64 {
        debug_assert!(coords.0 < self.primary_size, "primary coord out of bounds");
        debug_assert!(coords.1 < self.secondary_size, "secondary coord out of bounds");
        coords.0 * self.secondary_size + coords.1
    }

    #[inline]
    fn dim_sizes(&self) -> &[u64] {
        &self.sizes
    }

    #[inline]
    fn strides(&self) -> &[u64] {
        &self.strides
    }
}

// =============================================================================
// MultiDim - N-dimensional coordinate space
// =============================================================================

/// An N-dimensional coordinate space.
///
/// Represents a Cartesian product of N ranges. Coordinates are stored in a
/// `SmallVec<[u64; 4]>` which is stack-allocated for up to 4 dimensions.
///
/// # Index Layout
///
/// Indices are laid out in row-major order (last dimension varies fastest):
/// - For dimensions `[D0, D1, D2]`: index = `i0 * (D1*D2) + i1 * D2 + i2`
///
/// # Examples
///
/// ```rust,ignore
/// use iterator_core::{CoordinateSpace, MultiDim, Coords};
///
/// let space = MultiDim::new(&[10, 20, 30]);
/// assert_eq!(space.num_dimensions(), 3);
/// assert_eq!(space.total_size(), 6000);
///
/// // Index to coords
/// let coords = space.index_to_coords(0);
/// assert_eq!(&coords[..], &[0, 0, 0]);
///
/// let coords = space.index_to_coords(31);
/// assert_eq!(&coords[..], &[0, 1, 1]);
///
/// // Coords to index
/// let coords: Coords = smallvec::smallvec![1, 2, 3];
/// assert_eq!(space.coords_to_index(&coords), 1 * 600 + 2 * 30 + 3);
/// ```
#[derive(Clone, Debug)]
pub struct MultiDim {
    sizes: Vec<u64>,
    strides: Vec<u64>,
    total: u64,
}

impl MultiDim {
    /// Create a new N-dimensional space.
    ///
    /// # Arguments
    ///
    /// * `sizes` - Sizes of each dimension
    ///
    /// # Panics
    ///
    /// Panics if `sizes` is empty or contains zeros.
    pub fn new(sizes: &[u64]) -> Self {
        assert!(!sizes.is_empty(), "sizes must not be empty");
        assert!(sizes.iter().all(|&s| s > 0), "all sizes must be greater than 0");

        let n = sizes.len();
        let mut strides = vec![1u64; n];

        // Compute strides from right to left
        for i in (0..n - 1).rev() {
            strides[i] = strides[i + 1] * sizes[i + 1];
        }

        let total = sizes.iter().product();

        Self {
            sizes: sizes.to_vec(),
            strides,
            total,
        }
    }

    /// Create from ranges (for compatibility with iterators-rs style).
    ///
    /// # Arguments
    ///
    /// * `ranges` - Iterator of (start, end) pairs
    ///
    /// # Note
    ///
    /// Start values are ignored; only range sizes are used.
    pub fn from_ranges<I>(ranges: I) -> Self
    where
        I: IntoIterator<Item = (u64, u64)>,
    {
        let sizes: Vec<u64> = ranges.into_iter().map(|(start, end)| end - start).collect();
        Self::new(&sizes)
    }
}

impl CoordinateSpace for MultiDim {
    type Output = Coords;

    #[inline]
    fn num_dimensions(&self) -> usize {
        self.sizes.len()
    }

    #[inline]
    fn total_size(&self) -> u64 {
        self.total
    }

    #[inline]
    fn dim_size(&self, dim: usize) -> u64 {
        self.sizes[dim]
    }

    fn index_to_coords(&self, index: u64) -> Coords {
        debug_assert!(index < self.total, "index out of bounds");

        let mut coords = Coords::with_capacity(self.sizes.len());
        let mut remaining = index;

        for i in 0..self.sizes.len() {
            coords.push(remaining / self.strides[i]);
            remaining %= self.strides[i];
        }

        coords
    }

    fn coords_to_index(&self, coords: &Coords) -> u64 {
        debug_assert_eq!(coords.len(), self.sizes.len(), "coords dimension mismatch");

        let mut index = 0u64;
        for i in 0..self.sizes.len() {
            debug_assert!(coords[i] < self.sizes[i], "coord {} out of bounds", i);
            index += coords[i] * self.strides[i];
        }
        index
    }

    #[inline]
    fn dim_sizes(&self) -> &[u64] {
        &self.sizes
    }

    #[inline]
    fn strides(&self) -> &[u64] {
        &self.strides
    }
}

// =============================================================================
// Coords Extension Trait
// =============================================================================

/// Extension methods for coordinate vectors.
pub trait CoordsExt {
    /// Get the first coordinate (dimension 0).
    fn first_coord(&self) -> u64;

    /// Get the second coordinate (dimension 1), if present.
    fn second_coord(&self) -> Option<u64>;

    /// Convert to a 2-tuple if exactly 2 dimensions.
    fn as_pair(&self) -> Option<(u64, u64)>;
}

impl CoordsExt for Coords {
    #[inline]
    fn first_coord(&self) -> u64 {
        self[0]
    }

    #[inline]
    fn second_coord(&self) -> Option<u64> {
        self.get(1).copied()
    }

    #[inline]
    fn as_pair(&self) -> Option<(u64, u64)> {
        if self.len() == 2 {
            Some((self[0], self[1]))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SingleDim Tests
    // =========================================================================

    #[test]
    fn test_single_dim_new() {
        let space = SingleDim::new(100);
        assert_eq!(space.size(), 100);
        assert_eq!(space.num_dimensions(), 1);
        assert_eq!(space.total_size(), 100);
    }

    #[test]
    #[should_panic(expected = "size must be greater than 0")]
    fn test_single_dim_zero_panics() {
        SingleDim::new(0);
    }

    #[test]
    fn test_single_dim_coords() {
        let space = SingleDim::new(100);

        assert_eq!(space.index_to_coords(0), 0);
        assert_eq!(space.index_to_coords(42), 42);
        assert_eq!(space.index_to_coords(99), 99);

        assert_eq!(space.coords_to_index(&0), 0);
        assert_eq!(space.coords_to_index(&42), 42);
        assert_eq!(space.coords_to_index(&99), 99);
    }

    #[test]
    fn test_single_dim_dim_size() {
        let space = SingleDim::new(100);
        assert_eq!(space.dim_size(0), 100);
    }

    // =========================================================================
    // TwoDim Tests
    // =========================================================================

    #[test]
    fn test_two_dim_new() {
        let space = TwoDim::new(100, 50);
        assert_eq!(space.primary_size(), 100);
        assert_eq!(space.secondary_size(), 50);
        assert_eq!(space.num_dimensions(), 2);
        assert_eq!(space.total_size(), 5000);
    }

    #[test]
    #[should_panic(expected = "primary_size must be greater than 0")]
    fn test_two_dim_zero_primary_panics() {
        TwoDim::new(0, 50);
    }

    #[test]
    #[should_panic(expected = "secondary_size must be greater than 0")]
    fn test_two_dim_zero_secondary_panics() {
        TwoDim::new(100, 0);
    }

    #[test]
    fn test_two_dim_coords() {
        let space = TwoDim::new(10, 5);

        // Index 0 -> (0, 0)
        assert_eq!(space.index_to_coords(0), (0, 0));
        // Index 1 -> (0, 1)
        assert_eq!(space.index_to_coords(1), (0, 1));
        // Index 5 -> (1, 0)
        assert_eq!(space.index_to_coords(5), (1, 0));
        // Index 6 -> (1, 1)
        assert_eq!(space.index_to_coords(6), (1, 1));
        // Index 49 -> (9, 4)
        assert_eq!(space.index_to_coords(49), (9, 4));

        // Reverse
        assert_eq!(space.coords_to_index(&(0, 0)), 0);
        assert_eq!(space.coords_to_index(&(0, 1)), 1);
        assert_eq!(space.coords_to_index(&(1, 0)), 5);
        assert_eq!(space.coords_to_index(&(1, 1)), 6);
        assert_eq!(space.coords_to_index(&(9, 4)), 49);
    }

    #[test]
    fn test_two_dim_dim_sizes() {
        let space = TwoDim::new(100, 50);
        assert_eq!(space.dim_size(0), 100);
        assert_eq!(space.dim_size(1), 50);
        assert_eq!(space.dim_sizes(), &[100, 50]);
        assert_eq!(space.strides(), &[50, 1]);
    }

    // =========================================================================
    // MultiDim Tests
    // =========================================================================

    #[test]
    fn test_multi_dim_new() {
        let space = MultiDim::new(&[10, 20, 30]);
        assert_eq!(space.num_dimensions(), 3);
        assert_eq!(space.total_size(), 6000);
        assert_eq!(space.dim_sizes(), &[10, 20, 30]);
        assert_eq!(space.strides(), &[600, 30, 1]);
    }

    #[test]
    #[should_panic(expected = "sizes must not be empty")]
    fn test_multi_dim_empty_panics() {
        MultiDim::new(&[]);
    }

    #[test]
    #[should_panic(expected = "all sizes must be greater than 0")]
    fn test_multi_dim_zero_panics() {
        MultiDim::new(&[10, 0, 30]);
    }

    #[test]
    fn test_multi_dim_coords() {
        let space = MultiDim::new(&[10, 20, 30]);

        // Index 0 -> [0, 0, 0]
        let coords = space.index_to_coords(0);
        assert_eq!(&coords[..], &[0, 0, 0]);

        // Index 1 -> [0, 0, 1]
        let coords = space.index_to_coords(1);
        assert_eq!(&coords[..], &[0, 0, 1]);

        // Index 30 -> [0, 1, 0]
        let coords = space.index_to_coords(30);
        assert_eq!(&coords[..], &[0, 1, 0]);

        // Index 600 -> [1, 0, 0]
        let coords = space.index_to_coords(600);
        assert_eq!(&coords[..], &[1, 0, 0]);

        // Index 631 -> [1, 1, 1]
        let coords = space.index_to_coords(631);
        assert_eq!(&coords[..], &[1, 1, 1]);

        // Reverse
        let coords: Coords = smallvec::smallvec![0, 0, 0];
        assert_eq!(space.coords_to_index(&coords), 0);

        let coords: Coords = smallvec::smallvec![1, 2, 3];
        assert_eq!(space.coords_to_index(&coords), 1 * 600 + 2 * 30 + 3);
    }

    #[test]
    fn test_multi_dim_1d() {
        // MultiDim with 1 dimension should behave like SingleDim
        let space = MultiDim::new(&[100]);
        assert_eq!(space.num_dimensions(), 1);
        assert_eq!(space.total_size(), 100);

        let coords = space.index_to_coords(42);
        assert_eq!(&coords[..], &[42]);

        let coords: Coords = smallvec::smallvec![42];
        assert_eq!(space.coords_to_index(&coords), 42);
    }

    #[test]
    fn test_multi_dim_2d() {
        // MultiDim with 2 dimensions should behave like TwoDim
        let space = MultiDim::new(&[10, 5]);
        assert_eq!(space.num_dimensions(), 2);
        assert_eq!(space.total_size(), 50);

        let coords = space.index_to_coords(6);
        assert_eq!(&coords[..], &[1, 1]);

        let coords: Coords = smallvec::smallvec![1, 1];
        assert_eq!(space.coords_to_index(&coords), 6);
    }

    #[test]
    fn test_coords_ext() {
        let coords: Coords = smallvec::smallvec![10, 20];
        assert_eq!(coords.first_coord(), 10);
        assert_eq!(coords.second_coord(), Some(20));
        assert_eq!(coords.as_pair(), Some((10, 20)));

        let coords: Coords = smallvec::smallvec![10];
        assert_eq!(coords.first_coord(), 10);
        assert_eq!(coords.second_coord(), None);
        assert_eq!(coords.as_pair(), None);

        let coords: Coords = smallvec::smallvec![10, 20, 30];
        assert_eq!(coords.first_coord(), 10);
        assert_eq!(coords.second_coord(), Some(20));
        assert_eq!(coords.as_pair(), None);
    }
}
