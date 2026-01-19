//! x86_64 SIMD bitmap operations with runtime CPU detection.
//!
//! Supports multiple x86_64 feature levels with optimized paths:
//! - **AVX-512 + VPOPCNTDQ**: Ice Lake+, fastest bulk popcount
//! - **AVX-512**: Skylake-X+, 512-bit vector operations
//! - **AVX2**: Haswell+, 256-bit vector operations
//! - **POPCNT**: Nehalem+ (~2008), hardware popcount instruction
//! - **Scalar**: Fallback for ancient CPUs
//!
//! Runtime detection uses `std::arch::is_x86_feature_detected!` to select
//! the best implementation. Each optimized function uses `#[target_feature]`
//! for compile-time optimization even when cross-compiling.

use std::sync::atomic::{AtomicU64, Ordering};

use super::scalar;

// ============================================================================
// Constants
// ============================================================================

/// Cache line size for x86_64 (64 bytes)
pub const CACHE_LINE_SIZE: usize = 64;

/// Words per cache line
pub const WORDS_PER_CACHE_LINE: usize = CACHE_LINE_SIZE / 8;

// ============================================================================
// CPU Feature Detection
// ============================================================================

/// Detected x86_64 CPU capabilities
#[derive(Clone, Copy, Debug)]
pub struct X86Capabilities {
    /// Hardware POPCNT instruction (virtually universal since ~2008)
    pub popcnt: bool,
    /// AVX2 (256-bit vectors) - Haswell+
    pub avx2: bool,
    /// AVX-512F (Foundation) - Skylake-X+
    pub avx512f: bool,
    /// AVX-512 VPOPCNTDQ (native vector popcount) - Ice Lake+
    pub avx512vpopcntdq: bool,
}

impl X86Capabilities {
    /// Detect CPU capabilities at runtime
    #[inline]
    pub fn detect() -> Self {
        Self {
            popcnt: is_x86_feature_detected!("popcnt"),
            avx2: is_x86_feature_detected!("avx2"),
            avx512f: is_x86_feature_detected!("avx512f"),
            avx512vpopcntdq: is_x86_feature_detected!("avx512vpopcntdq"),
        }
    }

    /// Get a static reference to detected capabilities (cached)
    #[inline]
    pub fn get() -> &'static Self {
        use std::sync::OnceLock;
        static CAPS: OnceLock<X86Capabilities> = OnceLock::new();
        CAPS.get_or_init(Self::detect)
    }
}

// ============================================================================
// Prefetch Hints
// ============================================================================

/// Prefetch data for reading into L1 cache
#[inline(always)]
pub fn prefetch_read<T>(ptr: *const T) {
    unsafe {
        use std::arch::x86_64::*;
        _mm_prefetch(ptr as *const i8, _MM_HINT_T0);
    }
}

/// Prefetch data for writing (exclusive) into L1 cache
#[inline(always)]
pub fn prefetch_write<T>(ptr: *const T) {
    unsafe {
        use std::arch::x86_64::*;
        _mm_prefetch(ptr as *const i8, _MM_HINT_ET0);
    }
}

/// Prefetch next cache line during iteration
#[inline(always)]
pub fn prefetch_next_cacheline(words: &[AtomicU64], current_idx: usize) {
    let next_cacheline_idx = (current_idx + WORDS_PER_CACHE_LINE) & !(WORDS_PER_CACHE_LINE - 1);
    if next_cacheline_idx < words.len() {
        prefetch_read(words[next_cacheline_idx..].as_ptr());
    }
}

// ============================================================================
// POPCNT Implementation (Nehalem+ baseline)
// ============================================================================

mod popcnt {
    use super::*;
    use std::arch::x86_64::_popcnt64;

    /// Hardware POPCNT instruction wrapper
    #[inline]
    #[target_feature(enable = "popcnt")]
    pub unsafe fn popcnt64(x: u64) -> u64 {
        _popcnt64(x as i64) as u64
    }

