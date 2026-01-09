//! Addressable space abstraction for workloads
//!
//! This module provides unified addressing for all workload types:
//! - Simple keys (GET/SET/DEL)
//! - Hash keys with field iteration (HSET key field value)
//! - JSON keys with path iteration (JSON.SET key $.path value)
//!
//! The central `AddressSpec` struct consolidates:
//! - Key prefix and width calculation
//! - Optional sub-key specification (fields or paths)
//! - Template placeholder generation
//! - Key parsing for recall computation

use std::sync::atomic::{AtomicU64, Ordering};

use super::command_template::CommandTemplate;

/// Default key width (zero-padded decimal) - supports up to 999,999,999,999 IDs
pub const DEFAULT_KEY_WIDTH: usize = 12;

/// Type of address being used
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddressType {
    /// Simple key (default)
    #[default]
    Key,
    /// Hash field (key + field name)
    HashField,
    /// JSON path (key + JSON path)
    JsonPath,
}

impl std::fmt::Display for AddressType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressType::Key => write!(f, "key"),
            AddressType::HashField => write!(f, "hash"),
            AddressType::JsonPath => write!(f, "json"),
        }
    }
}

// ============================================================================
// AddressSpec: Unified address specification for templates and iteration
// ============================================================================

/// Specification for a numeric placeholder component
///
/// Each component (key, field, path) can have its own prefix, width, and range.
/// This enables patterns like: `HSET 'keya{0-1000}' 'field{0-10}' 'value'`
/// where key and field iterate independently.
#[derive(Debug, Clone)]
pub struct PlaceholderSpec {
    /// Prefix before the numeric part (e.g., "key:", "field")
    pub prefix: String,
    /// Width of zero-padded numeric part
    pub width: usize,
    /// Start of ID range (inclusive)
    pub range_start: u64,
    /// End of ID range (exclusive)
    pub range_end: u64,
}

impl PlaceholderSpec {
    /// Create a new placeholder spec
    pub fn new(prefix: &str, range_end: u64) -> Self {
        Self {
            prefix: prefix.to_string(),
            width: Self::calculate_width(range_end),
            range_start: 0,
            range_end,
        }
    }

    /// Create with explicit range
    pub fn with_range(prefix: &str, range_start: u64, range_end: u64) -> Self {
        Self {
            prefix: prefix.to_string(),
            width: Self::calculate_width(range_end),
            range_start,
            range_end,
        }
    }

    /// Create with explicit width
    pub fn with_width(prefix: &str, width: usize, range_end: u64) -> Self {
        Self {
            prefix: prefix.to_string(),
            width,
            range_start: 0,
            range_end,
        }
    }

    /// Calculate width from range (minimum DEFAULT_KEY_WIDTH)
    fn calculate_width(max_id: u64) -> usize {
        if max_id == 0 {
            return DEFAULT_KEY_WIDTH;
        }
        let digits = ((max_id - 1) as f64).log10().ceil() as usize + 1;
        digits.max(DEFAULT_KEY_WIDTH)
    }

    /// Total length: prefix + width
    pub fn total_len(&self) -> usize {
        self.prefix.len() + self.width
    }

    /// Range size
    pub fn range_size(&self) -> u64 {
        self.range_end.saturating_sub(self.range_start)
    }

    /// Format value at given index (wraps within range)
    pub fn format(&self, idx: u64) -> String {
        let id = self.range_start + (idx % self.range_size().max(1));
        format!("{}{:0width$}", self.prefix, id, width = self.width)
    }

    /// Parse a formatted string to extract the numeric ID
    ///
    /// Example: "vec:000000000123" with prefix "vec:" -> 123
    pub fn parse(&self, s: &str) -> Option<u64> {
        let rest = s.strip_prefix(&self.prefix)?;
        let trimmed = rest.trim_start_matches('0');
        if trimmed.is_empty() { Some(0) } else { trimmed.parse().ok() }
    }
}

