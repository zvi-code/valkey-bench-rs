//! Configurable INFO field definitions for temporal tracking
//!
//! This module provides a flexible system for defining which fields to track
//! from INFO SEARCH, FT.INFO, and other server responses. Each field can have:
//! - Custom parsing strategy (integer, memory, percentile, cmdstat)
//! - Aggregation type (sum, average, max, min/max)
//! - Display format (integer, memory MB, percentage, latency)
//! - Diff type for temporal comparison (rate, memory growth, percentage change)
//! - Node filtering (primary only, replica only, all)

use std::collections::HashMap;

// ============================================================================
// Enums and Config Types
// ============================================================================

/// Parse strategy for extracting values from INFO responses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseStrategy {
    Integer,
    Memory,
    FloatFixed,
    Percentile,
    CmdStats,
}

/// Configuration for parsing
#[derive(Debug, Clone)]
pub struct ParseConfig {
    pub strategy: ParseStrategy,
    pub key: Option<String>,
}

impl ParseConfig {
    pub fn integer() -> Self {
        Self { strategy: ParseStrategy::Integer, key: None }
    }

    pub fn memory() -> Self {
        Self { strategy: ParseStrategy::Memory, key: None }
    }

    pub fn float_fixed() -> Self {
        Self { strategy: ParseStrategy::FloatFixed, key: None }
    }

    pub fn percentile(key: &str) -> Self {
        Self { strategy: ParseStrategy::Percentile, key: Some(key.to_string()) }
    }

    pub fn cmdstats(key: &str) -> Self {
        Self { strategy: ParseStrategy::CmdStats, key: Some(key.to_string()) }
    }
}

/// How to aggregate values across nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregationType {
    Sum,
    Average,
    Max,
    MinMax,
}

/// How to display the value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayFormat {
    Integer,
    MemoryMb,
    MemoryHuman,
    Percentage,
    Float,
    LatencyUsec,
    MinMax,
}

/// How to calculate diff between snapshots
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffType {
    None,
    RateCount,
    RateMicrosec,
    MemoryGrowth,
    PercentageChange,
}

/// Which nodes to aggregate from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeFilter {
    PrimaryOnly,
    ReplicaOnly,
    All,
}

/// Field matching strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStrategy {
    Exact,
    Prefix,
}

// ============================================================================
// Field Type Definition
// ============================================================================

/// Definition of a field to track
#[derive(Debug, Clone)]
pub struct InfoFieldType {
    pub name: String,
    pub match_strategy: MatchStrategy,
    pub parse_config: ParseConfig,
    pub aggregation_type: AggregationType,
    pub display_format: DisplayFormat,
    pub diff_type: DiffType,
    pub track_per_node: bool,
    pub node_filter: NodeFilter,
}

impl InfoFieldType {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            match_strategy: MatchStrategy::Exact,
            parse_config: ParseConfig::integer(),
            aggregation_type: AggregationType::Sum,
            display_format: DisplayFormat::Integer,
            diff_type: DiffType::None,
            track_per_node: true,
            node_filter: NodeFilter::All,
        }
    }

    // Builder methods
    pub fn prefix_match(mut self) -> Self {
        self.match_strategy = MatchStrategy::Prefix;
        self
    }

    pub fn parse(mut self, config: ParseConfig) -> Self {
        self.parse_config = config;
        self
    }

    pub fn aggregate(mut self, agg: AggregationType) -> Self {
        self.aggregation_type = agg;
        self
    }

    pub fn display(mut self, fmt: DisplayFormat) -> Self {
        self.display_format = fmt;
        self
    }

    pub fn diff(mut self, diff: DiffType) -> Self {
        self.diff_type = diff;
        self
    }

    pub fn from_primary_only(mut self) -> Self {
        self.node_filter = NodeFilter::PrimaryOnly;
        self
    }

    pub fn from_replica_only(mut self) -> Self {
        self.node_filter = NodeFilter::ReplicaOnly;
        self
    }

    pub fn no_per_node(mut self) -> Self {
        self.track_per_node = false;
        self
    }

    pub fn matches(&self, field_name: &str) -> bool {
        let field_lower = field_name.to_ascii_lowercase();
        let name_lower = self.name.to_ascii_lowercase();
        match self.match_strategy {
            MatchStrategy::Exact => field_lower == name_lower,
            MatchStrategy::Prefix => field_lower.starts_with(&name_lower),
        }
    }
}

// ============================================================================
// Field Presets (Common Patterns)
// ============================================================================

