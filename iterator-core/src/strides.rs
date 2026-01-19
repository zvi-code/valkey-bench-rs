//! Optimized stride computation for bitmap index calculation.
//!
//! This module provides const generic and dynamic stride types for efficient
//! bitmap index computation. The const generic version eliminates bounds checks
//! and enables loop unrolling for common dimension counts (≤4).

/// Trait for stride-based index computation.
///
/// Abstracts over const generic and dynamic stride implementations,
/// allowing the same interface for both optimized and fallback cases.
pub trait Strides {
    /// Compute the bitmap index from coordinates.
    ///
    /// # Arguments
    ///
    /// * `coords` - The coordinates (must have at least as many elements as strides)
    ///
    /// # Returns
    ///
    /// The computed bitmap index.
    fn compute_index(&self, coords: &[u64]) -> usize;

    /// Get the number of dimensions covered by these strides.
    fn len(&self) -> usize;

    /// Check if strides are empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the stride values as a slice.
    fn as_slice(&self) -> &[u64];
}

/// Fixed-size stride array with const generic parameter.
///
/// Optimized for compile-time known dimension counts. The compiler can
/// unroll loops and eliminate bounds checks when N is known.
///
/// # Type Parameters
///
/// * `N` - The number of dimensions (strides)
///
/// # Examples
///
/// ```ignore
/// use iterator_core::stride::StrideArray;
///
/// // Create strides for 2 dimensions with sizes [100, 200]
/// let strides = StrideArray::<2>::from_sizes(&[100, 200]);
/// assert_eq!(strides.compute_index(&[5, 10]), 5 * 200 + 10);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrideArray<const N: usize> {
    strides: [u64; N],
}

impl<const N: usize> StrideArray<N> {
    /// Create a new stride array from precomputed strides.
    ///
    /// # Arguments
    ///
    /// * `strides` - The precomputed stride values
    #[inline]
    pub const fn new(strides: [u64; N]) -> Self {
        Self { strides }
    }

    /// Create strides from dimension sizes.
    ///
    /// Computes strides where `strides[i] = product(sizes[i+1..N])`.
    ///
    /// # Arguments
    ///
    /// * `sizes` - The dimension sizes (must have at least N elements)
    ///
    /// # Panics
    ///
    /// Panics if `sizes` has fewer than N elements.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let strides = StrideArray::<3>::from_sizes(&[100, 200, 50]);
    /// // strides = [200*50, 50, 1] = [10000, 50, 1]
    /// ```
    pub fn from_sizes(sizes: &[u64]) -> Self {
        assert!(
            sizes.len() >= N,
            "sizes must have at least {} elements, got {}",
            N,
            sizes.len()
        );

        let mut strides = [0u64; N];
        if N > 0 {
            strides[N - 1] = 1;
            for i in (0..N - 1).rev() {
                strides[i] = strides[i + 1] * sizes[i + 1];
            }
        }
        Self { strides }
    }

    /// Get the stride at the given index.
    ///
    /// # Safety
    ///
    /// This method uses unchecked indexing for performance.
    /// The caller must ensure `index < N`.
    #[inline]
    pub unsafe fn get_unchecked(&self, index: usize) -> u64 {
        debug_assert!(index < N, "index {} out of bounds for StrideArray<{}>", index, N);
        *self.strides.get_unchecked(index)
    }

    /// Get the stride at the given index with bounds checking.
    #[inline]
    pub fn get(&self, index: usize) -> Option<u64> {
        self.strides.get(index).copied()
    }
}

impl<const N: usize> Strides for StrideArray<N> {
    #[inline]
    fn compute_index(&self, coords: &[u64]) -> usize {
        debug_assert!(
            coords.len() >= N,
            "coords must have at least {} elements, got {}",
            N,
            coords.len()
        );

        let mut index = 0u64;
        // The compiler can unroll this loop when N is known at compile time
        for i in 0..N {
            // SAFETY: We've asserted coords.len() >= N and i < N
            unsafe {
                index += coords.get_unchecked(i) * self.strides.get_unchecked(i);
            }
        }
        index as usize
    }

