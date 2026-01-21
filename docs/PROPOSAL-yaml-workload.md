# Proposal: YAML Workload Definition

**Status**: Proposed
**Author**: Design Discussion
**Created**: 2024-12-23

## Overview

Add support for defining complex multi-stage benchmark workloads via YAML files. This provides a declarative, version-controllable way to express benchmark scenarios that currently require complex CLI flags.

## Motivation

Current approach requires unwieldy command lines for multi-stage benchmarks:

```bash
./valkey-bench-rs --composite "set:10000000:sequential,set:1000000,..." \
  --parallel "get:80,set:20" -r 10000000 -d 100 ...
```

A YAML-based approach offers:
- Readable, self-documenting benchmark definitions
- Version control for benchmark configurations
- Reusable benchmark templates
- Per-stage and per-command configuration

## Design

### Hierarchy

```
Application (top level)
├── clients, threads, rfr (application-wide settings)
├── data_sources (named data definitions)
└── stages (sequential execution)
    └── commands (parallel execution with ratios)
```

### YAML Schema

```yaml
version: 1

application:
  name: benchmark_name
  clients: 100
  threads: 16
  rfr: prefer-replica          # optional: primary, prefer-replica, round-robin

  # Named data sources
  data_sources:
    kv_data:
      type: generated
      keyspace: 10000000
      key_pattern: "key:%012d"
      value_size: 500

    vectors:
      type: dataset
      schema: datasets/cohere-10m.yaml
      binary: datasets/cohere-10m.bin

  # Stages execute sequentially
  stages:
    - name: stage_name
      requests: 1000000         # total requests for this stage
      commands:
        - type: set             # command type
          data: kv_data         # reference to data_sources
          ratio: 80             # for parallel execution
          iteration: sequential # optional: sequential, random, zipfian
          pipeline: 10          # optional
        - type: get
          data: kv_data
          ratio: 20
```

## Examples

### Example 1: KV Progressive Benchmark

```yaml
version: 1

application:
  name: kv_progressive
  clients: 100
  threads: 16

  data_sources:
    kv_data:
      type: generated
      keyspace: 10000000
      key_pattern: "key:%012d"
      value_size: 100

  stages:
    - name: prefill
      requests: 10000000
      commands:
        - type: set
          data: kv_data
          iteration: sequential

    - name: random_writes
      requests: 1000000
      commands:
        - type: set
          data: kv_data

    - name: mix_90_10
      requests: 1000000
      commands:
        - type: set
          data: kv_data
          ratio: 90
        - type: get
          data: kv_data
          ratio: 10

    - name: mix_80_20
      requests: 1000000
      commands:
        - type: set
          data: kv_data
          ratio: 80
        - type: get
          data: kv_data
          ratio: 20

    - name: mix_50_50
      requests: 1000000
      commands:
        - type: set
          data: kv_data
          ratio: 50
        - type: get
          data: kv_data
          ratio: 50

    - name: pure_reads
      requests: 1000000
      commands:
        - type: get
          data: kv_data
```

### Example 2: Vector Search Benchmark

```yaml
version: 1

application:
  name: vector_benchmark
  clients: 50
  threads: 16

  data_sources:
    vectors:
      type: dataset
      schema: datasets/cohere-10m.yaml
      binary: datasets/cohere-10m.bin

  stages:
    - name: load_vectors
      commands:
        - type: vec-load
          data: vectors
          index: cohere_10m
          algorithm: HNSW
          distance: COSINE
          ef_construction: 200
          hnsw_m: 16

    - name: query_fast
      requests: 10000
      commands:
        - type: vec-query
          data: vectors
          index: cohere_10m
          k: 10
          ef_search: 50

    - name: query_accurate
      requests: 10000
      commands:
        - type: vec-query
          data: vectors
          index: cohere_10m
          k: 10
          ef_search: 400
```

### Example 3: Mixed KV + Vector Workload

