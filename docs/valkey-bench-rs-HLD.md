# High-Level Design: valkey-bench-rs

**Version:** 2.0
**Purpose:** Architectural overview of the Rust benchmark tool for Valkey/Redis

---

## 1. Executive Summary

`valkey-bench-rs` is a high-performance benchmarking tool written in Rust, designed for Valkey/Redis with first-class support for vector search operations. The tool features zero external Redis client dependencies, using a custom mio-based event-driven architecture for maximum throughput and minimal latency.

### 1.1 Key Design Principles

| Principle | Implementation |
|-----------|----------------|
| Zero Dependencies | Custom RESP codec, no redis-rs or external clients |
| Event-Driven I/O | mio polling with non-blocking sockets |
| Lock-Free Hot Path | Atomic counters, thread-local histograms |
| Memory Efficiency | Pre-allocated buffers, in-place placeholder replacement |
| Cluster Native | Full topology discovery, slot routing, MOVED/ASK handling |

### 1.2 Supported Features

- **Multiple Workloads**: PING, GET, SET, INCR, HSET, LPUSH, RPUSH, LPOP, RPOP, LRANGE, SADD, SPOP, ZADD, ZPOPMIN, MSET, and vector operations
- **Parallel Workloads**: Mixed traffic with weighted distribution (e.g., 80% GET, 20% SET)
- **Composite Workloads**: Sequential phases for setup-then-test patterns
- **Iteration Strategies**: Sequential, random, subset, and zipfian key access patterns
- **Addressable Spaces**: Hash fields, JSON paths, and channels beyond simple keys
- **Cluster Mode**: Automatic topology discovery, read-from-replica strategies, backend detection
- **Vector Search**: FT.CREATE, FT.SEARCH with recall@k computation
- **Filtered Search**: Tag and numeric field support with configurable distributions
- **Rate Limiting**: Token bucket for controlled load testing
- **TLS/SSL**: Full certificate authentication support
- **CLI Mode**: Interactive command interface (valkey-cli compatible)
- **Parameter Optimizer**: Multi-objective optimization with constraints and adaptive duration
- **Base RTT Measurement**: Network baseline measurement for normalized results
- **JSON/CSV Output**: Machine-readable results for CI/CD integration

---

## 2. Architecture Overview

### 2.1 System Layers

```
+-------------------------------------------------------------------------+
|                           valkey-bench-rs                        |
+-------------------------------------------------------------------------+
|                                                                         |
|  +-------------------+  +-------------------+  +---------------------+  |
|  |   Configuration   |  |    Orchestrator   |  |  Metrics Reporter   |  |
|  |  - CLI parsing    |  |  - Thread spawn   |  |  - HDR histograms   |  |
|  |  - TLS/Auth       |  |  - Progress bar   |  |  - Recall stats     |  |
|  |  - Workload setup |  |  - Result merge   |  |  - JSON/text output |  |
|  +--------+----------+  +---------+---------+  +----------+----------+  |
|           |                       |                       |             |
|           v                       v                       v             |
|  +----------------------------------------------------------------------+
|  |                        Event Workers (N threads)                      |
|  |  +----------------+  +----------------+       +----------------+      |
|  |  |   Worker 0     |  |   Worker 1     |  ...  |   Worker N     |      |
|  |  | +------------+ |  | +------------+ |       | +------------+ |      |
|  |  | | mio Poll   | |  | | mio Poll   | |       | | mio Poll   | |      |
|  |  | +------------+ |  | +------------+ |       | +------------+ |      |
|  |  | | Clients    | |  | | Clients    | |       | | Clients    | |      |
|  |  | | [0..M/N]   | |  | | [0..M/N]   | |       | | [0..M/N]   | |      |
|  |  | +------------+ |  | +------------+ |       | +------------+ |      |
|  |  | Thread-local: |  | Thread-local: |       | Thread-local: |       |
|  |  | - histogram   |  | - histogram   |       | - histogram   |       |
|  |  | - RNG         |  | - RNG         |       | - RNG         |       |
|  |  +----------------+  +----------------+       +----------------+      |
|  +----------------------------------------------------------------------+
|                                    |                                     |
|                                    v                                     |
|  +----------------------------------------------------------------------+
|  |                          Connection Layer                             |
|  |  +--------------------+  +--------------------+  +-----------------+  |
|  |  |   RawConnection    |  |  BenchmarkClient   |  | ClusterTopology |  |
|  |  | - TCP/TLS streams  |  | - Write buffers    |  | - Node discovery|  |
|  |  | - RESP codec       |  | - Placeholder mgmt |  | - Slot mapping  |  |
|  |  | - Auth handling    |  | - Response parsing |  | - RFR selection |  |
|  |  +--------------------+  +--------------------+  +-----------------+  |
|  +----------------------------------------------------------------------+
|                                                                         |
+-------------------------------------------------------------------------+
```