    #[inline]
    fn len(&self) -> usize {
        N
    }

    #[inline]
    fn as_slice(&self) -> &[u64] {
        &self.strides
    }
}

/// Dynamic stride storage for runtime-determined dimensions.
///
/// Used as a fallback when the number of dimensions exceeds the const generic
/// threshold or is not known at compile time.
///
/// # Examples
///
/// ```ignore
/// use iterator_core::stride::DynamicStrides;
///
/// // Create strides for 6 dimensions
/// let strides = DynamicStrides::from_sizes(&[10, 20, 30, 40, 50, 60], 6);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicStrides {
    strides: Vec<u64>,
}

impl DynamicStrides {
    /// Create a new dynamic stride storage from precomputed strides.
    ///
    /// # Arguments
    ///
    /// * `strides` - The precomputed stride values
    #[inline]
    pub fn new(strides: Vec<u64>) -> Self {
        Self { strides }
    }

    /// Create strides from dimension sizes.
    ///
    /// Computes strides where `strides[i] = product(sizes[i+1..prefix_dims])`.
    ///
    /// # Arguments
    ///
    /// * `sizes` - The dimension sizes
    /// * `prefix_dims` - Number of dimensions to compute strides for
    ///
    /// # Panics
    ///
    /// Panics if `prefix_dims` exceeds `sizes.len()`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let strides = DynamicStrides::from_sizes(&[100, 200, 50], 3);
    /// // strides = [200*50, 50, 1] = [10000, 50, 1]
    /// ```
    pub fn from_sizes(sizes: &[u64], prefix_dims: usize) -> Self {
        assert!(
            prefix_dims <= sizes.len(),
            "prefix_dims ({}) exceeds sizes length ({})",
            prefix_dims,
            sizes.len()
        );

        if prefix_dims == 0 {
            return Self { strides: vec![] };
        }

        let mut strides = vec![1u64; prefix_dims];
        for i in (0..prefix_dims - 1).rev() {
            strides[i] = strides[i + 1] * sizes[i + 1];
        }
        Self { strides }
    }
}

impl Strides for DynamicStrides {
    #[inline]
    fn compute_index(&self, coords: &[u64]) -> usize {
        let prefix_dims = self.strides.len();
        debug_assert!(
            coords.len() >= prefix_dims,
            "coords must have at least {} elements, got {}",
            prefix_dims,
            coords.len()
        );

        let mut index = 0u64;
        for i in 0..prefix_dims {
            index += coords[i] * self.strides[i];
        }
        index as usize
    }

    #[inline]
    fn len(&self) -> usize {
        self.strides.len()
    }

    #[inline]
    fn as_slice(&self) -> &[u64] {
        &self.strides
    }
}

/// Enum wrapper for optimized stride dispatch.
///
/// Uses const generic `StrideArray` for common cases (≤4 dimensions)
/// and falls back to `DynamicStrides` for larger dimension counts.
///
/// This allows the compiler to optimize the common cases while still
/// supporting arbitrary dimension counts.
#[derive(Clone, Debug)]
pub enum OptimizedStrides {
    /// 1 dimension - most common single-dimension case
    Fixed1(StrideArray<1>),
    /// 2 dimensions - common 2D case
    Fixed2(StrideArray<2>),
    /// 3 dimensions - common 3D case
    Fixed3(StrideArray<3>),
    /// 4 dimensions - threshold for SmallVec inline storage
    Fixed4(StrideArray<4>),
    /// 5+ dimensions - dynamic fallback
    Dynamic(DynamicStrides),
}