/// Common field patterns for compact definitions
pub mod presets {
    use super::*;

    /// Counter field: Sum, Integer, RateCount
    pub fn counter(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::Integer)
            .diff(DiffType::RateCount)
    }

    /// Counter field (primary only)
    pub fn counter_primary(name: &str) -> InfoFieldType {
        counter(name).from_primary_only()
    }

    /// Memory field: Sum, MemoryHuman, MemoryGrowth, primary only
    pub fn memory(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::MemoryHuman)
            .diff(DiffType::MemoryGrowth)
            .from_primary_only()
    }

    /// Memory field with MB display
    pub fn memory_mb(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::MemoryMb)
            .diff(DiffType::MemoryGrowth)
            .from_primary_only()
    }

    /// Max value field: Max, Integer, None, primary only
    pub fn max_value(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Max)
            .display(DisplayFormat::Integer)
            .diff(DiffType::None)
            .from_primary_only()
    }

    /// Config field: MinMax, MinMax display, None diff, primary only, no per-node
    pub fn config_field(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::MinMax)
            .display(DisplayFormat::MinMax)
            .diff(DiffType::None)
            .from_primary_only()
            .no_per_node()
    }

    /// Gauge field: Sum, Integer, None (no rate), primary only
    pub fn gauge(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::Integer)
            .diff(DiffType::None)
            .from_primary_only()
    }

    /// Gauge field (all nodes)
    pub fn gauge_all(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::Integer)
            .diff(DiffType::None)
    }

    /// Average field: Average, Integer, None
    pub fn average(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Average)
            .display(DisplayFormat::Integer)
            .diff(DiffType::None)
    }

    /// Average field (primary only)
    pub fn average_primary(name: &str) -> InfoFieldType {
        average(name).from_primary_only()
    }

    /// Percentage field: Average, Percentage, None
    pub fn percentage(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .parse(ParseConfig::float_fixed())
            .aggregate(AggregationType::Average)
            .display(DisplayFormat::Percentage)
            .diff(DiffType::None)
    }

    /// Percentage field (primary only)
    pub fn percentage_primary(name: &str) -> InfoFieldType {
        percentage(name).from_primary_only()
    }

    /// Float rate field: Sum, Float, RateCount
    pub fn float_rate(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .parse(ParseConfig::float_fixed())
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::Float)
            .diff(DiffType::RateCount)
    }

    /// Float rate field (primary only)
    pub fn float_rate_primary(name: &str) -> InfoFieldType {
        float_rate(name).from_primary_only()
    }

    /// Latency percentile field: Max, LatencyUsec, None
    pub fn latency_percentile(name: &str, percentile: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .parse(ParseConfig::percentile(percentile))
            .aggregate(AggregationType::Max)
            .display(DisplayFormat::LatencyUsec)
            .diff(DiffType::None)
    }

    /// Status field: Sum, Integer, None, primary only, no per-node
    pub fn status(name: &str) -> InfoFieldType {
        InfoFieldType::new(name)
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::Integer)
            .diff(DiffType::None)
            .from_primary_only()
            .no_per_node()
    }

    /// Command stats field with prefix match
    pub fn cmdstat(name: &str, stat: &str) -> InfoFieldType {
        let base = InfoFieldType::new(name)
            .prefix_match()
            .parse(ParseConfig::cmdstats(stat));

        match stat {
            "calls" => base
                .aggregate(AggregationType::Sum)
                .display(DisplayFormat::Integer)
                .diff(DiffType::RateCount),
            "usec" => base
                .aggregate(AggregationType::Sum)
                .display(DisplayFormat::Integer)
                .diff(DiffType::RateMicrosec),
            "usec_per_call" => base
                .aggregate(AggregationType::Average)
                .display(DisplayFormat::Integer)
                .diff(DiffType::None),
            "rejected" | "failed" => base
                .aggregate(AggregationType::Sum)
                .display(DisplayFormat::Integer)
                .diff(DiffType::RateCount),
            _ => base
                .aggregate(AggregationType::Sum)
                .display(DisplayFormat::Integer)
                .diff(DiffType::None),
        }
    }
}

// ============================================================================
// Field Value Types
// ============================================================================

#[derive(Debug, Clone)]
pub struct FieldValue {
    pub value: i64,
    pub min_value: Option<i64>,
    pub max_value: Option<i64>,
    pub value_str: Option<String>,
}

impl FieldValue {
    pub fn new(value: i64) -> Self {
        Self { value, min_value: None, max_value: None, value_str: None }
    }

