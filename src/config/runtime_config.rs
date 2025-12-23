//! Runtime configuration management
//!
//! Parses and applies server-side configurations via CONFIG SET before benchmarks.
//! Supports the standard valkey.conf key-value format.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use crate::client::{ConnectionFactory, ControlPlane, ControlPlaneExt, RawConnection};
use crate::cluster::ClusterTopology;
use crate::utils::RespValue;

/// A single configuration entry
#[derive(Debug, Clone)]
pub struct ConfigEntry {
    /// Configuration key (e.g., "io-threads", "maxmemory")
    pub key: String,
    /// Configuration value (e.g., "8", "10gb")
    pub value: String,
}

impl ConfigEntry {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// Runtime configuration parsed from a config file
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    /// Configuration entries to apply
    entries: Vec<ConfigEntry>,
}

impl RuntimeConfig {
    /// Create a new empty runtime config
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse runtime config from a file
    ///
    /// The file format is compatible with valkey.conf:
    /// - Lines starting with # are comments
    /// - Empty lines are ignored
    /// - Format: key value (space separated) or key=value
    /// - Values can be quoted with "" for empty or space-containing values
    pub fn from_file(path: &Path) -> io::Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse runtime config from a string
    pub fn parse(content: &str) -> io::Result<Self> {
        let mut entries = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse key-value pair
            let (key, value) = Self::parse_line(line).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Line {}: {}", line_num + 1, e),
                )
            })?;

            entries.push(ConfigEntry::new(key, value));
        }

        Ok(Self { entries })
    }

    /// Parse a single line into key-value pair
    fn parse_line(line: &str) -> Result<(String, String), String> {
        // Try key=value format first
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();
            return Ok((key.to_string(), Self::unquote(value)));
        }

        // Try key value format (space separated)
        // Handle quoted values: key "value with spaces"
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.len() < 2 {
            return Err(format!("Invalid format: expected 'key value' or 'key=value', got '{}'", line));
        }

        let key = parts[0].trim();
        let value = parts[1].trim();

        if key.is_empty() {
            return Err("Empty key".to_string());
        }

        Ok((key.to_string(), Self::unquote(value)))
    }

    /// Remove surrounding quotes from a value
    fn unquote(value: &str) -> String {
        let value = value.trim();
        if ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
            && value.len() >= 2 {
                return value[1..value.len() - 1].to_string();
            }
        value.to_string()
    }

    /// Get the number of configuration entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the configuration is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get iterator over configuration entries
    pub fn entries(&self) -> impl Iterator<Item = &ConfigEntry> {
        self.entries.iter()
    }
}

/// Result of applying a configuration to a node
#[derive(Debug)]
pub struct ConfigApplyResult {
    /// Host:port of the node
    pub node: String,
    /// Configurations successfully applied
    pub applied: Vec<ConfigEntry>,
    /// Configurations that failed to apply (key, error message)
    pub failed: Vec<(String, String)>,
}

impl ConfigApplyResult {
    pub fn new(node: &str) -> Self {
        Self {
            node: node.to_string(),
            applied: Vec::new(),
            failed: Vec::new(),
        }
    }

    pub fn add_success(&mut self, entry: ConfigEntry) {
        self.applied.push(entry);
    }

