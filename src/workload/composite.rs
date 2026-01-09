//! Composite workload for sequential phases
//!
//! This module provides support for running multiple workload phases sequentially,
//! with the ability to pass IDs between phases. For example:
//! - Phase 1: Load 10000 vectors (produces IDs)
//! - Phase 2: Query those vectors (consumes IDs)

use super::{PrepareResult, Workload, WorkloadType};
use crate::config::{SearchConfig, WorkloadConfig};
use crate::utils::{BenchmarkError, Result};

/// A phase within a composite workload
#[derive(Debug, Clone)]
pub struct WorkloadPhase {
    /// Configuration for this workload phase
    pub config: WorkloadConfig,
    /// Number of requests for this phase (None means use default)
    pub requests: Option<u64>,
    /// Whether this phase produces IDs for subsequent phases
    pub produce_ids: bool,
    /// Whether this phase consumes IDs from previous phases
    pub consume_ids: bool,
    /// Phase name for display
    pub name: String,
}

impl WorkloadPhase {
    /// Create a new workload phase with full config
    pub fn new(config: WorkloadConfig, requests: Option<u64>) -> Self {
        let name = config.workload_type.as_str().to_string();
        Self {
            config,
            requests,
            produce_ids: false,
            consume_ids: false,
            name,
        }
    }

    /// Create a new workload phase from just a workload type (uses defaults)
    pub fn from_type(workload_type: WorkloadType, requests: Option<u64>) -> Self {
        Self {
            config: WorkloadConfig::new(workload_type),
            requests,
            produce_ids: false,
            consume_ids: false,
            name: workload_type.as_str().to_string(),
        }
    }

    /// Get the workload type (convenience accessor)
    pub fn workload_type(&self) -> WorkloadType {
        self.config.workload_type
    }

    /// Mark this phase as producing IDs
    pub fn produces_ids(mut self) -> Self {
        self.produce_ids = true;
        self
    }

    /// Mark this phase as consuming IDs
    pub fn consumes_ids(mut self) -> Self {
        self.consume_ids = true;
        self
    }

    /// Set a custom name for this phase
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }
}

/// Composite workload that runs multiple phases sequentially
#[derive(Debug, Clone)]
pub struct CompositeWorkload {
    /// Ordered list of phases to execute
    phases: Vec<WorkloadPhase>,
    /// Combined name for display
    name: String,
}

