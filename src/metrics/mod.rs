//! Metrics collection and reporting
//!
//! This module provides:
//! - Per-node metrics tracking (ops, latency, errors)
//! - Aggregated metrics across all nodes
//! - Temporal diff calculation for INFO statistics
//! - FT.INFO parsing for EC and MemoryDB engines
//! - Index backfill progress monitoring
//! - Cluster snapshot comparison
//! - JSON/CSV export

pub mod backfill;
pub mod collector;
pub mod ft_info;
pub mod info_fields;
pub mod node_metrics;
pub mod reporter;
pub mod snapshot;

pub use backfill::{
    get_node_progress, get_node_progress_ec, get_node_progress_memorydb,
    get_node_progress_with_context, wait_for_index_backfill_complete, BackfillWaitConfig,
    ClusterBackfillProgress, MemoryDbProgressContext, NodeProgress,
};
pub use collector::MetricsCollector;
pub use ft_info::{
    convert_ftinfo_to_lines, fields, get_field, get_field_or, parse_ftinfo_lines,
    validate_memdb_ftinfo, validate_search_info, EngineType, FtInfoResult, IndexStatus,
    ResponseFormat, ValidationError,
};
pub use info_fields::{
    default_ftinfo_fields, default_info_fields, default_search_info_fields, AggregationType,
    DiffType, DisplayFormat, InfoFieldType, NodeFilter, ParseConfig, ParseStrategy,
};
pub use node_metrics::NodeMetrics;
pub use reporter::MetricsReporter;
pub use snapshot::{
    compare_snapshots, print_per_node_diff, print_per_node_diff_all, print_per_node_matrix,
    print_snapshot_diff, ClusterSnapshot, FieldDiff, SnapshotBuilder, SnapshotDiff,
};