    pub fn with_minmax(min: i64, max: i64) -> Self {
        Self { value: max, min_value: Some(min), max_value: Some(max), value_str: None }
    }
}

#[derive(Debug, Clone)]
pub struct FieldSnapshot {
    pub field_name: String,
    pub value: FieldValue,
    pub per_node_values: Option<Vec<(String, i64)>>,
    pub node_count: usize,
    pub valid: bool,
}

// ============================================================================
// Parsing Functions
// ============================================================================

pub fn parse_value(line: &str, config: &ParseConfig) -> Option<i64> {
    let value_str = line.split(':').nth(1)?.trim();

    match config.strategy {
        ParseStrategy::Integer => value_str.parse::<i64>().ok(),
        ParseStrategy::Memory => parse_memory_value(value_str),
        ParseStrategy::FloatFixed => {
            let f: f64 = value_str.parse().ok()?;
            Some((f * 1000.0) as i64)
        }
        ParseStrategy::Percentile => extract_kv_field(value_str, config.key.as_deref()?),
        ParseStrategy::CmdStats => extract_kv_field_numeric(value_str, config.key.as_deref()?),
    }
}

// Re-export memory parsing from utils
pub use crate::utils::parse_memory_value;

/// Extract key-value field from comma-separated string (returns i64)
fn extract_kv_field(s: &str, key: &str) -> Option<i64> {
    s.split(',')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .and_then(|(_, v)| v.trim().parse().ok())
}

/// Extract key-value field (handles both int and float)
fn extract_kv_field_numeric(s: &str, key: &str) -> Option<i64> {
    s.split(',')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .and_then(|(_, v)| {
            let v = v.trim();
            v.parse::<i64>()
                .ok()
                .or_else(|| v.parse::<f64>().ok().map(|f| f as i64))
        })
}

// ============================================================================
// Formatting Functions
// ============================================================================

pub fn format_value(value: &FieldValue, format: DisplayFormat) -> String {
    match format {
        DisplayFormat::Integer => format!("{}", value.value),
        DisplayFormat::MemoryMb => format!("{:.2} MB", value.value as f64 / (1024.0 * 1024.0)),
        DisplayFormat::MemoryHuman => format_memory_human(value.value),
        DisplayFormat::Percentage => format!("{:.2}%", value.value as f64 / 1000.0),
        DisplayFormat::Float => format!("{:.3}", value.value as f64 / 1000.0),
        DisplayFormat::LatencyUsec => format!("{} µs", value.value),
        DisplayFormat::MinMax => {
            if let (Some(min), Some(max)) = (value.min_value, value.max_value) {
                format!("{}/{}", min, max)
            } else {
                format!("{}", value.value)
            }
        }
    }
}

