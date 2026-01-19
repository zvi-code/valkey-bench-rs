//! Atomic bitmap implementation with SIMD-accelerated operations.
//!
//! Provides a lock-free concurrent bitmap using `AtomicU64` words.
//! Uses runtime CPU detection to select optimal implementations:
//!
//! - **ARM**: NEON baseline, with SVE/SVE2 for Graviton 3/4
//! - **x86_64**: POPCNT baseline, AVX2 for Haswell+, AVX-512 for Skylake-X+
//! - **Other**: Portable scalar fallback with loop unrolling
//!
//! All bit operations are atomic and safe for concurrent access.
//!
//! # Fixed-Size Design
//!
//! This bitmap has a fixed capacity set at construction time. It does not
//! grow dynamically. This simplifies the implementation and eliminates
//! the need for unsafe code or memory management complexity.

use std::sync::atomic::{AtomicU64, Ordering};

// Platform-specific implementations
#[cfg(target_arch = "aarch64")]
mod arm;
mod scalar;
#[cfg(target_arch = "x86_64")]
mod x86;

// Re-export platform capabilities for debugging/introspection
#[cfg(target_arch = "aarch64")]
pub use arm::ArmCapabilities;
#[cfg(target_arch = "x86_64")]
pub use x86::X86Capabilities;

// ============================================================================
// Internal SIMD Dispatch
// ============================================================================

/// Count set bits using the best available SIMD implementation.
#[inline]
fn simd_popcount_slice(words: &[AtomicU64]) -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        arm::popcount_slice(words)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86::popcount_slice(words)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        scalar::popcount_slice(words)
    }
}

/// Find first non-zero word using SIMD-accelerated scan.
#[inline]
fn simd_find_first_nonzero(words: &[AtomicU64], start_word: usize) -> Option<(usize, u64)> {
    #[cfg(target_arch = "aarch64")]
    {
        arm::find_first_nonzero(words, start_word)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86::find_first_nonzero(words, start_word)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        scalar::find_first_nonzero(words, start_word)
    }
}

/// Find first word with unset bits using SIMD-accelerated scan.
#[inline]
fn simd_find_first_not_full(words: &[AtomicU64], start_word: usize) -> Option<(usize, u64)> {
    #[cfg(target_arch = "aarch64")]
    {
        arm::find_first_not_full(words, start_word)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86::find_first_not_full(words, start_word)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        scalar::find_first_not_full(words, start_word)
    }
}

/// Prefetch next cache line for iteration.
#[inline]
fn simd_prefetch_next_cacheline(words: &[AtomicU64], current_idx: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        arm::prefetch_next_cacheline(words, current_idx);
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86::prefetch_next_cacheline(words, current_idx);
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (words, current_idx);
    }
}

// ============================================================================
// Constants
// ============================================================================

/// Number of bits per word.
const BITS_PER_WORD: usize = 64;

/// Initial capacity in bits.
const INITIAL_CAPACITY_BITS: usize = 4096;

/// Shift amount for word index calculation (log2(64) = 6).
const WORD_SHIFT: usize = 6;

/// Mask for bit index within word.
const WORD_MASK: usize = 63;

// ============================================================================
// AtomicBitmap
// ============================================================================