```yaml
version: 1

application:
  name: mixed_workload
  clients: 100
  threads: 16

  data_sources:
    kv_data:
      type: generated
      keyspace: 1000000
      key_pattern: "cache:%08d"
      value_size: 256

    products:
      type: dataset
      schema: datasets/products.yaml
      binary: datasets/products.bin

  stages:
    - name: load_all
      commands:
        - type: set
          data: kv_data
          requests: 1000000
          iteration: sequential

    - name: load_vectors
      commands:
        - type: vec-load
          data: products
          index: products_idx

    - name: mixed_traffic
      requests: 100000
      commands:
        - type: get
          data: kv_data
          ratio: 50
        - type: set
          data: kv_data
          ratio: 20
        - type: vec-query
          data: products
          index: products_idx
          k: 10
          ef_search: 100
          ratio: 30
```

### Example 4: Filtered Vector Search

```yaml
version: 1

application:
  name: filtered_search
  clients: 50
  threads: 16

  data_sources:
    products:
      type: dataset
      schema: datasets/products.yaml
      binary: datasets/products.bin

  stages:
    - name: load
      commands:
        - type: vec-load
          data: products
          index: products_idx
          algorithm: HNSW
          distance: COSINE

    - name: unfiltered_query
      requests: 10000
      commands:
        - type: vec-query
          data: products
          index: products_idx
          k: 10
          ef_search: 100

    - name: tag_filtered
      requests: 10000
      commands:
        - type: vec-query
          data: products
          index: products_idx
          k: 10
          ef_search: 100
          tag_field: category
          tag_filter: "electronics|clothing"

    - name: numeric_filtered
      requests: 10000
      commands:
        - type: vec-query
          data: products
          index: products_idx
          k: 10
          ef_search: 100
          numeric_filter: "price:[10,500]"
```

## Command Parameters Reference

### All Commands

| Parameter | Type | Description |
|-----------|------|-------------|
| `type` | string | Command type (required) |
| `data` | string | Reference to data_sources entry |
| `ratio` | int | Weight for parallel execution (default: 100) |
| `requests` | int | Override stage-level request count |

### KV Commands (set, get, del)

| Parameter | Type | Description |
|-----------|------|-------------|
| `iteration` | string | `sequential`, `random`, `zipfian:skew` |
| `pipeline` | int | Pipeline depth |

### Hash Commands (hset, hget)

| Parameter | Type | Description |
|-----------|------|-------------|
| `fields` | list | Field names for HSET |
| `field` | string | Field name for HGET |

### Vector Load (vec-load)

| Parameter | Type | Description |
|-----------|------|-------------|
| `index` | string | Index name |
| `algorithm` | string | `HNSW` or `FLAT` |
| `distance` | string | `L2`, `COSINE`, `IP` |
| `ef_construction` | int | HNSW build parameter |
| `hnsw_m` | int | HNSW max connections |

### Vector Query (vec-query)

| Parameter | Type | Description |
|-----------|------|-------------|
| `index` | string | Index name |
| `k` | int | Number of neighbors |
| `ef_search` | int | HNSW search parameter |
| `nocontent` | bool | Return only keys |
| `tag_field` | string | Tag field for filtering |
| `tag_filter` | string | Tag filter expression |
| `numeric_filter` | string | Numeric filter expression |

## Data Source Types

### Generated Data

For KV workloads with synthetic data:

```yaml
data_sources:
  my_data:
    type: generated
    keyspace: 10000000           # Number of unique keys
    key_pattern: "key:%012d"  # Key format
    value_size: 500              # Value size in bytes
```

### Dataset Reference

For vector workloads or pre-recorded data:

```yaml
data_sources:
  my_vectors:
    type: dataset
    schema: datasets/vectors.yaml    # Schema YAML path
    binary: datasets/vectors.bin     # Binary data path
```

## Implementation

### New Files

| File | Description |
|------|-------------|
| `src/config/workload.rs` | YAML schema structs and loader |
| `src/config/workload_executor.rs` | Convert to BenchmarkConfig and execute |

### Modified Files

| File | Change |
|------|--------|
| `src/config/cli.rs` | Add `--workload` flag |
| `src/config/mod.rs` | Export new modules |
| `src/main.rs` | Handle workload execution |
| `Cargo.toml` | Add `serde_yaml` dependency |