    pub fn add_failure(&mut self, key: String, error: String) {
        self.failed.push((key, error));
    }

    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Result of verifying configurations on a node
#[derive(Debug)]
pub struct ConfigVerifyResult {
    /// Host:port of the node
    pub node: String,
    /// Configurations that match expected values
    pub matched: Vec<ConfigEntry>,
    /// Configurations that don't match (key, expected, actual)
    pub mismatched: Vec<(String, String, String)>,
    /// Configurations that couldn't be verified (key, error)
    pub errors: Vec<(String, String)>,
}

impl ConfigVerifyResult {
    pub fn new(node: &str) -> Self {
        Self {
            node: node.to_string(),
            matched: Vec::new(),
            mismatched: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn add_match(&mut self, entry: ConfigEntry) {
        self.matched.push(entry);
    }

    pub fn add_mismatch(&mut self, key: String, expected: String, actual: String) {
        self.mismatched.push((key, expected, actual));
    }

    pub fn add_error(&mut self, key: String, error: String) {
        self.errors.push((key, error));
    }

    pub fn is_success(&self) -> bool {
        self.mismatched.is_empty() && self.errors.is_empty()
    }
}

/// Runtime configuration manager
///
/// Applies and verifies configurations across cluster nodes.
pub struct RuntimeConfigManager<'a> {
    config: &'a RuntimeConfig,
    connection_factory: &'a ConnectionFactory,
    quiet: bool,
}

impl<'a> RuntimeConfigManager<'a> {
    /// Create a new runtime config manager
    pub fn new(
        config: &'a RuntimeConfig,
        connection_factory: &'a ConnectionFactory,
        quiet: bool,
    ) -> Self {
        Self {
            config,
            connection_factory,
            quiet,
        }
    }

    /// Get all node addresses to configure
    fn get_node_addresses(
        &self,
        topology: Option<&ClusterTopology>,
        seed_addresses: &[(String, u16)],
    ) -> Vec<(String, u16)> {
        if let Some(topo) = topology {
            // Cluster mode: configure all nodes (primaries and replicas)
            topo.all_node_addresses()
                .iter()
                .map(|(h, p, _)| (h.clone(), *p))
                .collect()
        } else {
            // Standalone mode: use seed addresses
            seed_addresses.to_vec()
        }
    }

    /// Apply all configurations to all nodes
    ///
    /// Returns results for each node.
    pub fn apply_all(
        &self,
        topology: Option<&ClusterTopology>,
        seed_addresses: &[(String, u16)],
    ) -> Vec<ConfigApplyResult> {
        let addresses = self.get_node_addresses(topology, seed_addresses);
        let mut results = Vec::new();

        if !self.quiet {
            println!("\n=== Applying Runtime Configuration ===");
            println!("Configuration entries: {}", self.config.len());
        }

        for (host, port) in addresses {
            let node_id = format!("{}:{}", host, port);
            let result = self.apply_to_node(&host, port);

            if !self.quiet {
                for entry in &result.applied {
                    println!("✓ Set {} = {} on {}", entry.key, entry.value, node_id);
                }
                for (key, error) in &result.failed {
                    println!("✗ Failed to set {} on {}: {}", key, node_id, error);
                }
            }

            results.push(result);
        }

        if !self.quiet {
            let total_applied: usize = results.iter().map(|r| r.applied.len()).sum();
            let total_failed: usize = results.iter().map(|r| r.failed.len()).sum();
            println!("Total configurations applied: {}", total_applied);
            if total_failed > 0 {
                println!("Total configurations failed: {}", total_failed);
            }
            println!("=======================================\n");
        }

        results
    }

    /// Apply configurations to a single node
    fn apply_to_node(&self, host: &str, port: u16) -> ConfigApplyResult {
        let node_id = format!("{}:{}", host, port);
        let mut result = ConfigApplyResult::new(&node_id);

        // Create connection
        let mut conn = match self.connection_factory.create(host, port) {
            Ok(c) => c,
            Err(e) => {
                // All entries failed due to connection error
                for entry in self.config.entries() {
                    result.add_failure(entry.key.clone(), format!("Connection failed: {}", e));
                }
                return result;
            }
        };

        // Apply each configuration
        for entry in self.config.entries() {
            match self.apply_config(&mut conn, &entry.key, &entry.value) {
                Ok(()) => result.add_success(entry.clone()),
                Err(e) => result.add_failure(entry.key.clone(), e),
            }
        }

        result
    }

    /// Apply a single CONFIG SET command
    fn apply_config(&self, conn: &mut RawConnection, key: &str, value: &str) -> Result<(), String> {
        let response = conn
            .execute(&["CONFIG", "SET", key, value])
            .map_err(|e| format!("Command failed: {}", e))?;

        match response {
            RespValue::SimpleString(s) if s == "OK" => Ok(()),
            RespValue::Error(e) => Err(e),
            other => Err(format!("Unexpected response: {:?}", other)),
        }
    }