/// Sub-key specification for hash fields or JSON paths
#[derive(Debug, Clone)]
pub enum SubKeySpec {
    /// Single literal field/path (no iteration)
    Literal(String),
    /// Numeric placeholder with its own range (e.g., "field{0-100}")
    Numeric(PlaceholderSpec),
    /// Multiple literal values to iterate over
    List {
        /// Field names or JSON paths
        values: Vec<String>,
        /// Maximum byte length for placeholder (padded with spaces)
        max_len: usize,
    },
}

impl SubKeySpec {
    /// Create a literal sub-key (no iteration)
    pub fn literal(value: &str) -> Self {
        Self::Literal(value.to_string())
    }

    /// Create a numeric placeholder sub-key
    pub fn numeric(prefix: &str, range_end: u64) -> Self {
        Self::Numeric(PlaceholderSpec::new(prefix, range_end))
    }

    /// Create a numeric placeholder with explicit range
    pub fn numeric_range(prefix: &str, range_start: u64, range_end: u64) -> Self {
        Self::Numeric(PlaceholderSpec::with_range(prefix, range_start, range_end))
    }

    /// Create a list sub-key with multiple values (backward compat: iterable)
    pub fn iterable(values: Vec<String>) -> Self {
        let max_len = values.iter().map(|s| s.len()).max().unwrap_or(16).max(16);
        Self::List { values, max_len }
    }

    /// Get maximum byte length for this sub-key
    pub fn max_len(&self) -> usize {
        match self {
            Self::Literal(s) => s.len(),
            Self::Numeric(spec) => spec.total_len(),
            Self::List { max_len, .. } => *max_len,
        }
    }

    /// Number of sub-key values
    pub fn len(&self) -> u64 {
        match self {
            Self::Literal(_) => 1,
            Self::Numeric(spec) => spec.range_size(),
            Self::List { values, .. } => values.len() as u64,
        }
    }

    /// Check if this is an iterable sub-key (not literal)
    pub fn is_iterable(&self) -> bool {
        !matches!(self, Self::Literal(_))
    }

    /// Check if this uses a numeric placeholder
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Numeric(_))
    }
}

/// Unified address specification for template creation and runtime iteration
///
/// Consolidates key prefix, width calculation, and sub-key handling into a single
/// struct that can:
/// - Add placeholders to CommandTemplate
/// - Parse keys to extract IDs (for recall computation)
/// - Define iteration space for addressable workloads
///
/// # Examples
///
/// ```ignore
/// // Simple key-only addressing
/// let spec = AddressSpec::key("vec:", 1_000_000);
///
/// // Hash with numeric field iteration (key and field have independent ranges)
/// let spec = AddressSpec::new(
///     PlaceholderSpec::new("obj:", 100_000),
///     Some((AddressType::HashField, SubKeySpec::numeric("field", 10))),
/// );
/// // This creates: HSET obj:000000000042 field000000000003 value
/// ```
#[derive(Debug, Clone)]
pub struct AddressSpec {
    /// Key placeholder specification
    pub key: PlaceholderSpec,
    /// Optional sub-key (hash field or JSON path)
    pub sub_key: Option<(AddressType, SubKeySpec)>,
}

impl AddressSpec {
    /// Create from explicit key spec and optional sub-key
    pub fn new(key: PlaceholderSpec, sub_key: Option<(AddressType, SubKeySpec)>) -> Self {
        Self { key, sub_key }
    }

    /// Create a simple key-only spec
    ///
    /// Key format: `prefix + zero_padded_id` (e.g., "vec:000000000123")
    pub fn key(prefix: &str, max_id: u64) -> Self {
        Self {
            key: PlaceholderSpec::new(prefix, max_id),
            sub_key: None,
        }
    }

    /// Create spec with hash field (single literal field)
    pub fn hash_field(prefix: &str, max_id: u64, field: &str) -> Self {
        Self {
            key: PlaceholderSpec::new(prefix, max_id),
            sub_key: Some((AddressType::HashField, SubKeySpec::literal(field))),
        }
    }

