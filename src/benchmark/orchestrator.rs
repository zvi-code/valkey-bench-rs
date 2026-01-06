//! Benchmark orchestrator
//!
//! Coordinates worker threads, collects results, and manages the benchmark lifecycle.

use std::path::Path;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::info;

use super::counters::GlobalCounters;
use super::event_worker::{EventWorker, EventWorkerResult, RecallStats, WeightedTemplates};
use crate::client::{ConnectionFactory, ControlPlane, ControlPlaneExt};
use crate::cluster::{
    create_backend, AutoDetectBackend, ClusterBackend, ClusterTopology, TopologyManager,
};
use crate::config::BenchmarkConfig;
use crate::dataset::DatasetContext;
use crate::keyspace::{
    build_vector_id_mappings, ClusterScanConfig, ProtectedIds, VectorExistenceMap,
};
use crate::metrics::info_fields::{default_info_fields, default_search_info_fields, InfoFieldType};
use crate::metrics::reporter::{BenchmarkResults, OutputFormat};
use crate::metrics::snapshot::{compare_snapshots, print_per_node_matrix, ClusterSnapshot, SnapshotBuilder};
use crate::metrics::{BackfillWaitConfig, EngineType, MetricsCollector, MetricsReporter, NodeMetrics};
use crate::utils::Result;
use crate::workload::{
    create_index, create_template, create_workload_context_with_iteration,
    create_workload_context_with_shared_tracker, drop_index, get_index_info,
    index_exists, IterationStrategy, NumericFieldSet, ParallelWorkload, SimpleContext, Workload,
    WorkloadContext, WorkloadType,
};

/// Keyspace hit/miss statistics from INFO stats
#[derive(Debug, Clone, Default)]
pub struct KeyspaceStats {
    /// Total keyspace hits during the benchmark
    pub hits: u64,
    /// Total keyspace misses during the benchmark
    pub misses: u64,
}

impl KeyspaceStats {
    /// Calculate hit rate as a percentage (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Check if any hits or misses were recorded
    pub fn has_data(&self) -> bool {
        self.hits > 0 || self.misses > 0
    }
}

/// Base latency measurement result
///
/// Measures network round-trip time using single-client, no-pipeline operations.
/// This helps normalize benchmark results across different network conditions.
#[derive(Debug, Clone)]
pub struct BaseLatency {
    /// PING latency in microseconds
    pub ping_avg_us: f64,
    pub ping_p99_us: u64,
    /// GET miss latency in microseconds (GET on non-existent key)
    pub get_miss_avg_us: f64,
    pub get_miss_p99_us: u64,
    /// Number of samples used
    pub samples: u32,
}

impl BaseLatency {
    /// Format as compact string for banner
    pub fn format(&self) -> String {
        format!(
            "PING avg={:.2}ms p99={:.2}ms | GET-miss avg={:.2}ms p99={:.2}ms",
            self.ping_avg_us / 1000.0,
            self.ping_p99_us as f64 / 1000.0,
            self.get_miss_avg_us / 1000.0,
            self.get_miss_p99_us as f64 / 1000.0
        )
    }
}

/// Benchmark result summary
pub struct BenchmarkResult {
    /// Test name
    pub test_name: String,
    /// Total requests completed
    pub total_requests: u64,
    /// Total duration
    pub duration: Duration,
    /// Throughput (requests per second)
    pub throughput: f64,
    /// Merged latency histogram
    pub histogram: Histogram<u64>,
    /// Merged recall statistics
    pub recall_stats: RecallStats,
    /// Total errors
    pub error_count: u64,
    /// Per-node metrics snapshots
    pub node_metrics: Vec<crate::metrics::node_metrics::NodeMetricsSnapshot>,
    /// Keyspace hit/miss statistics
    pub keyspace_stats: KeyspaceStats,
}

impl BenchmarkResult {
    /// Get percentile latency in microseconds
    pub fn percentile_us(&self, p: f64) -> u64 {
        self.histogram.value_at_percentile(p)
    }

    /// Get percentile latency in milliseconds
    pub fn percentile_ms(&self, p: f64) -> f64 {
        self.percentile_us(p) as f64 / 1000.0
    }

    /// Print summary (compact format)
    pub fn print_summary(&self) {
        println!("\n=== {} ===", self.test_name);
        println!(
            "Throughput: {} req/s | Requests: {} | Duration: {:.2}s{}",
            format_throughput(self.throughput),
            format_count(self.total_requests),
            self.duration.as_secs_f64(),
            if self.error_count > 0 {
                format!(" | Errors: {}", self.error_count)
            } else {
                String::new()
            }
        );
        println!(
            "Latency (ms): avg={:.2} p50={:.2} p95={:.2} p99={:.2} p99.9={:.2} max={:.2}",
            self.histogram.mean() / 1000.0,
            self.percentile_ms(50.0),
            self.percentile_ms(95.0),
            self.percentile_ms(99.0),
            self.percentile_ms(99.9),
            self.histogram.max() as f64 / 1000.0
        );

        // Show recall only if computed (vector search with ground truth)
        if self.recall_stats.total_queries > 0 {
            println!(
                "Recall: avg={:.4} min={:.4} max={:.4} | perfect={} zero={}",
                self.recall_stats.average(),
                self.recall_stats.min_recall,
                self.recall_stats.max_recall,
                self.recall_stats.perfect_count,
                self.recall_stats.zero_count
            );
        }

        // Show keyspace stats only if there was activity
        if self.keyspace_stats.has_data() {
            println!(
                "Keyspace: hits={} misses={} hit-rate={:.1}%",
                format_count(self.keyspace_stats.hits),
                format_count(self.keyspace_stats.misses),
                self.keyspace_stats.hit_rate() * 100.0
            );
        }
    }
}

/// Benchmark orchestrator
pub struct Orchestrator {
    config: Arc<BenchmarkConfig>,
    connection_factory: ConnectionFactory,
    dataset: Option<Arc<DatasetContext>>,
    /// Discovered cluster topology (if cluster mode enabled)
    cluster_topology: Option<ClusterTopology>,
    /// Shared topology manager for dynamic cluster updates
    topology_manager: Option<Arc<TopologyManager>>,
    /// Vector existence map for tracking which vectors exist (for vec-query with existing data)
    existence_map: Option<Arc<VectorExistenceMap>>,
    /// Protected vector IDs (ground truth) for deletion benchmarks
    protected_ids: Option<Arc<ProtectedIds>>,
    /// Backend abstraction for engine-specific behavior (ElastiCache, MemoryDB, Valkey OSS)
    backend: Arc<dyn ClusterBackend>,
}