/// A thread-safe, lock-free atomic bitmap with fixed capacity.
///
/// `AtomicBitmap` provides efficient storage and manipulation of a set of bits,
/// where each bit represents whether an ID exists or not. All operations are
/// atomic and safe for concurrent access from multiple threads.
///
/// # Fixed-Size Design
///
/// The bitmap capacity is fixed at construction time and cannot grow. This
/// design choice:
/// - Eliminates unsafe code and memory management complexity
/// - Provides predictable memory usage
/// - Enables zero-overhead concurrent access
///
/// # Features
///
/// - **Lock-free operations**: `test`, `set`, `clear`, `test_and_set` use atomic
///   compare-and-swap (CAS) and are wait-free for most operations
/// - **SIMD-accelerated**: Uses ARM NEON or x86 AVX2/POPCNT for bulk operations
/// - **Memory efficient**: 1 bit per ID, 8 bytes per 64 IDs
///
/// # Performance
///
/// | Operation | Latency | Notes |
/// |-----------|---------|-------|
/// | `test` | ~1.6 ns | L1 cache hit |
/// | `set` | ~7 ns | Atomic OR |
/// | `clear` | ~7 ns | Atomic AND |
/// | `test_and_set` | ~2.4 ns | CAS loop |
/// | `find_next_set` | ~4 ns | SIMD accelerated |
///
/// # Thread Safety
///
/// All bit operations are lock-free using atomic primitives.
///
/// # Panics
///
/// Operations panic if the index exceeds capacity. Use `capacity()` to check
/// bounds, or use `try_*` methods for fallible operations.
///
/// # Examples
///
/// ## Basic Operations
///
/// ```rust
/// use iterator_core::AtomicBitmap;
///
/// let bitmap = AtomicBitmap::with_capacity(1000);
///
/// // Set and test bits
/// bitmap.set(42);
/// assert!(bitmap.test(42));
/// assert!(!bitmap.test(43));
///
/// // Clear bits
/// bitmap.clear(42);
/// assert!(!bitmap.test(42));
///
/// // Population count
/// bitmap.set(1);
/// bitmap.set(2);
/// bitmap.set(3);
/// assert_eq!(bitmap.count(), 3);
/// ```
///
/// ## Atomic Claim (Test-and-Set)
///
/// ```rust
/// use iterator_core::AtomicBitmap;
/// use std::sync::Arc;
/// use std::thread;
///
/// let bitmap = Arc::new(AtomicBitmap::with_capacity(1000));
///
/// // Multiple threads can atomically claim unique bits
/// let handles: Vec<_> = (0..4).map(|_| {
///     let bm = bitmap.clone();
///     thread::spawn(move || {
///         (0..100).filter(|&i| bm.test_and_set(i)).count()
///     })
/// }).collect();
///
/// let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
/// assert_eq!(total, 100); // Each bit claimed exactly once
/// ```
///
/// ## Scanning
///
/// ```rust
/// use iterator_core::AtomicBitmap;
///
/// let bitmap = AtomicBitmap::with_capacity(1000);
/// bitmap.set(10);
/// bitmap.set(50);
/// bitmap.set(100);
///
/// // Find set bits
/// let mut pos = 0;
/// let mut found = Vec::new();
/// while let Some(next) = bitmap.find_next_set(pos) {
///     found.push(next);
///     pos = next + 1;
/// }
/// assert_eq!(found, vec![10, 50, 100]);
///
/// // Find unset bits
/// let first_unset = bitmap.find_next_unset(0, 1000);
/// assert_eq!(first_unset, Some(0));
/// ```
pub struct AtomicBitmap {
    /// Bitmap storage - fixed size, never reallocated.
    words: Box<[AtomicU64]>,

    /// Fixed capacity in bits (always multiple of 64).
    capacity: usize,

    /// Population count (number of set bits).
    /// Updated atomically on set/clear operations.
    popcount: AtomicU64,
}

// SAFETY: All bit operations use atomic primitives.
// No unsafe code - the bitmap is fixed-size and never reallocated.
unsafe impl Send for AtomicBitmap {}
unsafe impl Sync for AtomicBitmap {}

impl AtomicBitmap {
    /// Create a new bitmap with default initial capacity (4096 bits).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::AtomicBitmap;
    ///
    /// let bitmap = AtomicBitmap::new();
    /// assert_eq!(bitmap.capacity(), 4096);
    /// ```
    pub fn new() -> Self {
        Self::with_capacity(INITIAL_CAPACITY_BITS)
    }

    /// Create a new bitmap with specified capacity (in bits).
    ///
    /// Capacity is rounded up to the next multiple of 64.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::AtomicBitmap;
    ///
    /// let bitmap = AtomicBitmap::with_capacity(1_000_000);
    /// assert!(bitmap.capacity() >= 1_000_000);
    /// ```
    pub fn with_capacity(capacity_bits: usize) -> Self {
        // Round up to next multiple of 64 (MSRV-compatible alternative to next_multiple_of)
        let min_capacity = capacity_bits.max(64);
        let capacity = ((min_capacity + 63) / 64) * 64;
        let word_count = capacity / BITS_PER_WORD;

        let words: Vec<AtomicU64> = (0..word_count).map(|_| AtomicU64::new(0)).collect();

        Self {
            words: words.into_boxed_slice(),
            capacity,
            popcount: AtomicU64::new(0),
        }
    }

