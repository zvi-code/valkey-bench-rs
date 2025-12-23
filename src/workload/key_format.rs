//! Unified key format for vector search operations
//!
//! Key format: `prefix{tag}:vector_id`
//! Example: `zvec_:{ABC}:000000055083`
//!
//! This module provides a single source of truth for key format handling,
//! used by both key generation (templates) and key parsing (recall, cluster scan).

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
#[derive(Debug, Clone)]
pub struct KeyFormat {
    /// Key prefix (e.g., "zvec_:")
    pub prefix: String,
    /// Width of the numeric key part (zero-padded)
    pub key_width: usize,
    /// Whether cluster tags are enabled
    pub use_cluster_tags: bool,
}

impl KeyFormat {
    /// Create new key format with cluster tags enabled
    pub fn with_cluster_tags(prefix: &str, key_width: usize) -> Self {
        Self {
            prefix: prefix.to_string(),
            key_width,
            use_cluster_tags: true,
        }
    }

    /// Create new key format without cluster tags (standalone mode)
    pub fn without_cluster_tags(prefix: &str, key_width: usize) -> Self {
        Self {
            prefix: prefix.to_string(),
            key_width,
            use_cluster_tags: false,
        }
    }

    /// Calculate total key length
    /// With cluster tags: prefix_len + 5 ({ABC}) + 1 (:) + key_width
    /// Without: prefix_len + key_width
    pub fn total_len(&self) -> usize {
        if self.use_cluster_tags {
            self.prefix.len() + CLUSTER_TAG_LEN + 1 + self.key_width
        } else {
            self.prefix.len() + self.key_width
        }
    }

    /// Format a key with the given tag and vector ID
    ///
    /// Returns: "prefix{tag}:000000000123" or "prefix000000000123"
    pub fn format_key(&self, tag: Option<&[u8; 5]>, vector_id: u64) -> String {
        let mut key = self.prefix.clone();

        if self.use_cluster_tags {
            if let Some(t) = tag {
                key.push_str(std::str::from_utf8(t).unwrap_or("{???}"));
            } else {
                key.push_str("{000}");
            }
            key.push(TAG_KEY_SEPARATOR);
        }

        // Zero-padded vector ID
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
pub fn extract_numeric_ids_from_keys(doc_ids: &[String], prefix: &str) -> Vec<u64> {
    let format = KeyFormat::with_cluster_tags(prefix, DEFAULT_KEY_WIDTH);
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
        // Cluster tag region is always 5 bytes (FIXED)
        assert_eq!(CLUSTER_TAG_LEN, 5);

        // Maximum inner length is 3 chars (can be 1-3)
        assert_eq!(CLUSTER_TAG_INNER_LEN, 3);

        // All these are valid 5-byte tag regions:
        // {ABC} = 5 bytes (3-char tag, no padding)
        assert_eq!(b"{ABC}".len(), CLUSTER_TAG_LEN);
        // {AB}X = 5 bytes (2-char tag, 1 padding)
        assert_eq!(b"{AB}X".len(), CLUSTER_TAG_LEN);
        // {A}XX = 5 bytes (1-char tag, 2 padding)
        assert_eq!(b"{A}XX".len(), CLUSTER_TAG_LEN);
    }

    #[test]
    fn test_cluster_tag_fixed_length_in_keys() {
        // All generated keys must have exactly CLUSTER_TAG_LEN bytes for the tag
        let fmt = KeyFormat::with_cluster_tags("zvec_:", DEFAULT_KEY_WIDTH);

        // Test with various tags - all must produce same length key
        let tags = [
            [b'{', b'A', b'B', b'C', b'}'],
            [b'{', b'X', b'Y', b'Z', b'}'],
            [b'{', b'0', b'0', b'0', b'}'],
        ];

        let first_len = fmt.format_key(Some(&tags[0]), 123).len();
        for tag in &tags {
            let key = fmt.format_key(Some(tag), 123);
            assert_eq!(key.len(), first_len, "Key length must be consistent");
            assert_eq!(key.len(), fmt.total_len(), "Key length must match total_len()");
        }
    }

    #[test]
    fn test_default_key_width_constant() {
        // Default width supports up to 999,999,999,999 (12 digits)
        assert_eq!(DEFAULT_KEY_WIDTH, 12);
        // Verify it can represent max expected vector count
        let max_id: u64 = 10u64.pow(DEFAULT_KEY_WIDTH as u32) - 1;
        assert!(max_id >= 999_999_999_999);
    }

    #[test]
    fn test_tag_key_separator_constant() {
        assert_eq!(TAG_KEY_SEPARATOR, ':');
    }

    // ==========================================
    // Format/parse round-trip tests (CRITICAL)
    // These ensure generation and parsing are consistent
    // ==========================================

    #[test]
    fn test_roundtrip_with_cluster_tag() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", DEFAULT_KEY_WIDTH);
        let tag = [b'{', b'A', b'B', b'C', b'}'];

        for id in [0u64, 1, 42, 123, 999999, 999999999999] {
            let key = fmt.format_key(Some(&tag), id);
            let (parsed_id, parsed_tag) = fmt.parse_key(&key).expect("Parse should succeed");
            assert_eq!(parsed_id, id, "ID mismatch for {}", id);
            assert_eq!(parsed_tag, Some("{ABC}".to_string()));
        }
    }

