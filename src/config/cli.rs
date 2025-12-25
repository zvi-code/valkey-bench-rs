//! Command-line argument parsing
//!
//! This module defines all CLI arguments matching the C valkey-benchmark implementation.
//! Arguments are grouped by category for clarity.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

/// High-performance benchmark tool for Valkey with vector search support
#[derive(Parser, Debug, Clone)]
#[command(name = "valkey-bench-rs")]
#[command(version, about, long_about = "High-performance benchmark tool for Valkey/Redis with vector search support.\n\n\
Supports parallel workloads (--parallel), composite workloads (--composite),\n\
iteration strategies (--iteration), and addressable spaces (--address-type).\n\n\
For comprehensive examples, see EXAMPLES.md in the project directory.")]
#[command(arg_required_else_help = false)]
#[command(disable_help_flag = true)]
#[command(trailing_var_arg = true)]
#[allow(clippy::manual_non_exhaustive)]
pub struct CliArgs {
    /// Print help information
    #[arg(long = "help", action = clap::ArgAction::Help)]
    help: (),

    // ===== CLI Mode =====
    /// Run in interactive CLI mode (like valkey-cli)
    #[arg(long = "cli")]
    pub cli_mode: bool,

    /// Command arguments when using --cli (non-interactive mode)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub command_args: Vec<String>,

    // ===== Connection Options =====
    /// Server hostname (can be specified multiple times for cluster)
    #[arg(short = 'h', long = "host", default_value = "127.0.0.1", action = clap::ArgAction::Append)]
    pub hosts: Vec<String>,

    /// Server port
    #[arg(short = 'p', long = "port", default_value_t = 6379)]
    pub port: u16,

    /// Unix socket path (overrides host/port)
    #[arg(short = 's', long = "socket")]
    pub socket: Option<PathBuf>,

    /// Password for AUTH command
    #[arg(short = 'a', long = "auth")]
    pub password: Option<String>,

    /// Username for ACL AUTH (requires --auth)
    #[arg(long = "user")]
    pub username: Option<String>,

    // ===== TLS Options =====
    /// Enable TLS connection
    #[arg(long = "tls")]
    pub tls: bool,

    /// Skip TLS certificate verification (insecure)
    #[arg(long = "tls-skip-verify")]
    pub tls_skip_verify: bool,

    /// CA certificate file for TLS
    #[arg(long = "tls-ca-cert")]
    pub tls_ca_cert: Option<PathBuf>,

    /// Client certificate file for TLS
    #[arg(long = "tls-cert")]
    pub tls_cert: Option<PathBuf>,

    /// Client private key file for TLS
    #[arg(long = "tls-key")]
    pub tls_key: Option<PathBuf>,

    /// Server Name Indication for TLS
    #[arg(long = "tls-sni")]
    pub tls_sni: Option<String>,

    // ===== Benchmark Parameters =====
    /// Number of parallel connections (total across all threads)
    #[arg(short = 'c', long = "clients", default_value_t = 50)]
    pub clients: u32,

    /// Number of worker threads
    #[arg(long = "threads", default_value_t = 0)]
    pub threads: u32,

    /// Total number of requests to issue (default: 100000, or dataset size for vec-load)
    #[arg(short = 'n', long = "requests")]
    pub requests: Option<u64>,

    /// Database number to SELECT
    #[arg(long = "dbnum")]
    pub dbnum: Option<u32>,

    /// Pipeline depth (commands per batch)
    #[arg(short = 'P', long = "pipeline", default_value_t = 1)]
    pub pipeline: u32,

    // ===== Key Generation =====
    /// Size of key/value data in bytes
    #[arg(short = 'd', long = "data-size", default_value_t = 3)]
    pub data_size: usize,

    /// Use random keys within keyspace range
    #[arg(short = 'r', long = "random-keys", default_value_t = 0)]
    pub keyspace_len: u64,

    /// Use sequential keys instead of random
    #[arg(long = "sequential")]
    pub sequential: bool,

    /// Iteration strategy for key generation
    ///
    /// Formats:
    /// - "sequential" or "seq"
    /// - "random" or "random:SEED"
    /// - "subset:START:END"
    /// - "zipfian:SKEW" or "zipfian:SKEW:SEED"
    #[arg(long = "iteration")]
    pub iteration: Option<String>,