impl Orchestrator {
    /// Create new orchestrator
    pub fn new(config: BenchmarkConfig) -> Result<Self> {
        let connection_factory = ConnectionFactory {
            connect_timeout: Duration::from_millis(config.connect_timeout_ms),
            read_timeout: Duration::from_millis(config.request_timeout_ms),
            write_timeout: Duration::from_millis(config.request_timeout_ms),
            tls_config: config.tls.clone(),
            auth_password: config.auth.as_ref().map(|a| a.password.clone()),
            auth_username: config.auth.as_ref().and_then(|a| a.username.clone()),
            dbnum: config.dbnum,
        };

        // Discover cluster topology and detect backend engine type
        let (cluster_topology, backend) =
            Self::discover_cluster_and_backend(&config, &connection_factory)?;

        // Create topology manager for dynamic updates (cluster mode only)
        let topology_manager = cluster_topology.as_ref().map(|topo| {
            let seed_addresses: Vec<(String, u16)> = config
                .addresses
                .iter()
                .map(|a| (a.host.clone(), a.port))
                .collect();

            Arc::new(TopologyManager::new(
                topo.clone(),
                connection_factory.clone(),
                seed_addresses,
            ))
        });

        Ok(Self {
            config: Arc::new(config),
            connection_factory,
            dataset: None,
            cluster_topology,
            topology_manager,
            existence_map: None,
            protected_ids: None,
            backend,
        })
    }

    /// Create orchestrator with an existing cluster topology and backend (skips discovery)
    ///
    /// This is useful for optimization iterations where we don't want to
    /// rediscover the cluster topology on each iteration (saves connections).
    pub fn with_topology(
        config: BenchmarkConfig,
        topology: Option<ClusterTopology>,
        backend: Arc<dyn ClusterBackend>,
    ) -> Result<Self> {
        let connection_factory = ConnectionFactory {
            connect_timeout: Duration::from_millis(config.connect_timeout_ms),
            read_timeout: Duration::from_millis(config.request_timeout_ms),
            write_timeout: Duration::from_millis(config.request_timeout_ms),
            tls_config: config.tls.clone(),
            auth_password: config.auth.as_ref().map(|a| a.password.clone()),
            auth_username: config.auth.as_ref().and_then(|a| a.username.clone()),
            dbnum: config.dbnum,
        };

        // Create topology manager for dynamic updates (cluster mode only)
        let topology_manager = topology.as_ref().map(|topo| {
            let seed_addresses: Vec<(String, u16)> = config
                .addresses
                .iter()
                .map(|a| (a.host.clone(), a.port))
                .collect();

            Arc::new(TopologyManager::new(
                topo.clone(),
                connection_factory.clone(),
                seed_addresses,
            ))
        });

        Ok(Self {
            config: Arc::new(config),
            connection_factory,
            dataset: None,
            cluster_topology: topology,
            topology_manager,
            existence_map: None,
            protected_ids: None,
            backend,
        })
    }

    /// Get the cluster topology (for sharing across orchestrators)
    pub fn cluster_topology(&self) -> Option<&ClusterTopology> {
        self.cluster_topology.as_ref()
    }

    /// Set an existing existence map (for sharing across optimization iterations)
    pub fn set_existence_map(&mut self, existence_map: Arc<VectorExistenceMap>) {
        self.existence_map = Some(existence_map);
    }

    /// Build protected vector IDs from dataset ground truth
    ///
    /// This extracts all vector IDs that appear in the ground truth neighbor lists.
    /// These IDs will be skipped during vec-delete to ensure valid recall computation.
    pub fn build_protected_ids(&mut self) -> Result<()> {
        let dataset = self.dataset.as_ref().ok_or_else(|| {
            crate::utils::BenchmarkError::Config("Dataset required for protected IDs".to_string())
        })?;

        let ground_truth_ids = dataset.get_ground_truth_vector_ids();
        let protected_count = ground_truth_ids.len();
        let protected = ProtectedIds::new(ground_truth_ids, dataset.num_vectors());

        if !self.config.quiet {
            println!(
                "[PROTECTED-IDS] {} vectors protected (ground truth neighbors)",
                protected_count
            );
            println!(
                "[PROTECTED-IDS] {} vectors available for deletion",
                protected.deleteable_count()
            );
        }

        self.protected_ids = Some(Arc::new(protected));
        Ok(())
    }

    /// Get the protected vector IDs (for sharing across orchestrators)
    pub fn protected_ids(&self) -> Option<Arc<ProtectedIds>> {
        self.protected_ids.clone()
    }

    /// Set protected vector IDs (for sharing across optimization iterations)
    pub fn set_protected_ids(&mut self, protected: Arc<ProtectedIds>) {
        self.protected_ids = Some(protected);
    }

    /// Get the backend for engine-specific behavior
    pub fn backend(&self) -> &Arc<dyn ClusterBackend> {
        &self.backend
    }

    /// Get effective requests count based on config and dataset
    ///
    /// For vec-load workloads, defaults to dataset size if no explicit count was provided.
    /// For other workloads, defaults to 100,000.
    fn effective_requests(&self) -> u64 {
        let dataset_size = self.dataset.as_ref().map(|ds| ds.num_vectors());
        self.config.effective_requests(dataset_size)
    }

    /// Get the detected engine type
    pub fn engine_type(&self) -> EngineType {
        self.backend.engine_type()
    }

    /// Discover cluster topology and detect backend engine type
    fn discover_cluster_and_backend(
        config: &BenchmarkConfig,
        factory: &ConnectionFactory,
    ) -> Result<(Option<ClusterTopology>, Arc<dyn ClusterBackend>)> {
        use crate::cluster::UnknownBackend;

        if config.addresses.is_empty() {
            return Ok((None, Arc::new(UnknownBackend)));
        }

        let addr = &config.addresses[0];

        // Create a temporary connection to discover topology and detect engine
        let mut conn = factory.create(&addr.host, addr.port)?;

        // Detect engine type first
        let auto_detect = AutoDetectBackend::new();
        let engine_type = auto_detect.detect(&mut conn).unwrap_or(EngineType::Unknown);
        let backend: Arc<dyn ClusterBackend> = Arc::from(create_backend(engine_type));

        if !config.quiet {
            info!("Detected engine type: {}", backend.name());
        }

        // Try CLUSTER NODES to see if this is a cluster
        match conn.cluster_nodes() {
            Ok(nodes_response) => {
                let topology = ClusterTopology::from_cluster_nodes(&nodes_response)
                    .map_err(|e| crate::utils::BenchmarkError::Connection(
                        crate::utils::ConnectionError::ConnectFailed {
                            host: addr.host.clone(),
                            port: addr.port,
                            source: std::io::Error::other(e),
                        }
                    ))?;

                if !config.quiet {
                    info!(
                        "Discovered cluster with {} primaries, {} total nodes",
                        topology.num_primaries(),
                        topology.num_nodes()
                    );

                    // Log primary addresses
                    for primary in topology.primaries() {
                        info!("  Primary: {}:{} (slots: {})", primary.host, primary.port, primary.slots.len());
                    }
                }

                Ok((Some(topology), backend))
            }
            Err(e) => {
                // Not a cluster or cluster commands disabled
                if config.cluster_mode {
                    // User explicitly requested cluster mode but it's not available
                    return Err(crate::utils::BenchmarkError::Connection(
                        crate::utils::ConnectionError::ConnectFailed {
                            host: addr.host.clone(),
                            port: addr.port,
                            source: std::io::Error::other(
                                format!("Cluster mode requested but CLUSTER NODES failed: {}", e),
                            ),
                        }
                    ));
                }
                if !config.quiet {
                    info!("Running in standalone mode (CLUSTER NODES not available)");
                }
                Ok((None, backend))
            }
        }
    }