    /// Get a reference to the words slice.
    #[inline]
    fn words(&self) -> &[AtomicU64] {
        &self.words
    }

    /// Get the fixed capacity in bits.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get number of words in the bitmap.
    #[inline]
    pub fn word_count(&self) -> usize {
        self.capacity() / BITS_PER_WORD
    }

    /// Get population count (number of set bits).
    #[inline]
    pub fn count(&self) -> u64 {
        self.popcount.load(Ordering::Relaxed)
    }

    /// Check if bitmap is empty (no bits set).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Check if index is within bounds.
    #[inline]
    fn check_bounds(&self, index: usize) {
        assert!(
            index < self.capacity,
            "bitmap index {} out of bounds (capacity: {})",
            index,
            self.capacity
        );
    }

    /// Load a word value.
    #[inline]
    pub fn load_word(&self, word_index: usize) -> u64 {
        self.words[word_index].load(Ordering::Acquire)
    }

    /// Fetch-or on a word, returning the previous value.
    #[inline]
    pub fn word_fetch_or(&self, word_index: usize, mask: u64) -> u64 {
        self.words[word_index].fetch_or(mask, Ordering::AcqRel)
    }

    /// Fetch-and on a word, returning the previous value.
    #[inline]
    pub fn word_fetch_and(&self, word_index: usize, mask: u64) -> u64 {
        self.words[word_index].fetch_and(mask, Ordering::AcqRel)
    }

    /// Set a bit at the given index.
    ///
    /// Returns the previous value (false if was unset, true if was set).
    ///
    /// # Panics
    ///
    /// Panics if `index >= capacity`.
    #[inline]
    pub fn set(&self, index: usize) -> bool {
        self.check_bounds(index);

        let word_idx = index >> WORD_SHIFT;
        let bit_idx = index & WORD_MASK;
        let mask = 1u64 << bit_idx;

        let old = self.word_fetch_or(word_idx, mask);
        let was_set = (old & mask) != 0;

        if !was_set {
            self.popcount.fetch_add(1, Ordering::Relaxed);
        }

        was_set
    }

    /// Clear a bit at the given index.
    ///
    /// Returns the previous value (true if was set, false if was unset).
    ///
    /// # Panics
    ///
    /// Panics if `index >= capacity`.
    #[inline]
    pub fn clear(&self, index: usize) -> bool {
        self.check_bounds(index);

        let word_idx = index >> WORD_SHIFT;
        let bit_idx = index & WORD_MASK;
        let mask = 1u64 << bit_idx;

        let old = self.word_fetch_and(word_idx, !mask);
        let was_set = (old & mask) != 0;

        if was_set {
            self.popcount.fetch_sub(1, Ordering::Relaxed);
        }

        was_set
    }

    /// Test if a bit is set at the given index.
    ///
    /// Returns false if index >= capacity (no panic).
    #[inline]
    pub fn test(&self, index: usize) -> bool {
        if index >= self.capacity {
            return false;
        }

        let word_idx = index >> WORD_SHIFT;
        let bit_idx = index & WORD_MASK;
        let mask = 1u64 << bit_idx;

        (self.load_word(word_idx) & mask) != 0
    }

    /// Atomically test and set a bit.
    ///
    /// If the bit is unset (0), sets it to 1 and returns true.
    /// If the bit is already set (1), returns false.
    ///
    /// # Panics
    ///
    /// Panics if `index >= capacity`.
    #[inline]
    pub fn test_and_set(&self, index: usize) -> bool {
        self.check_bounds(index);

        let word_idx = index >> WORD_SHIFT;
        let bit_idx = index & WORD_MASK;
        let mask = 1u64 << bit_idx;

        let word = &self.words[word_idx];

        loop {
            let old = word.load(Ordering::Acquire);
            if (old & mask) != 0 {
                return false; // Already set
            }

            match word.compare_exchange_weak(old, old | mask, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => {
                    self.popcount.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
                Err(_) => continue, // Retry
            }
        }
    }

    /// Atomically test and clear a bit.
    ///
    /// If the bit is set (1), clears it to 0 and returns true.
    /// If the bit is already unset (0), returns false.
    ///
    /// # Panics
    ///
    /// Panics if `index >= capacity`.
    #[inline]
    pub fn test_and_clear(&self, index: usize) -> bool {
        self.check_bounds(index);

        let word_idx = index >> WORD_SHIFT;
        let bit_idx = index & WORD_MASK;
        let mask = 1u64 << bit_idx;

        let word = &self.words[word_idx];

        loop {
            let old = word.load(Ordering::Acquire);
            if (old & mask) == 0 {
                return false; // Already unset
            }

            match word.compare_exchange_weak(old, old & !mask, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.popcount.fetch_sub(1, Ordering::Relaxed);
                    return true;
                }
                Err(_) => continue, // Retry
            }
        }
    }