impl OptimizedStrides {
    /// Create optimized strides from dimension sizes.
    ///
    /// Automatically selects the appropriate storage based on `prefix_dims`:
    /// - 1-4 dimensions: Uses const generic `StrideArray`
    /// - 5+ dimensions: Uses `DynamicStrides`
    ///
    /// # Arguments
    ///
    /// * `sizes` - The dimension sizes
    /// * `prefix_dims` - Number of dimensions to compute strides for
    ///
    /// # Panics
    ///
    /// Panics if `prefix_dims` is 0 or exceeds `sizes.len()`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Uses StrideArray<2> internally
    /// let strides = OptimizedStrides::from_sizes(&[100, 200], 2);
    ///
    /// // Uses DynamicStrides internally
    /// let strides = OptimizedStrides::from_sizes(&[10, 20, 30, 40, 50, 60], 6);
    /// ```
    pub fn from_sizes(sizes: &[u64], prefix_dims: usize) -> Self {
        assert!(prefix_dims > 0, "prefix_dims must be greater than 0");
        assert!(
            prefix_dims <= sizes.len(),
            "prefix_dims ({}) exceeds sizes length ({})",
            prefix_dims,
            sizes.len()
        );

        match prefix_dims {
            1 => OptimizedStrides::Fixed1(StrideArray::<1>::from_sizes(sizes)),
            2 => OptimizedStrides::Fixed2(StrideArray::<2>::from_sizes(sizes)),
            3 => OptimizedStrides::Fixed3(StrideArray::<3>::from_sizes(sizes)),
            4 => OptimizedStrides::Fixed4(StrideArray::<4>::from_sizes(sizes)),
            _ => OptimizedStrides::Dynamic(DynamicStrides::from_sizes(sizes, prefix_dims)),
        }
    }