    /// Create spec with hash field iteration (list of fields)
    pub fn hash_fields(prefix: &str, max_id: u64, fields: Vec<String>) -> Self {
        Self {
            key: PlaceholderSpec::new(prefix, max_id),
            sub_key: Some((AddressType::HashField, SubKeySpec::iterable(fields))),
        }
    }

    /// Create spec with numeric hash field (independent key and field ranges)
    ///
    /// Example: `HSET key:000042 field:000003 value`
    pub fn hash_numeric_field(
        key_prefix: &str,
        key_max: u64,
        field_prefix: &str,
        field_max: u64,
    ) -> Self {
        Self {
            key: PlaceholderSpec::new(key_prefix, key_max),
            sub_key: Some((
                AddressType::HashField,
                SubKeySpec::numeric(field_prefix, field_max),
            )),
        }
    }

    /// Create spec with JSON path (single literal path)
    pub fn json_path(prefix: &str, max_id: u64, path: &str) -> Self {
        Self {
            key: PlaceholderSpec::new(prefix, max_id),
            sub_key: Some((AddressType::JsonPath, SubKeySpec::literal(path))),
        }
    }

    /// Create spec with JSON path iteration
    pub fn json_paths(prefix: &str, max_id: u64, paths: Vec<String>) -> Self {
        Self {
            key: PlaceholderSpec::new(prefix, max_id),
            sub_key: Some((AddressType::JsonPath, SubKeySpec::iterable(paths))),
        }
    }

    /// Create from explicit width (for backward compatibility with fixed-width keys)
    pub fn with_width(prefix: &str, key_width: usize, max_id: u64) -> Self {
        Self {
            key: PlaceholderSpec::with_width(prefix, key_width, max_id),
            sub_key: None,
        }
    }

    /// Get the address type
    pub fn address_type(&self) -> AddressType {
        self.sub_key
            .as_ref()
            .map(|(t, _)| *t)
            .unwrap_or(AddressType::Key)
    }

    /// Key prefix (convenience accessor)
    pub fn prefix(&self) -> &str {
        &self.key.prefix
    }

    /// Key width (convenience accessor)
    pub fn key_width(&self) -> usize {
        self.key.width
    }

    /// Max key ID (convenience accessor)
    pub fn max_id(&self) -> u64 {
        self.key.range_end
    }

    /// Total key length: prefix + key_width
    pub fn key_len(&self) -> usize {
        self.key.total_len()
    }

    /// Total address space size (keys × sub-keys)
    pub fn total_len(&self) -> u64 {
        let sub_key_count = self.sub_key.as_ref().map(|(_, s)| s.len()).unwrap_or(1);
        self.key.range_size() * sub_key_count
    }

    /// Add key placeholder to a CommandTemplate
    ///
    /// For simple keys: adds prefixed key placeholder
    /// For hash fields: adds key + field placeholder (if iterable/numeric)
    /// For JSON paths: adds key + path placeholder (if iterable/numeric)
    pub fn add_key_to_template(&self, template: CommandTemplate) -> CommandTemplate {
        let template = template.arg_prefixed_key(&self.key.prefix, self.key.width);

        // Add sub-key placeholder based on type
        match &self.sub_key {
            Some((AddressType::HashField, SubKeySpec::Numeric(spec))) => {
                // Numeric field: use prefixed field placeholder
                template.arg_prefixed_field(&spec.prefix, spec.width)
            }
            Some((AddressType::HashField, SubKeySpec::List { max_len, .. })) => {
                template.arg_field(*max_len)
            }
            Some((AddressType::HashField, SubKeySpec::Literal(field))) => {
                template.arg_str(field)
            }
            Some((AddressType::JsonPath, SubKeySpec::Numeric(spec))) => {
                template.arg_prefixed_json_path(&spec.prefix, spec.width)
            }
            Some((AddressType::JsonPath, SubKeySpec::List { max_len, .. })) => {
                template.arg_json_path_placeholder(*max_len)
            }
            Some((AddressType::JsonPath, SubKeySpec::Literal(path))) => {
                template.arg_str(path)
            }
            _ => template,
        }
    }