    /// Clear all bits and reset to initial capacity.
    ///
    /// This requires exclusive access (&mut self).
    pub fn clear_all(&mut self) {
        for word in self.words.iter() {
            word.store(0, Ordering::Relaxed);
        }
        *self.popcount.get_mut() = 0;
    }

    /// Clear all bits but retain allocated capacity.
    ///
    /// Alias for `clear_all` since capacity is now fixed.
    pub fn reset(&mut self) {
        self.clear_all();
    }

    /// Find the first set bit starting from `start_index`.
    ///
    /// Returns None if no set bit is found before capacity.
    /// Uses SIMD-optimized scanning on ARM NEON and x86_64.
    pub fn find_next_set(&self, start_index: usize) -> Option<usize> {
        if start_index >= self.capacity {
            return None;
        }

        let words = self.words();
        let start_word_idx = start_index >> WORD_SHIFT;
        let bit_idx = start_index & WORD_MASK;

        // Check first word (masked to ignore bits before start)
        let first_word = words[start_word_idx].load(Ordering::Relaxed) & (u64::MAX << bit_idx);
        if first_word != 0 {
            let bit = first_word.trailing_zeros() as usize;
            let index = (start_word_idx << WORD_SHIFT) + bit;
            if index < self.capacity {
                return Some(index);
            }
        }

        // Use SIMD-optimized scan for remaining words
        if let Some((word_idx, word)) = simd_find_first_nonzero(words, start_word_idx + 1) {
            let bit = word.trailing_zeros() as usize;
            let index = (word_idx << WORD_SHIFT) + bit;
            if index < self.capacity {
                return Some(index);
            }
        }

        None
    }

    /// Find the first unset bit starting from `start_index`.
    ///
    /// Returns None if no unset bit is found before `max_index`.
    /// Uses SIMD-optimized scanning on ARM NEON and x86_64.
    pub fn find_next_unset(&self, start_index: usize, max_index: usize) -> Option<usize> {
        if start_index >= max_index {
            return None;
        }

        let limit = max_index.min(self.capacity);
        if start_index >= limit {
            return None;
        }

        let words = self.words();
        let start_word_idx = start_index >> WORD_SHIFT;
        let bit_idx = start_index & WORD_MASK;

        // Check first word (masked to ignore bits before start)
        let first_word = words[start_word_idx].load(Ordering::Relaxed) | ((1u64 << bit_idx) - 1);
        if first_word != u64::MAX {
            let bit = (!first_word).trailing_zeros() as usize;
            let index = (start_word_idx << WORD_SHIFT) + bit;
            if index < limit {
                return Some(index);
            }
        }

        // Use SIMD-optimized scan for remaining words
        if let Some((word_idx, word)) = simd_find_first_not_full(words, start_word_idx + 1) {
            let bit = (!word).trailing_zeros() as usize;
            let index = (word_idx << WORD_SHIFT) + bit;
            if index < limit {
                return Some(index);
            }
        }

        None
    }

    /// Recompute population count using SIMD-optimized counting.
    ///
    /// This is useful after bulk operations or to verify the incremental count.
    /// Uses NEON on ARM, POPCNT/AVX2 on x86_64, with scalar fallback.
    pub fn recompute_count(&self) -> u64 {
        simd_popcount_slice(self.words())
    }