    /// Count set bits using hardware POPCNT with 8x unrolling.
    #[inline]
    #[target_feature(enable = "popcnt")]
    pub unsafe fn popcount_slice_popcnt(words: &[AtomicU64]) -> u64 {
        let mut total: u64 = 0;
        let mut i = 0;
        let len = words.len();

        // Process 8 words per iteration - CPU can execute multiple POPCNT in parallel
        while i + 8 <= len {
            // Prefetch next cache line
            if i + WORDS_PER_CACHE_LINE < len {
                prefetch_read(words[i + WORDS_PER_CACHE_LINE..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);
            let w4 = words[i + 4].load(Ordering::Relaxed);
            let w5 = words[i + 5].load(Ordering::Relaxed);
            let w6 = words[i + 6].load(Ordering::Relaxed);
            let w7 = words[i + 7].load(Ordering::Relaxed);

            // Parallel POPCNT - modern CPUs have multiple execution units
            total += popcnt64(w0) + popcnt64(w1) + popcnt64(w2) + popcnt64(w3);
            total += popcnt64(w4) + popcnt64(w5) + popcnt64(w6) + popcnt64(w7);

            i += 8;
        }

        // Handle remaining words
        while i < len {
            total += popcnt64(words[i].load(Ordering::Relaxed));
            i += 1;
        }

        total
    }

    /// Find first non-zero word with prefetching.
    #[inline]
    #[target_feature(enable = "popcnt")]
    pub unsafe fn find_first_nonzero_popcnt(
        words: &[AtomicU64],
        start_word: usize,
    ) -> Option<(usize, u64)> {
        let len = words.len();
        if start_word >= len {
            return None;
        }

        let mut i = start_word;

        // Process 4 words at a time with OR-reduction
        while i + 4 <= len {
            if i + WORDS_PER_CACHE_LINE < len {
                prefetch_read(words[i + WORDS_PER_CACHE_LINE..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);

            if (w0 | w1 | w2 | w3) != 0 {
                if w0 != 0 {
                    return Some((i, w0));
                }
                if w1 != 0 {
                    return Some((i + 1, w1));
                }
                if w2 != 0 {
                    return Some((i + 2, w2));
                }
                return Some((i + 3, w3));
            }

            i += 4;
        }

        // Scalar tail
        while i < len {
            let w = words[i].load(Ordering::Relaxed);
            if w != 0 {
                return Some((i, w));
            }
            i += 1;
        }

        None
    }

    /// Find first word with unset bits.
    #[inline]
    #[target_feature(enable = "popcnt")]
    pub unsafe fn find_first_not_full_popcnt(
        words: &[AtomicU64],
        start_word: usize,
    ) -> Option<(usize, u64)> {
        let len = words.len();
        if start_word >= len {
            return None;
        }

        let mut i = start_word;
        const ALL_ONES: u64 = !0u64;

        while i + 4 <= len {
            if i + WORDS_PER_CACHE_LINE < len {
                prefetch_read(words[i + WORDS_PER_CACHE_LINE..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);

            if (w0 & w1 & w2 & w3) != ALL_ONES {
                if w0 != ALL_ONES {
                    return Some((i, w0));
                }
                if w1 != ALL_ONES {
                    return Some((i + 1, w1));
                }
                if w2 != ALL_ONES {
                    return Some((i + 2, w2));
                }
                return Some((i + 3, w3));
            }

            i += 4;
        }

        while i < len {
            let w = words[i].load(Ordering::Relaxed);
            if w != ALL_ONES {
                return Some((i, w));
            }
            i += 1;
        }

        None
    }
}

// ============================================================================
// AVX2 Implementation (Haswell+)
// ============================================================================

mod avx2 {
    use super::*;
    use std::arch::x86_64::*;

    /// Harley-Seal popcount for 256-bit vectors.
    /// This is more efficient than individual POPCNT for large arrays.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn popcount256(v: __m256i) -> __m256i {
        // Lookup table for 4-bit popcount
        let lookup = _mm256_setr_epi8(
            0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, 0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2,
            3, 3, 4,
        );
        let low_mask = _mm256_set1_epi8(0x0f);

        let lo = _mm256_and_si256(v, low_mask);
        let hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), low_mask);
        let popcnt_lo = _mm256_shuffle_epi8(lookup, lo);
        let popcnt_hi = _mm256_shuffle_epi8(lookup, hi);

        _mm256_sad_epu8(_mm256_add_epi8(popcnt_lo, popcnt_hi), _mm256_setzero_si256())
    }

    /// Count set bits using AVX2 with Harley-Seal algorithm.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub unsafe fn popcount_slice_avx2(words: &[AtomicU64]) -> u64 {
        let mut total: u64 = 0;
        let mut i = 0;
        let len = words.len();

        // Process 4 words (256 bits) per iteration
        while i + 4 <= len {
            if i + WORDS_PER_CACHE_LINE < len {
                prefetch_read(words[i + WORDS_PER_CACHE_LINE..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);

            // Pack into AVX2 vector
            let v = _mm256_set_epi64x(w3 as i64, w2 as i64, w1 as i64, w0 as i64);
            let cnt = popcount256(v);

            // Sum the 4 partial counts
            total += (_mm256_extract_epi64(cnt, 0) + _mm256_extract_epi64(cnt, 2)) as u64;

            i += 4;
        }

        // Scalar tail using POPCNT
        while i < len {
            total += words[i].load(Ordering::Relaxed).count_ones() as u64;
            i += 1;
        }

        total
    }

    /// Find first non-zero using AVX2 comparison.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub unsafe fn find_first_nonzero_avx2(
        words: &[AtomicU64],
        start_word: usize,
    ) -> Option<(usize, u64)> {
        let len = words.len();
        if start_word >= len {
            return None;
        }

        let mut i = start_word;
        let zero = _mm256_setzero_si256();

        // Process 4 words at a time
        while i + 4 <= len {
            if i + WORDS_PER_CACHE_LINE < len {
                prefetch_read(words[i + WORDS_PER_CACHE_LINE..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);

            let v = _mm256_set_epi64x(w3 as i64, w2 as i64, w1 as i64, w0 as i64);
            let cmp = _mm256_cmpeq_epi64(v, zero);
            let mask = _mm256_movemask_epi8(cmp) as u32;

            // If any word is non-zero, mask won't be all 1s
            if mask != 0xFFFF_FFFF {
                if w0 != 0 {
                    return Some((i, w0));
                }
                if w1 != 0 {
                    return Some((i + 1, w1));
                }
                if w2 != 0 {
                    return Some((i + 2, w2));
                }
                return Some((i + 3, w3));
            }

            i += 4;
        }

        // Scalar tail
        while i < len {
            let w = words[i].load(Ordering::Relaxed);
            if w != 0 {
                return Some((i, w));
            }
            i += 1;
        }

        None
    }

    /// Find first word with unset bits using AVX2.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub unsafe fn find_first_not_full_avx2(
        words: &[AtomicU64],
        start_word: usize,
    ) -> Option<(usize, u64)> {
        let len = words.len();
        if start_word >= len {
            return None;
        }

        let mut i = start_word;
        let all_ones = _mm256_set1_epi64x(-1i64);

        while i + 4 <= len {
            if i + WORDS_PER_CACHE_LINE < len {
                prefetch_read(words[i + WORDS_PER_CACHE_LINE..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);

            let v = _mm256_set_epi64x(w3 as i64, w2 as i64, w1 as i64, w0 as i64);
            let cmp = _mm256_cmpeq_epi64(v, all_ones);
            let mask = _mm256_movemask_epi8(cmp) as u32;

            // If any word is not full, mask won't be all 1s
            if mask != 0xFFFF_FFFF {
                const ALL_ONES: u64 = !0u64;
                if w0 != ALL_ONES {
                    return Some((i, w0));
                }
                if w1 != ALL_ONES {
                    return Some((i + 1, w1));
                }
                if w2 != ALL_ONES {
                    return Some((i + 2, w2));
                }
                return Some((i + 3, w3));
            }

            i += 4;
        }

        // Scalar tail
        const ALL_ONES: u64 = !0u64;
        while i < len {
            let w = words[i].load(Ordering::Relaxed);
            if w != ALL_ONES {
                return Some((i, w));
            }
            i += 1;
        }

        None
    }
}

// ============================================================================
// AVX-512 Implementation (Skylake-X+ with VPOPCNTDQ for Ice Lake+)
// ============================================================================

mod avx512 {
    use super::*;
    use std::arch::x86_64::*;

    /// Count set bits using AVX-512 VPOPCNTDQ (native 512-bit vector popcount).
    /// This is the fastest bulk popcount on Ice Lake, Zen 4+, Sapphire Rapids.
    #[inline]
    #[target_feature(enable = "avx512f,avx512vpopcntdq")]
    pub unsafe fn popcount_slice_avx512(words: &[AtomicU64]) -> u64 {
        let mut total: u64 = 0;
        let mut i = 0;
        let len = words.len();

        // Process 8 words (512 bits) per iteration
        while i + 8 <= len {
            // Prefetch 2 cache lines ahead for streaming access
            if i + 16 < len {
                prefetch_read(words[i + 16..].as_ptr());
            }

            // Load 8 u64s into 512-bit register
            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);
            let w4 = words[i + 4].load(Ordering::Relaxed);
            let w5 = words[i + 5].load(Ordering::Relaxed);
            let w6 = words[i + 6].load(Ordering::Relaxed);
            let w7 = words[i + 7].load(Ordering::Relaxed);

            let v = _mm512_set_epi64(
                w7 as i64, w6 as i64, w5 as i64, w4 as i64,
                w3 as i64, w2 as i64, w1 as i64, w0 as i64,
            );

            // Native vector popcount - single instruction for 8 u64s!
            let cnt = _mm512_popcnt_epi64(v);

            // Reduce: sum all 8 counts
            total += _mm512_reduce_add_epi64(cnt) as u64;

            i += 8;
        }

        // AVX2 tail for 4-word chunks
        while i + 4 <= len {
            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);

            total += w0.count_ones() as u64 + w1.count_ones() as u64;
            total += w2.count_ones() as u64 + w3.count_ones() as u64;

            i += 4;
        }

        // Scalar tail
        while i < len {
            total += words[i].load(Ordering::Relaxed).count_ones() as u64;
            i += 1;
        }

        total
    }

    /// Find first non-zero using AVX-512 masked comparison.
    #[inline]
    #[target_feature(enable = "avx512f")]
    pub unsafe fn find_first_nonzero_avx512(
        words: &[AtomicU64],
        start_word: usize,
    ) -> Option<(usize, u64)> {
        let len = words.len();
        if start_word >= len {
            return None;
        }

        let mut i = start_word;
        let zero = _mm512_setzero_si512();

        // Process 8 words at a time
        while i + 8 <= len {
            if i + 16 < len {
                prefetch_read(words[i + 16..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);
            let w4 = words[i + 4].load(Ordering::Relaxed);
            let w5 = words[i + 5].load(Ordering::Relaxed);
            let w6 = words[i + 6].load(Ordering::Relaxed);
            let w7 = words[i + 7].load(Ordering::Relaxed);

            let v = _mm512_set_epi64(
                w7 as i64, w6 as i64, w5 as i64, w4 as i64,
                w3 as i64, w2 as i64, w1 as i64, w0 as i64,
            );

            // Compare with zero, get 8-bit mask
            let mask = _mm512_cmpneq_epi64_mask(v, zero);

            if mask != 0 {
                // Find first non-zero in batch
                let idx = mask.trailing_zeros() as usize;
                let word = match idx {
                    0 => w0, 1 => w1, 2 => w2, 3 => w3,
                    4 => w4, 5 => w5, 6 => w6, _ => w7,
                };
                return Some((i + idx, word));
            }

            i += 8;
        }

        // Scalar tail
        while i < len {
            let w = words[i].load(Ordering::Relaxed);
            if w != 0 {
                return Some((i, w));
            }
            i += 1;
        }

        None
    }

    /// Find first word with unset bits using AVX-512.
    #[inline]
    #[target_feature(enable = "avx512f")]
    pub unsafe fn find_first_not_full_avx512(
        words: &[AtomicU64],
        start_word: usize,
    ) -> Option<(usize, u64)> {
        let len = words.len();
        if start_word >= len {
            return None;
        }

        let mut i = start_word;
        let all_ones = _mm512_set1_epi64(-1i64);

        while i + 8 <= len {
            if i + 16 < len {
                prefetch_read(words[i + 16..].as_ptr());
            }

            let w0 = words[i].load(Ordering::Relaxed);
            let w1 = words[i + 1].load(Ordering::Relaxed);
            let w2 = words[i + 2].load(Ordering::Relaxed);
            let w3 = words[i + 3].load(Ordering::Relaxed);
            let w4 = words[i + 4].load(Ordering::Relaxed);
            let w5 = words[i + 5].load(Ordering::Relaxed);
            let w6 = words[i + 6].load(Ordering::Relaxed);
            let w7 = words[i + 7].load(Ordering::Relaxed);

            let v = _mm512_set_epi64(
                w7 as i64, w6 as i64, w5 as i64, w4 as i64,
                w3 as i64, w2 as i64, w1 as i64, w0 as i64,
            );

            let mask = _mm512_cmpneq_epi64_mask(v, all_ones);

            if mask != 0 {
                let idx = mask.trailing_zeros() as usize;
                let word = match idx {
                    0 => w0, 1 => w1, 2 => w2, 3 => w3,
                    4 => w4, 5 => w5, 6 => w6, _ => w7,
                };
                return Some((i + idx, word));
            }

            i += 8;
        }

        // Scalar tail
        const ALL_ONES: u64 = !0u64;
        while i < len {
            let w = words[i].load(Ordering::Relaxed);
            if w != ALL_ONES {
                return Some((i, w));
            }
            i += 1;
        }

        None
    }
}

// ============================================================================
// Public API - Runtime Dispatch
// ============================================================================

/// Count set bits using the best available method.
#[inline]
pub fn popcount_slice(words: &[AtomicU64]) -> u64 {
    let caps = X86Capabilities::get();

    if caps.avx512vpopcntdq {
        unsafe { avx512::popcount_slice_avx512(words) }
    } else if caps.avx2 {
        unsafe { avx2::popcount_slice_avx2(words) }
    } else if caps.popcnt {
        unsafe { popcnt::popcount_slice_popcnt(words) }
    } else {
        scalar::popcount_slice(words)
    }
}

/// Find first non-zero word starting from `start_word`.
#[inline]
pub fn find_first_nonzero(words: &[AtomicU64], start_word: usize) -> Option<(usize, u64)> {
    let caps = X86Capabilities::get();

    if caps.avx512f {
        unsafe { avx512::find_first_nonzero_avx512(words, start_word) }
    } else if caps.avx2 {
        unsafe { avx2::find_first_nonzero_avx2(words, start_word) }
    } else if caps.popcnt {
        unsafe { popcnt::find_first_nonzero_popcnt(words, start_word) }
    } else {
        scalar::find_first_nonzero(words, start_word)
    }
}

/// Find first word with unset bits starting from `start_word`.
#[inline]
pub fn find_first_not_full(words: &[AtomicU64], start_word: usize) -> Option<(usize, u64)> {
    let caps = X86Capabilities::get();

    if caps.avx512f {
        unsafe { avx512::find_first_not_full_avx512(words, start_word) }
    } else if caps.avx2 {
        unsafe { avx2::find_first_not_full_avx2(words, start_word) }
    } else if caps.popcnt {
        unsafe { popcnt::find_first_not_full_popcnt(words, start_word) }
    } else {
        scalar::find_first_not_full(words, start_word)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_words(values: &[u64]) -> Vec<AtomicU64> {
        values.iter().map(|&v| AtomicU64::new(v)).collect()
    }

    #[test]
    fn test_x86_capabilities() {
        let caps = X86Capabilities::detect();
        println!("x86_64 Capabilities: {:?}", caps);
        // POPCNT is virtually universal on x86_64
    }

    #[test]
    fn test_popcount_empty() {
        let words = make_words(&[]);
        assert_eq!(popcount_slice(&words), 0);
    }

    #[test]
    fn test_popcount_single() {
        let words = make_words(&[0b1010_1010]);
        assert_eq!(popcount_slice(&words), 4);
    }

    #[test]
    fn test_popcount_large() {
        let words = make_words(&[!0u64; 16]);
        assert_eq!(popcount_slice(&words), 64 * 16);
    }

    #[test]
    fn test_find_nonzero() {
        let words = make_words(&[0, 0, 0, 0, 0, 42, 0, 0]);
        assert_eq!(find_first_nonzero(&words, 0), Some((5, 42)));
    }

    #[test]
    fn test_find_not_full() {
        let words = make_words(&[!0u64, !0u64, !0u64, !0u64, !0u64, 42, !0u64, !0u64]);
        assert_eq!(find_first_not_full(&words, 0), Some((5, 42)));
    }

    #[test]
    fn test_prefetch_does_not_crash() {
        let words = make_words(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        prefetch_read(words.as_ptr());
        prefetch_write(words.as_ptr());
        prefetch_next_cacheline(&words, 0);
    }

    // Test all implementations directly if available
    #[test]
    fn test_popcnt_impl() {
        if X86Capabilities::get().popcnt {
            let words = make_words(&[!0u64; 8]);
            unsafe {
                assert_eq!(popcnt::popcount_slice_popcnt(&words), 64 * 8);
            }
        }
    }

    #[test]
    fn test_avx2_impl() {
        if X86Capabilities::get().avx2 {
            let words = make_words(&[!0u64; 8]);
            unsafe {
                assert_eq!(avx2::popcount_slice_avx2(&words), 64 * 8);
            }
        }
    }

    #[test]
    fn test_avx512_popcount_impl() {
        if X86Capabilities::get().avx512vpopcntdq {
            let words = make_words(&[!0u64; 16]);
            unsafe {
                assert_eq!(avx512::popcount_slice_avx512(&words), 64 * 16);
            }
        }
    }

    #[test]
    fn test_avx512_find_nonzero_impl() {
        if X86Capabilities::get().avx512f {
            let words = make_words(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 42, 0, 0]);
            unsafe {
                assert_eq!(avx512::find_first_nonzero_avx512(&words, 0), Some((9, 42)));
            }
        }
    }

    #[test]
    fn test_avx512_find_not_full_impl() {
        if X86Capabilities::get().avx512f {
            let words = make_words(&[!0u64; 10].into_iter().chain([42]).collect::<Vec<_>>());
            unsafe {
                assert_eq!(avx512::find_first_not_full_avx512(&words, 0), Some((10, 42)));
            }
        }
    }
}
