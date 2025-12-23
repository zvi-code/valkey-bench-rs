//! Addressable space abstraction for workloads
//!
//! This module provides support for addressing beyond simple keys:
//! - Top-level keys (default)
//! - Hash fields (HSET key field value)
//! - JSON paths (JSON.SET key $.path value)
//! - Pub/Sub channels

use std::sync::atomic::{AtomicU64, Ordering};

/// Type of address being used
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum AddressType {
    /// Simple key (default)
    #[default]
    Key,
    /// Hash field (key + field name)
    HashField,
    /// JSON path (key + JSON path)
    JsonPath,
    /// Pub/Sub channel
    Channel,
}


impl std::fmt::Display for AddressType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressType::Key => write!(f, "key"),
            AddressType::HashField => write!(f, "hash"),
            AddressType::JsonPath => write!(f, "json"),
            AddressType::Channel => write!(f, "channel"),
        }
    }
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

/// Simple key space (keys only)
#[derive(Debug, Clone)]
pub struct KeySpace {
    /// Key prefix
    prefix: String,
    /// Total number of keys
    size: u64,
    /// Key width for formatting
    key_width: usize,
}

impl KeySpace {
    /// Create a new key space
    pub fn new(prefix: &str, size: u64) -> Self {
        // Calculate key width to fit all keys
        let key_width = if size > 0 {
            ((size - 1) as f64).log10().ceil() as usize + 1
        } else {
            1
        };

        Self {
            prefix: prefix.to_string(),
            size,
            key_width: key_width.max(12), // Minimum 12 digits for consistency
        }
    }

    /// Format a key from an index
    fn format_key(&self, idx: u64) -> String {
        format!("{}{:0width$}", self.prefix, idx, width = self.key_width)
    }
}

impl AddressableSpace for KeySpace {
    fn len(&self) -> u64 {
        self.size
    }

    fn address_at(&self, idx: u64) -> Address {
        Address::key(self.format_key(idx % self.size))
    }

    fn address_type(&self) -> AddressType {
        AddressType::Key
    }

    fn key_prefix(&self) -> &str {
        &self.prefix
    }
}

/// Hash field space (keys with multiple fields)
#[derive(Debug, Clone)]
pub struct HashFieldSpace {
    /// Key prefix
    prefix: String,
    /// Number of keys
    num_keys: u64,
    /// Field names
    fields: Vec<String>,
    /// Key width for formatting
    key_width: usize,
}

impl HashFieldSpace {
    /// Create a new hash field space
    pub fn new(prefix: &str, num_keys: u64, fields: Vec<String>) -> Self {
        let key_width = if num_keys > 0 {
            ((num_keys - 1) as f64).log10().ceil() as usize + 1
        } else {
            1
        };

        Self {
            prefix: prefix.to_string(),
            num_keys,
            fields,
            key_width: key_width.max(12),
        }
    }

    /// Parse from CLI specification: "prefix:field1,field2,field3"
    pub fn parse(spec: &str, num_keys: u64) -> Result<Self, String> {
        let parts: Vec<&str> = spec.splitn(2, ':').collect();
        if parts.len() < 2 {
            return Err("Hash field spec requires 'prefix:field1,field2,...' format".to_string());
        }

        let prefix = parts[0];
        let fields: Vec<String> = parts[1]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if fields.is_empty() {
            return Err("At least one field name is required".to_string());
        }

        Ok(Self::new(prefix, num_keys, fields))
    }

    /// Get field names
    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    /// Format a key from an index
    fn format_key(&self, key_idx: u64) -> String {
        format!("{}{:0width$}", self.prefix, key_idx, width = self.key_width)
    }
}

impl AddressableSpace for HashFieldSpace {
    fn len(&self) -> u64 {
        self.num_keys * self.fields.len() as u64
    }

    fn address_at(&self, idx: u64) -> Address {
        let num_fields = self.fields.len() as u64;
        let key_idx = (idx / num_fields) % self.num_keys;
        let field_idx = (idx % num_fields) as usize;

        Address::hash_field(
            self.format_key(key_idx),
            self.fields[field_idx].clone(),
        )
    }

    fn address_type(&self) -> AddressType {
        AddressType::HashField
    }

    fn key_prefix(&self) -> &str {
        &self.prefix
    }
}

/// JSON path space (keys with JSON paths)
#[derive(Debug, Clone)]
pub struct JsonPathSpace {
    /// Key prefix
    prefix: String,
    /// Number of keys
    num_keys: u64,
    /// JSON paths
    paths: Vec<String>,
    /// Key width for formatting
    key_width: usize,
}

impl JsonPathSpace {
    /// Create a new JSON path space
    pub fn new(prefix: &str, num_keys: u64, paths: Vec<String>) -> Self {
        let key_width = if num_keys > 0 {
            ((num_keys - 1) as f64).log10().ceil() as usize + 1
        } else {
            1
        };

        Self {
            prefix: prefix.to_string(),
            num_keys,
            paths,
            key_width: key_width.max(12),
        }
    }

    /// Parse from CLI specification: "prefix:$.path1,$.path2"
    pub fn parse(spec: &str, num_keys: u64) -> Result<Self, String> {
        let parts: Vec<&str> = spec.splitn(2, ':').collect();
        if parts.len() < 2 {
            return Err("JSON path spec requires 'prefix:$.path1,$.path2,...' format".to_string());
        }

        let prefix = parts[0];
        let paths: Vec<String> = parts[1]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if paths.is_empty() {
            return Err("At least one JSON path is required".to_string());
        }

        Ok(Self::new(prefix, num_keys, paths))
    }