    /// Key prefix for generated keys
    #[arg(long = "key-prefix", default_value = "key:")]
    pub key_prefix: String,

    /// Address type for workload addressing
    ///
    /// Formats:
    /// - "key" or "key:prefix" - Simple keys (default)
    /// - "hash:prefix:field1,field2,field3" - Hash fields
    /// - "json:prefix:$.path1,$.path2" - JSON paths
    #[arg(long = "address-type")]
    pub address_type: Option<String>,

    // ===== Cluster Options =====
    /// Require cluster mode (auto-detected by default, this flag enforces it)
    #[arg(long = "cluster")]
    pub cluster_mode: bool,

    /// Read from replicas strategy
    #[arg(long = "rfr", value_enum, default_value_t = ReadFromReplica::Primary)]
    pub read_from_replica: ReadFromReplica,

    /// Enable node request balancing
    #[arg(long = "balance-nodes")]
    pub balance_nodes: bool,

    /// Node balance tolerance percentage
    #[arg(long = "balance-tolerance", default_value_t = 10)]
    pub balance_tolerance: u32,

    // ===== Workload Selection =====
    /// Benchmark type(s) to run
    #[arg(short = 't', long = "tests", value_delimiter = ',')]
    pub tests: Option<Vec<String>>,

    /// Parallel workload mix with weighted traffic
    ///
    /// Format: "workload1:weight1,workload2:weight2,..."
    /// Example: "get:0.8,set:0.2" for 80% GET, 20% SET
    #[arg(long = "parallel")]
    pub parallel: Option<String>,

    /// Composite workload for sequential phases
    ///
    /// Format: "workload1:count1,workload2:count2,..."
    /// Example: "vec-load:10000,vec-query:1000" for load then query
    #[arg(long = "composite")]
    pub composite: Option<String>,

    /// Custom command to benchmark (RESP format)
    #[arg(long = "command")]
    pub custom_command: Option<String>,

    // ===== Vector Search Options =====
    /// Index name for vector search
    #[arg(long = "search-index", default_value = "idx")]
    pub search_index: String,

    /// Vector field name in hash
    #[arg(long = "search-vector-field", default_value = "embedding")]
    pub search_vector_field: String,

    /// Key prefix for vector data
    #[arg(long = "search-prefix", default_value = "vec:")]
    pub search_prefix: String,

    /// Vector algorithm (HNSW or FLAT)
    #[arg(long = "search-algorithm", value_enum, default_value_t = VectorAlgorithm::Hnsw)]
    pub search_algorithm: VectorAlgorithm,

    /// Distance metric (L2, IP, COSINE)
    #[arg(long = "search-distance", value_enum, default_value_t = DistanceMetric::L2)]
    pub search_distance: DistanceMetric,

    /// Vector dimension
    #[arg(long = "vector-dim", default_value_t = 128)]
    pub vector_dim: u32,

    /// Number of nearest neighbors to return (k)
    #[arg(short = 'k', long = "search-k", default_value_t = 10)]
    pub search_k: u32,

    /// HNSW EF_CONSTRUCTION parameter
    #[arg(long = "ef-construction")]
    pub ef_construction: Option<u32>,

    /// HNSW M parameter
    #[arg(long = "hnsw-m")]
    pub hnsw_m: Option<u32>,

    /// EF_RUNTIME for search queries
    #[arg(long = "ef-search")]
    pub ef_search: Option<u32>,

    /// Use NOCONTENT in FT.SEARCH (return IDs only, faster)
    #[arg(long = "nocontent", default_value_t = true)]
    pub nocontent: bool,

    // ===== Tag and Attribute Options =====
    /// Tag field name in hash (for filtered search)
    #[arg(long = "tag-field")]
    pub tag_field: Option<String>,

    /// Tag distribution for vec-load. Format: "tag1:prob1,tag2:prob2,..."
    /// Each tag has independent probability (0-100) of being included.
    /// Use __rand_int__ for random numbers. Example: "electronics:50,clothing:30,id__rand_int__:10"
    #[arg(long = "search-tags")]
    pub search_tags: Option<String>,

