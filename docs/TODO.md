# TODO: Enhancements and Planned Features

This document tracks planned enhancements and feature ideas for the valkey-search-benchmark Rust implementation.

---

## In Progress

### Mixed Workloads and Hybrid Search
**Status:** In Progress
**Description:** Support for mixed workloads involving different operation types (e.g., search, insert, delete) in a single benchmark run. Background non-search operations while search benchmarks run.
**Implementation:** `--parallel` and `--composite` CLI flags implemented. Per-workload `WorkloadConfig` foundation complete.
**Benefits:** More realistic testing scenarios that mimic production workloads.

### Numeric and Tag Filters
**Status:** In Progress
**Description:** Numeric and tag filters for queries, including `--numeric-filter` for query-side filtering and `--numeric-field-config` for load-side field generation.
**Benefits:** Test filtered search scenarios with realistic data distributions.

### Variable Tag Lengths
**Status:** In Progress
**Description:** Support for different tag lengths via `--tag-max-len` and tag distributions.
**Benefits:** Test impact of metadata size on performance.

### Range-Based Address Specification
**Status:** In Progress
**Description:** Allow specifying address ranges not starting from 0 via `--iteration` flag (e.g., `subset:1000:5000`).
**Benefits:** More flexible data addressing schemes.

### Collect Latency Per-Node
**Status:** In Progress
**Description:** Collect and report latency metrics for each individual node in cluster environments (CME and CMD with replicas).
**Benefits:** Deeper insights into per-node performance variations.

### Dataset Extension
**Status:** In Progress
**Description:** Extend dataset by shifting existing dataset by the diameter of current dataset.
**Note:** Will not work for cosine similarity.
**Benefits:** Create larger synthetic datasets from existing ones.

### Extended Delete Operations
**Status:** In Progress
**Description:** Support deleting by range, by percentage, or by capacity target.
**Benefits:** More flexible data management scenarios.

### Valkey Logic Encapsulation
**Status:** In Progress
**Description:** Encapsulate Valkey-specific logic to prepare for future extensions to other vector databases.
**Benefits:** Better code organization and easier support for multiple backends.

---

## Planned - Core Features

### Keyspace iteration Strategies
**Status:** planned
**Description:** Implement various keyspace iteration using https://github.com/zvi-code/keyspace_tracker: sequential, random, subset, zipfian. Keyspace splitting for multiple threads with no overlap and flexible 2 dimentional boundaries. Use keyspace_tracker to track deleted\existing keys and allow combined logic. Enhance with vector space vector-mapper to track existing vectors. Remove the vector-mapper from benchmark and use keyspace_tracker instead.
**CLI:** `--iteration "subset:1000:5000"`
**Files:** `src/workload/iteration.rs`
**Benefits:** Flexible and realistic data access patterns.

### Add data verification capability to workloads
**Status:** Planned
**Description:** Add capability to verify correctness of return data. Verification at 2 levels: key->valkey verification - the returned value for a key is correct; value verification - the data returned is not corrupted. These capabilities will be provided by allowing users to specity data-content that is key dependent. For example, for key "user:1001", the value is "vector:1001" followed by padding bytes. The verification logic will be implemented in the workload execution engine. And for data verification, various levels of verification may be provided: checksum, hash, full byte-by-byte comparison. There would be supported both in file based data definiton as well as via benchmark arguments. 
**Benefits:** Ensure data integrity and correctness during benchmarks.

### YAML Workload Definition
**Status:** Planned
**Priority:** High
**Proposal:** [PROPOSAL-yaml-workload.md](PROPOSAL-yaml-workload.md)
**Description:** Define complete multi-stage benchmark workloads via YAML files with hierarchical structure: Application > Stages > Commands.

**Key Features:**
- Application-level settings (clients, threads, rfr)
- Named data sources (generated KV data or dataset references)
- Sequential stages with parallel command execution
- Per-command configuration (iteration, pipeline, search params, filters)

**Example:**
```yaml
application:
  name: kv_progressive
  clients: 100
  threads: 16
  data_sources:
    kv_data:
      type: generated
      keyspace: 10000000
      value_size: 100
  stages:
    - name: prefill
      commands:
        - type: set
          data: kv_data
          iteration: sequential
      requests: 10000000
    - name: mixed
      requests: 1000000
      commands:
        - type: set
          ratio: 20
        - type: get
          ratio: 80
```