### 2.2 Request Flow

```
+--------------------------------------------------------------------------+
|                           REQUEST LIFECYCLE                               |
+--------------------------------------------------------------------------+
|                                                                          |
|  1. CLAIM REQUEST QUOTA                                                  |
|     +------------------+                                                 |
|     | atomic_fetch_add |  <-- requests_issued counter                    |
|     | (requests_issued)|                                                 |
|     +--------+---------+                                                 |
|              |                                                           |
|              v                                                           |
|  2. RATE LIMITING (optional)                                             |
|     +------------------+                                                 |
|     | Token Bucket     |  <-- if --rps specified                         |
|     | acquire_tokens() |                                                 |
|     +--------+---------+                                                 |
|              |                                                           |
|              v                                                           |
|  3. PLACEHOLDER REPLACEMENT (in-place, zero-allocation)                  |
|     +------------------+                                                 |
|     | Replace:         |                                                 |
|     | - Random keys    |  random() % keyspace_len                        |
|     | - Dataset vectors|  memcpy from mmap'd region                      |
|     | - Cluster tags   |  lookup from tag map                            |
|     +--------+---------+                                                 |
|              |                                                           |
|              v                                                           |
|  4. SOCKET WRITE (non-blocking)                                          |
|     +------------------+                                                 |
|     | write_buf -> TCP |  <-- mio WRITABLE event                         |
|     | (pipelined cmds) |                                                 |
|     +--------+---------+                                                 |
|              |                                                           |
|              v                                                           |
|  5. SOCKET READ (non-blocking)                                           |
|     +------------------+                                                 |
|     | TCP -> read_buf  |  <-- mio READABLE event                         |
|     | RESP parsing     |                                                 |
|     +--------+---------+                                                 |
|              |                                                           |
|              v                                                           |
|  6. RESPONSE PROCESSING                                                  |
|     +------------------+                                                 |
|     | - Record latency |  thread-local histogram                         |
|     | - Verify recall  |  if FT.SEARCH with ground truth                 |
|     | - Handle errors  |  MOVED/ASK -> queue retry                       |
|     +--------+---------+                                                 |
|              |                                                           |
|              v                                                           |
|  7. UPDATE COUNTERS                                                      |
|     +------------------+                                                 |
|     | atomic_fetch_add |  <-- requests_finished counter                  |
|     +------------------+                                                 |
|                                                                          |
+--------------------------------------------------------------------------+
```

---

## 3. Threading Model

### 3.1 Thread Architecture