impl CompositeWorkload {
    /// Create a new composite workload from phases
    pub fn new(phases: Vec<WorkloadPhase>) -> Result<Self> {
        if phases.is_empty() {
            return Err(BenchmarkError::Config(
                "CompositeWorkload requires at least one phase".to_string(),
            ));
        }

        // Build display name
        let name = phases
            .iter()
            .map(|p| {
                if let Some(n) = p.requests {
                    format!("{}:{}", p.name, n)
                } else {
                    p.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("->");

        Ok(Self { phases, name })
    }

    /// Parse composite workload specification from CLI string
    ///
    /// Format: "workload1:count1,workload2:count2,..."
    /// Example: "vec-load:10000,vec-query:1000" for load then query
    pub fn parse(spec: &str) -> Result<Self> {
        let mut phases = Vec::new();

        for part in spec.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let mut split = part.splitn(2, ':');
            let workload_str = split
                .next()
                .ok_or_else(|| BenchmarkError::Config("Missing workload type".to_string()))?;

            let workload_type = WorkloadType::parse(workload_str).ok_or_else(|| {
                BenchmarkError::Config(format!("Unknown workload type: {}", workload_str))
            })?;

            let requests = if let Some(count_str) = split.next() {
                Some(count_str.parse().map_err(|_| {
                    BenchmarkError::Config(format!("Invalid request count: {}", count_str))
                })?)
            } else {
                None
            };

            // Auto-detect produce/consume based on workload type
            let mut phase = WorkloadPhase::from_type(workload_type, requests);

            // VecLoad produces IDs, VecQuery/VecDel consume IDs
            if workload_type == WorkloadType::VecLoad {
                phase = phase.produces_ids();
            } else if matches!(workload_type, WorkloadType::VecQuery | WorkloadType::VecDel) {
                phase = phase.consumes_ids();
            }

            phases.push(phase);
        }

        Self::new(phases)
    }

    /// Get all phases
    pub fn phases(&self) -> &[WorkloadPhase] {
        &self.phases
    }

    /// Get number of phases
    pub fn len(&self) -> usize {
        self.phases.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.phases.is_empty()
    }

    /// Check if any phase is a write operation
    pub fn has_writes(&self) -> bool {
        self.phases.iter().any(|p| p.config.is_write())
    }

    /// Check if any phase requires a dataset
    pub fn requires_dataset(&self) -> bool {
        self.phases.iter().any(|p| p.config.requires_dataset())
    }

    /// Get the first phase
    pub fn first_phase(&self) -> Option<&WorkloadPhase> {
        self.phases.first()
    }

    /// Get phase at index
    pub fn phase_at(&self, idx: usize) -> Option<&WorkloadPhase> {
        self.phases.get(idx)
    }

    /// Apply global defaults to all phase configs
    ///
    /// This updates phases with values from BenchmarkConfig for settings
    /// that weren't explicitly specified per-phase. This is typically called
    /// after parsing to apply CLI defaults.
    pub fn apply_defaults(
        &mut self,
        key_prefix: &str,
        keyspace: u64,
        data_size: usize,
        search_config: Option<&SearchConfig>,
        schema_path: Option<&std::path::PathBuf>,
        data_path: Option<&std::path::PathBuf>,
    ) {
        for phase in &mut self.phases {
            // Apply defaults to each phase's config
            phase.config.key_prefix = key_prefix.to_string();
            phase.config.keyspace = keyspace;
            phase.config.data_size = data_size;
            if let Some(sc) = search_config {
                phase.config.search_config = Some(sc.clone());
            }
            if let Some(sp) = schema_path {
                phase.config.schema_path = Some(sp.clone());
            }
            if let Some(dp) = data_path {
                phase.config.data_path = Some(dp.clone());
            }
        }
    }
}

impl Workload for CompositeWorkload {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_write(&self) -> bool {
        self.has_writes()
    }

    fn requires_dataset(&self) -> bool {
        CompositeWorkload::requires_dataset(self)
    }

    fn prepare(&self) -> Result<PrepareResult> {
        // Preparation happens in orchestrator per phase
        Ok(PrepareResult::empty())
    }
}

/// Builder for composite workloads
#[derive(Default)]
pub struct CompositeWorkloadBuilder {
    phases: Vec<WorkloadPhase>,
}

impl CompositeWorkloadBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a phase with the given workload type (uses default config)
    pub fn phase(mut self, workload_type: WorkloadType) -> Self {
        self.phases.push(WorkloadPhase::from_type(workload_type, None));
        self
    }

    /// Add a phase with request count (uses default config)
    pub fn phase_with_count(mut self, workload_type: WorkloadType, requests: u64) -> Self {
        self.phases.push(WorkloadPhase::from_type(workload_type, Some(requests)));
        self
    }

    /// Add a phase with full configuration
    pub fn phase_with_config(mut self, config: WorkloadConfig, requests: Option<u64>) -> Self {
        self.phases.push(WorkloadPhase::new(config, requests));
        self
    }

    /// Add a fully configured phase
    pub fn add_phase(mut self, phase: WorkloadPhase) -> Self {
        self.phases.push(phase);
        self
    }

    /// Mark the last phase as producing IDs
    pub fn produces_ids(mut self) -> Self {
        if let Some(last) = self.phases.last_mut() {
            last.produce_ids = true;
        }
        self
    }

    /// Mark the last phase as consuming IDs
    pub fn consumes_ids(mut self) -> Self {
        if let Some(last) = self.phases.last_mut() {
            last.consume_ids = true;
        }
        self
    }

    /// Build the composite workload
    pub fn build(self) -> Result<CompositeWorkload> {
        CompositeWorkload::new(self.phases)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let cw = CompositeWorkload::parse("vec-load:10000,vec-query:1000").unwrap();
        assert_eq!(cw.len(), 2);
        assert_eq!(cw.phases[0].workload_type(), WorkloadType::VecLoad);
        assert_eq!(cw.phases[0].requests, Some(10000));
        assert!(cw.phases[0].produce_ids);
        assert_eq!(cw.phases[1].workload_type(), WorkloadType::VecQuery);
        assert_eq!(cw.phases[1].requests, Some(1000));
        assert!(cw.phases[1].consume_ids);
    }

    #[test]
    fn test_parse_no_count() {
        let cw = CompositeWorkload::parse("set,get").unwrap();
        assert_eq!(cw.len(), 2);
        assert_eq!(cw.phases[0].workload_type(), WorkloadType::Set);
        assert!(cw.phases[0].requests.is_none());
        assert_eq!(cw.phases[1].workload_type(), WorkloadType::Get);
        assert!(cw.phases[1].requests.is_none());
    }

    #[test]
    fn test_builder() {
        let cw = CompositeWorkloadBuilder::new()
            .phase_with_count(WorkloadType::VecLoad, 60000)
            .produces_ids()
            .phase_with_count(WorkloadType::VecQuery, 10000)
            .consumes_ids()
            .build()
            .unwrap();

        assert_eq!(cw.len(), 2);
        assert!(cw.phases[0].produce_ids);
        assert!(cw.phases[1].consume_ids);
    }

    #[test]
    fn test_name_display() {
        let cw = CompositeWorkload::parse("vec-load:10000,vec-query:1000").unwrap();
        assert_eq!(cw.name(), "VEC-LOAD:10000->VEC-QUERY:1000");
    }

    #[test]
    fn test_has_writes() {
        let cw_write = CompositeWorkload::parse("set:100,get:100").unwrap();
        assert!(cw_write.has_writes());

        let cw_read = CompositeWorkload::parse("get:100,ping:100").unwrap();
        assert!(!cw_read.has_writes());
    }

    #[test]
    fn test_requires_dataset() {
        let cw_vec = CompositeWorkload::parse("vec-load:1000,vec-query:100").unwrap();
        assert!(cw_vec.requires_dataset());

        let cw_basic = CompositeWorkload::parse("set:100,get:100").unwrap();
        assert!(!cw_basic.requires_dataset());
    }

    #[test]
    fn test_empty_fails() {
        let result = CompositeWorkload::parse("");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_workload() {
        let result = CompositeWorkload::parse("unknown:100");
        assert!(result.is_err());
    }

    #[test]
    fn test_workload_trait() {
        let cw = CompositeWorkload::parse("vec-load:10000,vec-query:1000").unwrap();
        assert_eq!(cw.name(), "VEC-LOAD:10000->VEC-QUERY:1000");
        assert!(cw.is_write()); // VecLoad is a write
        assert!(cw.requires_dataset());
    }

    #[test]
    fn test_phase_accessors() {
        let cw = CompositeWorkload::parse("set:100,get:200,ping:300").unwrap();
        assert_eq!(cw.first_phase().unwrap().workload_type(), WorkloadType::Set);
        assert_eq!(cw.phase_at(1).unwrap().workload_type(), WorkloadType::Get);
        assert_eq!(cw.phase_at(2).unwrap().workload_type(), WorkloadType::Ping);
        assert!(cw.phase_at(3).is_none());
    }
}