    /// Tag filter for vec-query (FT.SEARCH). Example: "electronics|clothing"
    #[arg(long = "tag-filter")]
    pub tag_filter: Option<String>,

    /// Maximum tag field payload length (for fixed-size templates)
    #[arg(long = "tag-max-len", default_value_t = 128)]
    pub tag_max_len: usize,

    /// Numeric field name in hash (for filtered search)
    /// Simple format: just the field name (uses key_num as value)
    #[arg(long = "numeric-field")]
    pub numeric_field: Option<String>,

    /// Extended numeric field configuration (can be repeated for multiple fields)
    ///
    /// Format: "name:type:distribution:params..."
    ///
    /// Types:
    ///   int            - Integer values
    ///   float          - Float with 6 decimal places (default)
    ///   float:N        - Float with N decimal places (e.g., float:2)
    ///   unix_timestamp - Unix timestamp (seconds since epoch)
    ///   iso_datetime   - ISO 8601 datetime string
    ///   date_only      - Date only (YYYY-MM-DD)
    ///
    /// Distributions:
    ///   uniform:min:max        - Uniform random between min and max
    ///   zipfian:skew:min:max   - Zipfian (power-law), skew typically 0.5-2.0
    ///   normal:mean:stddev     - Normal/Gaussian distribution
    ///   sequential:start:step  - Sequential values starting at start
    ///   constant:value         - Fixed constant value
    ///   key_based:min:max      - Derive from key number (deterministic)
    ///
    /// Examples:
    ///   --numeric-field-config "price:float:uniform:0.99:999.99:2"
    ///   --numeric-field-config "quantity:int:zipfian:1.5:1:1000"
    ///   --numeric-field-config "rating:float:normal:4.0:0.5:1"
    ///   --numeric-field-config "created:unix_timestamp:uniform:1672531200:1735689600"
    ///   --numeric-field-config "views:int:sequential:0:1"
    #[arg(long = "numeric-field-config", action = clap::ArgAction::Append)]
    pub numeric_field_configs: Vec<String>,

    /// Numeric filter for vec-query (FT.SEARCH). Can be repeated for multiple filters.
    ///
    /// Format: "field:[min,max]" or "field:(min,max)" for exclusive bounds
    /// Use -inf/+inf for unbounded ranges.
    ///
    /// Examples:
    ///   --numeric-filter "price:[10,100]"      -> @price:[10 100]
    ///   --numeric-filter "score:(0,100]"       -> @score:[(0 100]
    ///   --numeric-filter "rating:[-inf,4.5]"   -> @rating:[-inf 4.5]
    ///   --numeric-filter "count:[100,+inf)"    -> @count:[100 (+inf]
    #[arg(long = "numeric-filter", action = clap::ArgAction::Append)]
    pub numeric_filters: Vec<String>,

    // ===== Dataset Options =====
    /// Path to dataset schema YAML file
    #[arg(long = "schema")]
    pub schema: Option<PathBuf>,

    /// Path to binary dataset file
    #[arg(long = "data")]
    pub data: Option<PathBuf>,

    /// Enable filtered search (use dataset metadata)
    #[arg(long = "filtered-search")]
    pub filtered_search: bool,

    /// Number of vectors to load (0 = all)
    #[arg(long = "num-vectors", default_value_t = 0)]
    pub num_vectors: u64,

    /// Starting vector index
    #[arg(long = "vector-offset", default_value_t = 0)]
    pub vector_offset: u64,

    // ===== Rate Limiting =====
    /// Target requests per second (0 = unlimited)
    #[arg(long = "rps", default_value_t = 0)]
    pub requests_per_second: u64,

    // ===== Optimizer Options =====
    /// Enable automatic parameter optimization
    #[arg(long = "optimize")]
    pub optimize: bool,

    /// Optimization objective(s). Multiple goals separated by comma.
    /// Format: "direction:metric[:op:value]" where direction is maximize/minimize.
    /// Examples:
    ///   "maximize:qps" - maximize QPS
    ///   "maximize:qps,minimize:p99_ms" - max QPS, tiebreak on lowest p99
    ///   "maximize:qps:lt:1000000" - max QPS but must be < 1M (bounded)
    #[arg(long = "objective", default_value = "maximize:qps")]
    pub optimize_objective: String,