**CLI:** `--workload workloads/benchmark.yaml`
**Effort:** ~8 hours
**Benefits:** Readable, version-controllable, reusable benchmark definitions.

### Per-Workload Configuration File Support
**Status:** Planned (Foundation complete)
**Description:** Extend per-workload configuration to support complex scenarios via external config files (YAML/TOML).

**Current State:**
- `WorkloadConfig` struct in `src/config/workload_config.rs`
- ParallelComponent and WorkloadPhase use WorkloadConfig
- `apply_defaults()` propagates global CLI settings
- Simple CLI: `--parallel "get:0.8,set:0.2"` and `--composite "vec-load:10000,vec-query:1000"`

**Note:** This will be superseded by the YAML Workload Definition feature above.

**Key Files:** `workload_config.rs`, `cli.rs`, `parallel.rs`, `composite.rs`, `orchestrator.rs`
**Benefits:** Full per-workload customization, reusable config files.

### Support Multiple Indexes
**Status:** Planned
**Description:** Benchmark multiple indexes simultaneously within a single test run.
**Benefits:** Test scenarios involving multiple indexes and their interactions.

### TTL/Expiry Support
**Status:** Planned
**Description:** Set TTL/expiry on inserted vectors and keys.
**Benefits:** Test data expiration and cache eviction scenarios.

### JSON Data Type Support
**Status:** Planned
**Description:** Support search data in JSON data type (HASH is the default).
**Benefits:** Test JSON-based vector storage patterns.

### JSON Path Addressing
**Status:** Planned
**Description:** Extend `AddressSpec` to support JSON path addressing (`$.field.nested`) for JSON data type operations.
**Note:** Hash field addressing already implemented via `AddressSpec::hash_fields()`. Numeric field iteration via `AddressSpec::hash_numeric_field()`.
**Benefits:** Test JSON-based data structures with nested field access.

### Node Balanced Load
**Status:** Planned
**Description:** Re-implement node balanced load feature to ensure even distribution of workload across cluster nodes.
**Note:** Feature existed in C implementation.
**Benefits:** Improved performance and resource utilization in cluster mode.

### Sharded Index Workloads
**Status:** Planned
**Description:** Workloads for sharded index ingestion and querying to simulate per-key/slot indexes without fan-out.
**Benefits:** Test performance in sharded index environments.

### Dynamic Thread and Client Scaling
**Status:** Planned
**Description:** Dynamic adjustment of thread and client count without terminating threads.
**Benefits:** Eliminate warmup overhead between tests.

---

## Planned - Enhancements

### Reproduce workload from info metrics
**Status:** Planned
**Description:** Given Valkey INFO metrics, reproduce the workload in benchmark. I advanced phase, provided info metrics over time allows reproducing time-varying workloads including their over-time behavior.
**Benefits:** Easier replication of production workloads for testing.

### CLI Node Selection
**Status:** Planned
**Description:** In CLI mode, allow interactive selection of node and direct node queries in cluster mode.
**Benefits:** Easier debugging and testing of specific nodes.

### Index Configuration Verification
**Status:** Planned
**Description:** Verify index configuration for existing indexes. Option to clean/recreate if mismatch.
**Benefits:** Prevents test failures due to configuration mismatches.

### Ground Truth Query Vector Insertion
**Status:** Planned
**Description:** Option to insert all ground truth query vectors into the index.
**Benefits:** More comprehensive testing and validation scenarios.

### Ground Truth Generation via Flat Search
**Status:** Planned
**Description:** Generate ground truth using flat search for existing indexes.
**Benefits:** Create ground truth data without external dependencies.

### Test Stage and Tag Reporting
**Status:** Planned
**Description:** Report `test stage` and `test tag` for external profiling tools.
**Benefits:** Better integration with profiling and monitoring tools.

### Node Pre-Config Verification
**Status:** Planned
**Description:** Verify node pre-configuration settings are correctly applied in Rust implementation.
**Note:** Works in C code, needs verification in Rust.
**Benefits:** Reliable configuration management.

### Vector Scan Query Verify (vec-scan-q-verify)
**Status:** Planned
**Description:** New operation type that performs vector scan queries and verifies results against self. Statistical recall for datasets without ground truth.
**Benefits:** Quality sanity test without ground truth generation cost.

### Index Name with Parameters
**Status:** Planned
**Description:** Embed ef-construction and m settings in index names for easy identification.
**Benefits:** Simplifies index management with multiple configurations.