    /// Get addresses for benchmark clients (primaries for cluster, seed for standalone)
    fn get_benchmark_addresses(&self) -> Vec<(String, u16)> {
        if let Some(ref topology) = self.cluster_topology {
            // Use selected nodes based on read-from-replica strategy
            topology
                .select_nodes(self.config.read_from_replica)
                .iter()
                .map(|(_, node)| (node.host.clone(), node.port))
                .collect()
        } else {
            // Use configured addresses for standalone
            self.config
                .addresses
                .iter()
                .map(|a| (a.host.clone(), a.port))
                .collect()
        }
    }

    /// Set dataset (for vector search)
    pub fn set_dataset(&mut self, dataset: DatasetContext) {
        self.dataset = Some(Arc::new(dataset));
    }

    /// Set dataset from Arc (for sharing across optimization iterations)
    pub fn set_dataset_arc(&mut self, dataset: Arc<DatasetContext>) {
        self.dataset = Some(dataset);
    }

    /// Measure base network latency using single-client PING and GET miss
    ///
    /// This provides a baseline for network round-trip time that can be used
    /// to normalize benchmark results across different network conditions.
    /// Uses 1000 samples by default for stable measurements.
    pub fn measure_base_latency(&self) -> Result<BaseLatency> {
        self.measure_base_latency_samples(1000)
    }