    /// Tolerance for multi-objective equivalence (0.04 = 4%).
    /// Configs within tolerance on primary goal are compared by secondary goals.
    #[arg(long = "tolerance", default_value_t = 0.04)]
    pub optimize_tolerance: f64,

    /// Constraint for optimization (can be repeated). Format: "metric:op:value"
    /// Examples: "recall:gt:0.95", "p99_ms:lt:0.1", "qps:gte:100000"
    /// Valid metrics: qps, recall, p50_ms, p95_ms, p99_ms, p999_ms, mean_latency_ms, error_rate
    /// Valid operators: gt, gte, lt, lte, eq
    #[arg(long = "constraint", action = clap::ArgAction::Append)]
    pub optimize_constraints: Vec<String>,

    /// Parameter to tune (can be repeated). Format: "param:min:max:step"
    /// Examples: "clients:10:200:10", "threads:1:16:1", "ef_search:10:500:10", "pipeline:1:20:1"
    #[arg(long = "tune", action = clap::ArgAction::Append)]
    pub optimize_parameters: Vec<String>,

    /// Maximum optimization iterations
    #[arg(long = "max-optimize-iterations", default_value_t = 50)]
    pub max_optimize_iterations: u32,

    // ===== Output Options =====
    /// Output file path
    #[arg(short = 'o', long = "output")]
    pub output: Option<PathBuf>,

    /// Output format
    #[arg(long = "output-format", value_enum, default_value_t = OutputFormat::Text)]
    pub output_format: OutputFormat,

    /// Output CSV file for per-second stats
    #[arg(long = "csv")]
    pub csv_output: Option<PathBuf>,

    /// Quiet mode (minimal output)
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,

    /// Verbose output
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    // ===== Timing Options =====
    /// Run duration in seconds (overrides -n)
    #[arg(long = "duration")]
    pub duration_secs: Option<u64>,

    /// Connection timeout in milliseconds
    #[arg(long = "connect-timeout", default_value_t = 5000)]
    pub connect_timeout_ms: u64,

    /// Request timeout in milliseconds
    #[arg(long = "request-timeout", default_value_t = 30000)]
    pub request_timeout_ms: u64,

    // ===== Runtime Configuration =====
    /// Path to runtime configuration file (applied via CONFIG SET before benchmark)
    ///
    /// The config file uses a simple key-value format compatible with valkey.conf:
    ///   # Comments start with #
    ///   io-threads 8
    ///   maxmemory 10gb
    ///   save ""
    ///
    /// All configurations are applied to all cluster nodes after discovery.
    /// Only mutable/modifiable parameters can be set at runtime.
    #[arg(long = "runtime-config")]
    pub runtime_config: Option<PathBuf>,

    // ===== Advanced Options =====
    /// Drop existing index before creating new one
    #[arg(long = "dropindex")]
    pub dropindex: bool,

    /// Skip data loading (assume loaded)
    #[arg(long = "skip-load")]
    pub skip_load: bool,

    /// Delete index after benchmark
    #[arg(long = "cleanup")]
    pub cleanup: bool,

    /// Warmup requests before measurement
    #[arg(long = "warmup", default_value_t = 0)]
    pub warmup_requests: u64,

    /// Seed for random number generation (0 = random seed each run)
    #[arg(long = "seed", default_value_t = 12345)]
    pub seed: u64,
}

/// Read-from-replica strategy
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadFromReplica {
    /// Always read from primary
    #[default]
    Primary,
    /// Prefer replicas, fallback to primary
    PreferReplica,
    /// Round-robin across all nodes
    RoundRobin,
    /// Prefer same availability zone
    AzAffinity,
}

/// Vector search algorithm
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VectorAlgorithm {
    #[default]
    Hnsw,
    Flat,
}

impl VectorAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            VectorAlgorithm::Hnsw => "HNSW",
            VectorAlgorithm::Flat => "FLAT",
        }
    }
}

/// Distance metric for vector search
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistanceMetric {
    #[default]
    L2,
    #[value(name = "IP")]
    InnerProduct,
    Cosine,
}