    /// Format a key from ID (for display/debugging)
    pub fn format_key(&self, id: u64) -> String {
        self.key.format(id)
    }

    /// Parse a key to extract vector ID
    ///
    /// Example: "vec:000000000123" with prefix "vec:" -> 123
    pub fn parse_key(&self, key: &str) -> Option<u64> {
        self.key.parse(key)
    }

    /// Create an iterator for this address space
    pub fn to_iterator(&self) -> AddressIterator {
        AddressIterator::from_spec(self.clone())
    }
}

/// Extract numeric IDs from document keys
///
/// Convenience function for recall computation.
///
/// # Example
/// ```ignore
/// let ids = extract_numeric_ids_from_keys(
///     &["vec:000000000001".to_string(), "vec:000000000042".to_string()],
///     "vec:"
/// );
/// assert_eq!(ids, vec![1, 42]);
/// ```
pub fn extract_numeric_ids_from_keys(doc_ids: &[String], prefix: &str) -> Vec<u64> {
    let spec = AddressSpec::with_width(prefix, DEFAULT_KEY_WIDTH, u64::MAX);
    doc_ids
        .iter()
        .filter_map(|id| spec.parse_key(id))
        .collect()
}

/// A complete address in the data space
#[derive(Debug, Clone, Default)]
pub struct Address {
    /// The key (always present)
    pub key: String,
    /// Field name for hash operations
    pub field: Option<String>,
    /// JSON path for JSON operations
    pub path: Option<String>,
    /// Database number (optional)
    pub db: Option<u32>,
}

impl Address {
    /// Create a simple key address
    pub fn key(key: String) -> Self {
        Self {
            key,
            field: None,
            path: None,
            db: None,
        }
    }

    /// Create a hash field address
    pub fn hash_field(key: String, field: String) -> Self {
        Self {
            key,
            field: Some(field),
            path: None,
            db: None,
        }
    }

    /// Create a JSON path address
    pub fn json_path(key: String, path: String) -> Self {
        Self {
            key,
            field: None,
            path: Some(path),
            db: None,
        }
    }

    /// Create a channel address
    pub fn channel(channel: String) -> Self {
        Self {
            key: channel,
            field: None,
            path: None,
            db: None,
        }
    }

    /// Get the address type
    pub fn address_type(&self) -> AddressType {
        if self.field.is_some() {
            AddressType::HashField
        } else if self.path.is_some() {
            AddressType::JsonPath
        } else {
            AddressType::Key
        }
    }
}

/// Trait for addressable data spaces
///
/// An addressable space defines how to map indices to addresses.
/// This allows workloads to iterate over keys, hash fields, JSON paths, etc.
pub trait AddressableSpace: Send + Sync {
    /// Total number of addresses in this space
    fn len(&self) -> u64;

    /// Check if the space is empty
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the address at the given index
    fn address_at(&self, idx: u64) -> Address;

    /// Get the type of addresses in this space
    fn address_type(&self) -> AddressType;

    /// Get the key prefix (if applicable)
    fn key_prefix(&self) -> &str {
        ""
    }
}

// Implement AddressableSpace for AddressSpec
impl AddressableSpace for AddressSpec {
    fn len(&self) -> u64 {
        self.total_len()
    }

