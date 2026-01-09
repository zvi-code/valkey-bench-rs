//! Workload lifecycle management
//!
//! This module provides traits and types for managing workload lifecycle:
//! - Pre-configuration (index creation, setup)
//! - Preparation (data loading, warmup)
//! - Execution (benchmark run)
//! - Post-processing (result analysis)
//! - Post-configuration (cleanup)

use std::time::Duration;

use crate::client::ControlPlane;
use crate::metrics::reporter::TestResult;
use crate::utils::Result;

/// Result of the prepare phase
#[derive(Debug, Default, Clone)]
pub struct PrepareResult {
    /// Number of items prepared (e.g., vectors loaded)
    pub items_prepared: u64,
    /// Duration of the prepare phase
    pub duration: Duration,
    /// IDs produced that can be consumed by subsequent workloads
    pub consumable_ids: Vec<u64>,
}

impl PrepareResult {
    /// Create a new prepare result
    pub fn new(items_prepared: u64, duration: Duration) -> Self {
        Self {
            items_prepared,
            duration,
            consumable_ids: Vec::new(),
        }
    }

    /// Create a prepare result with consumable IDs
    pub fn with_consumable_ids(items_prepared: u64, duration: Duration, ids: Vec<u64>) -> Self {
        Self {
            items_prepared,
            duration,
            consumable_ids: ids,
        }
    }

    /// Create an empty result (no preparation needed)
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Workload lifecycle trait
///
/// Defines the lifecycle hooks for a benchmark workload:
/// 1. `preconfigure` - Setup before benchmark (e.g., create index)
/// 2. `prepare` - Prepare data (e.g., load vectors)
/// 3. `run` - Execute the benchmark
/// 4. `postprocess` - Analyze results
/// 5. `postconfigure` - Cleanup (e.g., drop index)
pub trait Workload: Send + Sync {
    /// Get the workload name for display
    fn name(&self) -> &str;

    /// Check if this workload modifies data (for read-from-replica routing)
    fn is_write(&self) -> bool {
        true
    }

    /// Check if this workload requires a dataset
    fn requires_dataset(&self) -> bool {
        false
    }

    /// Pre-configuration hook (runs before prepare)
    ///
    /// Use for setup tasks like creating indexes.
    /// Default implementation does nothing.
    fn preconfigure<C: ControlPlane>(&self, _conn: &mut C) -> Result<()> {
        Ok(())
    }

    /// Prepare hook (runs before run)
    ///
    /// Use for data preparation like loading vectors.
    /// Returns information about what was prepared.
    /// Default implementation returns empty result.
    fn prepare(&self) -> Result<PrepareResult> {
        Ok(PrepareResult::empty())
    }

    /// Post-process hook (runs after run)
    ///
    /// Use for result analysis like computing recall.
    /// Default implementation does nothing.
    fn postprocess(&self, _result: &TestResult) -> Result<()> {
        Ok(())
    }

    /// Post-configuration hook (runs after postprocess)
    ///
    /// Use for cleanup tasks like dropping indexes.
    /// Default implementation does nothing.
    fn postconfigure<C: ControlPlane>(&self, _conn: &mut C) -> Result<()> {
        Ok(())
    }
}

/// Adapter to wrap legacy WorkloadType in the Workload trait
///
/// This provides backward compatibility with existing code that uses WorkloadType.
pub struct LegacyWorkloadAdapter {
    name: String,
    is_write: bool,
    requires_dataset: bool,
}

impl LegacyWorkloadAdapter {
    /// Create a new adapter from a workload type
    pub fn new(name: &str, is_write: bool, requires_dataset: bool) -> Self {
        Self {
            name: name.to_string(),
            is_write,
            requires_dataset,
        }
    }

    /// Create from a WorkloadType
    pub fn from_workload_type(workload: super::WorkloadType) -> Self {
        Self {
            name: workload.as_str().to_string(),
            is_write: workload.is_write(),
            requires_dataset: workload.requires_dataset(),
        }
    }
}

impl Workload for LegacyWorkloadAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_write(&self) -> bool {
        self.is_write
    }

    fn requires_dataset(&self) -> bool {
        self.requires_dataset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::WorkloadType;

    #[test]
    fn test_prepare_result_new() {
        let result = PrepareResult::new(100, Duration::from_secs(5));
        assert_eq!(result.items_prepared, 100);
        assert_eq!(result.duration, Duration::from_secs(5));
        assert!(result.consumable_ids.is_empty());
    }

    #[test]
    fn test_prepare_result_with_ids() {
        let ids = vec![1, 2, 3];
        let result = PrepareResult::with_consumable_ids(3, Duration::from_millis(100), ids);
        assert_eq!(result.items_prepared, 3);
        assert_eq!(result.consumable_ids.len(), 3);
    }

    #[test]
    fn test_prepare_result_empty() {
        let result = PrepareResult::empty();
        assert_eq!(result.items_prepared, 0);
        assert_eq!(result.duration, Duration::ZERO);
        assert!(result.consumable_ids.is_empty());
    }

    #[test]
    fn test_legacy_adapter() {
        let adapter = LegacyWorkloadAdapter::new("TEST", true, false);
        assert_eq!(adapter.name(), "TEST");
        assert!(adapter.is_write());
        assert!(!adapter.requires_dataset());
    }

    #[test]
    fn test_legacy_adapter_from_workload_type() {
        let adapter = LegacyWorkloadAdapter::from_workload_type(WorkloadType::VecQuery);
        assert_eq!(adapter.name(), "VEC-QUERY");
        assert!(!adapter.is_write());
        assert!(adapter.requires_dataset());
    }

    #[test]
    fn test_legacy_adapter_set() {
        let adapter = LegacyWorkloadAdapter::from_workload_type(WorkloadType::Set);
        assert_eq!(adapter.name(), "SET");
        assert!(adapter.is_write());
        assert!(!adapter.requires_dataset());
    }
}