    /// Get the number of dimensions.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            OptimizedStrides::Fixed1(_) => 1,
            OptimizedStrides::Fixed2(_) => 2,
            OptimizedStrides::Fixed3(_) => 3,
            OptimizedStrides::Fixed4(_) => 4,
            OptimizedStrides::Dynamic(d) => d.len(),
        }
    }

    /// Check if strides are empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Strides for OptimizedStrides {
    #[inline]
    fn compute_index(&self, coords: &[u64]) -> usize {
        match self {
            OptimizedStrides::Fixed1(s) => s.compute_index(coords),
            OptimizedStrides::Fixed2(s) => s.compute_index(coords),
            OptimizedStrides::Fixed3(s) => s.compute_index(coords),
            OptimizedStrides::Fixed4(s) => s.compute_index(coords),
            OptimizedStrides::Dynamic(s) => s.compute_index(coords),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        OptimizedStrides::len(self)
    }

    #[inline]
    fn as_slice(&self) -> &[u64] {
        match self {
            OptimizedStrides::Fixed1(s) => s.as_slice(),
            OptimizedStrides::Fixed2(s) => s.as_slice(),
            OptimizedStrides::Fixed3(s) => s.as_slice(),
            OptimizedStrides::Fixed4(s) => s.as_slice(),
            OptimizedStrides::Dynamic(s) => s.as_slice(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Sanity Tests: Basic Construction
    // =========================================================================

    #[test]
    fn test_stride_array_from_sizes() {
        // 1D
        let strides = StrideArray::<1>::from_sizes(&[100]);
        assert_eq!(strides.as_slice(), &[1]);

        // 2D
        let strides = StrideArray::<2>::from_sizes(&[100, 200]);
        assert_eq!(strides.as_slice(), &[200, 1]);

        // 3D
        let strides = StrideArray::<3>::from_sizes(&[100, 200, 50]);
        assert_eq!(strides.as_slice(), &[10000, 50, 1]);

        // 4D: strides = [20*30*40, 30*40, 40, 1] = [24000, 1200, 40, 1]
        let strides = StrideArray::<4>::from_sizes(&[10, 20, 30, 40]);
        assert_eq!(strides.as_slice(), &[24000, 1200, 40, 1]);
    }

    #[test]
    fn test_stride_array_compute_index() {
        // 1D
        let strides = StrideArray::<1>::from_sizes(&[100]);
        assert_eq!(strides.compute_index(&[0]), 0);
        assert_eq!(strides.compute_index(&[42]), 42);

        // 2D
        let strides = StrideArray::<2>::from_sizes(&[100, 200]);
        assert_eq!(strides.compute_index(&[0, 0]), 0);
        assert_eq!(strides.compute_index(&[1, 0]), 200);
        assert_eq!(strides.compute_index(&[5, 10]), 5 * 200 + 10);

        // 3D
        let strides = StrideArray::<3>::from_sizes(&[100, 200, 50]);
        assert_eq!(strides.compute_index(&[0, 0, 0]), 0);
        assert_eq!(strides.compute_index(&[2, 3, 4]), 2 * 10000 + 3 * 50 + 4);
    }

    #[test]
    fn test_dynamic_strides_from_sizes() {
        let strides = DynamicStrides::from_sizes(&[100, 200, 50], 3);
        assert_eq!(strides.as_slice(), &[10000, 50, 1]);

        // Partial prefix
        let strides = DynamicStrides::from_sizes(&[100, 200, 50], 2);
        assert_eq!(strides.as_slice(), &[200, 1]);
    }

    #[test]
    fn test_dynamic_strides_compute_index() {
        let strides = DynamicStrides::from_sizes(&[100, 200, 50], 3);
        assert_eq!(strides.compute_index(&[0, 0, 0]), 0);
        assert_eq!(strides.compute_index(&[2, 3, 4]), 2 * 10000 + 3 * 50 + 4);
    }

    // =========================================================================
    // Sanity Tests: OptimizedStrides Variant Selection
    // =========================================================================

    #[test]
    fn test_optimized_strides_variant_selection() {
        // 1D -> Fixed1
        let strides = OptimizedStrides::from_sizes(&[100], 1);
        assert!(matches!(strides, OptimizedStrides::Fixed1(_)));
        assert_eq!(strides.len(), 1);

        // 2D -> Fixed2
        let strides = OptimizedStrides::from_sizes(&[100, 200], 2);
        assert!(matches!(strides, OptimizedStrides::Fixed2(_)));
        assert_eq!(strides.len(), 2);

        // 3D -> Fixed3
        let strides = OptimizedStrides::from_sizes(&[100, 200, 50], 3);
        assert!(matches!(strides, OptimizedStrides::Fixed3(_)));
        assert_eq!(strides.len(), 3);

        // 4D -> Fixed4
        let strides = OptimizedStrides::from_sizes(&[10, 20, 30, 40], 4);
        assert!(matches!(strides, OptimizedStrides::Fixed4(_)));
        assert_eq!(strides.len(), 4);

        // 5D -> Dynamic
        let strides = OptimizedStrides::from_sizes(&[10, 20, 30, 40, 50], 5);
        assert!(matches!(strides, OptimizedStrides::Dynamic(_)));
        assert_eq!(strides.len(), 5);
    }

    // =========================================================================
    // Sanity Tests: Accessor Methods
    // =========================================================================

    #[test]
    fn test_stride_array_get() {
        let strides = StrideArray::<3>::from_sizes(&[100, 200, 50]);
        assert_eq!(strides.get(0), Some(10000));
        assert_eq!(strides.get(1), Some(50));
        assert_eq!(strides.get(2), Some(1));
        assert_eq!(strides.get(3), None);
    }

    #[test]
    fn test_stride_array_get_unchecked() {
        let strides = StrideArray::<3>::from_sizes(&[100, 200, 50]);
        unsafe {
            assert_eq!(strides.get_unchecked(0), 10000);
            assert_eq!(strides.get_unchecked(1), 50);
            assert_eq!(strides.get_unchecked(2), 1);
        }
    }

    // =========================================================================
    // Sanity Tests: Edge Cases
    // =========================================================================

    #[test]
    fn test_dynamic_strides_empty() {
        let strides = DynamicStrides::from_sizes(&[100, 200], 0);
        assert!(strides.is_empty());
        assert_eq!(strides.len(), 0);
    }

    #[test]
    fn test_stride_trait_len() {
        let fixed: &dyn Strides = &StrideArray::<2>::from_sizes(&[100, 200]);
        assert_eq!(fixed.len(), 2);

        let dynamic: &dyn Strides = &DynamicStrides::from_sizes(&[100, 200, 50], 3);
        assert_eq!(dynamic.len(), 3);
    }
}