    /// Get JSON paths
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// Format a key from an index
    fn format_key(&self, key_idx: u64) -> String {
        format!("{}{:0width$}", self.prefix, key_idx, width = self.key_width)
    }
}

impl AddressableSpace for JsonPathSpace {
    fn len(&self) -> u64 {
        self.num_keys * self.paths.len() as u64
    }

    fn address_at(&self, idx: u64) -> Address {
        let num_paths = self.paths.len() as u64;
        let key_idx = (idx / num_paths) % self.num_keys;
        let path_idx = (idx % num_paths) as usize;

        Address::json_path(
            self.format_key(key_idx),
            self.paths[path_idx].clone(),
        )
    }

    fn address_type(&self) -> AddressType {
        AddressType::JsonPath
    }

    fn key_prefix(&self) -> &str {
        &self.prefix
    }
}

/// Thread-safe address iterator
pub struct AddressIterator {
    space: Box<dyn AddressableSpace>,
    counter: AtomicU64,
}

impl AddressIterator {
    /// Create a new address iterator
    pub fn new(space: Box<dyn AddressableSpace>) -> Self {
        Self {
            space,
            counter: AtomicU64::new(0),
        }
    }

    /// Claim the next address
    pub fn next_address(&self) -> Address {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed);
        self.space.address_at(idx)
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
        self.space.address_type()
    }

    /// Get the space length
    pub fn len(&self) -> u64 {
        self.space.len()
    }
}

/// Parse address type specification from CLI
///
/// Formats:
/// - "key" or "key:prefix" - Simple keys
/// - "hash:prefix:field1,field2" - Hash fields
/// - "json:prefix:$.path1,$.path2" - JSON paths
pub fn parse_address_type(spec: &str, num_keys: u64) -> Result<Box<dyn AddressableSpace>, String> {
    let spec = spec.trim();

    if spec.is_empty() || spec == "key" {
        return Ok(Box::new(KeySpace::new("", num_keys)));
    }

    if let Some(rest) = spec.strip_prefix("key:") {
        return Ok(Box::new(KeySpace::new(rest, num_keys)));
    }

    if let Some(rest) = spec.strip_prefix("hash:") {
        return Ok(Box::new(HashFieldSpace::parse(rest, num_keys)?));
    }

    if let Some(rest) = spec.strip_prefix("json:") {
        return Ok(Box::new(JsonPathSpace::parse(rest, num_keys)?));
    }

    Err(format!("Unknown address type: {}. Use 'key', 'hash:prefix:fields', or 'json:prefix:paths'", spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_space() {
        let space = KeySpace::new("test:", 1000);
        assert_eq!(space.len(), 1000);
        assert_eq!(space.address_type(), AddressType::Key);

        let addr = space.address_at(0);
        assert!(addr.key.starts_with("test:"));
        assert!(addr.field.is_none());
    }

    #[test]
    fn test_key_space_wraparound() {
        let space = KeySpace::new("k:", 10);
        let addr1 = space.address_at(5);
        let addr2 = space.address_at(15); // Should wrap to 5
        assert_eq!(addr1.key, addr2.key);
    }

    #[test]
    fn test_hash_field_space() {
        let space = HashFieldSpace::new("obj:", 100, vec!["f1".to_string(), "f2".to_string(), "f3".to_string()]);
        assert_eq!(space.len(), 300); // 100 keys * 3 fields
        assert_eq!(space.address_type(), AddressType::HashField);

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
        let space = HashFieldSpace::parse("obj:f1,f2,f3", 100).unwrap();
        assert_eq!(space.fields().len(), 3);
        assert_eq!(space.key_prefix(), "obj");
    }

    #[test]
    fn test_json_path_space() {
        let space = JsonPathSpace::new("doc:", 50, vec!["$.name".to_string(), "$.value".to_string()]);
        assert_eq!(space.len(), 100); // 50 keys * 2 paths
        assert_eq!(space.address_type(), AddressType::JsonPath);

        let addr = space.address_at(0);
        assert!(addr.key.starts_with("doc:"));
        assert_eq!(addr.path, Some("$.name".to_string()));
    }

    #[test]
    fn test_parse_address_type_key() {
        let space = parse_address_type("key", 100).unwrap();
        assert_eq!(space.address_type(), AddressType::Key);
        assert_eq!(space.len(), 100);
    }

    #[test]
    fn test_parse_address_type_key_with_prefix() {
        let space = parse_address_type("key:myprefix:", 100).unwrap();
        assert_eq!(space.address_type(), AddressType::Key);
        let addr = space.address_at(0);
        assert!(addr.key.starts_with("myprefix:"));
    }

    #[test]
    fn test_parse_address_type_hash() {
        let space = parse_address_type("hash:obj:f1,f2,f3", 100).unwrap();
        assert_eq!(space.address_type(), AddressType::HashField);
        assert_eq!(space.len(), 300);
    }

    #[test]
    fn test_parse_address_type_json() {
        let space = parse_address_type("json:doc:$.name,$.age", 50).unwrap();
        assert_eq!(space.address_type(), AddressType::JsonPath);
        assert_eq!(space.len(), 100);
    }

    #[test]
    fn test_parse_address_type_invalid() {
        let result = parse_address_type("unknown:something", 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_address_iterator() {
        let space = Box::new(KeySpace::new("k:", 10));
        let iter = AddressIterator::new(space);

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
