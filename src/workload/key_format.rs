//! Unified key format for vector search operations
//!
//! Key format: `prefix + zero_padded_id`
//! Example: `vec:000000055083`
//!
//! This module provides a single source of truth for key format handling,
//! used by both key generation (templates) and key parsing (recall, cluster scan).
//!
//! Note: Cluster hash tag injection has been removed. The parser still supports
//! legacy keys with cluster tags (e.g., `vec:{ABC}:000123`) for backward
//! compatibility during data migration.

/// Cluster tag region: FIXED 5 bytes total
/// This is fixed-length because we reuse command buffers and RESP encoding
/// requires consistent blob sizes.
///
/// The tag inside braces can be 1-3 chars, with padding outside for shorter tags:
/// - `{A}XX` - 1-char tag + 2 padding chars = 5 bytes (hashes on "A")
/// - `{AB}X` - 2-char tag + 1 padding char = 5 bytes (hashes on "AB")
/// - `{ABC}` - 3-char tag, no padding = 5 bytes (hashes on "ABC")
///
/// Examples: {ABC}, {AB}X, {A}XX
pub const CLUSTER_TAG_LEN: usize = 5;

/// Maximum number of characters inside the cluster tag braces (1-3 valid)
pub const CLUSTER_TAG_INNER_LEN: usize = 3;

/// Default key width (zero-padded decimal)
pub const DEFAULT_KEY_WIDTH: usize = 12;

/// Separator between cluster tag and key
pub const TAG_KEY_SEPARATOR: char = ':';

/// Key format configuration
/// 
/// Note: Cluster hash tag injection has been removed. Keys are now
/// in simple format: `prefix + zero_padded_id` (e.g., "vec:000000000123").
#[derive(Debug, Clone)]
pub struct KeyFormat {
    /// Key prefix (e.g., "vec:")
    pub prefix: String,
    /// Width of the numeric key part (zero-padded)
    pub key_width: usize,
    /// Legacy field - always false (cluster tags removed)
    use_cluster_tags: bool,
}

impl KeyFormat {
    /// Create new key format with simple keys (no cluster tags)
    /// 
    /// Keys will be formatted as: `prefix + zero_padded_id`
    /// Example: "vec:000000000123"
    pub fn new(prefix: &str, key_width: usize) -> Self {
        Self {
            prefix: prefix.to_string(),
            key_width,
            use_cluster_tags: false,
        }
    }

    /// Calculate total key length: prefix_len + key_width
    pub fn total_len(&self) -> usize {
        self.prefix.len() + self.key_width
    }

    /// Format a key with the given vector ID
    ///
    /// Returns: "prefix000000000123" (simple format)
    /// 
    /// Note: The `tag` parameter is ignored - cluster hash tags have been removed.
    pub fn format_key(&self, _tag: Option<&[u8; 5]>, vector_id: u64) -> String {
        let mut key = self.prefix.clone();
        // Zero-padded vector ID (simple format)
        key.push_str(&format!("{:0width$}", vector_id, width = self.key_width));
        key
    }