    #[test]
    fn test_roundtrip_without_cluster_tag() {
        let fmt = KeyFormat::without_cluster_tags("vec:", DEFAULT_KEY_WIDTH);

        for id in [0u64, 1, 42, 123, 999999, 999999999999] {
            let key = fmt.format_key(None, id);
            let (parsed_id, parsed_tag) = fmt.parse_key(&key).expect("Parse should succeed");
            assert_eq!(parsed_id, id, "ID mismatch for {}", id);
            assert!(parsed_tag.is_none());
        }
    }

    #[test]
    fn test_roundtrip_various_tags() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", DEFAULT_KEY_WIDTH);

        let tags = [
            [b'{', b'A', b'A', b'A', b'}'],
            [b'{', b'Z', b'Z', b'Z', b'}'],
            [b'{', b'X', b'Y', b'Z', b'}'],
            [b'{', b'0', b'0', b'0', b'}'],
        ];

        for tag in tags {
            let key = fmt.format_key(Some(&tag), 12345);
            let (parsed_id, parsed_tag) = fmt.parse_key(&key).expect("Parse should succeed");
            assert_eq!(parsed_id, 12345);
            let expected_tag = std::str::from_utf8(&tag).unwrap();
            assert_eq!(parsed_tag, Some(expected_tag.to_string()));
        }
    }

    // ==========================================
    // Key length validation tests
    // ==========================================

    #[test]
    fn test_generated_key_length_with_cluster_tag() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", DEFAULT_KEY_WIDTH);
        let tag = [b'{', b'A', b'B', b'C', b'}'];
        let key = fmt.format_key(Some(&tag), 123);

        // Verify actual length matches computed length
        assert_eq!(key.len(), fmt.total_len());

        // Verify structure: prefix + tag + separator + id
        // "zvec_:" (6) + "{ABC}" (5) + ":" (1) + "000000000123" (12) = 24
        assert_eq!(key.len(), 24);
    }

    #[test]
    fn test_generated_key_length_without_cluster_tag() {
        let fmt = KeyFormat::without_cluster_tags("vec:", DEFAULT_KEY_WIDTH);
        let key = fmt.format_key(None, 123);

        assert_eq!(key.len(), fmt.total_len());
        // "vec:" (4) + "000000000123" (12) = 16
        assert_eq!(key.len(), 16);
    }

    // ==========================================
    // Original basic tests
    // ==========================================

    #[test]
    fn test_format_key_with_cluster_tag() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);
        let tag = [b'{', b'A', b'B', b'C', b'}'];
        let key = fmt.format_key(Some(&tag), 123);
        assert_eq!(key, "zvec_:{ABC}:000000000123");
    }

    #[test]
    fn test_format_key_without_cluster_tag() {
        let fmt = KeyFormat::without_cluster_tags("vec:", 12);
        let key = fmt.format_key(None, 123);
        assert_eq!(key, "vec:000000000123");
    }

    #[test]
    fn test_parse_key_with_cluster_tag() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);
        let result = fmt.parse_key("zvec_:{ABC}:000000000123");
        assert_eq!(result, Some((123, Some("{ABC}".to_string()))));
    }

    #[test]
    fn test_parse_key_without_cluster_tag() {
        let fmt = KeyFormat::without_cluster_tags("vec:", 12);
        let result = fmt.parse_key("vec:000000000123");
        assert_eq!(result, Some((123, None)));
    }

    #[test]
    fn test_parse_key_zero() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);
        let result = fmt.parse_key("zvec_:{XYZ}:000000000000");
        assert_eq!(result, Some((0, Some("{XYZ}".to_string()))));
    }

    #[test]
    fn test_extract_numeric_ids() {
        let doc_ids = vec![
            "zvec_:{ABC}:000000000001".to_string(),
            "zvec_:{XYZ}:000000000042".to_string(),
            "zvec_:{DEF}:000000000100".to_string(),
        ];
        let ids = extract_numeric_ids_from_keys(&doc_ids, "zvec_:");
        assert_eq!(ids, vec![1, 42, 100]);
    }

    #[test]
    fn test_total_len() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);
        // "zvec_:" (6) + "{ABC}" (5) + ":" (1) + "000000000000" (12) = 24
        assert_eq!(fmt.total_len(), 24);

        let fmt2 = KeyFormat::without_cluster_tags("vec:", 12);
        // "vec:" (4) + "000000000000" (12) = 16
        assert_eq!(fmt2.total_len(), 16);
    }

    // ==========================================
    // Edge case and error handling tests
    // ==========================================

    #[test]
    fn test_parse_invalid_prefix() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);
        assert!(fmt.parse_key("other:{ABC}:000000000123").is_none());
        assert!(fmt.parse_key("vec:{ABC}:000000000123").is_none());
    }

    #[test]
    fn test_parse_key_without_tag_using_cluster_format() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);
        // Parsing auto-detects format, so keys without cluster tags
        // should still parse successfully (for backwards compatibility)
        let result = fmt.parse_key("zvec_:000000000123");
        // Parses successfully, returns no cluster tag
        assert_eq!(result, Some((123, None)));
    }

    #[test]
    fn test_parse_malformed_cluster_tag() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);
        // Missing closing brace - should fail
        assert!(fmt.parse_key("zvec_:{ABC000000000123").is_none());
        // Empty braces {} - rejected because cluster tag must be 1-3 chars
        assert!(fmt.parse_key("zvec_:{}XX:000000000123").is_none());
        // Too many chars (>3) inside braces - should fail
        assert!(fmt.parse_key("zvec_:{ABCD}000000000123").is_none()); // 4 chars
    }

    #[test]
    fn test_parse_variable_length_cluster_tags() {
        let fmt = KeyFormat::with_cluster_tags("zvec_:", 12);

        // 3-char tag (no padding): {ABC}:000000000123
        let result = fmt.parse_key("zvec_:{ABC}:000000000123");
        assert_eq!(result, Some((123, Some("{ABC}".to_string()))));

        // 2-char tag (1 padding): {AB}X:000000000123
        let result = fmt.parse_key("zvec_:{AB}X:000000000123");
        assert_eq!(result, Some((123, Some("{AB}".to_string()))));

        // 1-char tag (2 padding): {A}XX:000000000123
        let result = fmt.parse_key("zvec_:{A}XX:000000000123");
        assert_eq!(result, Some((123, Some("{A}".to_string()))));
    }

    #[test]
    fn test_mixed_format_extraction() {
        // Test that extract_numeric_ids handles both formats
        // Parsing auto-detects format, so both should parse successfully
        let doc_ids = vec![
            "zvec_:{ABC}:000000000001".to_string(), // With cluster tag
            "zvec_:000000000002".to_string(),       // Without cluster tag
        ];
        let ids = extract_numeric_ids_from_keys(&doc_ids, "zvec_:");
        // Both should parse successfully
        assert_eq!(ids, vec![1, 2]);
    }
}