    /// Measure base latency with custom sample count
    pub fn measure_base_latency_samples(&self, samples: u32) -> Result<BaseLatency> {
        use crate::utils::RespEncoder;

        let addr = &self.config.addresses[0];
        let mut conn = self.connection_factory.create(&addr.host, addr.port)?;

        // Prepare PING command
        let mut ping_encoder = RespEncoder::with_capacity(32);
        ping_encoder.encode_command_str(&["PING"]);

        // Prepare GET command with unlikely-to-exist key
        let miss_key = "__vsb_base_latency_miss_probe_12345678__";
        let mut get_encoder = RespEncoder::with_capacity(64);
        get_encoder.encode_command_str(&["GET", miss_key]);

        // Create histograms
        let mut ping_hist =
            Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3).expect("histogram");
        let mut get_hist =
            Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3).expect("histogram");

        // Warmup: 10 iterations to prime the connection
        for _ in 0..10 {
            let _ = conn.execute_encoded(&ping_encoder);
            let _ = conn.execute_encoded(&get_encoder);
        }

        // Measure PING latency
        for _ in 0..samples {
            let start = Instant::now();
            let _ = conn.execute_encoded(&ping_encoder);
            let elapsed = start.elapsed().as_micros() as u64;
            ping_hist.record(elapsed).ok();
        }

        // Measure GET miss latency
        for _ in 0..samples {
            let start = Instant::now();
            let _ = conn.execute_encoded(&get_encoder);
            let elapsed = start.elapsed().as_micros() as u64;
            get_hist.record(elapsed).ok();
        }

        Ok(BaseLatency {
            ping_avg_us: ping_hist.mean(),
            ping_p99_us: ping_hist.value_at_percentile(99.0),
            get_miss_avg_us: get_hist.mean(),
            get_miss_p99_us: get_hist.value_at_percentile(99.0),
            samples,
        })
    }

    /// Capture a cluster snapshot from all nodes using specified fields
    ///
    /// This is a generic helper that collects full INFO output from all cluster nodes
    /// (or standalone node) and builds a snapshot using the provided field definitions.
    /// The field definitions support both exact and prefix-based matching via
    /// `InfoFieldType::matches()`, so only relevant fields are extracted regardless
    /// of which INFO section they come from.
    ///
    /// # Arguments
    /// * `label` - Label for the snapshot (e.g., "before", "after")
    /// * `fields` - Field definitions that determine what to extract and how to aggregate
    fn capture_snapshot(&self, label: &str, fields: Vec<InfoFieldType>) -> ClusterSnapshot {
        let addresses = self.get_benchmark_addresses();
        let mut builder = SnapshotBuilder::new(label, fields);

        for (host, port) in addresses {
            let node_id = format!("{}:{}", host, port);
            // Determine if this is a primary node
            let is_primary = if let Some(ref topology) = self.cluster_topology {
                topology.primaries().any(|p| p.host == host && p.port == port)
            } else {
                true // Standalone mode - treat as primary
            };

            match self.connection_factory.create(&host, port) {
                Ok(mut conn) => {
                    // Fetch full INFO output including commandstats - SnapshotBuilder filters to relevant fields
                    match conn.info("all") {
                        Ok(info_str) => {
                            builder.add_node(&node_id, is_primary, &info_str);
                        }
                        Err(_) => {
                            // Silently ignore errors - stats capture is best-effort
                        }
                    }
                }
                Err(_) => {
                    // Silently ignore connection errors
                }
            }
        }

        builder.build()
    }

    /// Capture a cluster snapshot of INFO stats from all nodes
    ///
    /// Uses the snapshot infrastructure to collect and aggregate metrics
    /// across all cluster nodes (or standalone node).
    fn capture_info_snapshot(&self, label: &str) -> ClusterSnapshot {
        self.capture_snapshot(label, default_info_fields())
    }

    /// Capture a cluster snapshot of INFO SEARCH stats from all nodes
    ///
    /// Uses the snapshot infrastructure to collect and aggregate search metrics
    /// across all cluster nodes (or standalone node).
    fn capture_search_snapshot(&self, label: &str) -> ClusterSnapshot {
        self.capture_snapshot(label, default_search_info_fields())
    }

    /// Extract keyspace stats from a snapshot
    fn keyspace_stats_from_snapshot(snapshot: &ClusterSnapshot) -> KeyspaceStats {
        let hits = snapshot.get_value("keyspace_hits").unwrap_or(0) as u64;
        let misses = snapshot.get_value("keyspace_misses").unwrap_or(0) as u64;
        KeyspaceStats { hits, misses }
    }

    /// Build vector existence map by scanning cluster for existing vector keys
    ///
    /// This is used for vec-query with existing data to:
    /// 1. Know which vectors exist in the cluster
    /// 2. Validate recall only against vectors that actually exist
    /// 3. Support partial prefill by skipping existing vectors
    pub fn build_existence_map(&mut self) -> Result<()> {
        let search_config = match &self.config.search_config {
            Some(cfg) => cfg,
            None => {
                if !self.config.quiet {
                    info!("No search config - skipping existence map");
                }
                return Ok(());
            }
        };

        // Get capacity from dataset if available
        let capacity = self
            .dataset
            .as_ref()
            .map(|ds| ds.num_vectors())
            .unwrap_or(1_000_000);

        let is_cluster = self.cluster_topology.is_some();

        if !self.config.quiet {
            info!(
                "Building existence map for prefix '{}' (capacity: {}, cluster: {})",
                search_config.prefix, capacity, is_cluster
            );
        }

        let existence_map = Arc::new(VectorExistenceMap::new(
            &search_config.prefix,
            capacity,
            is_cluster,
        ));

        // Only scan if in cluster mode with topology
        if let Some(ref topology) = self.cluster_topology {
            let nodes: Vec<_> = topology.nodes.clone();

            let scan_config = ClusterScanConfig {
                pattern: format!("{}*", search_config.prefix),
                batch_size: 1000,
                timeout: Duration::from_secs(5),
                show_progress: !self.config.quiet,
            };

            match build_vector_id_mappings(&existence_map, &nodes, &scan_config) {
                Ok(results) => {
                    if !self.config.quiet {
                        info!(
                            "Existence map built: {} vectors mapped from {} keys",
                            existence_map.count(),
                            results.total_keys
                        );
                    }
                }
                Err(e) => {
                    return Err(crate::utils::BenchmarkError::Config(format!(
                        "Failed to build existence map: {}",
                        e
                    )));
                }
            }
        } else if !self.config.quiet {
            info!("Standalone mode - existence map will track local vectors");
        }

        self.existence_map = Some(existence_map);
        Ok(())
    }

    /// Get existence map
    pub fn existence_map(&self) -> Option<Arc<VectorExistenceMap>> {
        self.existence_map.clone()
    }

    /// Report progress during benchmark
    fn report_progress(counters: &GlobalCounters, total: u64, duration_secs: Option<u64>) {
        if let Some(duration) = duration_secs {
            // Duration-based progress (show elapsed time and throughput)
            let pb = ProgressBar::new(duration);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}s/{len}s | {msg}",
                    )
                    .unwrap()
                    .progress_chars("#>-"),
            );

            let start = std::time::Instant::now();
            let mut last_finished = 0u64;
            let mut last_time = start;

            while !counters.is_shutdown() && !counters.is_duration_exceeded() {
                let elapsed = start.elapsed().as_secs();
                let (finished, _) = counters.progress();
                pb.set_position(elapsed);

                // Calculate current throughput
                let now = std::time::Instant::now();
                let interval = now.duration_since(last_time).as_secs_f64();
                if interval >= 1.0 {
                    let throughput = (finished - last_finished) as f64 / interval;
                    pb.set_message(format!("{:.0} ops/s, total: {}", throughput, finished));
                    last_finished = finished;
                    last_time = now;
                }

                thread::sleep(Duration::from_millis(100));
            }

            let (finished, _) = counters.progress();
            pb.finish_with_message(format!("Complete - {} total ops", finished));
        } else {
            // Request count based progress
            let pb = ProgressBar::new(total);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({msg})",
                    )
                    .unwrap()
                    .progress_chars("#>-"),
            );

            let start = std::time::Instant::now();
            let mut last_finished = 0u64;
            let mut last_time = start;

            while !counters.is_shutdown() {
                let (finished, _) = counters.progress();
                pb.set_position(finished);

                // Calculate current throughput without decimals
                let now = std::time::Instant::now();
                let interval = now.duration_since(last_time).as_secs_f64();
                if interval >= 0.5 {
                    let throughput = (finished - last_finished) as f64 / interval;
                    pb.set_message(format!("{}/s", format_count(throughput as u64)));
                    last_finished = finished;
                    last_time = now;
                }

                if finished >= total {
                    break;
                }

                thread::sleep(Duration::from_millis(100));
            }

            pb.finish_with_message("done");
        }
    }

    /// Merge results from event-driven workers
    fn merge_event_results(
        &self,
        test_name: &str,
        results: Vec<EventWorkerResult>,
        duration: Duration,
    ) -> Result<BenchmarkResult> {
        let mut merged_histogram =
            Histogram::new_with_bounds(1, 3_600_000_000, 3).expect("Failed to create histogram");

        let mut merged_recall = RecallStats::new();
        let mut total_requests = 0u64;
        let mut error_count = 0u64;

        for result in results {
            merged_histogram.add(&result.histogram).ok();
            merged_recall.merge(&result.recall_stats);
            total_requests += result.requests_processed;
            error_count += result.error_count;
        }

        let throughput = total_requests as f64 / duration.as_secs_f64();

        Ok(BenchmarkResult {
            test_name: test_name.to_string(),
            total_requests,
            duration,
            throughput,
            histogram: merged_histogram,
            recall_stats: merged_recall,
            error_count,
            node_metrics: Vec::new(),
            keyspace_stats: KeyspaceStats::default(),
        })
    }

    /// Run a single benchmark test using event-driven I/O (like C's ae)
    pub fn run_test_event(&self, workload: WorkloadType) -> Result<BenchmarkResult> {
        // For vec-load with partial prefill, calculate actual vectors to load
        let _effective_requests = if matches!(workload, WorkloadType::VecLoad) {
            if let Some(ref existence_map) = self.existence_map {
                existence_map.reset_unmapped_counter();

                // Calculate vectors to actually load (skip existing)
                let already_mapped = existence_map.count();
                let dataset_size = self.dataset.as_ref().map(|ds| ds.num_vectors()).unwrap_or(0);
                let requested = self.effective_requests().min(dataset_size);
                let to_load = requested.saturating_sub(already_mapped);

                if already_mapped > 0 && !self.config.quiet {
                    info!(
                        "Partial prefill: {} vectors already exist, {} to load (of {} requested)",
                        already_mapped, to_load, requested
                    );
                }

                if to_load == 0 {
                    if !self.config.quiet {
                        info!("All {} vectors already loaded, nothing to do", already_mapped);
                    }
                    // Return early with empty result
                    return Ok(BenchmarkResult {
                        test_name: workload.to_string(),
                        total_requests: 0,
                        duration: Duration::from_secs(0),
                        throughput: 0.0,
                        histogram: Histogram::new_with_bounds(1, 3_600_000_000, 3)
                            .expect("Failed to create histogram"),
                        recall_stats: RecallStats::new(),
                        error_count: 0,
                        node_metrics: Vec::new(),
                        keyspace_stats: KeyspaceStats::default(),
                    });
                }

                to_load
            } else {
                self.effective_requests()
            }
        } else {
            self.effective_requests()
        };

        // Create command template (use cluster mode if we have cluster topology)
        let cluster_mode = self.cluster_topology.is_some();
        let template = create_template(
            workload,
            &self.config.key_prefix,
            self.config.data_size,
            self.config.search_config.as_ref(),
            cluster_mode,
        );

        // Build command buffer for pipeline
        let command_buffer = template.build(self.config.pipeline as usize);

        // Create global counters (duration-based or iterator-driven)
        let counters = Arc::new(if let Some(duration) = self.config.duration_secs {
            GlobalCounters::with_duration(duration)
        } else {
            GlobalCounters::new() // Request count driven by iterator exhaustion
        });

        // Get addresses for workers
        let addresses = self.get_benchmark_addresses();
        if addresses.is_empty() {
            return Err(crate::utils::BenchmarkError::Config(
                "No available server addresses".to_string(),
            ));
        }

        // Calculate clients per thread per node
        // For slot-aware routing, we need at least 1 client per node per thread
        let clients_per_thread = self.config.clients_per_thread() as usize;
        if clients_per_thread == 0 {
            return Err(crate::utils::BenchmarkError::Config(
                format!(
                    "Not enough clients ({}) for threads ({}). Need at least 1 client per thread.",
                    self.config.clients, self.config.threads
                ),
            ));
        }

        // In cluster mode, total clients = clients_per_thread * num_nodes * threads
        let total_clients = clients_per_thread * addresses.len() * self.config.threads as usize;
        if self.cluster_topology.is_some() && total_clients != self.config.clients as usize && !self.config.quiet {
            eprintln!(
                "Note: Using {} total clients ({} per thread × {} nodes × {} threads)",
                total_clients, clients_per_thread, addresses.len(), self.config.threads
            );
        }

        // Capture snapshot before benchmark starts (for keyspace stats diff)
        let snapshot_before = self.capture_info_snapshot("before");
        let search_snapshot_before = self.capture_search_snapshot("before");

        // Spawn event-driven worker threads
        let mut handles: Vec<thread::JoinHandle<EventWorkerResult>> =
            Vec::with_capacity(self.config.threads as usize);

        let start_time = Instant::now();

        // Clone topology for thread sharing
        let topology = self.cluster_topology.clone();

        // Get k and key_prefix for workload context creation
        let k = self.config.search_config.as_ref().map(|sc| sc.k as usize).unwrap_or(10);
        let key_prefix = self.config.search_config.as_ref()
            .map(|sc| sc.prefix.as_str())
            .unwrap_or(&self.config.key_prefix);

        // Get tag distributions for tag field generation (passed to workload context)
        let tag_distributions = self.config.search_config.as_ref()
            .and_then(|sc| sc.tag_distributions.clone());

        // Get numeric field configurations
        let numeric_fields = self.config.search_config.as_ref()
            .map(|sc| sc.numeric_fields.clone())
            .unwrap_or_default();

        // For simple workloads (SET, GET, etc.), determine if we need shared tracker
        let total_workers = self.config.threads as usize;
        let is_write_workload = workload.is_write();
        let is_simple_workload = matches!(
            workload,
            WorkloadType::Set | WorkloadType::Get | WorkloadType::Incr | 
            WorkloadType::Ping | WorkloadType::Lpush | WorkloadType::Rpush | 
            WorkloadType::Lpop | WorkloadType::Rpop | WorkloadType::Sadd |
            WorkloadType::Spop | WorkloadType::Hset | WorkloadType::Zadd |
            WorkloadType::Zpopmin | WorkloadType::Lrange100 | WorkloadType::Lrange300 |
            WorkloadType::Lrange500 | WorkloadType::Lrange600 | WorkloadType::Mset
        );

        // Parse iteration strategy once
        let strategy = self.config.iteration.as_deref()
            .and_then(|s| IterationStrategy::parse(s).ok())
            .unwrap_or_else(|| {
                if self.config.sequential {
                    IterationStrategy::Sequential
                } else {
                    IterationStrategy::Random { seed: self.config.seed }
                }
            });

        // Calculate request limits for workers
        // For duration mode (None), workers iterate until time expires
        // For count mode, distribute requests across workers
        let total_requests = if self.config.duration_secs.is_some() {
            None // Duration mode - no request limit
        } else {
            Some(self.effective_requests())
        };

        // Create shared tracker for write workloads (all workers share the same atomic cursor)
        let shared_tracker = if is_simple_workload && is_write_workload {
            Some(SimpleContext::create_shared_tracker(self.config.keyspace_len))
        } else {
            None
        };

        for worker_id in 0..self.config.threads as usize {
            let config = Arc::clone(&self.config);
            let counters = Arc::clone(&counters);
            let buffer = command_buffer.clone();
            let addrs = addresses.clone();
            let topo = topology.clone();
            let worker_tag_dist = tag_distributions.clone();
            let worker_numeric_fields = numeric_fields.clone();

            // Create workload context based on workload type
            let workload_ctx: Box<dyn WorkloadContext + Send> = if is_simple_workload {
                if let Some(ref tracker) = shared_tracker {
                    // Write workload: share tracker across all workers for atomic cursor
                    // For writes, the shared tracker handles distribution across workers
                    // Each worker gets an equal share of the total request limit
                    if let Some(total) = total_requests {
                        let per_worker = total / total_workers as u64;
                        // Last worker gets any remainder
                        let worker_limit = if worker_id == total_workers - 1 {
                            per_worker + (total % total_workers as u64)
                        } else {
                            per_worker
                        };
                        Box::new(SimpleContext::with_shared_tracker_and_limit(
                            Arc::clone(tracker),
                            self.config.keyspace_len,
                            strategy.clone(),
                            worker_limit,
                        ))
                    } else {
                        create_workload_context_with_shared_tracker(
                            Arc::clone(tracker),
                            self.config.keyspace_len,
                            strategy.clone(),
                        )
                    }
                } else {
                    // Read workload: each worker gets a partition for cache locality
                    if let Some(total) = total_requests {
                        let per_worker = total / total_workers as u64;
                        let worker_limit = if worker_id == total_workers - 1 {
                            per_worker + (total % total_workers as u64)
                        } else {
                            per_worker
                        };
                        Box::new(SimpleContext::with_partition_and_limit(
                            self.config.keyspace_len,
                            strategy.clone(),
                            worker_id,
                            total_workers,
                            worker_limit,
                        ))
                    } else {
                        Box::new(SimpleContext::with_partition(
                            self.config.keyspace_len,
                            strategy.clone(),
                            worker_id,
                            total_workers,
                        ))
                    }
                }
            } else {
                // Vector and other workloads use the existing function
                create_workload_context_with_iteration(
                    workload,
                    self.dataset.clone(),
                    self.existence_map.clone(),
                    self.protected_ids.clone(),
                    worker_tag_dist,
                    worker_numeric_fields,
                    self.config.keyspace_len,
                    self.config.sequential,
                    self.config.seed.wrapping_add(worker_id as u64),
                    k,
                    key_prefix,
                    self.config.iteration.as_deref(),
                    total_requests, // Pass request limit to context
                )
            };

            let handle = thread::Builder::new()
                .name(format!("event-worker-{}", worker_id))
                .spawn(move || {
                    // Create event-driven worker with topology for cluster-aware routing
                    match EventWorker::new(
                        worker_id,
                        addrs,
                        clients_per_thread,
                        &config,
                        buffer,
                        topo.as_ref(),
                        workload_ctx,
                    ) {
                        Ok(worker) => worker.run(counters),
                        Err(e) => {
                            if !config.quiet {
                                eprintln!("Worker {}: Failed to create: {}", worker_id, e);
                            }
                            EventWorkerResult {
                                worker_id,
                                histogram: Histogram::new_with_bounds(1, 3_600_000_000, 3)
                                    .expect("Failed to create histogram"),
                                recall_stats: RecallStats::new(),
                                redirect_count: 0,
                                topology_refresh_count: 0,
                                error_count: 1,
                                requests_processed: 0,
                            }
                        }
                    }
                })
                .expect("Failed to spawn worker thread");

            handles.push(handle);
        }

        // Progress reporting (if not quiet)
        if !self.config.quiet {
            let counters_clone = Arc::clone(&counters);
            let total = self.effective_requests();
            let duration_secs = self.config.duration_secs;
            thread::spawn(move || {
                Self::report_progress(&counters_clone, total, duration_secs);
            });
        }

        // Wait for workers to complete
        let results: Vec<EventWorkerResult> = handles
            .into_iter()
            .map(|h| h.join().expect("Worker thread panicked"))
            .collect();

        let duration = start_time.elapsed();

        // Capture snapshots after benchmark completes
        let snapshot_after = self.capture_info_snapshot("after");
        let search_snapshot_after = self.capture_search_snapshot("after");

        // Signal shutdown to progress reporter
        counters.signal_shutdown();

        // Calculate keyspace stats delta from snapshots
        let stats_before = Self::keyspace_stats_from_snapshot(&snapshot_before);
        let stats_after = Self::keyspace_stats_from_snapshot(&snapshot_after);

        // Merge results and add keyspace stats
        let mut result = self.merge_event_results(workload.as_str(), results, duration)?;
        result.keyspace_stats = KeyspaceStats {
            hits: stats_after.hits.saturating_sub(stats_before.hits),
            misses: stats_after.misses.saturating_sub(stats_before.misses),
        };

        // Display per-node statistics matrix (if not quiet and cluster mode)
        if !self.config.quiet && self.cluster_topology.is_some() {
            let topo = self.cluster_topology.as_ref();
            
            // Display INFO stats diff (keyspace hits/misses, cmdstats)
            let info_diff = compare_snapshots(&snapshot_before, &snapshot_after, &default_info_fields());
            print_per_node_matrix(&info_diff, topo);

            // Display INFO SEARCH diff (search-specific metrics)
            let search_diff = compare_snapshots(&search_snapshot_before, &search_snapshot_after, &default_search_info_fields());
            print_per_node_matrix(&search_diff, topo);
        }

        Ok(result)
    }

    /// Run all configured tests (uses event-driven I/O by default)
    pub fn run_all(&self) -> Result<Vec<BenchmarkResult>> {
        let mut results = Vec::new();

        // Check if parallel workload is specified
        if let Some(ref parallel_spec) = self.config.parallel {
            let mut parallel_workload = ParallelWorkload::parse(parallel_spec)?;
            // Apply global defaults from config to each component
            parallel_workload.apply_defaults(
                &self.config.key_prefix,
                self.config.keyspace_len,
                self.config.data_size,
                self.config.search_config.as_ref(),
                self.config.schema_path.as_ref(),
                self.config.data_path.as_ref(),
            );
            if !self.config.quiet {
                println!("\nRunning parallel test: {}", parallel_workload.name());
            }
            let result = self.run_parallel_test(&parallel_workload)?;
            if !self.config.quiet {
                result.print_summary();
            }
            results.push(result);
            return Ok(results);
        }

        for test_name in &self.config.tests {
            let workload = WorkloadType::parse(test_name).ok_or_else(|| {
                crate::utils::BenchmarkError::Config(format!("Unknown test: {}", test_name))
            })?;

            if !self.config.quiet {
                println!("\nRunning test: {}", workload);
            }
            // Use event-driven I/O for maximum performance (like C's ae)
            let result = self.run_test_event(workload)?;
            if !self.config.quiet {
                result.print_summary();
            }
            results.push(result);
        }

        Ok(results)
    }

    /// Run a parallel workload with weighted traffic
    pub fn run_parallel_test(&self, parallel: &ParallelWorkload) -> Result<BenchmarkResult> {
        let cluster_mode = self.cluster_topology.is_some();

        // Create templates for each component workload using per-component configs
        let mut weighted_templates = Vec::new();
        for component in parallel.components() {
            let template = component.config.build_template(cluster_mode);
            let buffer = template.build(self.config.pipeline as usize);
            weighted_templates.push((buffer, component.weight));
        }

        let templates = WeightedTemplates::weighted(weighted_templates);

        // Create global counters (duration-based or iterator-driven)
        let counters = Arc::new(if let Some(duration) = self.config.duration_secs {
            GlobalCounters::with_duration(duration)
        } else {
            GlobalCounters::new() // Request count driven by iterator exhaustion
        });

        // Get addresses for workers
        let addresses = self.get_benchmark_addresses();
        if addresses.is_empty() {
            return Err(crate::utils::BenchmarkError::Config(
                "No available server addresses".to_string(),
            ));
        }

        let clients_per_thread = self.config.clients_per_thread() as usize;
        if clients_per_thread == 0 {
            return Err(crate::utils::BenchmarkError::Config(
                format!(
                    "Not enough clients ({}) for threads ({})",
                    self.config.clients, self.config.threads
                ),
            ));
        }

        // Capture snapshot before benchmark
        let snapshot_before = self.capture_info_snapshot("before");
        let search_snapshot_before = self.capture_search_snapshot("before");

        // Spawn worker threads
        let mut handles: Vec<thread::JoinHandle<EventWorkerResult>> =
            Vec::with_capacity(self.config.threads as usize);

        let start_time = Instant::now();
        let topology = self.cluster_topology.clone();
        let total_workers = self.config.threads as usize;
        let is_write_workload = parallel.is_write();

        for worker_id in 0..self.config.threads as usize {
            let config = Arc::clone(&self.config);
            let counters = Arc::clone(&counters);
            let worker_templates = templates.clone();
            let addrs = addresses.clone();
            let topo = topology.clone();

            // Determine iteration strategy
            let strategy = if self.config.sequential {
                IterationStrategy::Sequential
            } else {
                IterationStrategy::Random { seed: self.config.seed.wrapping_add(worker_id as u64) }
            };

            // Use partitioned iteration for read workloads (each worker gets disjoint range)
            // Use atomic cursor (continue_write) for write workloads (exactly-once claiming)
            let workload_ctx: Box<dyn WorkloadContext + Send> = if is_write_workload {
                // Write workloads: use atomic cursor for exactly-once claiming
                Box::new(SimpleContext::with_strategy(
                    self.config.keyspace_len,
                    strategy,
                ))
            } else {
                // Read workloads: use partitioned iteration for better locality
                Box::new(SimpleContext::with_partition(
                    self.config.keyspace_len,
                    strategy,
                    worker_id,
                    total_workers,
                ))
            };

            let handle = thread::Builder::new()
                .name(format!("event-worker-{}", worker_id))
                .spawn(move || {
                    match EventWorker::with_templates(
                        worker_id,
                        addrs,
                        clients_per_thread,
                        &config,
                        worker_templates,
                        topo.as_ref(),
                        workload_ctx,
                    ) {
                        Ok(worker) => worker.run(counters),
                        Err(e) => {
                            if !config.quiet {
                                eprintln!("Worker {}: Failed to create: {}", worker_id, e);
                            }
                            EventWorkerResult {
                                worker_id,
                                histogram: Histogram::new_with_bounds(1, 3_600_000_000, 3)
                                    .expect("Failed to create histogram"),
                                recall_stats: RecallStats::new(),
                                redirect_count: 0,
                                topology_refresh_count: 0,
                                error_count: 1,
                                requests_processed: 0,
                            }
                        }
                    }
                })
                .expect("Failed to spawn worker thread");

            handles.push(handle);
        }

        // Collect results
        let mut merged_histogram =
            Histogram::new_with_bounds(1, 3_600_000_000, 3).expect("Failed to create histogram");
        let mut merged_recall = RecallStats::new();
        let mut total_requests = 0u64;
        let mut total_errors = 0u64;

        for handle in handles {
            let result = handle.join().expect("Worker thread panicked");
            merged_histogram.add(&result.histogram).ok();
            merged_recall.merge(&result.recall_stats);
            total_requests += result.requests_processed;
            total_errors += result.error_count;
        }

        let elapsed = start_time.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();
        let qps = total_requests as f64 / elapsed_secs;

        // Capture snapshot after benchmark
        let snapshot_after = self.capture_info_snapshot("after");
        let search_snapshot_after = self.capture_search_snapshot("after");

        // Calculate keyspace stats delta
        let stats_before = Self::keyspace_stats_from_snapshot(&snapshot_before);
        let stats_after = Self::keyspace_stats_from_snapshot(&snapshot_after);

        // Build result
        let result = BenchmarkResult {
            test_name: parallel.name().to_string(),
            total_requests,
            duration: elapsed,
            throughput: qps,
            histogram: merged_histogram,
            recall_stats: merged_recall,
            error_count: total_errors,
            node_metrics: Vec::new(),
            keyspace_stats: KeyspaceStats {
                hits: stats_after.hits.saturating_sub(stats_before.hits),
                misses: stats_after.misses.saturating_sub(stats_before.misses),
            },
        };

        // Display per-node statistics matrix
        if !self.config.quiet && self.cluster_topology.is_some() {
            let topo = self.cluster_topology.as_ref();
            let info_diff = compare_snapshots(&snapshot_before, &snapshot_after, &default_info_fields());
            print_per_node_matrix(&info_diff, topo);
            let search_diff = compare_snapshots(&search_snapshot_before, &search_snapshot_after, &default_search_info_fields());
            print_per_node_matrix(&search_diff, topo);
        }

        Ok(result)
    }

    /// Create vector search index using FT.CREATE
    ///
    /// # Arguments
    /// * `overwrite` - If true, drop existing index first
    pub fn create_search_index(&self, overwrite: bool) -> Result<()> {
        let search_config = self.config.search_config.as_ref().ok_or_else(|| {
            crate::utils::BenchmarkError::Config(
                "Search config required for index creation".to_string(),
            )
        })?;

        let addresses = self.get_benchmark_addresses();
        if addresses.is_empty() {
            return Err(crate::utils::BenchmarkError::Config(
                "No available server addresses".to_string(),
            ));
        }

        // Connect to first available node
        let (host, port) = &addresses[0];
        let mut conn = self.connection_factory.create(host, *port)?;

        info!(
            "Creating index '{}' with algorithm {:?}, dim={}, metric={:?}",
            search_config.index_name,
            search_config.algorithm,
            search_config.dim,
            search_config.distance_metric
        );

        create_index(&mut conn, search_config, overwrite).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!("Failed to create index: {}", e))
        })?;

        info!("Index '{}' created successfully", search_config.index_name);
        Ok(())
    }

    /// Drop vector search index
    pub fn drop_search_index(&self) -> Result<()> {
        let search_config = self.config.search_config.as_ref().ok_or_else(|| {
            crate::utils::BenchmarkError::Config(
                "Search config required for index operations".to_string(),
            )
        })?;

        let addresses = self.get_benchmark_addresses();
        if addresses.is_empty() {
            return Err(crate::utils::BenchmarkError::Config(
                "No available server addresses".to_string(),
            ));
        }

        let (host, port) = &addresses[0];
        let mut conn = self.connection_factory.create(host, *port)?;

        info!("Dropping index '{}'", search_config.index_name);

        drop_index(&mut conn, &search_config.index_name).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!("Failed to drop index: {}", e))
        })?;

        info!("Index '{}' dropped", search_config.index_name);
        Ok(())
    }

    /// Check if vector search index exists
    pub fn search_index_exists(&self) -> bool {
        let search_config = match self.config.search_config.as_ref() {
            Some(c) => c,
            None => return false,
        };

        let addresses = self.get_benchmark_addresses();
        if addresses.is_empty() {
            return false;
        }

        let (host, port) = &addresses[0];
        let mut conn = match self.connection_factory.create(host, *port) {
            Ok(c) => c,
            Err(_) => return false,
        };

        index_exists(&mut conn, &search_config.index_name)
    }

    /// Wait for index background indexing to complete
    pub fn wait_for_search_indexing(&self, timeout_secs: u64) -> Result<()> {
        use crate::metrics::backfill::wait_for_backfill;

        let search_config = self.config.search_config.as_ref().ok_or_else(|| {
            crate::utils::BenchmarkError::Config(
                "Search config required for index operations".to_string(),
            )
        })?;

        // Detect engine type from first connection
        let addresses = self.get_benchmark_addresses();
        if addresses.is_empty() {
            return Err(crate::utils::BenchmarkError::Config(
                "No available server addresses".to_string(),
            ));
        }

        let (host, port) = &addresses[0];
        let mut conn = self.connection_factory.create(host, *port)?;
        let engine_type = detect_engine_type(&mut conn);

        // Use cluster-aware backfill wait if we have cluster topology
        if let Some(ref topology) = self.cluster_topology {
            let config = BackfillWaitConfig {
                max_wait: Some(Duration::from_secs(timeout_secs)),
                ..Default::default()
            };

            let index_names = [search_config.index_name.as_str()];
            let conn_factory = &self.connection_factory;

            wait_for_backfill(
                engine_type,
                &index_names,
                &topology.nodes,
                |node| {
                    conn_factory.create(&node.host, node.port).ok()
                },
                &config,
            )
            .map_err(|e| {
                crate::utils::BenchmarkError::Config(format!("Backfill wait failed: {}", e))
            })?;
        } else {
            // Single-node mode: use simple wait
            info!(
                "Waiting for index '{}' to complete indexing (timeout: {}s)",
                search_config.index_name, timeout_secs
            );

            crate::workload::wait_for_indexing(&mut conn, &search_config.index_name, timeout_secs)
                .map_err(|e| {
                    crate::utils::BenchmarkError::Config(format!("Indexing wait failed: {}", e))
                })?;

            info!("Index '{}' is fully indexed", search_config.index_name);
        }

        Ok(())
    }

    /// Get index information
    pub fn get_search_index_info(&self) -> Result<crate::workload::IndexInfo> {
        let search_config = self.config.search_config.as_ref().ok_or_else(|| {
            crate::utils::BenchmarkError::Config(
                "Search config required for index operations".to_string(),
            )
        })?;

        let addresses = self.get_benchmark_addresses();
        if addresses.is_empty() {
            return Err(crate::utils::BenchmarkError::Config(
                "No available server addresses".to_string(),
            ));
        }

        let (host, port) = &addresses[0];
        let mut conn = self.connection_factory.create(host, *port)?;

        get_index_info(&mut conn, &search_config.index_name).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!("Failed to get index info: {}", e))
        })
    }

    /// Export results to JSON file
    pub fn export_json(&self, results: &[BenchmarkResult], path: &Path) -> Result<()> {
        let config_summary = format!(
            "hosts={:?}, clients={}, threads={}, pipeline={}, requests={}",
            self.config.addresses.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            self.config.clients,
            self.config.threads,
            self.config.pipeline,
            self.effective_requests()
        );

        let mut benchmark_results = BenchmarkResults::new(&config_summary);

        for result in results {
            let aggregated = crate::metrics::collector::AggregatedMetrics {
                test_name: result.test_name.clone(),
                duration_secs: result.duration.as_secs_f64(),
                total_ops: result.total_requests,
                total_errors: result.error_count,
                throughput: result.throughput,
                mean_latency_ms: result.histogram.mean() / 1000.0,
                p50_latency_ms: result.percentile_ms(50.0),
                p95_latency_ms: result.percentile_ms(95.0),
                p99_latency_ms: result.percentile_ms(99.0),
                p999_latency_ms: result.percentile_ms(99.9),
                max_latency_ms: result.histogram.max() as f64 / 1000.0,
                node_count: result.node_metrics.len().max(1),
            };
            benchmark_results.add_test(aggregated, result.node_metrics.clone());
        }

        benchmark_results.write_json(path).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!("Failed to write JSON: {}", e))
        })
    }

    /// Apply runtime configuration to all nodes
    ///
    /// Parses the configuration file and applies all settings via CONFIG SET
    /// to all cluster nodes (or the standalone node).
    ///
    /// # Arguments
    /// * `config_path` - Path to the configuration file
    ///
    /// # Returns
    /// The number of configurations successfully applied across all nodes
    pub fn apply_runtime_config(&self, config_path: &Path) -> Result<usize> {
        use crate::config::{RuntimeConfig, RuntimeConfigManager};

        // Parse the configuration file
        let runtime_config = RuntimeConfig::from_file(config_path).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!(
                "Failed to read runtime config '{}': {}",
                config_path.display(),
                e
            ))
        })?;

        if runtime_config.is_empty() {
            if !self.config.quiet {
                info!("Runtime config file is empty, nothing to apply");
            }
            return Ok(0);
        }

        // Get seed addresses for standalone mode
        let seed_addresses: Vec<(String, u16)> = self.config
            .addresses
            .iter()
            .map(|a| (a.host.clone(), a.port))
            .collect();

        // Create config manager and apply
        let manager = RuntimeConfigManager::new(
            &runtime_config,
            &self.connection_factory,
            self.config.quiet,
        );

        let results = manager.apply_all(self.cluster_topology.as_ref(), &seed_addresses);

        // Count total successful applications
        let total_applied: usize = results.iter().map(|r| r.applied.len()).sum();
        let total_failed: usize = results.iter().map(|r| r.failed.len()).sum();

        if total_failed > 0 {
            return Err(crate::utils::BenchmarkError::Config(format!(
                "Failed to apply {} configuration(s)",
                total_failed
            )));
        }

        if !self.config.quiet {
            info!("Successfully applied {} runtime configuration(s)", total_applied);
        }

        Ok(total_applied)
    }

    /// Verify runtime configuration on all nodes
    ///
    /// Checks that all configurations in the file match the current server values.
    ///
    /// # Arguments
    /// * `config_path` - Path to the configuration file
    ///
    /// # Returns
    /// True if all configurations match on all nodes
    pub fn verify_runtime_config(&self, config_path: &Path) -> Result<bool> {
        use crate::config::{RuntimeConfig, RuntimeConfigManager};

        // Parse the configuration file
        let runtime_config = RuntimeConfig::from_file(config_path).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!(
                "Failed to read runtime config '{}': {}",
                config_path.display(),
                e
            ))
        })?;

        if runtime_config.is_empty() {
            return Ok(true);
        }

        // Get seed addresses for standalone mode
        let seed_addresses: Vec<(String, u16)> = self.config
            .addresses
            .iter()
            .map(|a| (a.host.clone(), a.port))
            .collect();

        // Create config manager and verify
        let manager = RuntimeConfigManager::new(
            &runtime_config,
            &self.connection_factory,
            self.config.quiet,
        );

        let results = manager.verify_all(self.cluster_topology.as_ref(), &seed_addresses);

        // Check if all nodes verified successfully
        let all_success = results.iter().all(|r| r.is_success());

        if all_success && !self.config.quiet {
            info!("All runtime configurations verified successfully");
        }

        Ok(all_success)
    }

    /// Export results to CSV file
    pub fn export_csv(&self, results: &[BenchmarkResult], path: &Path) -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        let mut file = File::create(path).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!("Failed to create CSV file: {}", e))
        })?;

        // Write header
        writeln!(
            file,
            "test_name,duration_secs,total_ops,errors,throughput,mean_ms,p50_ms,p95_ms,p99_ms,p999_ms,max_ms"
        ).map_err(|e| {
            crate::utils::BenchmarkError::Config(format!("Failed to write CSV header: {}", e))
        })?;

        // Write rows
        for result in results {
            writeln!(
                file,
                "{},{:.2},{},{},{:.2},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
                result.test_name,
                result.duration.as_secs_f64(),
                result.total_requests,
                result.error_count,
                result.throughput,
                result.histogram.mean() / 1000.0,
                result.percentile_ms(50.0),
                result.percentile_ms(95.0),
                result.percentile_ms(99.0),
                result.percentile_ms(99.9),
                result.histogram.max() as f64 / 1000.0
            ).map_err(|e| {
                crate::utils::BenchmarkError::Config(format!("Failed to write CSV row: {}", e))
            })?;
        }

        Ok(())
    }
}