    /// Parse a key to extract cluster tag and vector ID
    ///
    /// Returns: (vector_id, cluster_tag) if successfully parsed
    ///
    /// Note: Parsing auto-detects format based on presence of '{' in the key.
    /// The `use_cluster_tags` flag only affects key generation, not parsing.
    /// This allows parsing both formats regardless of how the KeyFormat was created.
    pub fn parse_key(&self, key: &str) -> Option<(u64, Option<String>)> {
        // Check prefix
        let rest = key.strip_prefix(&self.prefix)?;

        // Auto-detect format: parse with cluster tag only if key contains '{'
        if rest.starts_with('{') {
            // Format with cluster tag region (always CLUSTER_TAG_LEN bytes total):
            // - {A}XX:000123 (1-char tag + 2 padding)
            // - {AB}X:000123 (2-char tag + 1 padding)
            // - {ABC}:000123 (3-char tag, no padding)
            // The hash is computed on what's inside {}, but the region is fixed-length
            let tag_end = rest.find('}')?;

            // Validate tag structure: 1-3 chars inside braces (longer tags use more slots)
            // tag_end - 1 = number of chars inside braces (since rest starts with '{')
            let inner_len = tag_end.saturating_sub(1);
            if !(1..=CLUSTER_TAG_INNER_LEN).contains(&inner_len) {
                return None;
            }

            // Extract just the tag portion (including braces)
            let cluster_tag = rest[0..=tag_end].to_string();

            // The key starts after the fixed CLUSTER_TAG_LEN region, then separator
            // For shorter tags, there's padding between '}' and ':'
            let after_tag_region = &rest[CLUSTER_TAG_LEN..];
            let id_str = after_tag_region
                .strip_prefix(TAG_KEY_SEPARATOR)
                .unwrap_or(after_tag_region);

            let vector_id: u64 = id_str.trim_start_matches('0').parse().unwrap_or(0);
            Some((vector_id, Some(cluster_tag)))
        } else {
            // Format without cluster tag: 000123
            let trimmed = rest.trim_start_matches('0');
            let vector_id = if trimmed.is_empty() {
                0
            } else {
                trimmed.parse().ok()?
            };
            Some((vector_id, None))
        }
    }

    /// Extract vector ID from a document key (for recall computation)
    ///
    /// Handles both formats:
    /// - Simple: "vec:000000000123" -> 123
    /// - With cluster tag: "vec:{ABC}:000000000123" -> 123
    pub fn extract_vector_id(&self, key: &str) -> Option<u64> {
        self.parse_key(key).map(|(id, _)| id)
    }
}