    fn address_at(&self, idx: u64) -> Address {
        let key_range = self.key.range_size().max(1);
        let wrapped_key_idx = idx % key_range;
        let key = self.key.format(wrapped_key_idx);

        match &self.sub_key {
            None => Address::key(key),
            Some((AddressType::HashField, SubKeySpec::Literal(field))) => {
                Address::hash_field(key, field.clone())
            }
            Some((AddressType::HashField, SubKeySpec::Numeric(spec))) => {
                let field_range = spec.range_size().max(1);
                let key_idx = (idx / field_range) % key_range;
                let field_idx = idx % field_range;
                Address::hash_field(self.key.format(key_idx), spec.format(field_idx))
            }
            Some((AddressType::HashField, SubKeySpec::List { values, .. })) => {
                let num_fields = values.len() as u64;
                let key_idx = (idx / num_fields) % key_range;
                let field_idx = (idx % num_fields) as usize;
                Address::hash_field(self.key.format(key_idx), values[field_idx].clone())
            }
            Some((AddressType::JsonPath, SubKeySpec::Literal(path))) => {
                Address::json_path(key, path.clone())
            }
            Some((AddressType::JsonPath, SubKeySpec::Numeric(spec))) => {
                let path_range = spec.range_size().max(1);
                let key_idx = (idx / path_range) % key_range;
                let path_idx = idx % path_range;
                Address::json_path(self.key.format(key_idx), spec.format(path_idx))
            }
            Some((AddressType::JsonPath, SubKeySpec::List { values, .. })) => {
                let num_paths = values.len() as u64;
                let key_idx = (idx / num_paths) % key_range;
                let path_idx = (idx % num_paths) as usize;
                Address::json_path(self.key.format(key_idx), values[path_idx].clone())
            }
            Some((AddressType::Key, _)) => Address::key(key),
        }
    }

    fn address_type(&self) -> AddressType {
        AddressSpec::address_type(self)
    }

    fn key_prefix(&self) -> &str {
        &self.key.prefix
    }
}

/// Thread-safe address iterator
pub struct AddressIterator {
    spec: AddressSpec,
    counter: AtomicU64,
}

impl AddressIterator {
    /// Create a new address iterator from AddressSpec
    pub fn from_spec(spec: AddressSpec) -> Self {
        Self {
            spec,
            counter: AtomicU64::new(0),
        }
    }

    /// Claim the next address
    pub fn next_address(&self) -> Address {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed);
        self.address_at(idx)
    }

    /// Get address at specific index
    fn address_at(&self, idx: u64) -> Address {
        let key_range = self.spec.key.range_size().max(1);
        let wrapped_key_idx = idx % key_range;
        let key = self.spec.key.format(wrapped_key_idx);

        match &self.spec.sub_key {
            None => Address::key(key),
            Some((AddressType::HashField, SubKeySpec::Literal(field))) => {
                Address::hash_field(key, field.clone())
            }
            Some((AddressType::HashField, SubKeySpec::Numeric(spec))) => {
                // Independent ranges: key and field iterate separately
                let field_range = spec.range_size().max(1);
                let key_idx = (idx / field_range) % key_range;
                let field_idx = idx % field_range;
                Address::hash_field(
                    self.spec.key.format(key_idx),
                    spec.format(field_idx),
                )
            }
            Some((AddressType::HashField, SubKeySpec::List { values, .. })) => {
                let num_fields = values.len() as u64;
                let key_idx = (idx / num_fields) % key_range;
                let field_idx = (idx % num_fields) as usize;
                Address::hash_field(
                    self.spec.key.format(key_idx),
                    values[field_idx].clone(),
                )
            }
            Some((AddressType::JsonPath, SubKeySpec::Literal(path))) => {
                Address::json_path(key, path.clone())
            }
            Some((AddressType::JsonPath, SubKeySpec::Numeric(spec))) => {
                let path_range = spec.range_size().max(1);
                let key_idx = (idx / path_range) % key_range;
                let path_idx = idx % path_range;
                Address::json_path(
                    self.spec.key.format(key_idx),
                    spec.format(path_idx),
                )
            }
            Some((AddressType::JsonPath, SubKeySpec::List { values, .. })) => {
                let num_paths = values.len() as u64;
                let key_idx = (idx / num_paths) % key_range;
                let path_idx = (idx % num_paths) as usize;
                Address::json_path(
                    self.spec.key.format(key_idx),
                    values[path_idx].clone(),
                )
            }
            Some((AddressType::Key, _)) => Address::key(key),
        }
    }

    /// Get the current counter value
    pub fn counter(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }

    /// Reset the counter
    pub fn reset(&self) {
        self.counter.store(0, Ordering::Relaxed);
    }

    /// Get the address type
    pub fn address_type(&self) -> AddressType {
        self.spec.address_type()
    }

    /// Get the space length
    pub fn len(&self) -> u64 {
        self.spec.total_len()
    }

    /// Get the underlying spec
    pub fn spec(&self) -> &AddressSpec {
        &self.spec
    }
}

