//! Workload context trait and implementations
//!
//! This module provides the WorkloadContext trait that abstracts workload-specific
//! behavior from the generic EventWorker infrastructure. This decoupling enables:
//! - Easy addition of new workload types
//! - Workload-specific metrics (e.g., recall for vector search)
//! - Workload-specific ID claiming logic (e.g., skipping existing vectors)
//! - Workload-specific placeholder filling (e.g., tag generation)
//!
//! ## Tracker-Driven Iteration
//!
//! All contexts use `keyspace_tracker::PrefixTracker` for atomic iteration:
//! - `claim_next_id()` no longer requires `GlobalCounters` parameter
//! - Each context owns its iteration state via `PrefixTracker`
//! - `IterationStrategy` maps to `TrackerIterBuilder` configurations
//! - Atomic claim semantics ensure no duplicate IDs across threads

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use keyspace_tracker::{AccessDistribution, PrefixTracker, TrackerConfig};

use crate::benchmark::RecallStats;
use crate::client::PlaceholderType;
use crate::dataset::DatasetContext;
use crate::keyspace::{ProtectedIds, KeyGroupExistanceTracker};
use crate::utils::RespValue;
use crate::workload::{
    extract_numeric_ids, parse_search_response, Address, AddressType, AddressableSpace,
    IterationStrategy, NumericFieldSet, TagDistributionSet, WorkloadType,
};

/// Metrics collected by workload context
#[derive(Debug, Default)]
pub enum WorkloadMetrics {
    #[default]
    None,
    Recall(RecallStats),
}

/// Trait for workload-specific context and behavior
///
/// Implementations encapsulate workload-specific logic like:
/// - ID claiming (with tracker-based atomic iteration)
/// - Response processing (recall computation)
/// - Placeholder filling for workload-specific data (e.g., tag generation)
///
/// ## Tracker-Driven Iteration
///
/// All contexts use `PrefixTracker` internally for atomic ID claiming.
/// The `claim_next_id()` method no longer requires external counters -
/// each context manages its own iteration state.
pub trait WorkloadContext: Send {
    /// Claim the next key/item ID for this workload
    ///
    /// Uses internal PrefixTracker for atomic iteration.
    /// Returns None when no more IDs are available (e.g., all vectors loaded,
    /// all deleteable vectors claimed).
    fn claim_next_id(&self) -> Option<u64>;

    /// Get the dataset index for vector operations
    /// Returns None for non-vector workloads
    fn next_dataset_idx(&self) -> Option<u64>;

    /// Get query vector bytes for FT.SEARCH operations
    /// Returns None for non-query workloads
    fn get_query_bytes(&self, idx: u64) -> Option<&[u8]>;

    /// Get vector bytes for HSET operations
    /// Returns None for non-vector workloads
    fn get_vector_bytes(&self, idx: u64) -> Option<&[u8]>;

    /// Fill a Tag placeholder with generated tag values
    ///
    /// Uses key_num as seed for reproducible tag generation.
    /// Buffer is filled with tag values or padding (commas) if no tags configured.
    fn fill_tag_placeholder(&self, _key_num: u64, buf: &mut [u8]) {
        // Default: fill with commas (no-op for workloads without tag support)
        buf.fill(b',');
    }

    /// Fill a NumericField placeholder with generated values
    ///
    /// Uses key_num as seed for reproducible value generation.
    /// The field_idx identifies which NumericFieldConfig to use.
    fn fill_numeric_field(&self, field_idx: usize, key_num: u64, seq_counter: u64, buf: &mut [u8]) {
        // Default: fill with zeros (no-op for workloads without numeric field support)
        let _ = (field_idx, key_num, seq_counter);
        buf.fill(b'0');
    }

    /// Compute and record recall for a query response
    /// Called after receiving a response with the corresponding query index
    fn compute_and_record_recall(&mut self, query_idx: u64, response: &RespValue);

    /// Take accumulated metrics (consumes the internal state)
    fn take_metrics(&mut self) -> WorkloadMetrics;

    /// Get number of items in dataset (for modulo operations)
    fn num_items(&self) -> u64;

    /// Get number of queries in dataset
    fn num_queries(&self) -> u64;

    /// Check if this workload uses dataset-based keys
    /// When true, the Key placeholder is filled by the Vector handler
    fn uses_dataset_keys(&self) -> bool;

    /// Check if this workload's key is the claimed ID (for delete operations)
    fn key_is_claimed_id(&self) -> bool;

    /// Get the address type for this context
    fn address_type(&self) -> AddressType {
        AddressType::Key
    }

    /// Get the current address (key + optional field/path)
    /// Default returns None, meaning only simple keys are used.
    fn current_address(&self) -> Option<&Address> {
        None
    }

    /// Fill a Field placeholder with the current field name
    ///
    /// Uses the address from AddressableSpace to get field name.
    /// Buffer is filled with field name or padding (spaces) if no field.
    fn fill_field_placeholder(&self, buf: &mut [u8]) {
        // Default: fill with spaces (no-op for workloads without field support)
        buf.fill(b' ');
    }

    /// Fill a JsonPath placeholder with the current JSON path
    ///
    /// Uses the address from AddressableSpace to get JSON path.
    /// Buffer is filled with path or padding (spaces) if no path.
    fn fill_json_path_placeholder(&self, buf: &mut [u8]) {
        // Default: fill with spaces (no-op for workloads without path support)
        buf.fill(b' ');
    }
}

// =============================================================================
// SimpleContext - For key-value workloads (SET, GET, INCR, etc.)
// =============================================================================

/// Context for simple key-value workloads without dataset or recall
///
/// Uses `PrefixTracker` internally for atomic iteration with support for:
/// - Sequential iteration (partitioned for reads, atomic cursor for writes)
/// - Random iteration (with seed for reproducibility)
/// - Zipfian distribution (hot keys)
/// - Subset ranges
///
/// ## Partitioning for Multi-Threaded Reads
///
/// When `partition_info` is set (via `with_partition`), each worker gets a
/// disjoint range of the keyspace for better cache locality and no contention.
/// This is ideal for read workloads (GET, queries).
///
/// ## Atomic Cursor for Writes
///
/// When `partition_info` is None (default), uses atomic cursor via `continue_write()`
/// to ensure exactly-once ID claiming across threads. This is ideal for write
/// workloads (SET, load operations).
pub struct SimpleContext {
    /// Tracker for atomic iteration
    tracker: Arc<PrefixTracker>,
    /// Keyspace length (for modulo operations)
    keyspace_len: u64,
    /// Iteration strategy (for distribution configuration)
    strategy: IterationStrategy,
    /// Partition info: (worker_index, total_workers)
    /// When set, uses partitioned iteration instead of atomic cursor
    partition_info: Option<(usize, usize)>,
    /// Request limit for this worker (enables "1M requests on 100K keyspace")
    /// When set, limits how many IDs this worker can claim
    request_limit: Option<u64>,
    /// Count of IDs claimed so far (for limit enforcement)
    claimed_count: std::sync::atomic::AtomicU64,
}

impl SimpleContext {
    /// Create a new SimpleContext with sequential or random iteration (backward compatible)
    pub fn new(keyspace_len: u64, sequential: bool, seed: u64) -> Self {
        let strategy = if sequential {
            IterationStrategy::Sequential
        } else {
            IterationStrategy::Random { seed }
        };
        Self::with_strategy(keyspace_len, strategy)
    }