/// Extract numeric IDs from document keys using the standard key format
/// This is a convenience function for recall computation
///
/// Note: This function auto-detects key format, so it works with both
/// simple keys ("vec:000123") and legacy cluster-tagged keys ("vec:{ABC}:000123")
pub fn extract_numeric_ids_from_keys(doc_ids: &[String], prefix: &str) -> Vec<u64> {
    // Use simple format - parse_key auto-detects cluster tags if present
    let format = KeyFormat::new(prefix, DEFAULT_KEY_WIDTH);
    doc_ids
        .iter()
        .filter_map(|id| format.extract_vector_id(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================
    // Constants validation tests
    // ==========================================

    #[test]
    fn test_cluster_tag_len_constant() {
        // Constants kept for legacy parsing compatibility
        assert_eq!(CLUSTER_TAG_LEN, 5);
        assert_eq!(CLUSTER_TAG_INNER_LEN, 3);
    }

    #[test]
    fn test_default_key_width_constant() {
        // Default width supports up to 999,999,999,999 (12 digits)
        assert_eq!(DEFAULT_KEY_WIDTH, 12);
        let max_id: u64 = 10u64.pow(DEFAULT_KEY_WIDTH as u32) - 1;
        assert!(max_id >= 999_999_999_999);
    }

    #[test]
    fn test_tag_key_separator_constant() {
        assert_eq!(TAG_KEY_SEPARATOR, ':');
    }

    // ==========================================
    // Format/parse round-trip tests (CRITICAL)
    // ==========================================

    #[test]
    fn test_roundtrip_simple_format() {
        let fmt = KeyFormat::new("vec:", DEFAULT_KEY_WIDTH);

        for id in [0u64, 1, 42, 123, 999999, 999999999999] {
            let key = fmt.format_key(None, id);
            let (parsed_id, parsed_tag) = fmt.parse_key(&key).expect("Parse should succeed");
            assert_eq!(parsed_id, id, "ID mismatch for {}", id);
            assert!(parsed_tag.is_none(), "Simple format should have no tag");
        }
    }

    #[test]
    fn test_simple_key_format() {
        let fmt = KeyFormat::new("vec:", 12);
        let key = fmt.format_key(None, 123);
        
        // Verify simple format: prefix + zero-padded id
        assert_eq!(key, "vec:000000000123");
        assert!(!key.contains('{'), "Key should not contain cluster hash tag");
        assert_eq!(key.len(), fmt.total_len());
    }

    #[test]
    fn test_key_ignores_tag_parameter() {
        let fmt = KeyFormat::new("vec:", 12);
        let tag = [b'{', b'A', b'B', b'C', b'}'];
        
        // Tag parameter should be ignored (always simple format)
        let key_with_tag = fmt.format_key(Some(&tag), 123);
        let key_without_tag = fmt.format_key(None, 123);
        
        assert_eq!(key_with_tag, key_without_tag);
        assert_eq!(key_with_tag, "vec:000000000123");
    }

    // ==========================================
    // Key length validation tests
    // ==========================================

    #[test]
    fn test_simple_key_length() {
        let fmt = KeyFormat::new("vec:", DEFAULT_KEY_WIDTH);
        let key = fmt.format_key(None, 123);

        assert_eq!(key.len(), fmt.total_len());
        // "vec:" (4) + "000000000123" (12) = 16
        assert_eq!(key.len(), 16);
    }

    #[test]
    fn test_total_len() {
        let fmt = KeyFormat::new("vec:", 12);
        // "vec:" (4) + "000000000000" (12) = 16
        assert_eq!(fmt.total_len(), 16);

        let fmt2 = KeyFormat::new("zvec_:", 12);
        // "zvec_:" (6) + "000000000000" (12) = 18
        assert_eq!(fmt2.total_len(), 18);
    }

    // ==========================================
    // Parsing tests (backward compatibility)
    // ==========================================

    #[test]
    fn test_parse_simple_key() {
        let fmt = KeyFormat::new("vec:", 12);
        let result = fmt.parse_key("vec:000000000123");
        assert_eq!(result, Some((123, None)));
    }

    #[test]
    fn test_parse_key_zero() {
        let fmt = KeyFormat::new("vec:", 12);
        let result = fmt.parse_key("vec:000000000000");
        assert_eq!(result, Some((0, None)));
    }

    #[test]
    fn test_parse_legacy_cluster_tag_keys() {
        // Parser still supports legacy keys for backward compatibility
        let fmt = KeyFormat::new("zvec_:", 12);
        
        // Can parse old cluster-tagged keys
        let result = fmt.parse_key("zvec_:{ABC}:000000000123");
        assert_eq!(result, Some((123, Some("{ABC}".to_string()))));
    }

    #[test]
    fn test_parse_invalid_prefix() {
        let fmt = KeyFormat::new("zvec_:", 12);
        assert!(fmt.parse_key("other:{ABC}:000000000123").is_none());
        assert!(fmt.parse_key("vec:{ABC}:000000000123").is_none());
    }

    #[test]
    fn test_extract_numeric_ids() {
        // Test with simple format keys
        let doc_ids = vec![
            "vec:000000000001".to_string(),
            "vec:000000000042".to_string(),
            "vec:000000000100".to_string(),
        ];
        let ids = extract_numeric_ids_from_keys(&doc_ids, "vec:");
        assert_eq!(ids, vec![1, 42, 100]);
    }

    #[test]
    fn test_extract_numeric_ids_legacy_format() {
        // Extraction also works with legacy cluster-tagged keys
        let doc_ids = vec![
            "zvec_:{ABC}:000000000001".to_string(),
            "zvec_:000000000002".to_string(), // Mixed with simple
        ];
        let ids = extract_numeric_ids_from_keys(&doc_ids, "zvec_:");
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn test_parse_variable_length_cluster_tags() {
        // Parser still handles legacy variable-length tags
        let fmt = KeyFormat::new("zvec_:", 12);

        let result = fmt.parse_key("zvec_:{ABC}:000000000123");
        assert_eq!(result, Some((123, Some("{ABC}".to_string()))));

        let result = fmt.parse_key("zvec_:{AB}X:000000000123");
        assert_eq!(result, Some((123, Some("{AB}".to_string()))));

        let result = fmt.parse_key("zvec_:{A}XX:000000000123");
        assert_eq!(result, Some((123, Some("{A}".to_string()))));
    }
}