    /// Verify all configurations on all nodes
    ///
    /// Returns results for each node.
    pub fn verify_all(
        &self,
        topology: Option<&ClusterTopology>,
        seed_addresses: &[(String, u16)],
    ) -> Vec<ConfigVerifyResult> {
        let addresses = self.get_node_addresses(topology, seed_addresses);
        let mut results = Vec::new();

        if !self.quiet {
            println!("\n=== Verifying Runtime Configuration ===");
        }

        for (host, port) in addresses {
            let result = self.verify_on_node(&host, port);
            results.push(result);
        }

        if !self.quiet {
            let total_matched: usize = results.iter().map(|r| r.matched.len()).sum();
            let total_mismatched: usize = results.iter().map(|r| r.mismatched.len()).sum();
            let total_errors: usize = results.iter().map(|r| r.errors.len()).sum();

            println!("Verified: {} matched, {} mismatched, {} errors",
                total_matched, total_mismatched, total_errors);

            for result in &results {
                for (key, expected, actual) in &result.mismatched {
                    println!("✗ {} on {}: expected '{}', got '{}'",
                        key, result.node, expected, actual);
                }
                for (key, error) in &result.errors {
                    println!("✗ {} on {}: {}", key, result.node, error);
                }
            }
            println!("========================================\n");
        }

        results
    }

    /// Verify configurations on a single node
    fn verify_on_node(&self, host: &str, port: u16) -> ConfigVerifyResult {
        let node_id = format!("{}:{}", host, port);
        let mut result = ConfigVerifyResult::new(&node_id);

        // Create connection
        let mut conn = match self.connection_factory.create(host, port) {
            Ok(c) => c,
            Err(e) => {
                for entry in self.config.entries() {
                    result.add_error(entry.key.clone(), format!("Connection failed: {}", e));
                }
                return result;
            }
        };

        // Verify each configuration
        for entry in self.config.entries() {
            match self.get_config(&mut conn, &entry.key) {
                Ok(actual) => {
                    if Self::values_match(&entry.value, &actual) {
                        result.add_match(entry.clone());
                    } else {
                        result.add_mismatch(entry.key.clone(), entry.value.clone(), actual);
                    }
                }
                Err(e) => {
                    result.add_error(entry.key.clone(), e);
                }
            }
        }

        result
    }

    /// Get a configuration value using CONFIG GET
    fn get_config(&self, conn: &mut RawConnection, key: &str) -> Result<String, String> {
        let response = conn
            .execute(&["CONFIG", "GET", key])
            .map_err(|e| format!("Command failed: {}", e))?;

        match response {
            RespValue::Array(arr) if arr.len() >= 2 => {
                // CONFIG GET returns [key, value, ...]
                match &arr[1] {
                    RespValue::BulkString(data) => {
                        String::from_utf8(data.clone())
                            .map_err(|e| format!("Invalid UTF-8: {}", e))
                    }
                    RespValue::SimpleString(s) => Ok(s.clone()),
                    other => Err(format!("Unexpected value type: {:?}", other)),
                }
            }
            RespValue::Array(arr) if arr.is_empty() => {
                Err("Configuration key not found".to_string())
            }
            RespValue::Error(e) => Err(e),
            other => Err(format!("Unexpected response: {:?}", other)),
        }
    }

    /// Compare config values (handles various formats)
    fn values_match(expected: &str, actual: &str) -> bool {
        // Normalize both values for comparison
        let expected = expected.trim().to_lowercase();
        let actual = actual.trim().to_lowercase();

        if expected == actual {
            return true;
        }

        // Handle empty string representations
        if (expected.is_empty() || expected == "\"\"") && actual.is_empty() {
            return true;
        }

        // Handle yes/no vs 1/0
        if (expected == "yes" && actual == "1") || (expected == "1" && actual == "yes") {
            return true;
        }
        if (expected == "no" && actual == "0") || (expected == "0" && actual == "no") {
            return true;
        }

        // Handle memory units (10gb vs 10737418240)
        if let (Some(expected_bytes), Some(actual_bytes)) =
            (Self::parse_memory_value(&expected), Self::parse_memory_value(&actual)) {
            return expected_bytes == actual_bytes;
        }

        false
    }