### Rust Structs

```rust
#[derive(Debug, Deserialize)]
pub struct WorkloadDefinition {
    pub version: u8,
    pub application: Application,
}

#[derive(Debug, Deserialize)]
pub struct Application {
    pub name: String,
    pub clients: usize,
    pub threads: usize,
    #[serde(default)]
    pub rfr: Option<String>,
    pub data_sources: HashMap<String, DataSource>,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum DataSource {
    #[serde(rename = "generated")]
    Generated {
        keyspace: u64,
        key_pattern: String,
        value_size: usize,
    },
    #[serde(rename = "dataset")]
    Dataset {
        schema: PathBuf,
        binary: PathBuf,
    },
}

#[derive(Debug, Deserialize)]
pub struct Stage {
    pub name: String,
    #[serde(default)]
    pub requests: Option<u64>,
    pub commands: Vec<CommandSpec>,
}

#[derive(Debug, Deserialize)]
pub struct CommandSpec {
    #[serde(rename = "type")]
    pub cmd_type: String,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub ratio: Option<u32>,
    #[serde(default)]
    pub requests: Option<u64>,
    #[serde(default)]
    pub iteration: Option<String>,
    #[serde(default)]
    pub pipeline: Option<usize>,

    // Vector search options
    #[serde(default)]
    pub index: Option<String>,
    #[serde(default)]
    pub k: Option<usize>,
    #[serde(default)]
    pub ef_search: Option<usize>,
    #[serde(default)]
    pub ef_construction: Option<usize>,
    #[serde(default)]
    pub hnsw_m: Option<usize>,
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub distance: Option<String>,
    #[serde(default)]
    pub nocontent: Option<bool>,
    #[serde(default)]
    pub tag_field: Option<String>,
    #[serde(default)]
    pub tag_filter: Option<String>,
    #[serde(default)]
    pub numeric_filter: Option<String>,

    // Hash options
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub field: Option<String>,
}
```

### CLI Usage

```bash
# Run workload from YAML
./valkey-bench-rs -h $HOST --cluster --workload workloads/benchmark.yaml

# Override application settings
./valkey-bench-rs -h $HOST --cluster --workload workloads/benchmark.yaml \
  --clients 200 --threads 32
```

## Execution Flow

1. Parse `--workload` YAML file
2. Apply application-level settings (clients, threads, rfr)
3. Load all referenced data sources (memory-map datasets)
4. For each stage:
   a. Print stage name
   b. Build parallel config from commands with ratios
   c. Apply command-specific settings
   d. Execute using existing orchestrator
   e. Report metrics
5. Print summary

## Scope Exclusions (Future Work)

The following are explicitly out of scope for initial implementation:

- **Address Space Tracking**: No tracking of created/deleted keys between stages
- **Delete Impact**: Delete operations don't affect subsequent stage address spaces
- **Multiple Applications**: Only single application per workload file
- **Conditional Stages**: No if/else or loop constructs
- **Stage Dependencies**: Stages always run sequentially, no DAG execution

## Estimated Effort

| Component | Effort |
|-----------|--------|
| YAML parsing structs | 1 hour |
| Workload loader | 2 hours |
| Stage executor | 2 hours |
| CLI integration | 30 min |
| Testing | 2 hours |
| Documentation | 1 hour |
| **Total** | **~8 hours** |

## Testing Plan

1. Unit tests for YAML parsing
2. Unit tests for config conversion
3. Integration tests with simple workloads
4. End-to-end test against real cluster
5. Verify output matches equivalent CLI invocations

## Success Criteria

- [ ] Parse YAML workload files without errors
- [ ] Execute all stage types correctly
- [ ] Support both generated and dataset data sources
- [ ] Output per-stage metrics
- [ ] Equivalent results to CLI-based invocations

## References

- [DATASETS.md](DATASETS.md) - Dataset schema format
- [EXAMPLES.md](EXAMPLES.md) - CLI examples for reference
- [BENCHMARKING.md](BENCHMARKING.md) - Benchmarking guide