### Vector Field Name Deduction
**Status:** Planned
**Description:** Automatically deduce vector field name from dataset name during index creation.
**Benefits:** Reduces configuration errors.

### Vector Load Progress Reporting
**Status:** Planned
**Description:** Real-time progress feedback for vector load operations (percentage, ETA, throughput).
**Benefits:** Better user experience for long-running loads.

### Improved Node Workload Distribution
**Status:** Planned
**Description:** Enhanced workload distribution in CME/CMD for balanced resource utilization.
**Benefits:** Optimizes performance, reduces bottlenecks.

### Out-of-Sync Replica Handling
**Status:** Planned
**Description:** Detection and handling of lagging replicas during benchmarks.
**Benefits:** More accurate results, clearer cluster health insights.

### Graceful Ctrl+C Handling
**Status:** Planned
**Description:** Generate partial output when interrupted during load operations.
**Benefits:** Preserve work done before interruption.

### Dynamic Load During Cluster Events
**Status:** Planned
**Description:** Adjust load distribution during node additions/removals.
**Benefits:** Optimal performance during topology changes.

### Multiple Cluster Support
**Status:** Planned
**Description:** Benchmark across multiple Valkey clusters simultaneously.
**Benefits:** Test distributed scenarios and cluster interactions.

### Custom Binary Format Ingestion
**Status:** Planned
**Description:** Data loaders for custom binary formats with layout descriptor files.
**Benefits:** Broader applicability with various data sources.

### Refactor WorkloadContext
**Status:** Planned
**Description:** Decompose into separate abstractions: data source, address iterator, workload execution.
**Benefits:** Improved modularity and extensibility.

---

## Planned - Wrapper Scripts

### Memory Saturation Testing
**Status:** Planned
**Description:** Wrapper for testing with 100% memory utilization.
**Benefits:** Test behavior under memory pressure and eviction.

### Payload Impact Testing
**Status:** Planned
**Description:** Test impact of different payload sizes and types.
**Benefits:** Understand memory/performance tradeoffs.

---

## Planned - Search Quality Metrics

### Additional Quality Metrics
**Status:** Planned
**Description:** Implement ranking-aware metrics beyond simple recall:

**Mean Average Precision (MAP):**
```
AP = (1 / N_relevant) * Σ(k=1 to K) P@k * rel(k)
```
Rewards correct items appearing early in ranking.

**Normalized Discounted Cumulative Gain (NDCG):**
```
DCG@k = Σ(i=1 to k) (2^rel_i - 1) / log₂(i + 1)
NDCG@k = DCG@k / IDCG@k
```
Measures how well-ordered results are.

**Mean Reciprocal Rank (MRR):**
```
MRR = (1 / N) * Σ(i=1 to N) (1 / rank_i)
```
Measures how soon first relevant item appears.

**Recall@R curves:** Recall at different depth thresholds (10, 100, 1000).

**Benefits:** Industry-standard metrics for comprehensive search quality evaluation.

---

## Architecture Exploration

Design investigations for future evolution. Not immediate tasks.

### Abstraction Dimensions

| Dimension | Current State | Future Direction |
|-----------|---------------|------------------|
| Cluster Type | `EngineType` enum | Full `Platform` trait |
| Addressable Space | `AddressSpec` (keys, hash fields, numeric fields) | JSON paths, PubSub, multi-DB |
| Iterators | `IterationStrategy` enum | Composable iterator chains |
| Workload | `ParallelWorkload`, `CompositeWorkload` | Nested composition |

### Full Platform Trait
**Status:** Exploration
**Description:** Expand `EngineType` into a full trait for engine-specific differences:
```rust
pub trait Platform: Send + Sync {
    fn engine_type(&self) -> EngineType;
    fn supports_localonly(&self) -> bool;
    fn ft_info_fields(&self) -> &[&str];
    fn backfill_metric_name(&self) -> &str;
}
```
**Current:** `EngineType` enum with auto-detection implemented.
**Remaining:** Abstract engine-specific behaviors into trait implementations.

### Extended Address Spaces
**Status:** Exploration
**Description:** Extend `AddressSpec` beyond current capabilities:
- JSON path addressing (`$.field.nested`)
- PubSub channel addressing
- Multi-DB addressing
- Stream entry addressing