```
+------------------------------------------------------------------------+
|                         THREAD HIERARCHY                                |
+------------------------------------------------------------------------+
|                                                                        |
|  MAIN THREAD                                                           |
|  +------------------------------------------------------------------+  |
|  | - Parse CLI arguments                                             |  |
|  | - Discover cluster topology (control plane)                       |  |
|  | - Load dataset (mmap)                                             |  |
|  | - Build command templates                                         |  |
|  | - Spawn worker threads                                            |  |
|  | - Display progress bar                                            |  |
|  | - Collect and merge results                                       |  |
|  +------------------------------------------------------------------+  |
|           |                                                            |
|           v spawns N worker threads                                    |
|  +------------------------------------------------------------------+  |
|  |                     WORKER THREADS (N)                            |  |
|  |                                                                    |  |
|  |  Each worker:                                                      |  |
|  |  - Owns M/N clients exclusively (no sharing)                       |  |
|  |  - Runs its own mio event loop                                     |  |
|  |  - Maintains thread-local histogram                                |  |
|  |  - Uses thread-local RNG (fastrand)                                |  |
|  |                                                                    |  |
|  |  Synchronization (atomic counters only):                           |  |
|  |  - requests_issued: claim quota before sending                     |  |
|  |  - requests_finished: increment after response                     |  |
|  |  - dataset_counter: claim unique vector indices                    |  |
|  |                                                                    |  |
|  +------------------------------------------------------------------+  |
|                                                                        |
+------------------------------------------------------------------------+
```

### 3.2 Synchronization Points

| Counter | Type | Access Pattern | Purpose |
|---------|------|----------------|---------|
| `requests_issued` | AtomicU64 | fetch_add before send | Global request quota |
| `requests_finished` | AtomicU64 | fetch_add after reply | Progress tracking |
| `dataset_counter` | AtomicU64 | fetch_add for unique claim | Vector insertion |
| `seq_key_counter` | AtomicU64 | fetch_add if sequential | Sequential keys |

**Design Decision:** Thread-local histograms are merged only at benchmark completion, avoiding runtime synchronization.

---

## 4. Connection Architecture

### 4.1 Connection Types

```
+------------------------------------------------------------------------+
|                       CONNECTION LAYER                                  |
+------------------------------------------------------------------------+
|                                                                        |
|  RawConnection (Direct TCP/TLS)                                        |
|  +------------------------------------------------------------------+  |
|  | - std::net::TcpStream with non-blocking mode                      |  |
|  | - Optional native-tls wrapper for TLS                             |  |
|  | - Custom RESP protocol encoder/decoder                            |  |
|  | - No external Redis client library                                |  |
|  +------------------------------------------------------------------+  |
|                                                                        |
|  BenchmarkClient (Per-connection state)                                |
|  +------------------------------------------------------------------+  |
|  | - Pre-allocated write buffer (RESP-encoded template)              |  |
|  | - Pre-allocated read buffer                                        |  |
|  | - Placeholder offsets (computed once at init)                      |  |
|  | - Assigned cluster node reference                                  |  |
|  | - Pipeline state (pending responses count)                         |  |
|  | - Query indices queue (for recall verification)                    |  |
|  +------------------------------------------------------------------+  |
|                                                                        |
+------------------------------------------------------------------------+
```

### 4.2 RESP Protocol

The tool implements a custom RESP (Redis Serialization Protocol) codec:

```
Encoding:
  *<num_args>\r\n           -- Array header
  $<len>\r\n<data>\r\n      -- Bulk string for each argument

Decoding:
  +<message>\r\n            -- Simple string
  -<error>\r\n              -- Error
  :<number>\r\n             -- Integer
  $<len>\r\n<data>\r\n      -- Bulk string
  *<count>\r\n...           -- Array
```

---

## 5. Cluster Support

### 5.1 Topology Discovery

```
+------------------------------------------------------------------------+
|                      CLUSTER TOPOLOGY                                   |
+------------------------------------------------------------------------+
|                                                                        |
|  Discovery Flow:                                                       |
|  1. Connect to seed node                                               |
|  2. Execute CLUSTER NODES                                              |
|  3. Parse node list (id, host:port, flags, slots)                      |
|  4. Build slot-to-node mapping (16384 slots)                           |
|  5. Select nodes based on read-from-replica strategy                   |
|                                                                        |
|  Slot Mapping:                                                         |
|  +---------------------------+                                         |
|  | slot_map: [Option<Node>; 16384]                                    |
|  | get_node(key) -> compute_crc16(key) % 16384 -> lookup              |
|  +---------------------------+                                         |
|                                                                        |
|  Read-From-Replica Strategies:                                         |
|  +---------------------------+                                         |
|  | primary        | Only primary nodes (default)                      |
|  | prefer-replica | Prefer replicas, fallback to primary              |
|  | round-robin    | Distribute across all nodes                       |
|  +---------------------------+                                         |
|                                                                        |
+------------------------------------------------------------------------+
```