impl DistanceMetric {
    pub fn as_str(&self) -> &'static str {
        match self {
            DistanceMetric::L2 => "L2",
            DistanceMetric::InnerProduct => "IP",
            DistanceMetric::Cosine => "COSINE",
        }
    }

    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "l2" => Some(DistanceMetric::L2),
            "ip" | "innerproduct" | "inner_product" => Some(DistanceMetric::InnerProduct),
            "cosine" => Some(DistanceMetric::Cosine),
            _ => None,
        }
    }
}

/// Output format for results
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Csv,
}

impl CliArgs {
    /// Parse CLI arguments from command line
    pub fn parse_args() -> Self {
        Self::parse()
    }

    /// Validate argument combinations
    pub fn validate(&self) -> Result<(), String> {
        // Username requires password
        if self.username.is_some() && self.password.is_none() {
            return Err("--user requires --auth to be set".to_string());
        }

        // TLS cert requires TLS key
        if self.tls_cert.is_some() != self.tls_key.is_some() {
            return Err("--tls-cert and --tls-key must both be specified".to_string());
        }

        // Client certificate requires CA cert (usually)
        if self.tls_cert.is_some() && self.tls_ca_cert.is_none() && !self.tls_skip_verify {
            return Err(
                "--tls-cert typically requires --tls-ca-cert (or use --tls-skip-verify)"
                    .to_string(),
            );
        }

        // Pipeline must be positive
        if self.pipeline == 0 {
            return Err("--pipeline must be at least 1".to_string());
        }

        // Schema and data must both be specified together
        if self.schema.is_some() != self.data.is_some() {
            return Err("--schema and --data must both be specified".to_string());
        }

        // Dataset required for vector search tests
        if let Some(ref tests) = self.tests {
            let needs_dataset = tests.iter().any(|t| {
                matches!(
                    t.to_lowercase().as_str(),
                    "vecload" | "vecquery" | "vec-load" | "vec-query"
                )
            });
            if needs_dataset && self.schema.is_none() {
                return Err("Vector search tests require --schema and --data".to_string());
            }
        }

        // Keyspace length for random keys
        if self.keyspace_len == 0 && !self.sequential {
            // Will use default keyspace
        }

        Ok(())
    }

    /// Get effective number of threads (0 = auto-detect)
    pub fn effective_threads(&self) -> u32 {
        if self.threads == 0 {
            std::thread::available_parallelism()
                .map(|p| p.get() as u32)
                .unwrap_or(4)
        } else {
            self.threads
        }
    }

    /// Get effective keyspace length
    pub fn effective_keyspace(&self) -> u64 {
        const DEFAULT_REQUESTS: u64 = 100_000;
        if self.keyspace_len == 0 {
            self.requests.unwrap_or(DEFAULT_REQUESTS)
        } else {
            self.keyspace_len
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_args() {
        let args = CliArgs::parse_from(["test"]);
        assert_eq!(args.port, 6379);
        assert_eq!(args.clients, 50);
        assert_eq!(args.requests, None); // None means use dataset size or default 100000
        assert_eq!(args.pipeline, 1);
    }

    #[test]
    fn test_multiple_hosts() {
        let args = CliArgs::parse_from(["test", "-h", "host1", "-h", "host2", "-h", "host3"]);
        assert_eq!(args.hosts, vec!["host1", "host2", "host3"]);
    }

    #[test]
    fn test_vector_search_args() {
        let args = CliArgs::parse_from([
            "test",
            "--search-index",
            "myidx",
            "--vector-dim",
            "256",
            "-k",
            "20",
            "--ef-search",
            "100",
        ]);
        assert_eq!(args.search_index, "myidx");
        assert_eq!(args.vector_dim, 256);
        assert_eq!(args.search_k, 20);
        assert_eq!(args.ef_search, Some(100));
    }

    #[test]
    fn test_validation_user_without_auth() {
        let args = CliArgs::parse_from(["test", "--user", "admin"]);
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validation_tls_cert_without_key() {
        let args = CliArgs::parse_from(["test", "--tls-cert", "cert.pem"]);
        assert!(args.validate().is_err());
    }
}