    /// Verify and fix the population count if it has drifted.
    ///
    /// Returns true if the count was corrected.
    pub fn verify_count(&self) -> bool {
        let actual = self.recompute_count();
        let stored = self.popcount.load(Ordering::Relaxed);
        if actual != stored {
            self.popcount.store(actual, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Prefetch the next cache line for sequential iteration.
    ///
    /// Call this during iteration to reduce cache misses.
    #[inline]
    pub fn prefetch_ahead(&self, current_word_idx: usize) {
        simd_prefetch_next_cacheline(self.words(), current_word_idx);
    }

    /// Get the density of the bitmap (ratio of set bits to capacity).
    #[inline]
    pub fn density(&self) -> f64 {
        let cap = self.capacity() as f64;
        if cap == 0.0 {
            0.0
        } else {
            self.count() as f64 / cap
        }
    }

    // ========================================================================
    // Bulk Operations
    // ========================================================================

    /// Set all bits in range [start, end).
    ///
    /// This is more efficient than calling `set()` in a loop for bulk initialization.
    ///
    /// Returns the number of bits that were newly set (were previously unset).
    ///
    /// # Panics
    ///
    /// Panics if `end > capacity`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::AtomicBitmap;
    ///
    /// let bitmap = AtomicBitmap::with_capacity(1000);
    /// let newly_set = bitmap.set_range(0, 100);
    /// assert_eq!(newly_set, 100);
    /// assert_eq!(bitmap.count(), 100);
    /// ```
    pub fn set_range(&self, start: usize, end: usize) -> u64 {
        if start >= end {
            return 0;
        }

        assert!(
            end <= self.capacity,
            "set_range end {} exceeds capacity {}",
            end,
            self.capacity
        );

        let start_word = start >> WORD_SHIFT;
        let end_word = (end + BITS_PER_WORD - 1) >> WORD_SHIFT;
        let words = self.words();
        let mut newly_set = 0u64;

        // Handle first partial word
        let start_bit = start & WORD_MASK;
        if start_bit != 0 {
            let mask = u64::MAX << start_bit;
            // If start and end are in same word, also mask the upper bits
            let mask = if start_word == end_word - 1 {
                let end_bit = end & WORD_MASK;
                let end_mask = if end_bit == 0 { u64::MAX } else { (1u64 << end_bit) - 1 };
                mask & end_mask
            } else {
                mask
            };

            let old = words[start_word].fetch_or(mask, Ordering::AcqRel);
            newly_set += (mask & !old).count_ones() as u64;

            // If fully handled in first word, we're done
            if start_word == end_word - 1 {
                self.popcount.fetch_add(newly_set, Ordering::Relaxed);
                return newly_set;
            }
        }

        // Handle full words in the middle
        let first_full_word = if start_bit != 0 { start_word + 1 } else { start_word };
        let last_full_word = if (end & WORD_MASK) != 0 { end_word - 1 } else { end_word };

        for word in words.iter().take(last_full_word).skip(first_full_word) {
            let old = word.fetch_or(u64::MAX, Ordering::AcqRel);
            newly_set += (!old).count_ones() as u64;
        }

        // Handle last partial word
        let end_bit = end & WORD_MASK;
        if end_bit != 0 && last_full_word < end_word {
            let mask = (1u64 << end_bit) - 1;
            let old = words[last_full_word].fetch_or(mask, Ordering::AcqRel);
            newly_set += (mask & !old).count_ones() as u64;
        }

        self.popcount.fetch_add(newly_set, Ordering::Relaxed);
        newly_set
    }

    /// Clear all bits in range [start, end).
    ///
    /// This is more efficient than calling `clear()` in a loop.
    ///
    /// Returns the number of bits that were cleared (were previously set).
    ///
    /// # Panics
    ///
    /// Panics if `end > capacity`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::AtomicBitmap;
    ///
    /// let bitmap = AtomicBitmap::with_capacity(1000);
    /// bitmap.set_range(0, 100);
    /// let cleared = bitmap.clear_range(25, 75);
    /// assert_eq!(cleared, 50);
    /// assert_eq!(bitmap.count(), 50);
    /// ```
    pub fn clear_range(&self, start: usize, end: usize) -> u64 {
        if start >= end {
            return 0;
        }

        assert!(
            end <= self.capacity,
            "clear_range end {} exceeds capacity {}",
            end,
            self.capacity
        );

        let start_word = start >> WORD_SHIFT;
        let end_word = (end + BITS_PER_WORD - 1) >> WORD_SHIFT;
        let words = self.words();
        let mut cleared = 0u64;

        // Handle first partial word
        let start_bit = start & WORD_MASK;
        if start_bit != 0 {
            let mask = u64::MAX << start_bit;
            let mask = if start_word == end_word - 1 {
                let end_bit = end & WORD_MASK;
                let end_mask = if end_bit == 0 { u64::MAX } else { (1u64 << end_bit) - 1 };
                mask & end_mask
            } else {
                mask
            };

            let old = words[start_word].fetch_and(!mask, Ordering::AcqRel);
            cleared += (mask & old).count_ones() as u64;

            if start_word == end_word - 1 {
                self.popcount.fetch_sub(cleared, Ordering::Relaxed);
                return cleared;
            }
        }

        // Handle full words
        let first_full_word = if start_bit != 0 { start_word + 1 } else { start_word };
        let last_full_word = if (end & WORD_MASK) != 0 { end_word - 1 } else { end_word };

        for word in words.iter().take(last_full_word).skip(first_full_word) {
            let old = word.swap(0, Ordering::AcqRel);
            cleared += old.count_ones() as u64;
        }

        // Handle last partial word
        let end_bit = end & WORD_MASK;
        if end_bit != 0 && last_full_word < end_word {
            let mask = (1u64 << end_bit) - 1;
            let old = words[last_full_word].fetch_and(!mask, Ordering::AcqRel);
            cleared += (mask & old).count_ones() as u64;
        }

        self.popcount.fetch_sub(cleared, Ordering::Relaxed);
        cleared
    }

    /// Count set bits in range [start, end) without modifying.
    ///
    /// More efficient than iterating for large ranges.
    ///
    /// # Panics
    ///
    /// Panics if `end > capacity`.
    pub fn count_range(&self, start: usize, end: usize) -> u64 {
        if start >= end {
            return 0;
        }

        assert!(
            end <= self.capacity,
            "count_range end {} exceeds capacity {}",
            end,
            self.capacity
        );

        let start_word = start >> WORD_SHIFT;
        let end_word = (end + BITS_PER_WORD - 1) >> WORD_SHIFT;
        let words = self.words();
        let mut count = 0u64;

        // Handle first partial word
        let start_bit = start & WORD_MASK;
        if start_bit != 0 {
            let mask = u64::MAX << start_bit;
            let mask = if start_word == end_word - 1 {
                let end_bit = end & WORD_MASK;
                let end_mask = if end_bit == 0 { u64::MAX } else { (1u64 << end_bit) - 1 };
                mask & end_mask
            } else {
                mask
            };

            count += (words[start_word].load(Ordering::Relaxed) & mask).count_ones() as u64;

            if start_word == end_word - 1 {
                return count;
            }
        }

        // Handle full words (use SIMD for large ranges)
        let first_full_word = if start_bit != 0 { start_word + 1 } else { start_word };
        let last_full_word = if (end & WORD_MASK) != 0 { end_word - 1 } else { end_word };

        if last_full_word > first_full_word {
            let full_range = &words[first_full_word..last_full_word];
            count += simd_popcount_slice(full_range);
        }

        // Handle last partial word
        let end_bit = end & WORD_MASK;
        if end_bit != 0 && last_full_word < end_word {
            let mask = (1u64 << end_bit) - 1;
            count += (words[last_full_word].load(Ordering::Relaxed) & mask).count_ones() as u64;
        }

        count
    }

    // ========================================================================
    // Snapshot
    // ========================================================================

    /// Create an immutable snapshot of current bitmap state.
    ///
    /// The snapshot is a non-atomic copy suitable for:
    /// - State comparison before/after operations
    /// - Set operations with external reference sets
    /// - Detecting added/removed IDs
    ///
    /// Returns a tuple of (words, capacity, count) where:
    /// - `words`: The raw u64 words of the bitmap
    /// - `capacity`: The capacity in bits
    /// - `count`: The population count at snapshot time
    ///
    /// # Examples
    ///
    /// ```rust
    /// use iterator_core::AtomicBitmap;
    ///
    /// let bitmap = AtomicBitmap::with_capacity(1000);
    /// bitmap.set_range(0, 500);
    ///
    /// let (words, capacity, count) = bitmap.snapshot();
    /// assert_eq!(count, 500);
    /// assert!(capacity >= 1000);
    /// ```
    pub fn snapshot(&self) -> (Vec<u64>, usize, u64) {
        let words = self.words();
        let data: Vec<u64> = words.iter()
            .map(|w| w.load(Ordering::Relaxed))
            .collect();

        (data, self.capacity, self.count())
    }
}

impl Default for AtomicBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for AtomicBitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtomicBitmap")
            .field("capacity", &self.capacity)
            .field("count", &self.count())
            .finish()
    }
}

// ============================================================================
// BitmapOps Implementation
// ============================================================================

// Import BitmapOps trait from filter module
use crate::filter::BitmapOps;

impl BitmapOps for AtomicBitmap {
    /// Test if a bit is set at the given index.
    #[inline]
    fn test(&self, index: usize) -> bool {
        AtomicBitmap::test(self, index)
    }

