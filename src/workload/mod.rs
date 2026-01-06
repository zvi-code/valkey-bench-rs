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
    parse_address_type, Address, AddressIterator, AddressType, AddressableSpace,
    HashFieldSpace, JsonPathSpace, KeySpace,
};
pub use command_template::{CommandTemplate, TemplateArg};
pub use context::{
    create_workload_context, create_workload_context_with_iteration,
    create_workload_context_with_shared_tracker, AddressableContext,
    SimpleContext, VectorDeleteContext, VectorLoadContext, VectorQueryContext,
    VectorUpdateContext, WorkloadContext, WorkloadMetrics,
};
pub use iteration::{ExistenceFilter, IterationState, IterationStrategy, KeyspaceIterator};
pub use lifecycle::{LegacyWorkloadAdapter, PrepareResult, Workload};
pub use parallel::{ParallelComponent, ParallelWorkload, ParallelWorkloadBuilder};
pub use composite::{CompositeWorkload, CompositeWorkloadBuilder, WorkloadPhase};
pub use key_format::{
    extract_numeric_ids_from_keys, KeyFormat, CLUSTER_TAG_INNER_LEN, CLUSTER_TAG_LEN,
    DEFAULT_KEY_WIDTH, TAG_KEY_SEPARATOR,
};
pub use search_ops::{
    create_index, drop_index, extract_numeric_ids, get_index_info, index_exists,
    parse_search_response, wait_for_indexing, IndexInfo,
};
pub use numeric_field::{NumericDistribution, NumericFieldConfig, NumericFieldSet, NumericValueType};
pub use tag_distribution::{TagDistribution, TagDistributionSet};
pub use template_factory::{create_template, create_template_with_address, AddressConfig};
pub use workload_type::WorkloadType;
