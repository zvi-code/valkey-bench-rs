//! Benchmark configuration derived from CLI arguments

use super::cli::{CliArgs, OutputFormat, ReadFromReplica};
use super::search_config::SearchConfig;
use super::tls_config::TlsConfig;
use super::workload_config::WorkloadConfig;
use crate::workload::WorkloadType;
use std::fmt;
use std::path::PathBuf;

/// Resolved server address
#[derive(Debug, Clone)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// Authentication configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub password: String,
    pub username: Option<String>,
}

/// Complete benchmark configuration
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    // Connection
    pub addresses: Vec<ServerAddress>,
    pub socket_path: Option<PathBuf>,
    pub auth: Option<AuthConfig>,
    pub tls: Option<TlsConfig>,
    pub dbnum: Option<u32>,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,

    // Parallelism
    pub clients: u32,
    pub threads: u32,
    pub pipeline: u32,

    // Request generation
    /// Explicit request count from CLI (None = use default or dataset size)
    pub requests: Option<u64>,
    pub duration_secs: Option<u64>,
    pub warmup_requests: u64,
    pub keyspace_len: u64,
    pub sequential: bool,
    pub iteration: Option<String>,
    pub key_prefix: String,
    pub address_type: Option<String>,
    pub data_size: usize,
    pub seed: u64,

    // Cluster
    pub cluster_mode: bool,
    pub read_from_replica: ReadFromReplica,
    pub balance_nodes: bool,
    pub balance_tolerance: u32,

    // Rate limiting
    pub requests_per_second: u64,

    // Workload
    pub tests: Vec<String>,
    pub parallel: Option<String>,
    pub composite: Option<String>,
    pub custom_command: Option<String>,

    // Vector search
    pub search_config: Option<SearchConfig>,
    pub schema_path: Option<PathBuf>,
    pub data_path: Option<PathBuf>,
    pub filtered_search: bool,
    pub num_vectors: u64,
    pub vector_offset: u64,

    // Optimizer
    pub optimize: bool,
    pub optimize_objective: String,
    pub optimize_tolerance: f64,
    pub optimize_constraints: Vec<String>,
    pub optimize_parameters: Vec<String>,
    pub max_optimize_iterations: u32,

    // Output
    pub output_path: Option<PathBuf>,
    pub output_format: OutputFormat,
    pub csv_output: Option<PathBuf>,
    pub quiet: bool,
    pub verbose: bool,

    // Control
    pub dropindex: bool,
    pub skip_load: bool,
    pub cleanup: bool,

    // Runtime configuration
    pub runtime_config_path: Option<PathBuf>,
}

impl BenchmarkConfig {
    /// Create configuration from CLI arguments
    pub fn from_cli(args: &CliArgs) -> Result<Self, String> {
        // Validate first
        args.validate()?;

        // Build server addresses
        let addresses: Vec<ServerAddress> = args
            .hosts
            .iter()
            .map(|h| ServerAddress {
                host: h.clone(),
                port: args.port,
            })
            .collect();

        // Build auth config
        let auth = args.password.as_ref().map(|p| AuthConfig {
            password: p.clone(),
            username: args.username.clone(),
        });

        // Build TLS config
        let tls = if args.tls {
            Some(TlsConfig {
                skip_verify: args.tls_skip_verify,
                ca_cert: args.tls_ca_cert.clone(),
                client_cert: args.tls_cert.clone(),
                client_key: args.tls_key.clone(),
                sni: args.tls_sni.clone(),
            })
        } else {
            None
        };

        // Build search config if needed
        let search_config = if args.schema.is_some()
            || args
                .tests
                .as_ref()
                .map(|t| t.iter().any(|s| s.to_lowercase().contains("vec")))
                .unwrap_or(false)
        {
            Some(SearchConfig::from_cli(args))
        } else {
            None
        };

        // Determine tests to run
        let tests = args
            .tests
            .clone()
            .unwrap_or_else(|| vec!["ping".to_string()]);

        // Effective thread count
        let threads = args.effective_threads();

        // Effective keyspace
        let keyspace_len = args.effective_keyspace();

        Ok(Self {
            addresses,
            socket_path: args.socket.clone(),
            auth,
            tls,
            dbnum: args.dbnum,
            connect_timeout_ms: args.connect_timeout_ms,
            request_timeout_ms: args.request_timeout_ms,

            clients: args.clients,
            threads,
            pipeline: args.pipeline,

            requests: args.requests,
            duration_secs: args.duration_secs,
            warmup_requests: args.warmup_requests,
            keyspace_len,
            sequential: args.sequential,
            iteration: args.iteration.clone(),
            key_prefix: args.key_prefix.clone(),
            address_type: args.address_type.clone(),
            data_size: args.data_size,
            seed: args.seed,

            cluster_mode: args.cluster_mode,
            read_from_replica: args.read_from_replica,
            balance_nodes: args.balance_nodes,
            balance_tolerance: args.balance_tolerance,

            requests_per_second: args.requests_per_second,

            tests,
            parallel: args.parallel.clone(),
            composite: args.composite.clone(),
            custom_command: args.custom_command.clone(),

            search_config,
            schema_path: args.schema.clone(),
            data_path: args.data.clone(),
            filtered_search: args.filtered_search,
            num_vectors: args.num_vectors,
            vector_offset: args.vector_offset,

            optimize: args.optimize,
            optimize_objective: args.optimize_objective.clone(),
            optimize_tolerance: args.optimize_tolerance,
            optimize_constraints: args.optimize_constraints.clone(),
            optimize_parameters: args.optimize_parameters.clone(),
            max_optimize_iterations: args.max_optimize_iterations,

            output_path: args.output.clone(),
            output_format: args.output_format,
            csv_output: args.csv_output.clone(),
            quiet: args.quiet,
            verbose: args.verbose,

            dropindex: args.dropindex,
            skip_load: args.skip_load,
            cleanup: args.cleanup,

            runtime_config_path: args.runtime_config.clone(),
        })
    }

    /// Get clients per thread
    pub fn clients_per_thread(&self) -> u32 {
        self.clients.div_ceil(self.threads)
    }

    /// Get effective requests count
    ///
    /// Returns:
    /// - Explicit value if provided via CLI
    /// - dataset_size if provided (for vec-load workloads)
    /// - 100,000 as default fallback
    pub fn effective_requests(&self, dataset_size: Option<u64>) -> u64 {
        const DEFAULT_REQUESTS: u64 = 100_000;

        if let Some(explicit) = self.requests {
            // User explicitly set requests via CLI
            explicit
        } else if let Some(ds_size) = dataset_size {
            // Use dataset size for dataset-based workloads
            ds_size
        } else {
            // Default fallback
            DEFAULT_REQUESTS
        }
    }

    /// Check if this config has vec-load workload
    pub fn has_vec_load_workload(&self) -> bool {
        self.tests.iter().any(|t| {
            let lower = t.to_lowercase();
            lower == "vecload" || lower == "vec-load"
        })
    }

    /// Create a WorkloadConfig for the given workload type using global defaults
    ///
    /// This is useful for creating per-component configs from global settings.
    pub fn workload_config(&self, workload_type: WorkloadType) -> WorkloadConfig {
        WorkloadConfig::from_type_with_defaults(
            workload_type,
            &self.key_prefix,
            self.keyspace_len,
            self.data_size,
            self.search_config.clone(),
            self.schema_path.clone(),
            self.data_path.clone(),
        )
    }
}