**Current:** `AddressSpec` supports keys with numeric ranges, hash fields (literal, list, numeric), and independent key/field ranges via `PlaceholderSpec`.

### Nested Workload Composition
**Status:** Exploration
**Description:** Allow workloads to contain other workloads recursively:
```rust
pub enum Phase {
    Configure(ConfigAction),
    Run(Box<dyn Workload>),
    Parallel(Vec<Box<dyn Workload>>),
    Sequence(Vec<Phase>),
}
```
**Current:** `CompositeWorkload` (sequential) and `ParallelWorkload` (concurrent) implemented. Nested combinations not yet supported.

### Benchmark vs Wrapper Boundary
**Status:** Exploration
**Description:** Define clear separation:

**Keep in benchmark:**
- Low-level execution (connections, pipelining, event loop)
- Single-phase workload execution
- Metrics collection
- Basic iterators

**Push to wrappers:**
- Multi-phase orchestration
- Complex application logic
- Cross-phase dependencies

---

## Completed

### Iteration Strategies
**Completed**
`IterationStrategy` enum with sequential, random, subset, and zipfian patterns. CLI: `--iteration "subset:1000:5000"`.
**Files:** `src/workload/iteration.rs`

### Addressable Spaces (Hash Fields)
**Completed**
`AddressSpec` for unified key/field addressing with independent numeric ranges. Supports:
- Key-only addressing: `AddressSpec::key(prefix, max_id)`
- Hash field literals: `AddressSpec::hash_field(prefix, max_id, field_name)`
- Hash field lists: `AddressSpec::hash_fields(prefix, max_id, vec![fields])`
- Independent key/field ranges: `AddressSpec::hash_numeric_field(key_spec, field_spec)`
**Files:** `src/workload/addressable.rs`, `src/workload/template_factory.rs`

### Parallel Workloads
**Completed**
`ParallelWorkload` for weighted concurrent traffic. CLI: `--parallel "get:0.8,set:0.2"`.
**Files:** `src/workload/parallel.rs`

### Composite Workloads
**Completed**
`CompositeWorkload` for sequential phases with ID passing. CLI: `--composite "vec-load:10000,vec-query:1000"`.
**Files:** `src/workload/composite.rs`

### Per-Workload Configuration
**Completed**
`WorkloadConfig` struct for per-component settings in parallel/composite workloads. Each component can have different key_prefix, keyspace, data_size, search_config.
**Files:** `src/config/workload_config.rs`, `src/workload/parallel.rs`, `src/workload/composite.rs`

### Numeric Filters (Query-Side)
**Completed**
`NumericFilter` with inclusive/exclusive bounds for FT.SEARCH queries. CLI: `--numeric-filter "score:[50,100]"`.
**Files:** `src/config/search_config.rs`, `src/workload/template_factory.rs`

### Optimizer Support
**Completed**
Multi-goal optimization for QPS, latency, and recall with constraint support.

### Tags Support
**Completed**
Tag field support with distributions for vec-load and tag filters for vec-query.

### Ground Truth Vector Pinning
**Completed**
Ground truth vectors are pinned in memory and not evicted.

### Runtime Configuration Management
**Completed**
Set server-side configurations before tests and restore afterward.

### Persistent Configuration Storage
**Completed**
Save/restore configuration from file. Excludes `-t` and `-h` arguments.

### Base Latency Measurement
**Completed**
Measure baseline RTT using PING commands. Enabled by default, disable with `--no-baseline`.

### Wrapper Infrastructure
**Completed**
Python-based wrapper framework in `bench/wrappers/base_wrapper.py`:
- `ValKeyBenchmarkWrapper`, `BenchmarkConfig`, `BenchmarkResult` classes
- `binary_search_max_qps()`, `grid_search()`, `find_max_qps_with_constraints()`
- Stage signaling for external monitoring
- See `bench/wrappers/README.md`

### Max QPS at Target Recall
**Completed**
`bench/scripts/max_qps_recall.py` - Binary search algorithm for QPS discovery.

### Max QPS with Recall and Latency Constraints
**Completed**
`find_max_qps_with_constraints()` method, `--max-p99-latency` flag.

### Optimal Configuration Discovery
**Completed**
`grid_search()` method with filtering support.

### Profiling Integration with Test Stages
**Completed**
`bench/scripts/stage-monitor.sh` monitors `[STAGE:START/END]` signals.

---
## Example Benchmark Commands 
see EXAMPLES.md for more details.