//! Per-workload configuration
//!
//! This module provides `WorkloadConfig` which encapsulates all settings
//! specific to a single workload component. This enables parallel and composite
//! workloads to have independent configurations for each component.

use std::path::PathBuf;

use super::cli::ReadFromReplica;
use super::search_config::SearchConfig;
use crate::workload::command_template::CommandTemplate;
use crate::workload::template_factory::create_template;
use crate::workload::{AddressSpec, WorkloadType};

/// Configuration for a single workload component
///
/// This struct holds all settings that can vary between workloads in
/// parallel or composite execution. Infrastructure settings (clients,
/// threads, pipeline) remain global as they define the execution environment.
#[derive(Debug, Clone)]
pub struct WorkloadConfig {
    /// The type of workload (GET, SET, VecLoad, etc.)
    pub workload_type: WorkloadType,

    /// Key prefix for this workload (e.g., "user:", "vec:")
    pub key_prefix: String,

    /// Keyspace size - number of unique keys
    pub keyspace: u64,

    /// Data size in bytes for SET/HSET payloads
    pub data_size: usize,

    /// Search configuration for vector workloads
    pub search_config: Option<SearchConfig>,

    /// Schema path for vector workloads
    pub schema_path: Option<PathBuf>,

    /// Data path for vector workloads
    pub data_path: Option<PathBuf>,

    /// Read-from-replica override for this workload
    /// None means use global setting
    pub rfr_override: Option<ReadFromReplica>,
}

impl WorkloadConfig {
    /// Create a new workload config with required fields
    pub fn new(workload_type: WorkloadType) -> Self {
        Self {
            workload_type,
            key_prefix: "key:".to_string(),
            keyspace: 1_000_000,
            data_size: 3,
            search_config: None,
            schema_path: None,
            data_path: None,
            rfr_override: None,
        }
    }

    /// Create from workload type with global defaults applied
    pub fn from_type_with_defaults(
        workload_type: WorkloadType,
        key_prefix: &str,
        keyspace: u64,
        data_size: usize,
        search_config: Option<SearchConfig>,
        schema_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
    ) -> Self {
        Self {
            workload_type,
            key_prefix: key_prefix.to_string(),
            keyspace,
            data_size,
            search_config,
            schema_path,
            data_path,
            rfr_override: None,
        }
    }

    /// Builder-style: set key prefix
    pub fn with_key_prefix(mut self, prefix: &str) -> Self {
        self.key_prefix = prefix.to_string();
        self
    }

    /// Builder-style: set keyspace
    pub fn with_keyspace(mut self, keyspace: u64) -> Self {
        self.keyspace = keyspace;
        self
    }

    /// Builder-style: set data size
    pub fn with_data_size(mut self, size: usize) -> Self {
        self.data_size = size;
        self
    }

    /// Builder-style: set search config
    pub fn with_search_config(mut self, config: SearchConfig) -> Self {
        self.search_config = Some(config);
        self
    }

    /// Builder-style: set schema and data paths
    pub fn with_dataset(mut self, schema_path: PathBuf, data_path: PathBuf) -> Self {
        self.schema_path = Some(schema_path);
        self.data_path = Some(data_path);
        self
    }

    /// Builder-style: set RFR override
    pub fn with_rfr(mut self, rfr: ReadFromReplica) -> Self {
        self.rfr_override = Some(rfr);
        self
    }

    /// Check if this workload is a write operation
    pub fn is_write(&self) -> bool {
        self.workload_type.is_write()
    }

    /// Check if this workload requires a dataset
    pub fn requires_dataset(&self) -> bool {
        self.workload_type.requires_dataset()
    }

    /// Check if this is a vector workload
    pub fn is_vector_workload(&self) -> bool {
        self.workload_type.requires_dataset()
    }

    /// Get the effective key prefix (from search_config for vector workloads)
    pub fn effective_key_prefix(&self) -> &str {
        if self.is_vector_workload() {
            if let Some(ref sc) = self.search_config {
                return &sc.prefix;
            }
        }
        &self.key_prefix
    }

    /// Get the effective RFR setting given the global default
    pub fn effective_rfr(&self, global: ReadFromReplica) -> ReadFromReplica {
        self.rfr_override.unwrap_or(global)
    }

    /// Create a command template from this workload config
    ///
    /// Uses this config's settings (workload_type, key_prefix, data_size, search_config)
    /// to create an appropriate command template.
    pub fn build_template(&self) -> CommandTemplate {
        create_template(
            self.workload_type,
            self.effective_key_prefix(),
            self.data_size,
            self.search_config.as_ref(),
            None,
        )
    }

    /// Create a command template with address specification
    ///
    /// This variant allows specifying an AddressSpec to enable hash field
    /// or JSON path iteration with custom key ranges.
    pub fn build_template_with_address(
        &self,
        address_spec: Option<&AddressSpec>,
    ) -> CommandTemplate {
        create_template(
            self.workload_type,
            self.effective_key_prefix(),
            self.data_size,
            self.search_config.as_ref(),
            address_spec,
        )
    }
}

impl Default for WorkloadConfig {
    fn default() -> Self {
        Self::new(WorkloadType::Ping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_config() {
        let config = WorkloadConfig::new(WorkloadType::Get);
        assert_eq!(config.workload_type, WorkloadType::Get);
        assert_eq!(config.key_prefix, "key:");
        assert_eq!(config.keyspace, 1_000_000);
        assert_eq!(config.data_size, 3);
        assert!(config.search_config.is_none());
    }

    #[test]
    fn test_builder_pattern() {
        let config = WorkloadConfig::new(WorkloadType::Set)
            .with_key_prefix("user:")
            .with_keyspace(50_000)
            .with_data_size(100);

        assert_eq!(config.key_prefix, "user:");
        assert_eq!(config.keyspace, 50_000);
        assert_eq!(config.data_size, 100);
    }

    #[test]
    fn test_is_write() {
        assert!(WorkloadConfig::new(WorkloadType::Set).is_write());
        assert!(!WorkloadConfig::new(WorkloadType::Get).is_write());
    }

    #[test]
    fn test_effective_rfr() {
        let config = WorkloadConfig::new(WorkloadType::Get);
        assert_eq!(
            config.effective_rfr(ReadFromReplica::Primary),
            ReadFromReplica::Primary
        );

        let config_override = config.with_rfr(ReadFromReplica::PreferReplica);
        assert_eq!(
            config_override.effective_rfr(ReadFromReplica::Primary),
            ReadFromReplica::PreferReplica
        );
    }

    #[test]
    fn test_build_template() {
        // Test that build_template creates template with config's settings
        let config = WorkloadConfig::new(WorkloadType::Get).with_key_prefix("user:");

        let template = config.build_template();
        assert_eq!(template.name(), "GET");
    }

    #[test]
    fn test_build_template_with_data_size() {
        // Different data sizes should produce different templates
        let config_small = WorkloadConfig::new(WorkloadType::Set).with_data_size(10);
        let config_large = WorkloadConfig::new(WorkloadType::Set).with_data_size(1000);

        let template_small = config_small.build_template();
        let template_large = config_large.build_template();

        // Both should be SET commands
        assert_eq!(template_small.name(), "SET");
        assert_eq!(template_large.name(), "SET");
    }
}