### 5.2 Error Handling

```
MOVED <slot> <host>:<port>
  -> Refresh cluster topology
  -> Retry request to new node

ASK <slot> <host>:<port>
  -> Send ASKING command to target
  -> Retry request to target node

CLUSTERDOWN
  -> Wait and retry
```

---

## 6. Workload System

### 6.1 Command Templates

Templates are RESP-encoded command buffers with placeholder markers:

```
Template Structure:
  +------------------+------------------+------------------+
  | Static RESP      | Placeholder      | Static RESP      |
  | (command name)   | (key/vector)     | (options)        |
  +------------------+------------------+------------------+

Example - HSET for vector insert:
  *4                     -- 4 arguments
  $4 HSET               -- command
  $17 {clt}vec:000000   -- key with cluster tag + dataset key placeholder
  $9 embedding          -- field name
  $3072 <vector bytes>  -- vector data placeholder (768 dim * 4 bytes)
```

### 6.2 Placeholder Types

| Placeholder | Purpose | Replacement |
|-------------|---------|-------------|
| `{clt}` | Cluster routing tag | 5-char slot-based tag |
| Random key | Key identifier | Fixed-width decimal (12 chars) |
| Dataset vector | Vector blob | Raw f32 bytes from mmap |
| Dataset key | Vector ID | Claimed dataset index |

### 6.3 Workload Types

| Type | Command | Description |
|------|---------|-------------|
| ping | PING | Basic connectivity |
| get | GET key | Read operation |
| set | SET key value | Write operation |
| incr | INCR key | Increment counter |
| hset | HSET key field value | Hash write |
| lpush/rpush | LPUSH/RPUSH key value | List push |
| lpop/rpop | LPOP/RPOP key | List pop |
| lrange100-600 | LRANGE key 0 N | List range read |
| sadd | SADD key member | Set add |
| spop | SPOP key | Set pop |
| zadd | ZADD key score member | Sorted set add |
| zpopmin | ZPOPMIN key | Sorted set pop |
| mset | MSET k1 v1 ... k10 v10 | Multi-key set |
| vec-load | HSET with vector | Load vectors into index |
| vec-query | FT.SEARCH | Vector similarity search |
| vec-delete | DEL key | Delete vector keys |
| vec-update | HSET with vector | Update existing vectors |

### 6.4 Parallel Workloads

Mixed traffic with weighted distribution for realistic scenarios:

```
--parallel "get:0.8,set:0.2"     # 80% GET, 20% SET
--parallel "get:0.7,set:0.2,del:0.1"   # Multi-workload mix
```

Each request randomly selects a workload based on the weights, simulating production traffic patterns.

### 6.5 Composite Workloads

Sequential phases for setup-then-test patterns:

```
--composite "vec-load:10000,vec-query:50000"
```

Runs vec-load for 10,000 requests, then vec-query for 50,000 requests. Useful for benchmarking query performance on freshly loaded data.

### 6.6 Iteration Strategies

Key/vector selection patterns (`--iteration`):

| Strategy | Format | Description |
|----------|--------|-------------|
| sequential | `sequential` | Keys 0, 1, 2, ... N-1 |
| random | `random[:seed]` | Uniform random with optional seed |
| subset | `subset:start:end` | Only keys in range [start, end) |
| zipfian | `zipfian:skew[:seed]` | Power-law distribution (skew 0.5-2.0) |

### 6.7 Addressable Spaces

Beyond simple keys, the tool supports structured data (`--address-type`):

| Type | Format | Description |
|------|--------|-------------|
| key | `key:prefix` | Simple key with prefix |
| hash | `hash:prefix:field1,field2` | Hash with specified fields |
| json | `json:prefix:$.path` | JSON document with JSON path |
| channel | `channel:prefix` | Pub/Sub channel names |

### 6.8 Numeric Field Configuration