pub fn format_memory_human(bytes: i64) -> String {
    let abs = bytes.unsigned_abs();
    let sign = if bytes < 0 { "-" } else { "" };

    if abs >= 1024 * 1024 * 1024 * 1024 {
        format!("{}{:.2}T", sign, abs as f64 / (1024.0 * 1024.0 * 1024.0 * 1024.0))
    } else if abs >= 1024 * 1024 * 1024 {
        format!("{}{:.2}G", sign, abs as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if abs >= 1024 * 1024 {
        format!("{}{:.2}M", sign, abs as f64 / (1024.0 * 1024.0))
    } else if abs >= 1024 {
        format!("{}{:.2}K", sign, abs as f64 / 1024.0)
    } else {
        format!("{}{}B", sign, abs)
    }
}

pub fn calculate_diff(old_value: i64, new_value: i64, elapsed_secs: f64, diff_type: DiffType) -> Option<f64> {
    if elapsed_secs <= 0.0 {
        return None;
    }

    let delta = new_value - old_value;

    match diff_type {
        DiffType::None => None,
        DiffType::RateCount => Some(delta as f64 / elapsed_secs),
        DiffType::RateMicrosec => Some((delta as f64 / 1_000_000.0) / elapsed_secs),
        DiffType::MemoryGrowth => Some((delta as f64 / (1024.0 * 1024.0)) / elapsed_secs),
        DiffType::PercentageChange => {
            if old_value == 0 { None } else { Some((delta as f64 / old_value as f64) * 100.0) }
        }
    }
}

// ============================================================================
// Field Definitions Using Presets
// ============================================================================

/// Default INFO SEARCH fields (supports EC CME, EC CMD, and MemoryDB)
pub fn default_search_info_fields() -> Vec<InfoFieldType> {
    use presets::*;

    vec![
        // Request rates - Common to all systems
        counter("search_successful_requests_count"),
        counter("search_failure_requests_count"),
        counter("search_hybrid_requests_count"),

        // Memory - Common to all systems
        memory_mb("search_used_memory_bytes"),
        memory_mb("search_index_reclaimable_memory"),

        // EC CMD/CME specific - ingestion
        counter_primary("search_ingest_field_vector"),

        // EC CMD/CME specific - indexing rates
        counter("search_total_indexed_documents"),
        InfoFieldType::new("search_total_active_write_threads")
            .aggregate(AggregationType::MinMax)
            .display(DisplayFormat::MinMax)
            .diff(DiffType::None),

        // MemoryDB specific - indexing stats
        counter_primary("search_total_indexed_keys"),
        counter_primary("search_total_indexed_vectors"),
        counter_primary("search_total_indexed_hash_keys"),
        memory("search_total_index_size"),
        memory("search_total_vector_index_size"),
        max_value("search_max_index_degradation_percentage"),
        max_value("search_max_index_lag_ms"),

        // EC CMD/CME specific - CPU usage
        float_rate("search_read_cpu_time_sec"),
        float_rate_primary("search_write_cpu_time_sec"),
        percentage("search_used_read_cpu"),
        percentage_primary("search_used_write_cpu"),

        // EC CMD/CME specific - Queue sizes
        average("search_query_queue_size"),
        average_primary("search_writer_queue_size"),

        // Latencies - EC CMD/CME specific
        latency_percentile("search_hnsw_vector_index_search_latency_usec", "p50"),
        latency_percentile("search_hnsw_vector_index_search_latency_usec", "p99"),
        latency_percentile("search_hnsw_vector_index_search_latency_usec", "p99.9"),
        latency_percentile("search_flat_vector_index_search_latency_usec", "p99"),

        // EC CME specific - coordinator latencies
        latency_percentile("search_coordinator_server_search_index_partition_success_latency_usec", "p99"),
        latency_percentile("search_coordinator_client_search_index_partition_success_latency_usec", "p99"),

        // Error rates
        counter("search_hnsw_add_exceptions_count"),
        counter("search_bounds_check_errors"),

        // Index counters
        gauge("search_num_hnsw_edges"),
        gauge("search_num_flat_nodes"),
        gauge("search_num_vector_indexes"),
        gauge("search_num_hnsw_indexes"),
        gauge("search_num_flat_indexes"),
        gauge("search_number_of_indexes"),
        gauge("search_num_available_indexes"),
        gauge("search_vectors_marked_deleted"),
        InfoFieldType::new("search_num_hnsw_nodes")
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::Integer)
            .diff(DiffType::PercentageChange)
            .from_primary_only(),

        // Status - MemoryDB specific
        status("search_background_indexing_status"),
        gauge("search_num_active_backfills"),
        average_primary("search_current_backfill_progress_percentage"),
        gauge_all("search_num_active_queries"),

        // Memory stats
        memory("search_vectors_memory_marked_deleted"),
        memory("search_vectors_bytes"),
        memory("search_interned_strings_memory"),

        // Network stats - EC CME specific
        InfoFieldType::new("search_network_bytes_in")
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::MemoryHuman)
            .diff(DiffType::RateCount),
        InfoFieldType::new("search_network_bytes_out")
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::MemoryHuman)
            .diff(DiffType::RateCount),
        InfoFieldType::new("search_coordinator_bytes_in")
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::MemoryHuman)
            .diff(DiffType::RateCount),
        InfoFieldType::new("search_coordinator_bytes_out")
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::MemoryHuman)
            .diff(DiffType::RateCount),
    ]
}

/// Default FT.INFO fields (supports EC and MemoryDB)
pub fn default_ftinfo_fields() -> Vec<InfoFieldType> {
    use presets::*;

    vec![
        // EC format fields
        counter_primary("num_docs"),
        counter("hash_indexing_failures"),
        average_primary("mutation_queue_size"),
        gauge("num_records"),
        config_field("attributes.dim"),
        config_field("attributes.M"),
        config_field("attributes.capacity"),
        config_field("attributes.size"),

        // MemoryDB format fields
        status("index_name"),
        counter_primary("num_indexed_vectors"),
        memory("space_usage"),
        memory("vector_space_usage"),
        memory("fulltext_space_usage"),
        max_value("current_lag"),
        status("index_status"),
        max_value("index_degradation_percentage"),
        config_field("fields.vector_params.dimension"),
        config_field("fields.vector_params.maximum_edges"),
        config_field("fields.vector_params.current_capacity"),
    ]
}

