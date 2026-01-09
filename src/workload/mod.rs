//! Workload definitions and command templates

pub mod addressable;
pub mod command_template;
pub mod composite;
pub mod context;
pub mod iteration;
pub mod key_format;
pub mod lifecycle;
pub mod numeric_field;
pub mod parallel;
pub mod search_ops;
pub mod tag_distribution;
pub mod template_factory;
pub mod workload_type;

pub use addressable::{
    extract_numeric_ids_from_keys, parse_address_type, Address, AddressIterator, AddressSpec,
    AddressType, AddressableSpace, PlaceholderSpec, SubKeySpec, DEFAULT_KEY_WIDTH,
};
pub use command_template::{CommandTemplate, TemplateArg};
pub use context::{
    create_workload_context, create_workload_context_with_iteration,
    create_workload_context_with_shared_tracker, AddressableContext,
    AdjustedRecallAggregator, DeleteContext, SimpleContext,
    VectorLoadContext, VectorQueryContext, VectorQueryWithDeletesContext, VectorUpdateContext,
    WorkloadContext, WorkloadMetrics,
};
pub use iteration::{ExistenceFilter, IterationState, IterationStrategy, KeyspaceIterator};
pub use lifecycle::{LegacyWorkloadAdapter, PrepareResult, Workload};
pub use parallel::{ParallelComponent, ParallelWorkload, ParallelWorkloadBuilder};
pub use composite::{CompositeWorkload, CompositeWorkloadBuilder, WorkloadPhase};
pub use search_ops::{
    create_index, drop_index, extract_numeric_ids, get_index_info, index_exists,
    parse_search_response, wait_for_indexing, IndexInfo,
};
pub use numeric_field::{NumericDistribution, NumericFieldConfig, NumericFieldSet, NumericValueType};
pub use tag_distribution::{TagDistribution, TagDistributionSet};
pub use template_factory::create_template;
pub use workload_type::WorkloadType;