For vec-load, generate numeric fields with configurable distributions (`--numeric-field-config`):

```
Format: name:type:distribution:params

Types: int, float[:decimals], unix_timestamp, iso_datetime, date_only

Distributions:
  uniform:min:max        Uniform random
  zipfian:skew:min:max   Power-law (most values near min)
  normal:mean:stddev     Gaussian
  sequential:start:step  Incrementing
  constant:value         Fixed value
  key_based:min:max      Derived from key number

Example: --numeric-field-config "price:float:uniform:0.99:999.99:2"
```

### 6.9 Tag Distribution

For vec-load, generate tag fields with probabilistic selection (`--search-tags`):

```
--search-tags "electronics:30,clothing:25,home:20"

Each tag has independent probability (0-100%) of being selected.
A vector may have 0, 1, or multiple tags.
Pattern __rand_int__ replaced with random integer.
```

---

## 7. Vector Search

### 7.1 Dataset Format

Binary dataset with memory-mapped access:

```
+------------------+
| Header (128 B)   |  magic, dimensions, counts, offsets
+------------------+
| Database Vectors |  N x dim x sizeof(f32)
+------------------+
| Query Vectors    |  Q x dim x sizeof(f32)
+------------------+
| Ground Truth     |  Q x K neighbor IDs
+------------------+
```

### 7.2 Recall Computation

```
For each FT.SEARCH query:
  1. Extract returned document IDs from response
  2. Load ground truth neighbors for query index
  3. Compute intersection size
  4. recall = |intersection| / k
  5. Aggregate statistics (mean, min, max)
```

---

## 8. Metrics Collection

### 8.1 Latency Tracking

- **HDR Histogram**: High Dynamic Range histograms for accurate percentiles
- **Thread-Local**: Each worker maintains its own histogram
- **Merge at End**: Histograms combined after benchmark completion

### 8.2 Reported Metrics

| Metric | Description |
|--------|-------------|
| Throughput | Requests per second |
| p50/p95/p99/p999 | Latency percentiles |
| Mean/Max latency | Average and worst case |
| Recall@k | Vector search accuracy |
| Error count | Failed requests |

---

## 9. Rate Limiting

Token bucket algorithm for controlled load:

```
Configuration:
  --rps <N>        Target requests per second

Implementation:
  - time_per_token = 1_000_000_000 / rps (nanoseconds)
  - Atomic timestamp for last token issue
  - Workers wait if tokens unavailable
```

---

## 10. CLI Mode

Interactive command interface compatible with valkey-cli:

```
Features:
  - Connect to standalone or cluster
  - Execute any Redis/Valkey command
  - Format output (bulk strings, arrays, etc.)
  - TLS and authentication support

Usage:
  # Interactive mode
  ./valkey-bench-rs --cli -h host

  # Single command
  ./valkey-bench-rs --cli -h host PING
  ./valkey-bench-rs --cli -h host FT.INFO index
```

---

## 11. Build and Dependencies

### 11.1 Core Dependencies

| Dependency | Purpose |
|------------|---------|
| clap | CLI argument parsing |
| mio | Event-driven I/O |
| hdrhistogram | Latency percentiles |
| memmap2 | Memory-mapped datasets |
| native-tls | TLS support |
| fastrand | Fast random number generation |
| indicatif | Progress bars |

### 11.2 Build Commands

```bash
# Standard build
cargo build --release

# Run tests
cargo test

# With specific TLS backend
cargo build --release --features rustls-backend
```

---

## 12. Performance Characteristics

### 12.1 Design Optimizations

| Optimization | Benefit |
|--------------|---------|
| mio event loop | Non-blocking I/O, no thread-per-connection |
| Pre-allocated buffers | Zero allocation in hot path |
| Thread-local histograms | No runtime lock contention |
| Memory-mapped datasets | Zero-copy vector access |
| Pipeline batching | Amortize network round-trips |

### 12.2 Typical Performance

On a 16xlarge node cluster with 500-byte values:
- GET: ~980K requests/second
- SET: ~380K requests/second
- FT.SEARCH: ~13K queries/second (depending on index size)