    /// Create a new SimpleContext with a custom iteration strategy
    pub fn with_strategy(keyspace_len: u64, strategy: IterationStrategy) -> Self {
        // Create tracker with max_id = keyspace_len
        let config = TrackerConfig::simple("key:")
            .with_max_id(keyspace_len)
            .with_initial_capacity(keyspace_len as usize);
        let tracker = Arc::new(PrefixTracker::new(config));
        
        Self {
            tracker,
            keyspace_len,
            strategy,
            partition_info: None,
            request_limit: None,
            claimed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Create a new SimpleContext with strategy and request limit
    pub fn with_strategy_and_limit(keyspace_len: u64, strategy: IterationStrategy, request_limit: u64) -> Self {
        let config = TrackerConfig::simple("key:")
            .with_max_id(keyspace_len)
            .with_initial_capacity(keyspace_len as usize);
        let tracker = Arc::new(PrefixTracker::new(config));
        
        Self {
            tracker,
            keyspace_len,
            strategy,
            partition_info: None,
            request_limit: Some(request_limit),
            claimed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Create a SimpleContext that shares a tracker with other workers.
    ///
    /// This is required for multi-threaded write workloads where all workers
    /// must share the same atomic cursor to ensure exactly-once ID claiming.
    ///
    /// # Arguments
    /// * `shared_tracker` - Shared tracker (created with `create_shared_tracker()`)
    /// * `keyspace_len` - Total size of the keyspace
    /// * `strategy` - Iteration strategy (Sequential, Random, etc.)
    pub fn with_shared_tracker(
        shared_tracker: Arc<PrefixTracker>,
        keyspace_len: u64,
        strategy: IterationStrategy,
    ) -> Self {
        Self {
            tracker: shared_tracker,
            keyspace_len,
            strategy,
            partition_info: None,
            request_limit: None,
            claimed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Create a SimpleContext that shares a tracker with other workers (with limit).
    pub fn with_shared_tracker_and_limit(
        shared_tracker: Arc<PrefixTracker>,
        keyspace_len: u64,
        strategy: IterationStrategy,
        request_limit: u64,
    ) -> Self {
        Self {
            tracker: shared_tracker,
            keyspace_len,
            strategy,
            partition_info: None,
            request_limit: Some(request_limit),
            claimed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Create a shared tracker that can be passed to multiple workers.
    ///
    /// Use this when spawning worker threads for write workloads.
    pub fn create_shared_tracker(keyspace_len: u64) -> Arc<PrefixTracker> {
        let config = TrackerConfig::simple("key:")
            .with_max_id(keyspace_len)
            .with_initial_capacity(keyspace_len as usize);
        Arc::new(PrefixTracker::new(config))
    }

    /// Create a partitioned context for a specific worker.
    ///
    /// This is the preferred method for multi-threaded read workloads.
    /// Each worker gets a disjoint range of the keyspace.
    ///
    /// # Arguments
    /// * `keyspace_len` - Total size of the keyspace
    /// * `strategy` - Iteration strategy (Sequential, Random, etc.)
    /// * `worker_index` - This worker's index (0-based)
    /// * `total_workers` - Total number of workers
    pub fn with_partition(
        keyspace_len: u64,
        strategy: IterationStrategy,
        worker_index: usize,
        total_workers: usize,
    ) -> Self {
        let config = TrackerConfig::simple("key:")
            .with_max_id(keyspace_len)
            .with_initial_capacity(keyspace_len as usize);
        let tracker = Arc::new(PrefixTracker::new(config));

        Self {
            tracker,
            keyspace_len,
            strategy,
            partition_info: Some((worker_index, total_workers)),
            request_limit: None,
            claimed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Create a partitioned context with request limit
    pub fn with_partition_and_limit(
        keyspace_len: u64,
        strategy: IterationStrategy,
        worker_index: usize,
        total_workers: usize,
        request_limit: u64,
    ) -> Self {
        let config = TrackerConfig::simple("key:")
            .with_max_id(keyspace_len)
            .with_initial_capacity(keyspace_len as usize);
        let tracker = Arc::new(PrefixTracker::new(config));

        Self {
            tracker,
            keyspace_len,
            strategy,
            partition_info: Some((worker_index, total_workers)),
            request_limit: Some(request_limit),
            claimed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get the iteration strategy
    pub fn strategy(&self) -> &IterationStrategy {
        &self.strategy
    }

    /// Get request limit
    pub fn request_limit(&self) -> Option<u64> {
        self.request_limit
    }

    /// Get the underlying tracker (for testing)
    pub fn tracker(&self) -> &Arc<PrefixTracker> {
        &self.tracker
    }

    /// Helper to get the next ID using the appropriate iterator
    ///
    /// When partitioned: Uses `.partition(idx, total)` for disjoint ranges (ideal for reads)
    /// When not partitioned: Uses `continue_write()` for atomic cursor (ideal for writes)
    fn get_next_from_tracker(&self) -> Option<(u64, Option<u64>)> {
        // If partitioned, use partition() for disjoint ranges
        if let Some((worker_idx, total_workers)) = self.partition_info {
            return self.get_next_partitioned(worker_idx, total_workers);
        }

        // Not partitioned - use continue_write() for atomic cursor
        match &self.strategy {
            IterationStrategy::Sequential => {
                self.tracker.iter().continue_write().next()
            }
            IterationStrategy::Random { seed } => {
                self.tracker.iter().seed(*seed).continue_write().next()
            }
            IterationStrategy::Zipfian { skew, seed } => {
                self.tracker.iter()
                    .distribution(AccessDistribution::Zipfian { skew: *skew })
                    .seed(*seed)
                    .continue_write()
                    .next()
            }
            IterationStrategy::Subset { start, end, inner } => {
                let base_iter = self.tracker.iter().id_range(*start, *end);
                match inner.as_ref() {
                    IterationStrategy::Sequential => base_iter.continue_write().next(),
                    IterationStrategy::Random { seed } => base_iter.seed(*seed).continue_write().next(),
                    IterationStrategy::Zipfian { skew, seed } => {
                        base_iter
                            .distribution(AccessDistribution::Zipfian { skew: *skew })
                            .seed(*seed)
                            .continue_write()
                            .next()
                    }
                    IterationStrategy::Subset { .. } => base_iter.continue_write().next(),
                }
            }
        }
    }

    /// Get next ID using partitioned iteration (for read workloads)
    ///
    /// Each worker gets a disjoint range via `.partition(idx, total)`.
    /// This avoids contention and improves cache locality.
    fn get_next_partitioned(&self, worker_idx: usize, total_workers: usize) -> Option<(u64, Option<u64>)> {
        match &self.strategy {
            IterationStrategy::Sequential => {
                self.tracker.iter().partition(worker_idx, total_workers).next()
            }
            IterationStrategy::Random { seed } => {
                self.tracker.iter()
                    .seed(*seed)
                    .partition(worker_idx, total_workers)
                    .next()
            }
            IterationStrategy::Zipfian { skew, seed } => {
                self.tracker.iter()
                    .distribution(AccessDistribution::Zipfian { skew: *skew })
                    .seed(*seed)
                    .partition(worker_idx, total_workers)
                    .next()
            }
            IterationStrategy::Subset { start, end, inner } => {
                let base_iter = self.tracker.iter().id_range(*start, *end);
                match inner.as_ref() {
                    IterationStrategy::Sequential => {
                        base_iter.partition(worker_idx, total_workers).next()
                    }
                    IterationStrategy::Random { seed } => {
                        base_iter.seed(*seed).partition(worker_idx, total_workers).next()
                    }
                    IterationStrategy::Zipfian { skew, seed } => {
                        base_iter
                            .distribution(AccessDistribution::Zipfian { skew: *skew })
                            .seed(*seed)
                            .partition(worker_idx, total_workers)
                            .next()
                    }
                    IterationStrategy::Subset { .. } => {
                        base_iter.partition(worker_idx, total_workers).next()
                    }
                }
            }
        }
    }
}

impl WorkloadContext for SimpleContext {
    fn claim_next_id(&self) -> Option<u64> {
        // Check if we've hit the request limit
        if let Some(limit) = self.request_limit {
            let current = self.claimed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if current >= limit {
                // Undo the increment and return None
                self.claimed_count.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                return None;
            }
        }

        // Get next item from iterator
        // If iterator returns None but we haven't hit limit, generate ID from claimed_count
        if let Some((id, _)) = self.get_next_from_tracker() {
            Some(id % self.keyspace_len)
        } else {
            // Iterator exhausted - for workloads needing more requests than keyspace,
            // fall back to generating IDs from claimed_count (modulo keyspace)
            if self.request_limit.is_some() {
                // We already incremented claimed_count above, use it
                let count = self.claimed_count.load(std::sync::atomic::Ordering::Relaxed);
                Some((count - 1) % self.keyspace_len)
            } else {
                None
            }
        }
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        None
    }

    fn get_query_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn get_vector_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn compute_and_record_recall(&mut self, _query_idx: u64, _response: &RespValue) {
        // No-op for simple workloads
    }

    fn take_metrics(&mut self) -> WorkloadMetrics {
        WorkloadMetrics::None
    }

    fn num_items(&self) -> u64 {
        self.keyspace_len
    }

    fn num_queries(&self) -> u64 {
        0
    }

    fn uses_dataset_keys(&self) -> bool {
        false
    }

    fn key_is_claimed_id(&self) -> bool {
        false
    }
}

// =============================================================================
// VectorLoadContext - For VecLoad with partial prefill support
// =============================================================================

/// Context for vector loading workloads (HSET with vectors)
///
/// Uses tracker-based iteration with prefix "vec:<dataset-name>:"
/// - With `existence_map`: Claims only unmapped (unset) IDs atomically
/// - Without `existence_map`: Sequential iteration through dataset via tracker
pub struct VectorLoadContext {
    dataset: Arc<DatasetContext>,
    existence_map: Option<Arc<KeyGroupExistanceTracker>>,
    tag_distributions: Option<TagDistributionSet>,
    numeric_fields: NumericFieldSet,
    /// Tracker for vector iteration (prefix: "vec:<dataset-name>:")
    tracker: Arc<PrefixTracker>,
    /// Iteration strategy
    strategy: IterationStrategy,
    /// Request limit (for "N requests on M vectors" scenarios)
    request_limit: Option<u64>,
}

impl VectorLoadContext {
    pub fn new(
        dataset: Arc<DatasetContext>,
        existence_map: Option<Arc<KeyGroupExistanceTracker>>,
        tag_distributions: Option<TagDistributionSet>,
        numeric_fields: NumericFieldSet,
    ) -> Self {
        Self::with_strategy(dataset, existence_map, tag_distributions, numeric_fields, IterationStrategy::Sequential, None)
    }

    pub fn with_strategy(
        dataset: Arc<DatasetContext>,
        existence_map: Option<Arc<KeyGroupExistanceTracker>>,
        tag_distributions: Option<TagDistributionSet>,
        numeric_fields: NumericFieldSet,
        strategy: IterationStrategy,
        request_limit: Option<u64>,
    ) -> Self {
        // Create tracker with prefix "vec:<dataset-name>:"
        let prefix = format!("vec:{}:", dataset.name());
        let num_vectors = dataset.num_vectors();
        let config = TrackerConfig::simple(&prefix)
            .with_max_id(num_vectors)
            .with_initial_capacity(num_vectors as usize);
        let tracker = Arc::new(PrefixTracker::new(config));
        
        Self {
            dataset,
            existence_map,
            tag_distributions,
            numeric_fields,
            tracker,
            strategy,
            request_limit,
        }
    }

    /// Get the underlying tracker
    pub fn tracker(&self) -> &Arc<PrefixTracker> {
        &self.tracker
    }
}

impl WorkloadContext for VectorLoadContext {
    fn claim_next_id(&self) -> Option<u64> {
        // For VecLoad, use existence_map to skip existing vectors if available
        if let Some(ref em) = self.existence_map {
            // Get offset and count from dataset
            let offset = self.dataset.effective_offset();
            let num_vectors = self.dataset.num_vectors();
            
            // Limit claims to min(request_limit, dataset_size) to respect -n flag
            let count = self.request_limit
                .map(|limit| limit.min(num_vectors))
                .unwrap_or(num_vectors);
            
            // Claim in range [offset, offset + count)
            em.claim_unmapped_id_in_range(offset, offset + count)
        } else {
            // Use tracker-based iteration based on strategy
            // Iterator exhaustion signals completion - no wrap-around
            let id = match &self.strategy {
                IterationStrategy::Sequential => {
                    self.tracker.iter().continue_write().next().map(|(id, _)| id)
                }
                IterationStrategy::Random { seed } => {
                    self.tracker.iter().seed(*seed).continue_write().next().map(|(id, _)| id)
                }
                IterationStrategy::Zipfian { skew, seed } => {
                    self.tracker.iter()
                        .distribution(AccessDistribution::Zipfian { skew: *skew })
                        .seed(*seed)
                        .continue_write()
                        .next()
                        .map(|(id, _)| id)
                }
                IterationStrategy::Subset { start, end, inner } => {
                    let base = self.tracker.iter().id_range(*start, *end);
                    match inner.as_ref() {
                        IterationStrategy::Sequential => base.continue_write().next().map(|(id, _)| id),
                        IterationStrategy::Random { seed } => base.seed(*seed).continue_write().next().map(|(id, _)| id),
                        _ => base.continue_write().next().map(|(id, _)| id),
                    }
                }
            };

            // Iterator is the source of truth - return None when exhausted
            id.map(|i| i % self.dataset.num_vectors())
        }
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        self.claim_next_id()
    }

    fn get_query_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn get_vector_bytes(&self, idx: u64) -> Option<&[u8]> {
        Some(self.dataset.get_vector_bytes(idx))
    }

    fn fill_tag_placeholder(&self, key_num: u64, buf: &mut [u8]) {
        if let Some(ref tag_dist) = self.tag_distributions {
            if let Some(tags) = tag_dist.select_tags_seeded(key_num) {
                let tag_bytes = tags.as_bytes();
                let copy_len = tag_bytes.len().min(buf.len());
                buf[..copy_len].copy_from_slice(&tag_bytes[..copy_len]);
                buf[copy_len..].fill(b',');
            } else {
                buf.fill(b',');
            }
        } else {
            buf.fill(b',');
        }
    }

    fn fill_numeric_field(&self, field_idx: usize, key_num: u64, seq_counter: u64, buf: &mut [u8]) {
        if let Some(field_config) = self.numeric_fields.get(field_idx) {
            field_config.fill_buffer(key_num, seq_counter, buf);
        } else {
            buf.fill(b'0');
        }
    }

    fn compute_and_record_recall(&mut self, _query_idx: u64, _response: &RespValue) {
        // No-op for load workloads
    }

    fn take_metrics(&mut self) -> WorkloadMetrics {
        WorkloadMetrics::None
    }

    fn num_items(&self) -> u64 {
        self.dataset.num_vectors()
    }

    fn num_queries(&self) -> u64 {
        0
    }

    fn uses_dataset_keys(&self) -> bool {
        true // Key is the vector ID from dataset
    }

    fn key_is_claimed_id(&self) -> bool {
        false // Key is set by Vector handler, not directly from claim_next_id
    }
}

// =============================================================================
// VectorQueryContext - For VecQuery with recall computation
// =============================================================================

/// Context for vector query workloads (FT.SEARCH) with recall tracking
///
/// Uses tracker-based iteration with prefix "query_vec:<dataset-name>:"
/// The query keyspace is separate from the vector keyspace, enabling
/// independent iteration strategies for vectors vs queries.
pub struct VectorQueryContext {
    dataset: Arc<DatasetContext>,
    recall_stats: RecallStats,
    k: usize,
    key_prefix: String,
    /// Tracker for query iteration (prefix: "query_vec:<dataset-name>:")
    query_tracker: Arc<PrefixTracker>,
    /// Iteration strategy for queries
    strategy: IterationStrategy,
}

impl VectorQueryContext {
    pub fn new(dataset: Arc<DatasetContext>, k: usize, key_prefix: String) -> Self {
        Self::with_strategy(dataset, k, key_prefix, IterationStrategy::Sequential)
    }

    pub fn with_strategy(
        dataset: Arc<DatasetContext>,
        k: usize,
        key_prefix: String,
        strategy: IterationStrategy,
    ) -> Self {
        // Create tracker with prefix "query_vec:<dataset-name>:"
        let prefix = format!("query_vec:{}:", dataset.name());
        let num_queries = dataset.num_queries();
        let config = TrackerConfig::simple(&prefix)
            .with_max_id(num_queries)
            .with_initial_capacity(num_queries as usize);
        let query_tracker = Arc::new(PrefixTracker::new(config));

        Self {
            dataset,
            recall_stats: RecallStats::new(),
            k,
            key_prefix,
            query_tracker,
            strategy,
        }
    }

    /// Get the underlying query tracker
    pub fn query_tracker(&self) -> &Arc<PrefixTracker> {
        &self.query_tracker
    }
}

impl WorkloadContext for VectorQueryContext {
    fn claim_next_id(&self) -> Option<u64> {
        let num_queries = self.dataset.num_queries();
        
        // Use tracker-based iteration for queries
        // Iterator exhaustion signals completion - no wrap-around
        let id = match &self.strategy {
            IterationStrategy::Sequential => {
                self.query_tracker.iter().continue_write().next().map(|(id, _)| id)
            }
            IterationStrategy::Random { seed } => {
                self.query_tracker.iter().seed(*seed).continue_write().next().map(|(id, _)| id)
            }
            IterationStrategy::Zipfian { skew, seed } => {
                self.query_tracker.iter()
                    .distribution(AccessDistribution::Zipfian { skew: *skew })
                    .seed(*seed)
                    .continue_write()
                    .next()
                    .map(|(id, _)| id)
            }
            IterationStrategy::Subset { start, end, inner } => {
                let base = self.query_tracker.iter().id_range(*start, *end);
                match inner.as_ref() {
                    IterationStrategy::Sequential => base.continue_write().next().map(|(id, _)| id),
                    IterationStrategy::Random { seed } => base.seed(*seed).continue_write().next().map(|(id, _)| id),
                    _ => base.continue_write().next().map(|(id, _)| id),
                }
            }
        };

        // Iterator is the source of truth - return None when exhausted
        id.map(|i| i % num_queries)
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        None // Queries don't use dataset vectors
    }

    fn get_query_bytes(&self, idx: u64) -> Option<&[u8]> {
        Some(self.dataset.get_query_bytes(idx))
    }

    fn get_vector_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn compute_and_record_recall(&mut self, query_idx: u64, response: &RespValue) {
        let doc_ids = parse_search_response(response);
        let result_ids = extract_numeric_ids(&doc_ids, &self.key_prefix);
        let recall = self.dataset.compute_recall(query_idx, &result_ids, self.k);
        self.recall_stats.record(recall);
    }

    fn take_metrics(&mut self) -> WorkloadMetrics {
        let stats = std::mem::take(&mut self.recall_stats);
        // Re-initialize for next batch
        self.recall_stats = RecallStats::new();
        WorkloadMetrics::Recall(stats)
    }

    fn num_items(&self) -> u64 {
        self.dataset.num_vectors()
    }

    fn num_queries(&self) -> u64 {
        self.dataset.num_queries()
    }

    fn uses_dataset_keys(&self) -> bool {
        true // Query uses dataset queries
    }

    fn key_is_claimed_id(&self) -> bool {
        false
    }
}

// =============================================================================
// VectorQueryWithDeletesContext - For VecQuery after deletions with GT-aware recall
// =============================================================================

/// Context for vector query workloads after deletion, with ground-truth aware recall
///
/// This context extends VectorQueryContext with the ability to compute recall
/// while accounting for deleted ground truth vectors. It supports two modes:
///
/// - **Protected Mode**: Ground truth vectors are never deleted (use with DeleteContext
///   that has protected_ids). Recall computation is standard.
///
/// - **Adjusted Mode**: Ground truth vectors may be deleted. Recall is computed against
///   only the surviving ground truth vectors.
///
/// # Example: Delete-then-Query Benchmark Flow
///
/// ```ignore
/// // 1. Create GT-aware recall calculator
/// let gt_recall = dataset.build_gt_aware_recall(GroundTruthMode::Adjusted, k);
///
/// // 2. Create existence tracker for vectors
/// let existence_map = Arc::new(KeyGroupExistanceTracker::new("vec:", num_vectors, true));
/// // ... populate from scan ...
///
/// // 3. Run deletions (with or without GT protection)
/// // ... DeleteContext runs ...
///
/// // 4. Create query context with GT-aware recall
/// let query_ctx = VectorQueryWithDeletesContext::new(
///     dataset, k, key_prefix, gt_recall, existence_map.tracker().clone()
/// );
///
/// // 5. During queries, recall is computed against surviving GT
/// ```
pub struct VectorQueryWithDeletesContext {
    dataset: Arc<DatasetContext>,
    recall_stats: RecallStats,
    /// Aggregated adjusted recall stats for reporting
    adjusted_stats: AdjustedRecallAggregator,
    k: usize,
    key_prefix: String,
    /// Tracker for query iteration (prefix: "query_vec:<dataset-name>:")
    query_tracker: Arc<PrefixTracker>,
    /// Iteration strategy for queries
    strategy: IterationStrategy,
    /// Ground truth aware recall calculator
    gt_recall: Arc<crate::keyspace::GroundTruthAwareRecall>,
    /// Existence tracker for vectors (to check which GT still exist)
    vector_tracker: Arc<PrefixTracker>,
}

/// Aggregator for adjusted recall statistics
#[derive(Debug, Clone, Default)]
pub struct AdjustedRecallAggregator {
    pub total_queries: u64,
    pub total_gt_existing: u64,
    pub total_gt_deleted: u64,
    pub total_matches: u64,
    pub sum_recall: f64,
    pub sum_coverage: f64,
    pub min_coverage: f64,
    pub max_coverage: f64,
}

impl AdjustedRecallAggregator {
    pub fn new() -> Self {
        Self {
            min_coverage: 1.0,
            max_coverage: 0.0,
            ..Default::default()
        }
    }

    pub fn record(&mut self, stats: &crate::keyspace::AdjustedRecallStats) {
        self.total_queries += 1;
        self.total_gt_existing += stats.gt_existing;
        self.total_gt_deleted += stats.gt_deleted;
        self.total_matches += stats.matches as u64;
        self.sum_recall += stats.recall;
        self.sum_coverage += stats.gt_coverage;
        self.min_coverage = self.min_coverage.min(stats.gt_coverage);
        self.max_coverage = self.max_coverage.max(stats.gt_coverage);
    }

    pub fn avg_recall(&self) -> f64 {
        if self.total_queries > 0 {
            self.sum_recall / self.total_queries as f64
        } else {
            0.0
        }
    }

    pub fn avg_coverage(&self) -> f64 {
        if self.total_queries > 0 {
            self.sum_coverage / self.total_queries as f64
        } else {
            1.0
        }
    }

    pub fn coverage_range(&self) -> (f64, f64) {
        (self.min_coverage, self.max_coverage)
    }
}

impl VectorQueryWithDeletesContext {
    /// Create a new context for querying after deletions
    ///
    /// # Arguments
    /// * `dataset` - Dataset with queries and ground truth
    /// * `k` - Number of results to request/verify
    /// * `key_prefix` - Key prefix for parsing result IDs
    /// * `gt_recall` - Ground truth aware recall calculator
    /// * `vector_tracker` - Tracker of existing vectors (for adjusted recall)
    pub fn new(
        dataset: Arc<DatasetContext>,
        k: usize,
        key_prefix: String,
        gt_recall: Arc<crate::keyspace::GroundTruthAwareRecall>,
        vector_tracker: Arc<PrefixTracker>,
    ) -> Self {
        Self::with_strategy(
            dataset,
            k,
            key_prefix,
            gt_recall,
            vector_tracker,
            IterationStrategy::Sequential,
        )
    }

    /// Create with custom iteration strategy
    pub fn with_strategy(
        dataset: Arc<DatasetContext>,
        k: usize,
        key_prefix: String,
        gt_recall: Arc<crate::keyspace::GroundTruthAwareRecall>,
        vector_tracker: Arc<PrefixTracker>,
        strategy: IterationStrategy,
    ) -> Self {
        let prefix = format!("query_vec:{}:", dataset.name());
        let num_queries = dataset.num_queries();
        let config = TrackerConfig::simple(&prefix)
            .with_max_id(num_queries)
            .with_initial_capacity(num_queries as usize);
        let query_tracker = Arc::new(PrefixTracker::new(config));

        Self {
            dataset,
            recall_stats: RecallStats::new(),
            adjusted_stats: AdjustedRecallAggregator::new(),
            k,
            key_prefix,
            query_tracker,
            strategy,
            gt_recall,
            vector_tracker,
        }
    }

    /// Get GT coverage statistics before running queries
    pub fn gt_coverage(&self) -> (u64, u64, f64) {
        self.gt_recall.compute_coverage(&self.vector_tracker)
    }

    /// Get aggregated adjusted recall statistics
    pub fn adjusted_stats(&self) -> &AdjustedRecallAggregator {
        &self.adjusted_stats
    }

    /// Get the GT mode
    pub fn gt_mode(&self) -> crate::keyspace::GroundTruthMode {
        self.gt_recall.mode()
    }
}

impl WorkloadContext for VectorQueryWithDeletesContext {
    fn claim_next_id(&self) -> Option<u64> {
        let num_queries = self.dataset.num_queries();

        let id = match &self.strategy {
            IterationStrategy::Sequential => self
                .query_tracker
                .iter()
                .continue_write()
                .next()
                .map(|(id, _)| id),
            IterationStrategy::Random { seed } => self
                .query_tracker
                .iter()
                .seed(*seed)
                .continue_write()
                .next()
                .map(|(id, _)| id),
            IterationStrategy::Zipfian { skew, seed } => self
                .query_tracker
                .iter()
                .distribution(AccessDistribution::Zipfian { skew: *skew })
                .seed(*seed)
                .continue_write()
                .next()
                .map(|(id, _)| id),
            IterationStrategy::Subset { start, end, inner } => {
                let base = self.query_tracker.iter().id_range(*start, *end);
                match inner.as_ref() {
                    IterationStrategy::Sequential => {
                        base.continue_write().next().map(|(id, _)| id)
                    }
                    IterationStrategy::Random { seed } => {
                        base.seed(*seed).continue_write().next().map(|(id, _)| id)
                    }
                    _ => base.continue_write().next().map(|(id, _)| id),
                }
            }
        };

        id.map(|i| i % num_queries)
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        None
    }

    fn get_query_bytes(&self, idx: u64) -> Option<&[u8]> {
        Some(self.dataset.get_query_bytes(idx))
    }

    fn get_vector_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn compute_and_record_recall(&mut self, query_idx: u64, response: &RespValue) {
        let doc_ids = parse_search_response(response);
        let result_ids = extract_numeric_ids(&doc_ids, &self.key_prefix);

        // Use GT-aware recall computation
        let (recall, stats) = self.gt_recall.compute_recall_with_stats(
            query_idx,
            &result_ids,
            self.k,
            &self.vector_tracker,
        );

        // Record both standard and adjusted stats
        self.recall_stats.record(recall);
        self.adjusted_stats.record(&stats);
    }

    fn take_metrics(&mut self) -> WorkloadMetrics {
        let stats = std::mem::take(&mut self.recall_stats);
        self.recall_stats = RecallStats::new();
        // Note: adjusted_stats is preserved for final reporting
        WorkloadMetrics::Recall(stats)
    }

    fn num_items(&self) -> u64 {
        self.dataset.num_vectors()
    }

    fn num_queries(&self) -> u64 {
        self.dataset.num_queries()
    }

    fn uses_dataset_keys(&self) -> bool {
        true
    }

    fn key_is_claimed_id(&self) -> bool {
        false
    }
}

// =============================================================================
// DeleteContext - Uses shared ProtectedIds for VecDel
// =============================================================================

/// Context for vector deletion with GT protection
///
/// Uses shared ProtectedIds (like VecLoad uses shared KeyGroupExistanceTracker).
/// All workers share the same atomic cursor via Arc<ProtectedIds>.
/// Respects dataset offset and num_vectors like other contexts.
pub struct DeleteContext {
    /// Dataset for offset/range info
    dataset: Arc<DatasetContext>,
    /// Shared protected IDs with atomic cursor
    protected_ids: Arc<ProtectedIds>,
    /// Request limit (total across all workers)
    request_limit: Option<u64>,
}

impl DeleteContext {
    /// Create with shared ProtectedIds and dataset
    pub fn new(
        dataset: Arc<DatasetContext>,
        protected_ids: Arc<ProtectedIds>,
        request_limit: Option<u64>,
    ) -> Self {
        Self {
            dataset,
            protected_ids,
            request_limit,
        }
    }

    /// Get count of protected (GT) IDs
    pub fn protected_count(&self) -> u64 {
        self.protected_ids.protected_count()
    }

    /// Get count of deleteable (non-GT) IDs
    pub fn deleteable_count(&self) -> u64 {
        self.protected_ids.deleteable_count()
    }

    /// Check if an ID is protected (GT)
    #[inline]
    pub fn is_protected(&self, id: u64) -> bool {
        self.protected_ids.is_protected(id)
    }
}

impl WorkloadContext for DeleteContext {
    fn claim_next_id(&self) -> Option<u64> {
        // Check if we've hit the request limit
        if let Some(limit) = self.request_limit {
            if self.protected_ids.claimed_count() >= limit {
                return None;
            }
        }

        // Use dataset offset and num_vectors like other contexts
        let offset = self.dataset.effective_offset();
        let num_vectors = self.dataset.num_vectors();
        
        // Claim in range [offset, offset + num_vectors)
        self.protected_ids.claim_deleteable_id_in_range(offset, offset + num_vectors)
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        self.claim_next_id()
    }

    fn get_query_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn get_vector_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn compute_and_record_recall(&mut self, _query_idx: u64, _response: &RespValue) {}

    fn take_metrics(&mut self) -> WorkloadMetrics {
        WorkloadMetrics::None
    }

    fn num_items(&self) -> u64 {
        self.dataset.num_vectors()
    }

    fn num_queries(&self) -> u64 {
        0
    }

    fn uses_dataset_keys(&self) -> bool {
        true
    }

    fn key_is_claimed_id(&self) -> bool {
        true
    }
}

// =============================================================================
// GtLoadContext - For VecGtLoad (load only ground truth vectors)
// =============================================================================

/// Context for loading only ground truth vectors
///
/// Iterates through GT IDs from ProtectedIds using a shared atomic cursor.
/// Used to populate a minimal dataset containing just the vectors needed for recall.
pub struct GtLoadContext {
    dataset: Arc<DatasetContext>,
    /// Shared ProtectedIds containing GT vector IDs
    protected_ids: Arc<ProtectedIds>,
    /// Atomic cursor for iterating GT IDs
    cursor: AtomicU64,
    /// Cached sorted GT IDs for iteration
    gt_ids: Vec<u64>,
    tag_distributions: Option<TagDistributionSet>,
    numeric_fields: NumericFieldSet,
}

impl GtLoadContext {
    pub fn new(
        dataset: Arc<DatasetContext>,
        protected_ids: Arc<ProtectedIds>,
        tag_distributions: Option<TagDistributionSet>,
        numeric_fields: NumericFieldSet,
    ) -> Self {
        // Collect GT IDs into a sorted vector for iteration
        let gt_ids: Vec<u64> = protected_ids.reference_set().iter().collect();
        
        Self {
            dataset,
            protected_ids,
            cursor: AtomicU64::new(0),
            gt_ids,
            tag_distributions,
            numeric_fields,
        }
    }

    /// Get count of GT vectors to load
    pub fn gt_count(&self) -> usize {
        self.gt_ids.len()
    }
}

impl WorkloadContext for GtLoadContext {
    fn claim_next_id(&self) -> Option<u64> {
        let idx = self.cursor.fetch_add(1, Ordering::Relaxed) as usize;
        if idx < self.gt_ids.len() {
            Some(self.gt_ids[idx])
        } else {
            None
        }
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        self.claim_next_id()
    }

    fn get_query_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn get_vector_bytes(&self, idx: u64) -> Option<&[u8]> {
        Some(self.dataset.get_vector_bytes(idx))
    }

    fn fill_tag_placeholder(&self, key_num: u64, buf: &mut [u8]) {
        if let Some(ref tag_dist) = self.tag_distributions {
            if let Some(tags) = tag_dist.select_tags_seeded(key_num) {
                let tag_bytes = tags.as_bytes();
                let copy_len = tag_bytes.len().min(buf.len());
                buf[..copy_len].copy_from_slice(&tag_bytes[..copy_len]);
                buf[copy_len..].fill(b',');
            } else {
                buf.fill(b',');
            }
        } else {
            buf.fill(b',');
        }
    }

    fn fill_numeric_field(&self, field_idx: usize, key_num: u64, seq_counter: u64, buf: &mut [u8]) {
        if let Some(field_config) = self.numeric_fields.get(field_idx) {
            field_config.fill_buffer(key_num, seq_counter, buf);
        } else {
            buf.fill(b'0');
        }
    }

    fn compute_and_record_recall(&mut self, _query_idx: u64, _response: &RespValue) {}

    fn take_metrics(&mut self) -> WorkloadMetrics {
        WorkloadMetrics::None
    }

    fn num_items(&self) -> u64 {
        self.gt_ids.len() as u64
    }

    fn num_queries(&self) -> u64 {
        0
    }

    fn uses_dataset_keys(&self) -> bool {
        true
    }

    fn key_is_claimed_id(&self) -> bool {
        false // Key is the GT ID, not a simple claimed counter
    }
}

// =============================================================================
// VectorUpdateContext - For VecUpdate (similar to VecLoad but updates existing)
// =============================================================================

/// Context for vector update workloads (HSET updating existing vectors)
///
/// Uses tracker-based iteration for cycling through existing vectors.
/// Iterator exhaustion signals completion.
pub struct VectorUpdateContext {
    dataset: Arc<DatasetContext>,
    tag_distributions: Option<TagDistributionSet>,
    numeric_fields: NumericFieldSet,
    /// Tracker for iteration (prefix: "vec:<dataset-name>:")
    tracker: Arc<PrefixTracker>,
    /// Iteration strategy
    strategy: IterationStrategy,
}

impl VectorUpdateContext {
    pub fn new(
        dataset: Arc<DatasetContext>,
        tag_distributions: Option<TagDistributionSet>,
        numeric_fields: NumericFieldSet,
    ) -> Self {
        let prefix = format!("vec:{}:", dataset.name());
        let num_vectors = dataset.num_vectors();
        let config = TrackerConfig::simple(&prefix)
            .with_max_id(num_vectors)
            .with_initial_capacity(num_vectors as usize);
        let tracker = Arc::new(PrefixTracker::new(config));
        
        Self {
            dataset,
            tag_distributions,
            numeric_fields,
            tracker,
            strategy: IterationStrategy::Sequential,
        }
    }

    /// Create with strategy
    pub fn with_strategy(
        dataset: Arc<DatasetContext>,
        tag_distributions: Option<TagDistributionSet>,
        numeric_fields: NumericFieldSet,
        strategy: IterationStrategy,
    ) -> Self {
        let prefix = format!("vec:{}:", dataset.name());
        let num_vectors = dataset.num_vectors();
        let config = TrackerConfig::simple(&prefix)
            .with_max_id(num_vectors)
            .with_initial_capacity(num_vectors as usize);
        let tracker = Arc::new(PrefixTracker::new(config));
        
        Self {
            dataset,
            tag_distributions,
            numeric_fields,
            tracker,
            strategy,
        }
    }

    /// Get the tracker for sharing with other contexts
    pub fn tracker(&self) -> Arc<PrefixTracker> {
        Arc::clone(&self.tracker)
    }
}

impl WorkloadContext for VectorUpdateContext {
    fn claim_next_id(&self) -> Option<u64> {
        let num_vectors = self.dataset.num_vectors();
        
        // Use tracker-based iteration
        // Iterator exhaustion signals completion - no wrap-around
        let id = match &self.strategy {
            IterationStrategy::Sequential => {
                self.tracker.iter().continue_write().next().map(|(id, _)| id)
            }
            IterationStrategy::Random { seed } => {
                self.tracker.iter().seed(*seed).continue_write().next().map(|(id, _)| id)
            }
            IterationStrategy::Zipfian { skew, seed } => {
                self.tracker.iter()
                    .distribution(AccessDistribution::Zipfian { skew: *skew })
                    .seed(*seed)
                    .continue_write()
                    .next()
                    .map(|(id, _)| id)
            }
            IterationStrategy::Subset { start, end, inner } => {
                let base = self.tracker.iter().id_range(*start, *end);
                match inner.as_ref() {
                    IterationStrategy::Sequential => base.continue_write().next().map(|(id, _)| id),
                    IterationStrategy::Random { seed } => base.seed(*seed).continue_write().next().map(|(id, _)| id),
                    _ => base.continue_write().next().map(|(id, _)| id),
                }
            }
        };

        // Iterator is the source of truth - return None when exhausted
        id.map(|i| i % num_vectors)
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        self.claim_next_id()
    }

    fn get_query_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn get_vector_bytes(&self, idx: u64) -> Option<&[u8]> {
        Some(self.dataset.get_vector_bytes(idx))
    }

    fn fill_tag_placeholder(&self, key_num: u64, buf: &mut [u8]) {
        if let Some(ref tag_dist) = self.tag_distributions {
            if let Some(tags) = tag_dist.select_tags_seeded(key_num) {
                let tag_bytes = tags.as_bytes();
                let copy_len = tag_bytes.len().min(buf.len());
                buf[..copy_len].copy_from_slice(&tag_bytes[..copy_len]);
                buf[copy_len..].fill(b',');
            } else {
                buf.fill(b',');
            }
        } else {
            buf.fill(b',');
        }
    }

    fn fill_numeric_field(&self, field_idx: usize, key_num: u64, seq_counter: u64, buf: &mut [u8]) {
        if let Some(field_config) = self.numeric_fields.get(field_idx) {
            field_config.fill_buffer(key_num, seq_counter, buf);
        } else {
            buf.fill(b'0');
        }
    }

    fn compute_and_record_recall(&mut self, _query_idx: u64, _response: &RespValue) {
        // No-op for update workloads
    }

    fn take_metrics(&mut self) -> WorkloadMetrics {
        WorkloadMetrics::None
    }

    fn num_items(&self) -> u64 {
        self.dataset.num_vectors()
    }

    fn num_queries(&self) -> u64 {
        0
    }

    fn uses_dataset_keys(&self) -> bool {
        true // Key is the vector ID from dataset
    }

    fn key_is_claimed_id(&self) -> bool {
        false // Key is set by Vector handler
    }
}

// =============================================================================
// AddressableContext - For hash field and JSON path workloads
// =============================================================================

/// Context for workloads using AddressableSpace (hash fields, JSON paths)
///
/// This context iterates over an address space that may include:
/// - Simple keys (KeySpace)
/// - Hash keys with multiple fields (HashFieldSpace)
/// - JSON keys with multiple paths (JsonPathSpace)
///
/// Uses tracker-based iteration with support for various distribution strategies.
pub struct AddressableContext {
    space: Box<dyn AddressableSpace>,
    strategy: IterationStrategy,
    /// Last claimed address index (for fill methods to reference)
    last_address_idx: AtomicU64,
    /// Tracker for iteration
    tracker: Arc<PrefixTracker>,
}

impl AddressableContext {
    /// Create a new AddressableContext with the given address space
    pub fn new(space: Box<dyn AddressableSpace>, sequential: bool, seed: u64) -> Self {
        let strategy = if sequential {
            IterationStrategy::Sequential
        } else {
            IterationStrategy::Random { seed }
        };
        Self::with_strategy(space, strategy)
    }

    /// Create a new AddressableContext with custom iteration strategy
    pub fn with_strategy(space: Box<dyn AddressableSpace>, strategy: IterationStrategy) -> Self {
        // Create tracker with max_id = space.len()
        let space_len = space.len();
        let config = TrackerConfig::simple("addr:")
            .with_max_id(space_len)
            .with_initial_capacity(space_len as usize);
        let tracker = Arc::new(PrefixTracker::new(config));
        
        Self {
            space,
            strategy,
            last_address_idx: AtomicU64::new(0),
            tracker,
        }
    }

    /// Get the address space
    pub fn space(&self) -> &dyn AddressableSpace {
        self.space.as_ref()
    }

    /// Get address for the given key number
    /// Called by event worker to get the address components for placeholder filling
    pub fn address_for_key(&self, key_num: u64) -> Address {
        self.space.address_at(key_num)
    }

    /// Helper to get the next ID using the appropriate iterator
    ///
    /// For Sequential strategy, uses `continue_write()` which shares an atomic
    /// cursor across all threads. For Random/Zipfian, each call generates a
    /// new random position but the global counter ensures progress.
    fn get_next_from_tracker(&self) -> Option<(u64, Option<u64>)> {
        match &self.strategy {
            IterationStrategy::Sequential => {
                // Use continue_write() to share atomic cursor across threads
                self.tracker.iter().continue_write().next()
            }
            IterationStrategy::Random { seed } => {
                // Random iteration - use atomic cursor for progress, then map to random position
                self.tracker.iter().seed(*seed).continue_write().next()
            }
            IterationStrategy::Zipfian { skew, seed } => {
                // Zipfian distribution - atomic cursor + zipfian mapping
                self.tracker.iter()
                    .distribution(AccessDistribution::Zipfian { skew: *skew })
                    .seed(*seed)
                    .continue_write()
                    .next()
            }
            IterationStrategy::Subset { start, end, inner } => {
                let base_iter = self.tracker.iter().id_range(*start, *end);
                match inner.as_ref() {
                    IterationStrategy::Sequential => base_iter.continue_write().next(),
                    IterationStrategy::Random { seed } => base_iter.seed(*seed).continue_write().next(),
                    IterationStrategy::Zipfian { skew, seed } => {
                        base_iter
                            .distribution(AccessDistribution::Zipfian { skew: *skew })
                            .seed(*seed)
                            .continue_write()
                            .next()
                    }
                    IterationStrategy::Subset { .. } => base_iter.continue_write().next(),
                }
            }
        }
    }
}

impl WorkloadContext for AddressableContext {
    fn claim_next_id(&self) -> Option<u64> {
        if let Some((id, _)) = self.get_next_from_tracker() {
            let idx = id % self.space.len();
            // Store for later reference by fill methods
            self.last_address_idx.store(idx, Ordering::Relaxed);
            Some(idx)
        } else {
            None
        }
    }

    fn next_dataset_idx(&self) -> Option<u64> {
        None
    }

    fn get_query_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn get_vector_bytes(&self, _idx: u64) -> Option<&[u8]> {
        None
    }

    fn compute_and_record_recall(&mut self, _query_idx: u64, _response: &RespValue) {
        // No-op for addressable workloads
    }

    fn take_metrics(&mut self) -> WorkloadMetrics {
        WorkloadMetrics::None
    }

    fn num_items(&self) -> u64 {
        self.space.len()
    }

    fn num_queries(&self) -> u64 {
        0
    }

    fn uses_dataset_keys(&self) -> bool {
        false
    }

    fn key_is_claimed_id(&self) -> bool {
        false
    }

    fn address_type(&self) -> AddressType {
        self.space.address_type()
    }

    fn current_address(&self) -> Option<&Address> {
        // Cannot return reference to computed address
        // Use fill methods instead which compute on demand
        None
    }

    fn fill_field_placeholder(&self, buf: &mut [u8]) {
        let idx = self.last_address_idx.load(Ordering::Relaxed);
        let address = self.space.address_at(idx);

        if let Some(ref field) = address.field {
            let field_bytes = field.as_bytes();
            let copy_len = field_bytes.len().min(buf.len());
            buf[..copy_len].copy_from_slice(&field_bytes[..copy_len]);
            buf[copy_len..].fill(b' ');
        } else {
            buf.fill(b' ');
        }
    }

    fn fill_json_path_placeholder(&self, buf: &mut [u8]) {
        let idx = self.last_address_idx.load(Ordering::Relaxed);
        let address = self.space.address_at(idx);

        if let Some(ref path) = address.path {
            let path_bytes = path.as_bytes();
            let copy_len = path_bytes.len().min(buf.len());
            buf[..copy_len].copy_from_slice(&path_bytes[..copy_len]);
            buf[copy_len..].fill(b' ');
        } else {
            buf.fill(b' ');
        }
    }
}

// =============================================================================
// Factory function
// =============================================================================

/// Create appropriate WorkloadContext for the given workload type
pub fn create_workload_context(
    workload_type: WorkloadType,
    dataset: Option<Arc<DatasetContext>>,
    existence_map: Option<Arc<KeyGroupExistanceTracker>>,
    protected_ids: Option<Arc<ProtectedIds>>,
    tag_distributions: Option<TagDistributionSet>,
    numeric_fields: NumericFieldSet,
    keyspace_len: u64,
    sequential: bool,
    seed: u64,
    k: usize,
    key_prefix: &str,
) -> Box<dyn WorkloadContext> {
    create_workload_context_with_iteration(
        workload_type,
        dataset,
        existence_map,
        protected_ids,
        tag_distributions,
        numeric_fields,
        keyspace_len,
        sequential,
        seed,
        k,
        key_prefix,
        None, // No custom iteration strategy
        None, // No request limit
    )
}

/// Create appropriate WorkloadContext with optional custom iteration strategy
pub fn create_workload_context_with_iteration(
    workload_type: WorkloadType,
    dataset: Option<Arc<DatasetContext>>,
    existence_map: Option<Arc<KeyGroupExistanceTracker>>,
    protected_ids: Option<Arc<ProtectedIds>>,
    tag_distributions: Option<TagDistributionSet>,
    numeric_fields: NumericFieldSet,
    keyspace_len: u64,
    sequential: bool,
    seed: u64,
    k: usize,
    key_prefix: &str,
    iteration: Option<&str>,
    request_limit: Option<u64>,
) -> Box<dyn WorkloadContext> {
    // Parse iteration strategy if provided
    let strategy = if let Some(iter_str) = iteration {
        match IterationStrategy::parse(iter_str) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Warning: Invalid iteration strategy '{}', using default", iter_str);
                if sequential {
                    IterationStrategy::Sequential
                } else {
                    IterationStrategy::Random { seed }
                }
            }
        }
    } else if sequential {
        IterationStrategy::Sequential
    } else {
        IterationStrategy::Random { seed }
    };

    match workload_type {
        WorkloadType::VecLoad => {
            let ds = dataset.expect("VecLoad requires dataset");
            Box::new(VectorLoadContext::with_strategy(
                ds,
                existence_map,
                tag_distributions,
                numeric_fields,
                strategy,
                request_limit,
            ))
        }
        WorkloadType::VecGtLoad => {
            let ds = dataset.expect("VecGtLoad requires dataset");
            // Use ProtectedIds which contains GT IDs from dataset
            if let Some(pids) = protected_ids {
                Box::new(GtLoadContext::new(ds, pids, tag_distributions, numeric_fields))
            } else {
                panic!("VecGtLoad requires dataset with ground truth (protected_ids)")
            }
        }
        WorkloadType::VecQuery => {
            let ds = dataset.expect("VecQuery requires dataset");
            Box::new(VectorQueryContext::with_strategy(
                ds,
                k,
                key_prefix.to_string(),
                strategy,
            ))
        }
        WorkloadType::VecDel => {
            let ds = dataset.expect("VecDel requires dataset");
            // Use shared ProtectedIds (like VecLoad uses shared KeyGroupExistanceTracker)
            // The dataset's offset and num_vectors are used in claim_next_id
            if let Some(pids) = protected_ids {
                Box::new(DeleteContext::new(ds, pids, request_limit))
            } else {
                // No GT protection - create empty ProtectedIds with max possible range
                let max_id = ds.effective_offset() + ds.num_vectors();
                let empty_pids = Arc::new(ProtectedIds::new(std::iter::empty::<u64>(), max_id));
                Box::new(DeleteContext::new(ds, empty_pids, request_limit))
            }
        }
        WorkloadType::VecUpdate => {
            let ds = dataset.expect("VecUpdate requires dataset");
            Box::new(VectorUpdateContext::with_strategy(
                ds,
                tag_distributions,
                numeric_fields,
                strategy,
            ))
        }
        _ => {
            // All other workloads use simple key-value context
            Box::new(SimpleContext::with_strategy(keyspace_len, strategy))
        }
    }
}

/// Create a WorkloadContext with a shared tracker for atomic iteration across threads.
///
/// This is required for write workloads (SET, etc.) where all workers must share
/// the same atomic cursor to ensure exactly-once ID claiming.
///
/// # Arguments
/// * `shared_tracker` - Shared PrefixTracker (created with `SimpleContext::create_shared_tracker()`)
/// * `keyspace_len` - Total keyspace size
/// * `strategy` - Iteration strategy
pub fn create_workload_context_with_shared_tracker(
    shared_tracker: Arc<PrefixTracker>,
    keyspace_len: u64,
    strategy: IterationStrategy,
) -> Box<dyn WorkloadContext + Send> {
    Box::new(SimpleContext::with_shared_tracker(shared_tracker, keyspace_len, strategy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_context_sequential() {
        let ctx = SimpleContext::new(100, true, 0);

        // Sequential should return 0, 1, 2, ...
        assert_eq!(ctx.claim_next_id(), Some(0));
        assert_eq!(ctx.claim_next_id(), Some(1));
        assert_eq!(ctx.claim_next_id(), Some(2));
    }

    #[test]
    fn test_simple_context_shared_tracker_sequential() {
        // Create a shared tracker
        let shared_tracker = SimpleContext::create_shared_tracker(100);
        
        // Create two contexts sharing the same tracker
        let ctx1 = SimpleContext::with_shared_tracker(
            Arc::clone(&shared_tracker),
            100,
            IterationStrategy::Sequential,
        );
        let ctx2 = SimpleContext::with_shared_tracker(
            Arc::clone(&shared_tracker),
            100,
            IterationStrategy::Sequential,
        );
        
        // Both contexts should claim different IDs from the same sequence
        let id1 = ctx1.claim_next_id();
        let id2 = ctx2.claim_next_id();
        let id3 = ctx1.claim_next_id();
        let id4 = ctx2.claim_next_id();
        
        // IDs should be unique and sequential (0, 1, 2, 3)
        let mut ids = vec![id1, id2, id3, id4];
        ids.sort();
        assert_eq!(ids, vec![Some(0), Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn test_simple_context_shared_tracker_concurrent() {
        use std::sync::Arc;
        use std::thread;
        use std::collections::HashSet;
        
        let keyspace = 1000u64;
        let shared_tracker = SimpleContext::create_shared_tracker(keyspace);
        let ids_claimed = Arc::new(std::sync::Mutex::new(HashSet::new()));
        
        // Spawn multiple threads that each claim IDs
        let handles: Vec<_> = (0..4).map(|_| {
            let tracker = Arc::clone(&shared_tracker);
            let ids = Arc::clone(&ids_claimed);
            
            thread::spawn(move || {
                let ctx = SimpleContext::with_shared_tracker(
                    tracker,
                    keyspace,
                    IterationStrategy::Sequential,
                );
                
                // Each thread claims 100 IDs
                for _ in 0..100 {
                    if let Some(id) = ctx.claim_next_id() {
                        let mut guard = ids.lock().unwrap();
                        // Assert no duplicate IDs
                        assert!(guard.insert(id), "Duplicate ID claimed: {}", id);
                    }
                }
            })
        }).collect();
        
        // Wait for all threads to complete
        for h in handles {
            h.join().unwrap();
        }
        
        // Should have exactly 400 unique IDs
        let ids = ids_claimed.lock().unwrap();
        assert_eq!(ids.len(), 400, "Expected 400 unique IDs, got {}", ids.len());
    }

    #[test]
    fn test_workload_metrics_default() {
        let metrics = WorkloadMetrics::default();
        assert!(matches!(metrics, WorkloadMetrics::None));
    }

    #[test]
    fn test_simple_context_fill_tag_default() {
        // SimpleContext uses default fill_tag_placeholder which fills with commas
        let ctx = SimpleContext::new(100, true, 0);
        let mut buf = [0u8; 10];
        ctx.fill_tag_placeholder(42, &mut buf);
        assert!(buf.iter().all(|&b| b == b','));
    }

    #[test]
    fn test_tag_distribution_fill_logic() {
        // Test the tag distribution fill logic directly
        let tag_dist = TagDistributionSet::parse("test:100").unwrap();
        let mut buf = [0u8; 10];

        // Simulate what fill_tag_placeholder does
        if let Some(tags) = tag_dist.select_tags_seeded(42) {
            let tag_bytes = tags.as_bytes();
            let copy_len = tag_bytes.len().min(buf.len());
            buf[..copy_len].copy_from_slice(&tag_bytes[..copy_len]);
            buf[copy_len..].fill(b',');
        } else {
            buf.fill(b',');
        }

        // Should have "test" followed by commas
        assert_eq!(&buf[0..4], b"test");
        assert!(buf[4..].iter().all(|&b| b == b','));
    }

    #[test]
    fn test_tag_distribution_deterministic() {
        // Same key_num should produce same tags
        let tag_dist = TagDistributionSet::parse("a:50,b:50").unwrap();

        let tags1 = tag_dist.select_tags_seeded(12345);
        let tags2 = tag_dist.select_tags_seeded(12345);

        assert_eq!(tags1, tags2);
    }

    #[test]
    fn test_tag_distribution_different_keys() {
        // Different key_num can produce different tags
        let tag_dist = TagDistributionSet::parse("a:50,b:50").unwrap();

        // Run many times - at least some should be different
        let mut same_count = 0;
        for i in 0..100 {
            let tags1 = tag_dist.select_tags_seeded(i);
            let tags2 = tag_dist.select_tags_seeded(i + 1000);
            if tags1 == tags2 {
                same_count += 1;
            }
        }
        // With 50% probability for each of 2 tags, we expect significant variation
        assert!(same_count < 90, "Tags should vary across different keys");
    }

    #[test]
    fn test_simple_context_random() {
        let ctx = SimpleContext::new(1000, false, 42);
        
        // Random iteration should produce different keys
        let id1 = ctx.claim_next_id().unwrap();
        let id2 = ctx.claim_next_id().unwrap();
        let id3 = ctx.claim_next_id().unwrap();
        
        // Keys should be within range
        assert!(id1 < 1000);
        assert!(id2 < 1000);
        assert!(id3 < 1000);
    }

    #[test]
    fn test_simple_context_tracker_based() {
        // Verify that SimpleContext uses tracker-based iteration
        let ctx = SimpleContext::new(100, true, 0);
        
        // The tracker should be initialized
        assert!(ctx.tracker().count() == 0); // No IDs set yet
        
        // Claiming IDs should work
        let _ = ctx.claim_next_id();
        let _ = ctx.claim_next_id();
        
        // Tracker is used for iteration but doesn't track claimed IDs in this mode
        // (tracker.iter().sequential() doesn't set bits)
    }

    #[test]
    fn test_simple_context_with_request_limit() {
        // Test that request limit stops iteration after N claims
        let ctx = SimpleContext::with_strategy_and_limit(
            100,  // keyspace
            IterationStrategy::Random { seed: 42 },
            10,   // limit to 10 requests
        );

        // Should be able to claim exactly 10 IDs
        for i in 0..10 {
            assert!(ctx.claim_next_id().is_some(), "Expected ID at iteration {}", i);
        }

        // 11th claim should fail
        assert!(ctx.claim_next_id().is_none(), "Expected None after limit reached");
        assert!(ctx.claim_next_id().is_none(), "Should still be None");
    }

    #[test]
    fn test_request_limit_exceeds_keyspace() {
        // Test "1M requests on 100K keyspace" scenario (scaled down)
        // 20 requests on a keyspace of 5 - should wrap around
        let ctx = SimpleContext::with_strategy_and_limit(
            5,    // small keyspace
            IterationStrategy::Sequential,
            20,   // 4x the keyspace
        );

        let mut ids = Vec::new();
        for _ in 0..20 {
            if let Some(id) = ctx.claim_next_id() {
                ids.push(id);
            }
        }

        // Should have claimed 20 IDs
        assert_eq!(ids.len(), 20, "Expected 20 IDs");

        // All IDs should be in range [0, 5)
        for id in &ids {
            assert!(*id < 5, "ID {} should be < 5", id);
        }

        // After 20 claims, should return None
        assert!(ctx.claim_next_id().is_none());
    }

    // =========================================================================
    // DeleteContext Tests - Verify GT skipping logic
    // =========================================================================

    /// Test GT skipping with bitmap directly (no dataset needed)
    #[test]
    fn test_gt_bitmap_skipping_sequential() {
        use keyspace_tracker::{PrefixTracker, TrackerConfig};

        let max_id = 20u64;
        let config = TrackerConfig::simple("test:")
            .with_max_id(max_id)
            .with_initial_capacity(max_id as usize);
        let gt_bitmap = PrefixTracker::new(config);

        // Mark GT IDs: 5, 10, 15 (protected)
        gt_bitmap.add(5);
        gt_bitmap.add(10);
        gt_bitmap.add(15);

        assert_eq!(gt_bitmap.count(), 3, "Should have 3 GT IDs");

        // Use unset_only() to iterate only on non-GT IDs
        // WriteIter doesn't implement Iterator trait, so use next() method directly
        let mut deleted_ids: Vec<u64> = Vec::new();
        let mut iter = gt_bitmap.iter().unset_only().continue_write();
        while let Some((id, _)) = iter.next() {
            if id >= max_id {
                break;
            }
            deleted_ids.push(id);
        }

        // Should have deleted 17 IDs (20 - 3 GT)
        assert_eq!(deleted_ids.len(), 17, "Should have 17 deleteable IDs, got {}", deleted_ids.len());

        // Verify GT IDs are NOT in the deleted list
        assert!(!deleted_ids.contains(&5), "GT ID 5 should not be deleted");
        assert!(!deleted_ids.contains(&10), "GT ID 10 should not be deleted");
        assert!(!deleted_ids.contains(&15), "GT ID 15 should not be deleted");

        // Verify all other IDs are present
        for id in 0..max_id {
            if id != 5 && id != 10 && id != 15 {
                assert!(deleted_ids.contains(&id), "ID {} should be in deleted list", id);
            }
        }
    }

    /// Test GT skipping with random iteration
    #[test]
    fn test_gt_bitmap_skipping_random() {
        use keyspace_tracker::{PrefixTracker, TrackerConfig};
        use std::collections::HashSet;

        let max_id = 100u64;
        let config = TrackerConfig::simple("test:")
            .with_max_id(max_id)
            .with_initial_capacity(max_id as usize);
        let gt_bitmap = PrefixTracker::new(config);

        // Mark GT IDs: even numbers 0-20 (11 protected IDs)
        let gt_ids: Vec<u64> = (0..=20).step_by(2).collect();
        for &id in &gt_ids {
            gt_bitmap.add(id);
        }
        assert_eq!(gt_bitmap.count(), 11, "Should have 11 GT IDs");

        // Use unset_only() with random seed
        let mut deleted_ids: HashSet<u64> = HashSet::new();
        let mut iter = gt_bitmap.iter().unset_only().seed(42).continue_write();
        while let Some((id, _)) = iter.next() {
            if id >= max_id {
                break;
            }
            deleted_ids.insert(id);
        }

        // Should have deleted 89 IDs (100 - 11 GT)
        assert_eq!(deleted_ids.len(), 89, "Should have 89 deleteable IDs, got {}", deleted_ids.len());

        // Verify NO GT IDs are in the deleted set
        for &gt_id in &gt_ids {
            assert!(!deleted_ids.contains(&gt_id), "GT ID {} should not be deleted", gt_id);
        }
    }

    /// Test exhaustion - iteration should stop when all deleteable IDs are claimed
    #[test]
    fn test_gt_bitmap_exhaustion() {
        use keyspace_tracker::{PrefixTracker, TrackerConfig};

        let max_id = 10u64;
        let config = TrackerConfig::simple("test:")
            .with_max_id(max_id)
            .with_initial_capacity(max_id as usize);
        let gt_bitmap = PrefixTracker::new(config);

        // Mark half as GT: 0, 1, 2, 3, 4 (5 protected)
        for id in 0..5 {
            gt_bitmap.add(id);
        }

        // Create iterator
        let mut iter = gt_bitmap.iter().unset_only().continue_write();

        // Should get exactly 5 IDs: 5, 6, 7, 8, 9
        for expected_id in 5..10 {
            let result = iter.next();
            assert!(result.is_some(), "Expected ID {}", expected_id);
            let (id, _) = result.unwrap();
            assert_eq!(id, expected_id, "Expected ID {}, got {}", expected_id, id);
        }

        // Next call should return None (exhausted)
        assert!(iter.next().is_none(), "Should be exhausted");
    }

    /// Test that continue_write preserves cursor across multiple claim calls
    #[test]
    fn test_gt_bitmap_continue_write_cursor() {
        use keyspace_tracker::{PrefixTracker, TrackerConfig};
        use std::collections::HashSet;

        let max_id = 100u64;
        let config = TrackerConfig::simple("test:")
            .with_max_id(max_id)
            .with_initial_capacity(max_id as usize);
        let gt_bitmap = Arc::new(PrefixTracker::new(config));

        // Mark IDs 0, 10, 20, 30, ... as GT
        for id in (0..100).step_by(10) {
            gt_bitmap.add(id);
        }

        // Simulate multiple claim_next_id calls (like DeleteContext does)
        let mut claimed: Vec<u64> = Vec::new();

        // Each call to iter().unset_only().continue_write().next() should advance the cursor
        for _ in 0..50 {
            let iter = gt_bitmap.iter().unset_only();
            if let Some((id, _)) = iter.continue_write().next() {
                claimed.push(id);
            }
        }

        // Should have claimed 50 unique IDs (none should be GT)
        assert_eq!(claimed.len(), 50, "Should have claimed 50 IDs");

        // Check uniqueness
        let unique: HashSet<u64> = claimed.iter().copied().collect();
        assert_eq!(unique.len(), 50, "All claimed IDs should be unique");

        // No claimed ID should be a GT ID (multiple of 10)
        for &id in &claimed {
            assert!(id % 10 != 0, "Claimed ID {} should not be a GT ID (multiple of 10)", id);
        }
    }

    /// Test concurrent claiming doesn't produce duplicates
    #[test]
    fn test_gt_bitmap_concurrent_claiming() {
        use keyspace_tracker::{PrefixTracker, TrackerConfig};
        use std::sync::Arc;
        use std::thread;
        use std::collections::HashSet;

        let max_id = 1000u64;
        let config = TrackerConfig::simple("test:")
            .with_max_id(max_id)
            .with_initial_capacity(max_id as usize);
        let gt_bitmap = Arc::new(PrefixTracker::new(config));

        // Mark 100 GT IDs (0-99)
        for id in 0..100 {
            gt_bitmap.add(id);
        }

        let claimed = Arc::new(std::sync::Mutex::new(HashSet::new()));

        // Spawn 4 threads, each claiming 200 IDs
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let bitmap = Arc::clone(&gt_bitmap);
                let claimed = Arc::clone(&claimed);

                thread::spawn(move || {
                    for _ in 0..200 {
                        let iter = bitmap.iter().unset_only();
                        if let Some((id, _)) = iter.continue_write().next() {
                            let mut guard = claimed.lock().unwrap();
                            assert!(guard.insert(id), "Duplicate ID claimed: {}", id);
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let claimed = claimed.lock().unwrap();
        // Should have claimed 800 unique IDs (4 threads * 200 each)
        assert_eq!(claimed.len(), 800, "Expected 800 unique IDs, got {}", claimed.len());

        // None should be GT IDs
        for &id in claimed.iter() {
            assert!(id >= 100, "Claimed ID {} should not be in GT range [0, 100)", id);
        }
    }

    /// Test Zipfian distribution with GT protection
    #[test]
    fn test_gt_bitmap_zipfian() {
        use keyspace_tracker::{AccessDistribution, PrefixTracker, TrackerConfig};
        use std::collections::HashSet;

        let max_id = 100u64;
        let config = TrackerConfig::simple("test:")
            .with_max_id(max_id)
            .with_initial_capacity(max_id as usize);
        let gt_bitmap = PrefixTracker::new(config);

        // Mark first 10 IDs as GT (hot keys in Zipfian)
        for id in 0..10 {
            gt_bitmap.add(id);
        }

        // Use Zipfian distribution
        let mut claimed: HashSet<u64> = HashSet::new();
        let mut iter = gt_bitmap
            .iter()
            .unset_only()
            .distribution(AccessDistribution::Zipfian { skew: 1.0 })
            .seed(42)
            .continue_write();
        while let Some((id, _)) = iter.next() {
            if id >= max_id {
                break;
            }
            claimed.insert(id);
        }

        // Should have claimed 90 IDs (100 - 10 GT)
        assert_eq!(claimed.len(), 90, "Should have 90 deleteable IDs, got {}", claimed.len());

        // No GT IDs should be claimed
        for id in 0..10 {
            assert!(!claimed.contains(&id), "GT ID {} should not be claimed", id);
        }
    }
}