    /// Parse memory value with optional suffix (kb, mb, gb)
    fn parse_memory_value(value: &str) -> Option<u64> {
        let value = value.trim().to_lowercase();

        if let Ok(n) = value.parse::<u64>() {
            return Some(n);
        }

        let multiplier = if value.ends_with("kb") || value.ends_with("k") {
            1024
        } else if value.ends_with("mb") || value.ends_with("m") {
            1024 * 1024
        } else if value.ends_with("gb") || value.ends_with("g") {
            1024 * 1024 * 1024
        } else {
            return None;
        };

        let num_str = value.trim_end_matches(|c: char| c.is_alphabetic());
        num_str.parse::<u64>().ok().map(|n| n * multiplier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_config() {
        let content = r#"
# This is a comment
io-threads 8
maxmemory 10gb

# Another comment
timeout 300
"#;
        let config = RuntimeConfig::parse(content).unwrap();
        assert_eq!(config.len(), 3);

        let entries: Vec<_> = config.entries().collect();
        assert_eq!(entries[0].key, "io-threads");
        assert_eq!(entries[0].value, "8");
        assert_eq!(entries[1].key, "maxmemory");
        assert_eq!(entries[1].value, "10gb");
        assert_eq!(entries[2].key, "timeout");
        assert_eq!(entries[2].value, "300");
    }

    #[test]
    fn test_parse_key_equals_value() {
        let content = "io-threads=8\nmaxmemory=10gb";
        let config = RuntimeConfig::parse(content).unwrap();
        assert_eq!(config.len(), 2);

        let entries: Vec<_> = config.entries().collect();
        assert_eq!(entries[0].key, "io-threads");
        assert_eq!(entries[0].value, "8");
    }

    #[test]
    fn test_parse_quoted_values() {
        let content = r#"
save ""
loglevel "notice"
"#;
        let config = RuntimeConfig::parse(content).unwrap();
        assert_eq!(config.len(), 2);

        let entries: Vec<_> = config.entries().collect();
        assert_eq!(entries[0].key, "save");
        assert_eq!(entries[0].value, "");
        assert_eq!(entries[1].key, "loglevel");
        assert_eq!(entries[1].value, "notice");
    }

    #[test]
    fn test_values_match_basic() {
        assert!(RuntimeConfigManager::<'_>::values_match("8", "8"));
        assert!(RuntimeConfigManager::<'_>::values_match(" 8 ", "8"));
        assert!(RuntimeConfigManager::<'_>::values_match("YES", "yes"));
    }

    #[test]
    fn test_values_match_yes_no() {
        assert!(RuntimeConfigManager::<'_>::values_match("yes", "1"));
        assert!(RuntimeConfigManager::<'_>::values_match("no", "0"));
        assert!(RuntimeConfigManager::<'_>::values_match("1", "yes"));
        assert!(RuntimeConfigManager::<'_>::values_match("0", "no"));
    }

    #[test]
    fn test_values_match_memory() {
        assert!(RuntimeConfigManager::<'_>::values_match("10gb", "10737418240"));
        assert!(RuntimeConfigManager::<'_>::values_match("1024kb", "1048576"));
        assert!(RuntimeConfigManager::<'_>::values_match("1mb", "1048576"));
    }

    #[test]
    fn test_values_match_empty() {
        assert!(RuntimeConfigManager::<'_>::values_match("\"\"", ""));
        assert!(RuntimeConfigManager::<'_>::values_match("", ""));
    }

    #[test]
    fn test_parse_memory_value() {
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("1024"), Some(1024));
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("1kb"), Some(1024));
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("1k"), Some(1024));
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("1mb"), Some(1024 * 1024));
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("1m"), Some(1024 * 1024));
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("1gb"), Some(1024 * 1024 * 1024));
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("1g"), Some(1024 * 1024 * 1024));
        assert_eq!(RuntimeConfigManager::<'_>::parse_memory_value("10gb"), Some(10 * 1024 * 1024 * 1024));
    }
}