/// Parse address type specification from CLI
///
/// Formats:
/// - "key" or "key:prefix" - Simple keys
/// - "hash:prefix:field1,field2" - Hash fields with list iteration
/// - "hash:prefix:field_prefix:100" - Hash fields with numeric iteration (0-99)
/// - "json:prefix:$.path1,$.path2" - JSON paths
///
/// Returns an `AddressSpec` which implements `AddressableSpace`
pub fn parse_address_type(spec_str: &str, num_keys: u64) -> Result<AddressSpec, String> {
    let spec_str = spec_str.trim();

    if spec_str.is_empty() || spec_str == "key" {
        return Ok(AddressSpec::key("", num_keys));
    }

    if let Some(rest) = spec_str.strip_prefix("key:") {
        return Ok(AddressSpec::key(rest, num_keys));
    }

    if let Some(rest) = spec_str.strip_prefix("hash:") {
        return parse_hash_spec(rest, num_keys);
    }

    if let Some(rest) = spec_str.strip_prefix("json:") {
        return parse_json_spec(rest, num_keys);
    }

    Err(format!(
        "Unknown address type: {}. Use 'key', 'hash:prefix:fields', or 'json:prefix:paths'",
        spec_str
    ))
}

/// Parse hash field specification
/// Formats:
/// - "prefix:field1,field2" - List of field names
/// - "prefix:field_prefix:100" - Numeric field iteration
fn parse_hash_spec(spec: &str, num_keys: u64) -> Result<AddressSpec, String> {
    let parts: Vec<&str> = spec.splitn(2, ':').collect();
    if parts.len() < 2 {
        return Err("Hash spec requires 'prefix:field1,field2,...' format".to_string());
    }

    let key_prefix = parts[0];
    let field_part = parts[1];

    // Check if it's numeric format: "field_prefix:count"
    if let Some((field_prefix, count_str)) = field_part.rsplit_once(':') {
        if let Ok(count) = count_str.parse::<u64>() {
            // Numeric field iteration
            return Ok(AddressSpec::hash_numeric_field(key_prefix, num_keys, field_prefix, count));
        }
    }

    // List of fields
    let fields: Vec<String> = field_part
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if fields.is_empty() {
        return Err("At least one field name is required".to_string());
    }

    Ok(AddressSpec::hash_fields(key_prefix, num_keys, fields))
}