/// Format throughput without meaningless decimals
/// Examples: 1,234,567 req/s, 987,654 req/s
pub fn format_throughput(throughput: f64) -> String {
    let value = throughput as u64;
    format_count(value)
}

/// Format large numbers with thousands separators
/// Examples: 1,234,567 or 987,654
pub fn format_count(value: u64) -> String {
    let s = value.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.insert(0, ',');
        }
        result.insert(0, c);
    }
    result
}

/// Detect engine type from INFO SEARCH response
fn detect_engine_type(conn: &mut crate::client::RawConnection) -> EngineType {
    use crate::utils::{RespEncoder, RespValue};

    let mut encoder = RespEncoder::with_capacity(64);
    encoder.encode_command_str(&["INFO", "SEARCH"]);

    match conn.execute_encoded(&encoder) {
        Ok(RespValue::BulkString(data)) => {
            let info_str = String::from_utf8_lossy(&data);
            EngineType::detect(&info_str)
        }
        _ => EngineType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_result_percentiles() {
        let mut histogram =
            Histogram::new_with_bounds(1, 3_600_000_000, 3).expect("Failed to create histogram");

        // Record some values (in microseconds)
        for _ in 0..100 {
            histogram.record(1000).unwrap(); // 1ms
        }
        for _ in 0..10 {
            histogram.record(10000).unwrap(); // 10ms
        }

        let result = BenchmarkResult {
            test_name: "test".to_string(),
            total_requests: 110,
            duration: Duration::from_secs(1),
            throughput: 110.0,
            histogram,
            recall_stats: RecallStats::new(),
            error_count: 0,
            node_metrics: Vec::new(),
            keyspace_stats: KeyspaceStats::default(),
        };

        // p50 should be around 1ms
        assert!(result.percentile_ms(50.0) < 2.0);
    }

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(1), "1");
        assert_eq!(format_count(123), "123");
        assert_eq!(format_count(1234), "1,234");
        assert_eq!(format_count(12345), "12,345");
        assert_eq!(format_count(123456), "123,456");
        assert_eq!(format_count(1234567), "1,234,567");
        assert_eq!(format_count(1000000), "1,000,000");
    }

    #[test]
    fn test_format_throughput() {
        assert_eq!(format_throughput(937821.7051), "937,821");
        assert_eq!(format_throughput(1000000.5), "1,000,000");
        assert_eq!(format_throughput(123.456), "123");
    }
}