/// Default INFO fields (general server stats)
pub fn default_info_fields() -> Vec<InfoFieldType> {
    use presets::*;

    vec![
        // Memory
        InfoFieldType::new("used_memory")
            .parse(ParseConfig::memory())
            .aggregate(AggregationType::Sum)
            .display(DisplayFormat::MemoryMb)
            .diff(DiffType::None)
            .from_primary_only(),

        // Keyspace hit/miss counters
        counter("keyspace_hits"),
        counter("keyspace_misses"),

        // FT.SEARCH command stats
        cmdstat("cmdstat_FT.SEARCH", "calls"),
        cmdstat("cmdstat_FT.SEARCH", "usec"),
        cmdstat("cmdstat_FT.SEARCH", "usec_per_call"),
        cmdstat("cmdstat_FT.SEARCH", "rejected"),
        cmdstat("cmdstat_FT.SEARCH", "failed"),

        // Generic command stats
        cmdstat("cmdstat_", "calls"),
        cmdstat("cmdstat_", "usec"),
    ]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_integer() {
        let config = ParseConfig::integer();
        assert_eq!(parse_value("field:12345", &config), Some(12345));
        assert_eq!(parse_value("field:-100", &config), Some(-100));
    }

    #[test]
    fn test_parse_memory() {
        let config = ParseConfig::memory();
        assert_eq!(parse_value("field:1024", &config), Some(1024));
        assert_eq!(parse_value("field:1K", &config), Some(1024));
        assert_eq!(parse_value("field:1M", &config), Some(1024 * 1024));
        assert_eq!(parse_value("field:1.5G", &config), Some((1.5 * 1024.0 * 1024.0 * 1024.0) as i64));
    }

    #[test]
    fn test_parse_float_fixed() {
        let config = ParseConfig::float_fixed();
        assert_eq!(parse_value("field:1.5", &config), Some(1500));
        assert_eq!(parse_value("field:0.001", &config), Some(1));
    }

    #[test]
    fn test_parse_percentile() {
        let config = ParseConfig::percentile("p99");
        assert_eq!(parse_value("field:p50=100,p99=500,p99.9=1000", &config), Some(500));
    }

    #[test]
    fn test_parse_cmdstats() {
        let config = ParseConfig::cmdstats("calls");
        assert_eq!(
            parse_value("cmdstat_FT.SEARCH:calls=1000,usec=50000,usec_per_call=50.00", &config),
            Some(1000)
        );
    }

    #[test]
    fn test_format_memory_human() {
        assert_eq!(format_memory_human(500), "500B");
        assert_eq!(format_memory_human(1024), "1.00K");
        assert_eq!(format_memory_human(1024 * 1024), "1.00M");
        assert_eq!(format_memory_human(1024 * 1024 * 1024), "1.00G");
    }

    #[test]
    fn test_calculate_diff() {
        assert_eq!(calculate_diff(100, 200, 10.0, DiffType::RateCount), Some(10.0));
        assert_eq!(calculate_diff(0, 100, 10.0, DiffType::PercentageChange), None);
        assert_eq!(calculate_diff(100, 200, 10.0, DiffType::PercentageChange), Some(100.0));
    }

    #[test]
    fn test_field_matching() {
        let exact = InfoFieldType::new("search_used_memory_bytes");
        assert!(exact.matches("search_used_memory_bytes"));
        assert!(!exact.matches("search_used_memory"));

        let prefix = InfoFieldType::new("cmdstat_FT.").prefix_match();
        assert!(prefix.matches("cmdstat_FT.SEARCH"));
        assert!(prefix.matches("cmdstat_FT.INFO"));
        assert!(!prefix.matches("cmdstat_GET"));
    }

    #[test]
    fn test_presets() {
        let counter = presets::counter("test_field");
        assert_eq!(counter.aggregation_type, AggregationType::Sum);
        assert_eq!(counter.display_format, DisplayFormat::Integer);
        assert_eq!(counter.diff_type, DiffType::RateCount);

        let memory = presets::memory("test_memory");
        assert_eq!(memory.display_format, DisplayFormat::MemoryHuman);
        assert_eq!(memory.diff_type, DiffType::MemoryGrowth);
        assert_eq!(memory.node_filter, NodeFilter::PrimaryOnly);
    }
}