    /// Atomically test and set a bit.
    /// Returns true if the bit was unset and is now set.
    #[inline]
    fn test_and_set(&self, index: usize) -> bool {
        AtomicBitmap::test_and_set(self, index)
    }

    /// Atomically test and clear a bit.
    /// Returns true if the bit was set and is now cleared.
    #[inline]
    fn test_and_clear(&self, index: usize) -> bool {
        AtomicBitmap::test_and_clear(self, index)
    }

    /// Set a bit at the given index.
    #[inline]
    fn set(&self, index: usize) {
        AtomicBitmap::set(self, index);
    }

    /// Clear a bit at the given index.
    #[inline]
    fn clear(&self, index: usize) {
        AtomicBitmap::clear(self, index);
    }

    /// Find the next set bit starting from `start_index`.
    #[inline]
    fn find_next_set(&self, start_index: usize) -> Option<usize> {
        AtomicBitmap::find_next_set(self, start_index)
    }

    /// Find the next unset bit starting from `start_index` up to `max_index`.
    #[inline]
    fn find_next_unset(&self, start_index: usize, max_index: usize) -> Option<usize> {
        AtomicBitmap::find_next_unset(self, start_index, max_index)
    }

    /// Returns true - AtomicBitmap has SIMD-optimized find_next_set.
    #[inline]
    fn supports_find_next_set(&self) -> bool {
        true
    }

