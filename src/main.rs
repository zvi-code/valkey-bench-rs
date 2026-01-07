//! valkey-bench-rs - High-performance benchmark tool for Valkey
//!
//! This tool supports standard Redis/Valkey benchmarks as well as
//! vector search (FT.SEARCH) benchmarks with recall verification.
//!
//! When run with `--cli`, operates as an interactive CLI (like valkey-cli).

// Allow dead code during development - fields/types will be used in later phases
#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::Result;
use std::sync::Arc;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

mod benchmark;
mod cli_mode;
mod client;
mod cluster;
mod config;
mod dataset;
mod keyspace;
mod metrics;
mod optimizer;
mod utils;
mod workload;

use benchmark::Orchestrator;
use config::{BenchmarkConfig, CliArgs, DistanceMetric};
use dataset::DatasetContext;
use optimizer::{Constraint, Objectives, Optimizer, TunableParameter};

fn setup_logging(verbose: bool, quiet: bool) {
    let level = if quiet {
        Level::ERROR
    } else if verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .with_thread_ids(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");
}

fn print_banner(
    config: &BenchmarkConfig,
    base_latency: Option<&benchmark::BaseLatency>,
    dataset_size: Option<u64>,
) {
    if config.quiet {
        return;
    }

    println!("valkey-bench-rs v{}", env!("CARGO_PKG_VERSION"));
    println!("============================================================");

    // Connection info
    let hosts: Vec<_> = config.addresses.iter().map(|a| a.to_string()).collect();
    let host_str = if hosts.len() == 1 {
        hosts[0].clone()
    } else {
        format!("{} hosts", hosts.len())
    };

    let mut conn_parts = vec![host_str];
    if config.tls.is_some() {
        conn_parts.push("TLS".to_string());
    }
    if config.auth.is_some() {
        conn_parts.push("AUTH".to_string());
    }
    // Cluster mode: show "cluster(required)" if --cluster flag used, otherwise "cluster(auto)"
    // Actual detection happens during orchestrator initialization
    if config.cluster_mode {
        conn_parts.push(format!("cluster(required, rfr={:?})", config.read_from_replica));
    } else {
        conn_parts.push("cluster(auto-detect)".to_string());
    }
    println!("Connection: {}", conn_parts.join(" | "));

    // Base latency (network round-trip baseline)
    if let Some(bl) = base_latency {
        println!("Base RTT: {}", bl.format());
    }

    // Workload config - compact single line
    let effective_requests = config.effective_requests(dataset_size);
    let mut workload_parts = vec![
        format!("clients={}", config.clients),
        format!("threads={}", config.threads),
        format!("pipeline={}", config.pipeline),
        format!("requests={}", benchmark::format_count(effective_requests)),
        format!("keyspace={}", benchmark::format_count(config.keyspace_len)),
    ];
    if config.data_size > 0 {
        workload_parts.push(format!("datasize={}", config.data_size));
    }
    if config.sequential {
        workload_parts.push("sequential".to_string());
    }
    if config.requests_per_second > 0 {
        workload_parts.push(format!("rps={}", config.requests_per_second));
    }
    println!("Workload: {}", workload_parts.join(" "));

    // Tests
    println!("Tests: {}", config.tests.join(", "));

    // Vector search config (if applicable)
    if let Some(ref search) = config.search_config {
        let mut search_parts = vec![
            format!("index={}", search.index_name),
            format!("dim={}", search.dim),
            format!("k={}", search.k),
            format!("algo={:?}", search.algorithm),
        ];
        if let Some(ef) = search.ef_search {
            search_parts.push(format!("ef_search={}", ef));
        }
        if search.tag_field.is_some() {
            search_parts.push(format!("tag_field={}", search.tag_field.as_ref().unwrap()));
        }
        if search.tag_filter.is_some() {
            search_parts.push(format!("tag_filter={}", search.tag_filter.as_ref().unwrap()));
        }
        println!("Search: {}", search_parts.join(" "));
    }

    // Dataset
    if let Some(ref schema_path) = config.schema_path {
        println!("Schema: {}", schema_path.display());
        if let Some(ref data_path) = config.data_path {
            println!("Data: {}", data_path.display());
        }
    }

    // Runtime configuration
    if let Some(ref runtime_config_path) = config.runtime_config_path {
        println!("Runtime config: {}", runtime_config_path.display());
    }

    println!("============================================================\n");
}

/// Handle index management for vector workloads
/// - Drop index if --dropindex is specified
/// - Create index if it doesn't exist
/// - Wait for index to be ready
fn setup_vector_index(
    orchestrator: &Orchestrator,
    config: &BenchmarkConfig,
    has_vec_workload: bool,
) -> Result<()> {
    if !has_vec_workload || config.search_config.is_none() {
        return Ok(());
    }

    // Drop existing index if --dropindex is specified
    if config.dropindex {
        if orchestrator.search_index_exists() {
            info!("Dropping existing index (--dropindex specified)...");
            orchestrator.drop_search_index()?;
        } else {
            info!("Index does not exist, skipping drop");
        }
    }

    // Ensure index exists (create if not present)
    if !orchestrator.search_index_exists() {
        info!("Creating vector search index...");
        orchestrator.create_search_index(false)?;

        // Wait for index creation to propagate across cluster
        info!("Waiting 10 seconds for index to propagate...");
        std::thread::sleep(std::time::Duration::from_secs(10));
    } else {
        info!("Index already exists, skipping creation");
    }

    info!("Waiting for index to be ready...");
    orchestrator.wait_for_search_indexing(300)?; // 5 minute timeout

    Ok(())
}

/// Run optimization mode: iteratively test configurations to find optimal parameters
fn run_optimization(
    base_config: &BenchmarkConfig,
    dataset: Option<Arc<DatasetContext>>,
) -> Result<()> {
    // Parse objectives (supports multi-goal with tolerance)
    let objectives = Objectives::parse(&base_config.optimize_objective)
        .map_err(|e| anyhow::anyhow!("Invalid objective: {}", e))?
        .with_tolerance(base_config.optimize_tolerance);

    // Parse constraints
    let mut constraints = Vec::new();
    for constraint_str in &base_config.optimize_constraints {
        let constraint = Constraint::parse(constraint_str)
            .map_err(|e| anyhow::anyhow!("Invalid constraint '{}': {}", constraint_str, e))?;
        constraints.push(constraint);
    }

    // Parse parameters to tune
    let mut parameters = Vec::new();
    for param_str in &base_config.optimize_parameters {
        let param = TunableParameter::parse(param_str)
            .map_err(|e| anyhow::anyhow!("Invalid parameter '{}': {}", param_str, e))?;
        parameters.push(param);
    }

    // If no parameters specified, provide a helpful error
    if parameters.is_empty() {
        return Err(anyhow::anyhow!(
            "No parameters to tune. Use --tune to specify parameters.\n\
             Examples:\n\
             --tune \"clients:10:200:10\"     # Tune clients from 10 to 200 with step 10\n\
             --tune \"threads:1:16:1\"        # Tune threads from 1 to 16\n\
             --tune \"ef_search:10:500:10\"   # Tune ef_search for vector search\n\
             --tune \"pipeline:1:20:1\"       # Tune pipeline depth"
        ));
    }

    // Compute base requests for adaptive duration
    // Use config's effective request count as base, with a minimum for meaningful measurements
    let dataset_size = dataset.as_ref().map(|ds| ds.num_vectors());
    let config_requests = base_config.effective_requests(dataset_size);
    let base_requests = if config_requests >= 100_000 {
        config_requests
    } else {
        100_000
    };
    let exploitation_multiplier = 5u64;

    println!("\n=== OPTIMIZATION MODE ===\n");
    println!("Objectives: {}", objectives);
    if objectives.goals.len() > 1 {
        println!(
            "  Tolerance: {:.1}% (configs within this range compared by secondary goals)",
            objectives.tolerance * 100.0
        );
    }
    if !constraints.is_empty() {
        println!("Constraints:");
        for c in &constraints {
            println!("  - {}", c);
        }
    }
    println!("Parameters to tune:");
    for p in &parameters {
        println!("  - {}", p);
    }
    println!("Max iterations: {}", base_config.max_optimize_iterations);
    println!(
        "Adaptive duration: {}K requests (exploration) -> {}K (exploitation)\n",
        base_requests / 1000,
        base_requests * exploitation_multiplier / 1000
    );

    let mut optimizer_builder = Optimizer::builder()
        .objectives(objectives)
        .max_iterations(base_config.max_optimize_iterations)
        .base_requests(base_requests)
        .exploitation_multiplier(exploitation_multiplier as u32);

    for constraint in constraints {
        optimizer_builder = optimizer_builder.constraint(constraint);
    }
    for param in parameters {
        optimizer_builder = optimizer_builder.parameter(param);
    }

    let mut optimizer = optimizer_builder.build();

    // Create base orchestrator for index creation etc.
    let mut orchestrator = Orchestrator::new(base_config.clone())?;
    if let Some(ref ds) = dataset {
        orchestrator.set_dataset_arc(ds.clone());
    }

    // Handle index management for vector workloads (once before optimization loop)
    let has_vec_workload = base_config.tests.iter().any(|t| t.to_lowercase().starts_with("vec"));
    setup_vector_index(&orchestrator, base_config, has_vec_workload)?;

    if has_vec_workload && base_config.search_config.is_some() {
        info!("Building existence map...");
        orchestrator.build_existence_map()?;
    }

    // Build protected IDs for workloads that need ground truth
    let needs_protected_ids = base_config.tests.iter().any(|t| {
        let lower = t.to_lowercase();
        // Match vec-delete, vec-del-protected, and vec-gt-load
        lower.contains("delete") || lower.contains("del-protected") || lower.contains("gt-load")
    });
    if needs_protected_ids && dataset.is_some() {
        info!("Building protected vector IDs from ground truth...");
        orchestrator.build_protected_ids()?;
    }

    // Extract shared resources from base orchestrator for iteration reuse
    // This avoids rediscovering cluster topology on each iteration (prevents port exhaustion)
    let shared_topology = orchestrator.cluster_topology().cloned();
    let shared_backend = orchestrator.backend().clone();
    let shared_existence_map = orchestrator.existence_map();
    let shared_protected_ids = orchestrator.protected_ids();

    // Optimization loop
    let mut iteration = 0;
    while let Some(test_config) = optimizer.next_config() {
        iteration += 1;

        // Get recommended request count for current phase
        // Exploration uses base requests, exploitation uses longer runs for accuracy
        let recommended_requests = optimizer.recommended_requests();

        // Apply test configuration to base config
        let mut run_config = base_config.clone();
        run_config.requests = Some(recommended_requests);
        run_config.quiet = true; // Suppress verbose output during optimization iterations

        if let Some(clients) = test_config.clients {
            run_config.clients = clients;
        }
        if let Some(threads) = test_config.threads {
            run_config.threads = threads;
        }
        if let Some(pipeline) = test_config.pipeline {
            run_config.pipeline = pipeline;
        }
        if let Some(ef_search) = test_config.ef_search {
            if let Some(ref mut search_config) = run_config.search_config {
                search_config.ef_search = Some(ef_search);
            }
        }

        // Create orchestrator with shared topology and backend (avoids rediscovery connections)
        let mut iter_orchestrator = Orchestrator::with_topology(run_config.clone(), shared_topology.clone(), shared_backend.clone())?;
        if let Some(ref ds) = dataset {
            iter_orchestrator.set_dataset_arc(ds.clone());
        }
        if let Some(ref existence_map) = shared_existence_map {
            iter_orchestrator.set_existence_map(existence_map.clone());
        }
        if let Some(ref protected) = shared_protected_ids {
            iter_orchestrator.set_protected_ids(protected.clone());
        }

        // Run benchmark for the first test only
        let results = iter_orchestrator.run_all()?;

        if let Some(result) = results.first() {
            // Record result with optimizer
            optimizer.record_result(test_config.clone(), result);

            // Print single-line result summary
            // Format: [iteration] phase | config | qps | p99 | recall (if applicable) | status
            let qps_str = if result.throughput >= 1_000_000.0 {
                format!("{:.2}M", result.throughput / 1_000_000.0)
            } else if result.throughput >= 1_000.0 {
                format!("{:.0}K", result.throughput / 1_000.0)
            } else {
                format!("{:.0}", result.throughput)
            };

            let recall = result.recall_stats.average();
            let recall_str = if recall > 0.0 {
                format!(" recall={:.3}", recall)
            } else {
                String::new()
            };

            let best_marker = if optimizer.best_result().map(|b| &b.config) == Some(&test_config) {
                " *BEST*"
            } else {
                ""
            };

            println!(
                "[{:2}] {:?} | {} | {} req/s p99={:.2}ms{}{}",
                iteration,
                optimizer.phase(),
                test_config,
                qps_str,
                result.percentile_ms(99.0),
                recall_str,
                best_marker
            );
        } else {
            warn!("No results from benchmark iteration");
        }
    }

    // Print final summary
    println!("\n{}", optimizer.summary());

    // Print prominent warning if didn't converge
    if optimizer.hit_iteration_limit() {
        eprintln!("\n!!! OPTIMIZATION DID NOT CONVERGE !!!");
        eprintln!("The iteration limit ({}) was reached before completing all phases.", base_config.max_optimize_iterations);
        eprintln!("The best result found may not be optimal.\n");
    }

    // Print best configuration as full command line with expected performance
    if let Some(best) = optimizer.best_result() {
        println!("\n=== Recommended Command Line ===\n");

        // Build the full command line including connection options
        let mut cmd_parts = vec!["./valkey-bench-rs".to_string()];

        // Add host(s)
        for addr in &base_config.addresses {
            cmd_parts.push(format!("-h {}", addr.host));
        }
        if base_config.addresses.first().map(|a| a.port).unwrap_or(6379) != 6379 {
            cmd_parts.push(format!(
                "-p {}",
                base_config.addresses.first().unwrap().port
            ));
        }

        // Add cluster mode if enabled
        if base_config.cluster_mode {
            cmd_parts.push("--cluster".to_string());
        }

        // Add TLS if enabled
        if base_config.tls.is_some() {
            cmd_parts.push("--tls".to_string());
            if base_config
                .tls
                .as_ref()
                .map(|t| t.skip_verify)
                .unwrap_or(false)
            {
                cmd_parts.push("--tls-skip-verify".to_string());
            }
        }

        // Add test type
        if !base_config.tests.is_empty() {
            cmd_parts.push(format!("-t {}", base_config.tests.join(",")));
        }

        // Add optimized parameters
        if let Some(clients) = best.config.clients {
            cmd_parts.push(format!("-c {}", clients));
        }
        if let Some(threads) = best.config.threads {
            cmd_parts.push(format!("--threads {}", threads));
        }
        if let Some(pipeline) = best.config.pipeline {
            cmd_parts.push(format!("-P {}", pipeline));
        }
        if let Some(ef_search) = best.config.ef_search {
            cmd_parts.push(format!("--ef-search {}", ef_search));
        }

        // Add dataset if used
        if let Some(ref schema_path) = base_config.schema_path {
            cmd_parts.push(format!("--schema {}", schema_path.display()));
        }
        if let Some(ref data_path) = base_config.data_path {
            cmd_parts.push(format!("--data {}", data_path.display()));
        }

        // Add index name if not default
        if let Some(ref search_config) = base_config.search_config {
            if search_config.index_name != "idx" {
                cmd_parts.push(format!("--search-index {}", search_config.index_name));
            }
        }

        // Add request count suggestion (use a reasonable production run size)
        let effective_req = base_config.effective_requests(dataset_size);
        cmd_parts.push(format!("-n {}", std::cmp::max(effective_req, 1_000_000)));

        println!("{}", cmd_parts.join(" "));

        // Print expected performance
        let qps = best.metrics.get(&optimizer::Metric::Qps).unwrap_or(&0.0);
        let p99 = best
            .metrics
            .get(&optimizer::Metric::P99Ms)
            .unwrap_or(&0.0);
        let recall = best.metrics.get(&optimizer::Metric::Recall).unwrap_or(&0.0);

        // Format QPS nicely
        let qps_str = if *qps >= 1_000_000.0 {
            format!("{:.2}M", qps / 1_000_000.0)
        } else if *qps >= 1_000.0 {
            format!("{:.0}K", qps / 1_000.0)
        } else {
            format!("{:.0}", qps)
        };

        println!("\nExpected performance: {} req/sec, p99={:.2}ms", qps_str, p99);
        if *recall > 0.0 {
            println!("                      recall={:.4}", recall);
        }
    }

    Ok(())
}

fn run() -> Result<()> {
    // Parse CLI arguments
    let args = CliArgs::parse_args();

    // Check for CLI mode
    if args.cli_mode {
        // Setup minimal logging for CLI mode
        setup_logging(false, true); // quiet mode

        // If there are trailing args, execute them as a command and exit
        if !args.command_args.is_empty() {
            return cli_mode::run_cli_command(&args, &args.command_args);
        }

        // Otherwise run interactive CLI
        return cli_mode::run_cli_mode(&args);
    }

    // Setup logging
    setup_logging(args.verbose, args.quiet);

    // Build configuration
    let mut config = BenchmarkConfig::from_cli(&args)
        .map_err(|e| anyhow::anyhow!("Configuration error: {}", e))?;

    // Load dataset if specified and update config with dataset dimensions
    let dataset = match (&config.schema_path, &config.data_path) {
        (Some(schema_path), Some(data_path)) => {
            info!("Loading dataset: schema={:?}, data={:?}", schema_path, data_path);
            let mut dataset = DatasetContext::open(schema_path, data_path)
                .map_err(|e| anyhow::anyhow!("Failed to load dataset: {}", e))?;

            // Apply CLI num_vectors and vector_offset overrides if provided
            // CLI args override schema values when explicitly set (num_vectors > 0 or vector_offset > 0)
            if config.num_vectors > 0 || config.vector_offset > 0 {
                info!(
                    "Applying CLI overrides: num_vectors={} (0=all), vector_offset={}",
                    config.num_vectors, config.vector_offset
                );
                dataset.set_effective_limits(config.num_vectors, config.vector_offset);
            }

            info!("{}", dataset.summary());

            // Update search config with dataset dimension and distance metric
            if let Some(ref mut search_config) = config.search_config {
                search_config.set_dim(dataset.dim() as u32);

                // Apply distance metric from schema if present
                let vector_field = search_config.vector_field.clone();
                if let Some(metric_str) = dataset.schema().get_distance_metric(&vector_field) {
                    if let Some(metric) = DistanceMetric::from_str(metric_str) {
                        info!("Using distance metric from schema: {}", metric.as_str());
                        search_config.set_distance_metric(metric);
                    }
                }

                // Apply tag/numeric fields from schema when --filtered-search is enabled
                if config.filtered_search {
                    // Apply tag field from schema if not already set via CLI
                    if search_config.tag_field.is_none() {
                        if let Some(tag_field) = dataset.schema().first_tag_field() {
                            info!("Using tag field from schema: {}", tag_field);
                            search_config.tag_field = Some(tag_field.to_string());
                        }
                    }

                    // Apply numeric field from schema if not already set via CLI
                    if search_config.numeric_field.is_none() && search_config.numeric_fields.is_empty() {
                        if let Some(numeric_field) = dataset.schema().first_numeric_field() {
                            info!("Using numeric field from schema: {}", numeric_field);
                            search_config.numeric_field = Some(numeric_field.to_string());
                        }
                    }
                }
            }

            Some(dataset)
        }
        _ => None,
    };

    // Get dataset size for effective_requests calculation (before dataset is moved)
    let dataset_size = dataset.as_ref().map(|ds| ds.num_vectors());

    // If optimization mode is enabled, run the optimizer (no base latency needed)
    if config.optimize {
        print_banner(&config, None, dataset_size);
        let dataset_arc = dataset.map(Arc::new);
        return run_optimization(&config, dataset_arc);
    }

    // Create orchestrator
    let mut orchestrator = Orchestrator::new(config.clone())?;

    // Apply runtime configuration if specified (after cluster discovery, before benchmarks)
    if let Some(ref runtime_config_path) = config.runtime_config_path {
        info!("Applying runtime configuration from: {:?}", runtime_config_path);
        orchestrator.apply_runtime_config(runtime_config_path)?;

        // Verify the configuration was applied correctly
        let verified = orchestrator.verify_runtime_config(runtime_config_path)?;
        if !verified {
            warn!("Some runtime configurations could not be verified");
        }
    }

    // Measure base latency (single-client PING and GET miss)
    let base_latency = if !config.quiet {
        match orchestrator.measure_base_latency() {
            Ok(bl) => Some(bl),
            Err(e) => {
                eprintln!("Warning: Failed to measure base latency: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Print banner with base latency
    print_banner(&config, base_latency.as_ref(), dataset_size);

    // Set dataset on orchestrator if loaded
    if let Some(dataset) = dataset {
        orchestrator.set_dataset(dataset);
    }

    // Detect workload types
    let has_vec_workload = config.tests.iter().any(|t| {
        let lower = t.to_lowercase();
        lower.starts_with("vec")  // Match vecload, vecquery, vecdelete, vec-load, vec-query, etc.
    });

    // Handle index management for vector workloads
    setup_vector_index(&orchestrator, &config, has_vec_workload)?;

    // Build cluster tag map for all vector workloads
    // This scans the cluster to discover which vectors exist and their cluster tags
    // Used by:
    // - vec-load: skip keys that already exist (only insert missing)
    // - vec-query: know which vectors exist for recall computation
    // - vec-delete/vec-update: operate on existing vectors
    // If no keys exist, scan completes very fast
    if has_vec_workload
        && config.search_config.is_some() {
            info!("Building existence map for existing vectors...");
            orchestrator.build_existence_map()?;
        }

    // Build protected IDs for workloads that need ground truth
    // The orchestrator.build_protected_ids() will only succeed if a dataset is loaded
    let needs_protected_ids = config.tests.iter().any(|t| {
        let lower = t.to_lowercase();
        // Match vec-delete, vec-del-protected, and vec-gt-load
        lower.contains("delete") || lower.contains("del-protected") || lower.contains("gt-load")
    });
    if needs_protected_ids {
        // Try to build protected IDs - will fail gracefully if no dataset
        if let Err(e) = orchestrator.build_protected_ids() {
            info!("Note: Cannot build protected IDs: {} (deletion will not skip ground truth)", e);
        }
    }

    // Run all tests
    let results = orchestrator.run_all()?;

    // Export to JSON if requested
    if let Some(ref output_path) = config.output_path {
        info!("Writing results to: {:?}", output_path);
        orchestrator.export_json(&results, output_path)?;
    }

    // Export to CSV if requested
    if let Some(ref csv_path) = config.csv_output {
        info!("Writing CSV to: {:?}", csv_path);
        orchestrator.export_csv(&results, csv_path)?;
    }

    // Print final summary
    if !config.quiet {
        println!("\n============================================================");
        println!("BENCHMARK COMPLETE");
        println!("============================================================");

        for result in &results {
            // Throughput line
            let mut summary_parts = vec![
                format!("{}: {} req/s", result.test_name, benchmark::format_throughput(result.throughput)),
            ];

            // Latency summary
            summary_parts.push(format!(
                "avg={:.2}ms p50={:.2}ms p99={:.2}ms max={:.2}ms",
                result.histogram.mean() / 1000.0,
                result.percentile_ms(50.0),
                result.percentile_ms(99.0),
                result.histogram.max() as f64 / 1000.0
            ));

            // Recall (if applicable)
            if result.recall_stats.total_queries > 0 {
                summary_parts.push(format!("recall={:.4}", result.recall_stats.average()));
            }

            // Hit rate (if applicable)
            if result.keyspace_stats.has_data() {
                summary_parts.push(format!("hit-rate={:.1}%", result.keyspace_stats.hit_rate() * 100.0));
            }

            // Errors (if any)
            if result.error_count > 0 {
                summary_parts.push(format!("errors={}", result.error_count));
            }

            println!("{}", summary_parts.join(" | "));
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        error!("Error: {:#}", e);
        std::process::exit(1);
    }
}
