//! Key format constants and utilities
//!
//! This module provides constants for key formatting and re-exports
//! the main parsing/formatting functionality from addressable.rs.
//!
//! Key format: `prefix + zero_padded_id`
//! Example: `vec:000000055083`

// Re-export from addressable
pub use super::addressable::{extract_numeric_ids_from_keys, AddressSpec, DEFAULT_KEY_WIDTH};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_key_width_constant() {
        assert_eq!(DEFAULT_KEY_WIDTH, 12);
        let max_id: u64 = 10u64.pow(DEFAULT_KEY_WIDTH as u32) - 1;
        assert!(max_id >= 999_999_999_999);
    }

    #[test]
    fn test_extract_numeric_ids() {
        let doc_ids = vec![
            "vec:000000000001".to_string(),
            "vec:000000000042".to_string(),
            "vec:000000000100".to_string(),
        ];
        let ids = extract_numeric_ids_from_keys(&doc_ids, "vec:");
        assert_eq!(ids, vec![1, 42, 100]);
    }

    #[test]
    fn test_extract_numeric_ids_with_cluster_tag_in_prefix() {
        // User can include cluster tag in the prefix itself
        let doc_ids = vec![
            "vec{tag}:000000000001".to_string(),
            "vec{tag}:000000000002".to_string(),
        ];
        let ids = extract_numeric_ids_from_keys(&doc_ids, "vec{tag}:");
        assert_eq!(ids, vec![1, 2]);
    }
}