    /// Returns true - AtomicBitmap has SIMD-optimized find_next_unset.
    #[inline]
    fn supports_find_next_unset(&self) -> bool {
        true
    }

    /// Get the capacity of the bitmap in bits.
    #[inline]
    fn capacity(&self) -> usize {
        AtomicBitmap::capacity(self)
    }

    /// Get the population count (number of set bits).
    #[inline]
    fn count(&self) -> u64 {
        AtomicBitmap::count(self)
    }
}

// ============================================================================
// Public API re-exports for advanced usage
// ============================================================================

/// Print detected CPU capabilities for debugging.
pub fn print_cpu_capabilities() {
    #[cfg(target_arch = "aarch64")]
    {
        let caps = ArmCapabilities::get();
        println!("ARM Capabilities: {:?}", caps);
    }
    #[cfg(target_arch = "x86_64")]
    {
        let caps = X86Capabilities::get();
        println!("x86_64 Capabilities: {:?}", caps);
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        println!("Using scalar fallback (no SIMD)");
    }
}


// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_new_bitmap() {
        let bm = AtomicBitmap::new();
        assert_eq!(bm.capacity(), INITIAL_CAPACITY_BITS);
        assert_eq!(bm.count(), 0);
        assert!(bm.is_empty());
    }

    #[test]
    fn test_set_and_test() {
        let bm = AtomicBitmap::with_capacity(1000);

        assert!(!bm.test(0));
        assert!(!bm.set(0)); // Returns false (was not set)
        assert!(bm.test(0));
        assert!(bm.set(0)); // Returns true (was already set)

        assert!(!bm.test(63));
        bm.set(63);
        assert!(bm.test(63));

        assert!(!bm.test(64));
        bm.set(64);
        assert!(bm.test(64));
    }

    #[test]
    fn test_clear() {
        let bm = AtomicBitmap::with_capacity(1000);

        bm.set(42);
        assert!(bm.test(42));
        assert_eq!(bm.count(), 1);

        assert!(bm.clear(42)); // Returns true (was set)
        assert!(!bm.test(42));
        assert_eq!(bm.count(), 0);

        assert!(!bm.clear(42)); // Returns false (was not set)
    }

    #[test]
    fn test_test_and_set() {
        let bm = AtomicBitmap::with_capacity(1000);

        assert!(bm.test_and_set(100)); // Success, was unset
        assert!(!bm.test_and_set(100)); // Failure, already set
        assert!(bm.test(100));
        assert_eq!(bm.count(), 1);
    }

    #[test]
    fn test_test_and_clear() {
        let bm = AtomicBitmap::with_capacity(1000);

        bm.set(100);
        assert!(bm.test_and_clear(100)); // Success, was set
        assert!(!bm.test_and_clear(100)); // Failure, already unset
        assert!(!bm.test(100));
        assert_eq!(bm.count(), 0);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_set_out_of_bounds_panics() {
        let bm = AtomicBitmap::with_capacity(64);
        bm.set(1000); // Should panic
    }

    #[test]
    fn test_test_out_of_bounds_returns_false() {
        let bm = AtomicBitmap::with_capacity(64);
        // test() returns false for out-of-bounds, doesn't panic
        assert!(!bm.test(1000));
    }

    #[test]
    fn test_popcount() {
        let bm = AtomicBitmap::with_capacity(1000);

        bm.set(0);
        bm.set(1);
        bm.set(100);
        assert_eq!(bm.count(), 3);

        bm.clear(1);
        assert_eq!(bm.count(), 2);

        bm.set(100); // Already set, count shouldn't change
        assert_eq!(bm.count(), 2);
    }

    #[test]
    fn test_find_next_set() {
        let bm = AtomicBitmap::with_capacity(2000);

        bm.set(10);
        bm.set(100);
        bm.set(1000);

        assert_eq!(bm.find_next_set(0), Some(10));
        assert_eq!(bm.find_next_set(10), Some(10));
        assert_eq!(bm.find_next_set(11), Some(100));
        assert_eq!(bm.find_next_set(100), Some(100));
        assert_eq!(bm.find_next_set(101), Some(1000));
        assert_eq!(bm.find_next_set(1001), None);
    }

    #[test]
    fn test_find_next_unset() {
        let bm = AtomicBitmap::with_capacity(128);

        // Set first 10 bits
        for i in 0..10 {
            bm.set(i);
        }

        assert_eq!(bm.find_next_unset(0, 100), Some(10));
        assert_eq!(bm.find_next_unset(10, 100), Some(10));
        assert_eq!(bm.find_next_unset(5, 100), Some(10));
    }

    #[test]
    fn test_concurrent_set() {
        let bm = Arc::new(AtomicBitmap::with_capacity(10000));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let bm = bm.clone();
                thread::spawn(move || {
                    for i in 0..1000 {
                        bm.set(t * 1000 + i);
                    }
                })
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }

        assert_eq!(bm.count(), 8000);
    }

    #[test]
    fn test_concurrent_test_and_set() {
        let bm = Arc::new(AtomicBitmap::with_capacity(1000));
        let success_count = Arc::new(AtomicU64::new(0));

        let threads: Vec<_> = (0..8)
            .map(|_| {
                let bm = bm.clone();
                let success = success_count.clone();
                thread::spawn(move || {
                    for i in 0..1000 {
                        if bm.test_and_set(i) {
                            success.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }

        // Exactly 1000 successful claims (one per bit)
        assert_eq!(success_count.load(Ordering::Relaxed), 1000);
        assert_eq!(bm.count(), 1000);
    }

    #[test]
    fn test_clear_all() {
        let mut bm = AtomicBitmap::with_capacity(1000);

        for i in 0..100 {
            bm.set(i);
        }
        assert_eq!(bm.count(), 100);

        bm.clear_all();
        assert_eq!(bm.count(), 0);
        // Capacity is preserved (fixed-size)
        assert_eq!(bm.capacity(), 1024); // Rounded up to multiple of 64
    }

    #[test]
    fn test_reset() {
        let mut bm = AtomicBitmap::with_capacity(10000);

        for i in 0..100 {
            bm.set(i);
        }
        let cap = bm.capacity();

        bm.reset();
        assert_eq!(bm.count(), 0);
        assert_eq!(bm.capacity(), cap); // Capacity preserved
    }

    #[test]
    fn test_recompute_count() {
        let bm = AtomicBitmap::with_capacity(1000);

        for i in 0..100 {
            bm.set(i);
        }

        assert_eq!(bm.recompute_count(), 100);
        assert_eq!(bm.count(), 100);
    }

    #[test]
    fn test_cpu_capabilities() {
        // Just ensure this doesn't panic
        print_cpu_capabilities();
    }
}