/// Parse JSON path specification
/// Formats:
/// - "prefix:$.path1,$.path2" - List of paths
/// - "prefix:$.path_:100" - Numeric path iteration
fn parse_json_spec(spec: &str, num_keys: u64) -> Result<AddressSpec, String> {
    let parts: Vec<&str> = spec.splitn(2, ':').collect();
    if parts.len() < 2 {
        return Err("JSON spec requires 'prefix:$.path1,$.path2,...' format".to_string());
    }

    let key_prefix = parts[0];
    let path_part = parts[1];

    // Check if it's numeric format: "$.path_prefix:count"
    if let Some((path_prefix, count_str)) = path_part.rsplit_once(':') {
        if let Ok(count) = count_str.parse::<u64>() {
            // Numeric path iteration
            return Ok(AddressSpec::new(
                PlaceholderSpec::new(key_prefix, num_keys),
                Some((AddressType::JsonPath, SubKeySpec::numeric(path_prefix, count))),
            ));
        }
    }

    // List of paths
    let paths: Vec<String> = path_part
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if paths.is_empty() {
        return Err("At least one JSON path is required".to_string());
    }

    Ok(AddressSpec::json_paths(key_prefix, num_keys, paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // AddressSpec tests
    // ============================================================================

    #[test]
    fn test_address_spec_key_only() {
        let spec = AddressSpec::key("vec:", 1_000_000);
        assert_eq!(spec.prefix(), "vec:");
        assert_eq!(spec.key_width(), DEFAULT_KEY_WIDTH);
        assert_eq!(spec.max_id(), 1_000_000);
        assert_eq!(spec.address_type(), AddressType::Key);
        assert_eq!(spec.total_len(), 1_000_000);
    }

    #[test]
    fn test_address_spec_format_key() {
        let spec = AddressSpec::key("vec:", 1_000_000);
        assert_eq!(spec.format_key(0), "vec:000000000000");
        assert_eq!(spec.format_key(123), "vec:000000000123");
        assert_eq!(spec.format_key(999_999), "vec:000000999999");
    }

    #[test]
    fn test_address_spec_parse_key() {
        let spec = AddressSpec::key("vec:", 1_000_000);
        assert_eq!(spec.parse_key("vec:000000000000"), Some(0));
        assert_eq!(spec.parse_key("vec:000000000123"), Some(123));
        assert_eq!(spec.parse_key("vec:000000999999"), Some(999_999));
        // Wrong prefix
        assert_eq!(spec.parse_key("other:000000000123"), None);
    }

    #[test]
    fn test_address_spec_hash_field_literal() {
        let spec = AddressSpec::hash_field("obj:", 100_000, "embedding");
        assert_eq!(spec.address_type(), AddressType::HashField);
        assert_eq!(spec.total_len(), 100_000); // Single literal field doesn't multiply
    }

    #[test]
    fn test_address_spec_hash_fields_iterable() {
        let spec = AddressSpec::hash_fields(
            "obj:",
            100,
            vec!["f1".to_string(), "f2".to_string(), "f3".to_string()],
        );
        assert_eq!(spec.address_type(), AddressType::HashField);
        assert_eq!(spec.total_len(), 300); // 100 keys × 3 fields
    }

    #[test]
    fn test_address_spec_iterator() {
        let spec = AddressSpec::key("k:", 10);
        let iter = spec.to_iterator();

        let addr1 = iter.next_address();
        let addr2 = iter.next_address();
        assert_ne!(addr1.key, addr2.key);
        assert_eq!(iter.counter(), 2);

        iter.reset();
        assert_eq!(iter.counter(), 0);
    }

    #[test]
    fn test_address_spec_hash_iterator() {
        let spec = AddressSpec::hash_fields(
            "obj:",
            100,
            vec!["f1".to_string(), "f2".to_string()],
        );
        let iter = spec.to_iterator();

        // First iteration: key 0, field f1
        let addr0 = iter.next_address();
        assert_eq!(addr0.field, Some("f1".to_string()));

        // Second iteration: key 0, field f2
        let addr1 = iter.next_address();
        assert_eq!(addr0.key, addr1.key); // Same key
        assert_eq!(addr1.field, Some("f2".to_string()));

        // Third iteration: key 1, field f1
        let addr2 = iter.next_address();
        assert_ne!(addr0.key, addr2.key); // Different key
        assert_eq!(addr2.field, Some("f1".to_string()));
    }

    // ============================================================================
    // AddressSpec as AddressableSpace tests
    // ============================================================================

    #[test]
    fn test_key_space() {
        let space = AddressSpec::key("test:", 1000);
        assert_eq!(space.len(), 1000);
        assert_eq!(AddressableSpace::address_type(&space), AddressType::Key);

        let addr = space.address_at(0);
        assert!(addr.key.starts_with("test:"));
        assert!(addr.field.is_none());
    }

    #[test]
    fn test_key_space_wraparound() {
        let space = AddressSpec::key("k:", 10);
        let addr1 = space.address_at(5);
        let addr2 = space.address_at(15); // Should wrap to 5
        assert_eq!(addr1.key, addr2.key);
    }

    #[test]
    fn test_hash_field_space() {
        let space = AddressSpec::hash_fields("obj:", 100, vec!["f1".to_string(), "f2".to_string(), "f3".to_string()]);
        assert_eq!(space.len(), 300); // 100 keys * 3 fields
        assert_eq!(AddressableSpace::address_type(&space), AddressType::HashField);

        // First key, first field
        let addr0 = space.address_at(0);
        assert!(addr0.key.starts_with("obj:"));
        assert_eq!(addr0.field, Some("f1".to_string()));

        // First key, second field
        let addr1 = space.address_at(1);
        assert_eq!(addr0.key, addr1.key); // Same key
        assert_eq!(addr1.field, Some("f2".to_string()));

        // First key, third field
        let addr2 = space.address_at(2);
        assert_eq!(addr0.key, addr2.key); // Same key
        assert_eq!(addr2.field, Some("f3".to_string()));

        // Second key, first field
        let addr3 = space.address_at(3);
        assert_ne!(addr0.key, addr3.key); // Different key
        assert_eq!(addr3.field, Some("f1".to_string()));
    }

    #[test]
    fn test_hash_field_parse() {
        let spec = parse_address_type("hash:obj:f1,f2,f3", 100).unwrap();
        assert_eq!(spec.total_len(), 300);
        assert_eq!(spec.key_prefix(), "obj");
    }

    #[test]
    fn test_json_path_space() {
        let space = AddressSpec::json_paths("doc:", 50, vec!["$.name".to_string(), "$.value".to_string()]);
        assert_eq!(space.len(), 100); // 50 keys * 2 paths
        assert_eq!(AddressableSpace::address_type(&space), AddressType::JsonPath);

        let addr = space.address_at(0);
        assert!(addr.key.starts_with("doc:"));
        assert_eq!(addr.path, Some("$.name".to_string()));
    }

    #[test]
    fn test_parse_address_type_key() {
        let spec = parse_address_type("key", 100).unwrap();
        assert_eq!(spec.address_type(), AddressType::Key);
        assert_eq!(spec.total_len(), 100);
    }

    #[test]
    fn test_parse_address_type_key_with_prefix() {
        let spec = parse_address_type("key:myprefix:", 100).unwrap();
        assert_eq!(spec.address_type(), AddressType::Key);
        let addr = spec.address_at(0);
        assert!(addr.key.starts_with("myprefix:"));
    }

    #[test]
    fn test_parse_address_type_hash() {
        let spec = parse_address_type("hash:obj:f1,f2,f3", 100).unwrap();
        assert_eq!(spec.address_type(), AddressType::HashField);
        assert_eq!(spec.total_len(), 300);
    }

    #[test]
    fn test_parse_address_type_json() {
        let spec = parse_address_type("json:doc:$.name,$.age", 50).unwrap();
        assert_eq!(spec.address_type(), AddressType::JsonPath);
        assert_eq!(spec.total_len(), 100);
    }

    #[test]
    fn test_parse_address_type_invalid() {
        let result = parse_address_type("unknown:something", 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_address_iterator_legacy() {
        let spec = AddressSpec::key("k:", 10);
        let iter = AddressIterator::from_spec(spec);

        let addr1 = iter.next_address();
        let addr2 = iter.next_address();
        assert_ne!(addr1.key, addr2.key);
        assert_eq!(iter.counter(), 2);

        iter.reset();
        assert_eq!(iter.counter(), 0);
    }

    #[test]
    fn test_address_types() {
        assert_eq!(Address::key("k".to_string()).address_type(), AddressType::Key);
        assert_eq!(Address::hash_field("k".to_string(), "f".to_string()).address_type(), AddressType::HashField);
        assert_eq!(Address::json_path("k".to_string(), "$.p".to_string()).address_type(), AddressType::JsonPath);
    }
}
