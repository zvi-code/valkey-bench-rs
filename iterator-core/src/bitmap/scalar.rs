//! Scalar (portable) bitmap operations.
//!
//! This module provides optimized scalar implementations that work on any platform.
//! Uses loop unrolling for better instruction-level parallelism (ILP).
//!
//! These serve as the baseline fallback when SIMD is not available or beneficial.
//! On ARM and x86_64, the platform-specific implementations are used instead.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Population Count
// ============================================================================

/// Count set bits in a slice of AtomicU64.
/// Uses 4x loop unrolling for better ILP.
#[inline]
pub fn popcount_slice(words: &[AtomicU64]) -> u64 {
    let mut total: u64 = 0;
    let mut i = 0;
    let len = words.len();

    // Unroll by 4 for better ILP - CPU can execute multiple count_ones in parallel
    while i + 4 <= len {
        let w0 = words[i].load(Ordering::Relaxed);
        let w1 = words[i + 1].load(Ordering::Relaxed);
        let w2 = words[i + 2].load(Ordering::Relaxed);
        let w3 = words[i + 3].load(Ordering::Relaxed);

        total += w0.count_ones() as u64;
        total += w1.count_ones() as u64;
        total += w2.count_ones() as u64;
        total += w3.count_ones() as u64;

        i += 4;
    }

    // Handle remaining words
    while i < len {
        total += words[i].load(Ordering::Relaxed).count_ones() as u64;
        i += 1;
    }

    total
}

// ============================================================================
// Find Operations
// ============================================================================

/// Find first non-zero word starting from `start_word`.
/// Uses 4-word batching with OR-reduction to skip empty regions quickly.
#[inline]
pub fn find_first_nonzero(words: &[AtomicU64], start_word: usize) -> Option<(usize, u64)> {
    let len = words.len();
    if start_word >= len {
        return None;
    }

    let mut i = start_word;

    // Process 4 words at a time with OR-reduction
    // If OR of 4 words is 0, all 4 are zero - skip entire batch
    while i + 4 <= len {
        let w0 = words[i].load(Ordering::Relaxed);
        let w1 = words[i + 1].load(Ordering::Relaxed);
        let w2 = words[i + 2].load(Ordering::Relaxed);
        let w3 = words[i + 3].load(Ordering::Relaxed);

        if (w0 | w1 | w2 | w3) != 0 {
            // At least one is non-zero, find which one
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

    // Scalar tail for remaining words
    while i < len {
        let w = words[i].load(Ordering::Relaxed);
        if w != 0 {
            return Some((i, w));
        }
        i += 1;
    }

    None
}

/// Find first word with unset bits (not all 1s) starting from `start_word`.
/// Uses 4-word batching with AND-reduction to skip full regions quickly.
#[inline]
pub fn find_first_not_full(words: &[AtomicU64], start_word: usize) -> Option<(usize, u64)> {
    let len = words.len();
    if start_word >= len {
        return None;
    }

    let mut i = start_word;
    const ALL_ONES: u64 = !0u64;

    // Process 4 words at a time with AND-reduction
    // If AND of 4 words is all 1s, all 4 are full - skip entire batch
    while i + 4 <= len {
        let w0 = words[i].load(Ordering::Relaxed);
        let w1 = words[i + 1].load(Ordering::Relaxed);
        let w2 = words[i + 2].load(Ordering::Relaxed);
        let w3 = words[i + 3].load(Ordering::Relaxed);

        if (w0 & w1 & w2 & w3) != ALL_ONES {
            // At least one has unset bits
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
    while i < len {
        let w = words[i].load(Ordering::Relaxed);
        if w != ALL_ONES {
            return Some((i, w));
        }
        i += 1;
    }

    None
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
    fn test_popcount_multiple() {
        let words = make_words(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(popcount_slice(&words), 32);
    }

    #[test]
    fn test_popcount_large() {
        let words = make_words(&[!0u64; 16]);
        assert_eq!(popcount_slice(&words), 64 * 16);
    }

    #[test]
    fn test_find_nonzero_empty() {
        let words = make_words(&[]);
        assert_eq!(find_first_nonzero(&words, 0), None);
    }

    #[test]
    fn test_find_nonzero_all_zero() {
        let words = make_words(&[0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(find_first_nonzero(&words, 0), None);
    }

    #[test]
    fn test_find_nonzero_first() {
        let words = make_words(&[1, 0, 0, 0]);
        assert_eq!(find_first_nonzero(&words, 0), Some((0, 1)));
    }

    #[test]
    fn test_find_nonzero_middle() {
        let words = make_words(&[0, 0, 0, 0, 0, 42, 0, 0]);
        assert_eq!(find_first_nonzero(&words, 0), Some((5, 42)));
    }

    #[test]
    fn test_find_not_full_all_full() {
        let words = make_words(&[!0u64; 8]);
        assert_eq!(find_first_not_full(&words, 0), None);
    }

    #[test]
    fn test_find_not_full_first() {
        let words = make_words(&[0, !0u64, !0u64, !0u64]);
        assert_eq!(find_first_not_full(&words, 0), Some((0, 0)));
    }
}