---

## 13. Module Structure

```
src/
  main.rs                  Entry point, CLI dispatch, optimization loop
  lib.rs                   Library exports
  cli_mode.rs              Interactive CLI mode (valkey-cli compatible)

  config/
    mod.rs                 Module exports
    cli.rs                 Clap argument definitions
    benchmark_config.rs    Runtime configuration, ServerAddress, AuthConfig
    search_config.rs       Vector search configuration (SearchConfig, NumericBound)
    tls_config.rs          TLS configuration (certificates, skip verify, SNI)
    workload_config.rs     Per-workload configuration for parallel/composite

  client/
    mod.rs                 Module exports
    raw_connection.rs      TCP/TLS connection wrapper, ConnectionFactory
    benchmark_client.rs    Per-client state and buffers, placeholder replacement
    control_plane.rs       ControlPlane trait for cluster discovery

  cluster/
    mod.rs                 Module exports
    topology.rs            Node discovery (CLUSTER NODES), slot mapping
    topology_manager.rs    Dynamic refresh on MOVED/ASK
    node.rs                ClusterNode representation
    backend.rs             Backend detection (ElastiCache, MemoryDB, OSS)
    cluster_tag_map.rs     Vector ID to cluster tag mapping, cluster scanning
    protected_ids.rs       Protected IDs for deletion benchmarks

  benchmark/
    mod.rs                 Module exports
    orchestrator.rs        Thread spawning, result collection, JSON/CSV export
    event_worker.rs        mio-based worker implementation
    counters.rs            GlobalCounters for atomic cross-thread synchronization

  workload/
    mod.rs                 Module exports
    workload_type.rs       WorkloadType enum (Ping, Get, Set, VecLoad, VecQuery, etc.)
    lifecycle.rs           Workload trait (preconfigure, prepare, run, postprocess)
    context.rs             WorkloadContext trait and implementations
    command_template.rs    RESP command template builder with placeholder support
    template_factory.rs    Factory for creating templates for all workload types
    key_format.rs          Key formatting with cluster tag support
    addressable.rs         Addressable spaces (keys, hash fields, JSON paths, channels)
    iteration.rs           Iteration strategies (Sequential, Random, Subset, Zipfian)
    parallel.rs            ParallelWorkload for weighted concurrent execution
    composite.rs           CompositeWorkload for sequential phases with ID passing
    search_ops.rs          FT.CREATE, FT.SEARCH operations, index waiting
    numeric_field.rs       Numeric field generation (distributions: uniform, zipfian, normal)
    tag_distribution.rs    Tag distribution for filtered search (pattern + probability)

  dataset/
    mod.rs                 Module exports
    binary_dataset.rs      DatasetContext for zero-copy memory-mapped vector access
    header.rs              DatasetHeader, magic constants, distance metrics
    source.rs              DataSource and VectorDataSource traits

  metrics/
    mod.rs                 Module exports
    collector.rs           MetricsCollector for aggregating metrics across nodes
    reporter.rs            Output formatting (text/JSON/CSV)
    node_metrics.rs        Per-node metrics tracking (ops, latency, errors)
    snapshot.rs            ClusterSnapshot for temporal comparison and diff
    info_fields.rs         INFO field parsing strategies, aggregation types
    ft_info.rs             FT.INFO parsing for ElastiCache and MemoryDB engines
    backfill.rs            Index backfill progress monitoring and waiting

  optimizer/
    mod.rs                 Module exports
    optimizer.rs           Multi-objective optimizer with constraints and adaptive duration

  utils/
    mod.rs                 Module exports
    resp.rs                RESP protocol encoder/decoder
    error.rs               Error types (BenchmarkError, ClusterError, ConnectionError)
```

---

## 14. See Also

- [README.md](../README.md) - User documentation and CLI reference
- [EXAMPLES.md](EXAMPLES.md) - Comprehensive usage examples
- [valkey-bench-rs-LLD.md](valkey-bench-rs-LLD.md) - Low-Level Design with implementation details
