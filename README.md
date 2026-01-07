# Valkey Benchmark Tool with Valkey-Search Support

A high-performance benchmarking tool for Valkey/Redis clusters, designed to push your infrastructure to its limits while providing accurate, reproducible metrics. It delivers maximum throughput with minimal client-side overhead, ensuring your benchmarks measure server performance rather than client bottlenecks.

Native cluster support includes automatic topology discovery, read-from-replica distribution, and seamless failover handling. The tool provides native support for vector search workloads with recall computation against ground truth, plus integrated tooling to download and prepare standard datasets efficiently via vectordbbench. Datasets are memory-mapped for zero-copy access, eliminating preload overhead even for billion-scale vectors.

The tool supports flexible workload composition through parallel execution (mixed traffic with weighted distribution) and sequential chaining (load-then-query patterns), each with independent configuration. Extended reporting breaks down metrics by cluster node, helping identify performance imbalances across shards. All datasets and workloads can be defined via YAML schemas for reproducible, version-controlled benchmarks.

Key capabilities:
- **Production validation**: Measure throughput, latency, and recall before deployment
- **Capacity planning**: Test cluster limits with realistic mixed read/write patterns
- **Performance tuning**: Built-in optimizer finds optimal configurations for your latency constraints
- **Regression testing**: Reproducible benchmarks with schema-driven datasets for CI/CD pipelines
- **Cluster diagnostics**: Per-node metrics reveal hotspots and shard imbalances

## Upcoming Features

### Keyspace iteration Strategies
Implement various keyspace iteration using https://github.com/zvi-code/keyspace_tracker: sequential, random, subset, zipfian. Keyspace splitting for multiple threads with no overlap and flexible 2 dimentional boundaries. Use keyspace_tracker to track deleted\existing keys and allow combined logic. Enhance with vector space vector-mapper to track existing vectors. Remove the vector-mapper from benchmark and use keyspace_tracker instead.
**Benefits:** Flexible and realistic data access patterns.

## Features

- **High Performance**: Lock-free architecture with thread-local histograms and atomic counters
- **Pipeline Support**: Configurable command pipelining for maximum throughput
- **Cluster Support**: Automatic topology discovery, slot routing, and MOVED/ASK handling
- **Read-From-Replica**: Distribute read traffic across replicas for horizontal scaling
- **Schema-Driven Datasets**: Flexible YAML schema + binary data format for custom datasets
- **Vector Search**: FT.CREATE, FT.SEARCH with recall@k computation against ground truth
- **Filtered Search**: Tag and numeric field support with configurable distributions
- **Multiple Workloads**: PING, GET, SET, HSET, LPUSH, RPUSH, SADD, ZADD, and vector operations
- **Parallel Workloads**: Mixed traffic with weighted distribution (e.g., 80% GET, 20% SET)
- **Composite Workloads**: Sequential phases for setup-then-test patterns
- **Iteration Strategies**: Sequential, random, subset, and zipfian key access patterns
- **Addressable Spaces**: Hash field and JSON path iteration beyond simple keys
- **Rate Limiting**: Token bucket rate limiter for controlled load testing
- **TLS Support**: Full TLS/SSL support with certificate authentication
- **CLI Mode**: Interactive command-line interface (like valkey-cli)
- **JSON Output**: Machine-readable results for CI/CD integration
- **Parameter Optimizer**: Automatic tuning of clients, threads, pipeline, and ef_search to maximize throughput under constraints
- **Base RTT Measurement**: Measures single-client PING and GET-miss latency to establish network baseline
- **Custom Dataset Creation**: Python API (CommandRecorder) for creating benchmark datasets

For comprehensive examples of all features, see [EXAMPLES.md](EXAMPLES.md).

## Supported Platforms

Tested and verified on:
- **Amazon MemoryDB** (cluster mode)
- **Amazon ElastiCache for Valkey** (standalone and cluster mode)
- **Open-source Valkey/Redis** (standalone and cluster mode)

## Building

### Prerequisites

- Rust 1.70+ (install via [rustup](https://rustup.rs/))
- C compiler (for some dependencies)

### Build Commands

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Build and run
cargo run --release -- [OPTIONS]
```

The release binary will be at `target/release/valkey-bench-rs`.

## Usage

### Basic Usage

```bash
# Benchmark PING command
./valkey-bench-rs -h $HOST -p 6379 -t ping

# Benchmark SET/GET with 100 byte values
./valkey-bench-rs -h $HOST -p 6379 -t set,get -d 100

# Cluster mode with auto-discovery
./valkey-bench-rs -h node1 --cluster -t ping

# Read from replicas for higher throughput
./valkey-bench-rs -h node1 --cluster --rfr prefer-replica -t get
```

### CLI Mode (Interactive)

Use the `--cli` flag to run as an interactive command-line interface:

```bash
# Interactive mode
./valkey-bench-rs --cli -h $HOST -p 6379

# Non-interactive: execute a single command
./valkey-bench-rs --cli -h $HOST PING
./valkey-bench-rs --cli -h $HOST INFO server
./valkey-bench-rs --cli -h $HOST SCAN 0 COUNT 10

# With TLS
./valkey-bench-rs --cli -h secure-host --tls --tls-skip-verify PING
```

### Command Line Options

#### Connection Options

| Option | Description | Default |
|--------|-------------|---------|
| `-h, --host <HOST>` | Server hostname (can be repeated) | `127.0.0.1` |
| `-p, --port <PORT>` | Server port | `6379` |
| `-a, --auth <PASSWORD>` | Authentication password | None |
| `--user <USERNAME>` | Authentication username (ACL) | None |
| `--tls` | Enable TLS | `false` |
| `--tls-skip-verify` | Skip TLS certificate verification | `false` |
| `--tls-cert <FILE>` | TLS client certificate | None |
| `--tls-key <FILE>` | TLS client private key | None |
| `--tls-ca-cert <FILE>` | TLS CA certificate | None |
| `--tls-sni <HOST>` | TLS Server Name Indication | None |
| `--dbnum <NUM>` | Database number | `0` |

#### Cluster Options

| Option | Description | Default |
|--------|-------------|---------|
| `--cluster` | Enable cluster mode | `false` |
| `--rfr <STRATEGY>` | Read-from-replica strategy | `primary` |

Read-from-replica strategies:
- `primary` - Always read from primary (default)
- `prefer-replica` - Prefer replicas, fallback to primary
- `round-robin` - Round-robin across all nodes

#### Benchmark Options

| Option | Description | Default |
|--------|-------------|---------|
| `-t, --tests <TESTS>` | Workload types (comma-separated) | `ping` |
| `-c, --clients <NUM>` | Number of parallel connections | `50` |
| `--threads <NUM>` | Number of worker threads | `4` |
| `-P, --pipeline <NUM>` | Pipeline depth | `1` |
| `-n, --requests <NUM>` | Total number of requests | `100000` |
| `-r, --keyspace <NUM>` | Key space size (random keys 0 to N-1) | `1000000` |
| `-d, --data-size <BYTES>` | Data size for SET/HSET values | `3` |
| `--rps <NUM>` | Rate limit (requests per second) | Unlimited |
| `--sequential` | Use sequential keys instead of random | `false` |
| `--seed <NUM>` | Random seed for deterministic key generation | `12345` |

#### CLI Mode Options

| Option | Description | Default |
|--------|-------------|---------|
| `--cli` | Run in interactive CLI mode | `false` |

When `--cli` is specified, trailing arguments are executed as a command.
If no command is given, starts an interactive REPL.

#### Output Options

| Option | Description | Default |
|--------|-------------|---------|
| `-q, --quiet` | Quiet mode (no progress bar) | `false` |
| `-v, --verbose` | Verbose output | `false` |
| `-o, --output <FILE>` | Output file path | None |
| `--output-format <FMT>` | Output format (text, json) | `text` |
| `--csv <FILE>` | CSV file for per-second stats | None |

### Keyspace and Key Distribution

The `-r` (keyspace) option controls how keys are generated for SET/GET benchmarks. Understanding key distribution is critical for proper cache hit rate testing.

#### Deterministic Key Generation

Keys are generated using a deterministic algorithm based on:
- **Seed** (`--seed`): Fixed at 12345 by default for reproducibility
- **Global atomic counter**: Ensures the same key sequence across threads
- **SplitMix64 mixing**: Provides uniform distribution across the keyspace

This means **running SET and GET with the same seed and keyspace produces identical key sequences**, enabling reproducible benchmarks.

#### Random vs Sequential Mode

| Mode | Key Pattern | Use Case |
|------|-------------|----------|
| Random (default) | Uniform random within keyspace | Realistic cache access patterns |
| Sequential (`--sequential`) | 0, 1, 2, ... N-1 | Cache warming, 100% hit rate testing |

#### Understanding Hit Rates with Random Keys

**Important**: Random key distribution causes key collisions due to the birthday paradox.

With N random SET operations on a keyspace of size N:
- Only ~63% unique keys are created (not N unique keys)
- Formula: `unique_keys = N * (1 - e^(-1)) ~ 0.632 * N`

**Example**: 3M SET + 5M GET on 3M keyspace:
- SET creates ~1.9M unique keys (63% of 3M)
- First 3M GET requests hit the same keys as SET (100% hit rate)
- Remaining 2M GET requests hit ~63% of the time
- Total hit rate: (3M + 1.26M) / 5M = **85.2%**

#### Achieving 100% Hit Rate

Two approaches for guaranteed 100% hit rate:

**Option 1: Sequential mode**
```bash
# SET 3M keys sequentially
./valkey-bench-rs -h $HOST -t set -n 3000000 -r 3000000 --sequential

# GET the same keys sequentially
./valkey-bench-rs -h $HOST -t get -n 3000000 -r 3000000 --sequential
```

**Option 2: Match request counts**
```bash
# SET with 3M requests
./valkey-bench-rs -h $HOST -t set -n 3000000 -r 3000000 -d 500

# GET with same count (first 3M keys match SET exactly)
./valkey-bench-rs -h $HOST -t get -n 3000000 -r 3000000
```

#### Benchmark Examples for Maximum Throughput

```bash
# Step 1: Clear the database
./valkey-bench-rs --cli -h $HOST FLUSHALL

# Step 2: SET 3M keys with 500 byte values
./valkey-bench-rs -h $HOST --cluster --rfr no -t set \
  -n 3000000 -r 3000000 -d 500 -c 200 --threads 16 -P 100

# Step 3: GET the same 3M keys (100% hit rate guaranteed)
./valkey-bench-rs -h $HOST --cluster --rfr no -t get \
  -n 3000000 -r 3000000 -c 200 --threads 16 -P 100

# Alternative: More GET requests (will show 85% hit rate due to collisions)
./valkey-bench-rs -h $HOST --cluster --rfr no -t get \
  -n 5000000 -r 3000000 -c 200 --threads 16 -P 100
```

### Workload Types

| Workload | Description |
|----------|-------------|
| `ping` | PING command |
| `set` | SET key value |
| `get` | GET key |
| `incr` | INCR key |
| `hset` | HSET key field value |
| `lpush` | LPUSH key value |
| `rpush` | RPUSH key value |
| `lpop` | LPOP key |
| `rpop` | RPOP key |
| `lrange100` | LRANGE key 0 99 |
| `lrange300` | LRANGE key 0 299 |
| `lrange500` | LRANGE key 0 499 |
| `lrange600` | LRANGE key 0 599 |
| `sadd` | SADD key member |
| `spop` | SPOP key |
| `zadd` | ZADD key score member |
| `zpopmin` | ZPOPMIN key |
| `mset` | MSET with 10 key-value pairs |
| `vec-load` | HSET with vector data (supports --tag-field, --search-tags, --numeric-field-config) |
| `vec-query` | FT.SEARCH KNN query (supports --tag-filter, --numeric-filter for filtered search) |
| `vec-delete` | DEL vector keys |
| `vec-update` | Update existing vector keys (same as vec-load) |

### Examples

#### Basic Benchmarks

```bash
# Simple PING test
./valkey-bench-rs -h $HOST -p 6379 -t ping -n 100000

# SET/GET with 100 clients, pipeline of 10
./valkey-bench-rs -h $HOST -p 6379 -t set,get -c 100 -P 10 -n 1000000

# Rate-limited test at 10,000 requests/sec
./valkey-bench-rs -h $HOST -p 6379 -t set --rps 10000 -n 100000

# Sequential keys for cache warming
./valkey-bench-rs -h $HOST -p 6379 -t set --sequential -r 1000000 -n 1000000
```

#### Cluster Mode

```bash
# Enable cluster mode
./valkey-bench-rs -h $HOST --cluster -t ping

# Read from replicas for higher read throughput
./valkey-bench-rs -h $HOST --cluster --rfr prefer-replica -t get -n 1000000

# Round-robin across all nodes (primary + replicas)
./valkey-bench-rs -h $HOST --cluster --rfr round-robin -t get
```

#### TLS Connection

```bash
# TLS with certificate verification disabled (for testing)
./valkey-bench-rs -h secure-host --tls --tls-skip-verify -t ping

# TLS with CA certificate
./valkey-bench-rs -h secure-host --tls --tls-ca-cert ca.crt -t ping

# TLS with client certificate authentication
./valkey-bench-rs -h secure-host --tls \
  --tls-cert client.crt --tls-key client.key --tls-ca-cert ca.crt -t ping
```

#### Authentication

```bash
# Password authentication
./valkey-bench-rs -h $HOST -p 6379 -a mypassword -t ping

# ACL authentication (username + password)
./valkey-bench-rs -h $HOST -p 6379 --user myuser -a mypassword -t ping
```

#### Console Output

The benchmark displays a compact summary with base network latency and results:

```
valkey-bench-rs v0.1.0
============================================================
Connection: localhost:6379 | cluster(rfr=No)
Base RTT: PING avg=0.12ms p99=0.18ms | GET-miss avg=0.15ms p99=0.22ms
Workload: clients=50 threads=4 pipeline=1 requests=1,000,000 keyspace=1,000,000
Tests: get
============================================================

Running test: GET... (892,456/s)

=== GET ===
Throughput: 892,456 req/s | Requests: 1,000,000 | Duration: 1.12s
Latency (ms): avg=0.21 p50=0.18 p95=0.35 p99=0.52 p99.9=1.23 max=4.56
Keyspace: hits=892,456 misses=107,544 hit-rate=89.2%

============================================================
BENCHMARK COMPLETE
============================================================
GET: 892,456 req/s | avg=0.21ms p50=0.18ms p99=0.52ms max=4.56ms | hit-rate=89.2%
```

The **Base RTT** line shows single-client, no-pipeline latency for PING and GET-miss operations.
This establishes a network baseline that helps normalize results across different network conditions.

#### JSON Output

```bash
# Export results to JSON file
./valkey-bench-rs -h $HOST -p 6379 -t ping,set,get -o results.json --output-format json
```

Output format:
```json
{
  "config": "hosts=[\"localhost:6379\"], clients=50, threads=4, pipeline=1, requests=100000",
  "tests": [
    {
      "summary": {
        "test_name": "PING",
        "throughput": 245832.5,
        "total_ops": 100000,
        "total_errors": 0,
        "duration_secs": 0.407,
        "latency": {
          "mean_ms": 0.195,
          "p50_ms": 0.183,
          "p95_ms": 0.312,
          "p99_ms": 0.456,
          "p999_ms": 1.234,
          "max_ms": 2.567
        },
        "node_count": 1
      },
      "nodes": []
    }
  ]
}
```

## CLI Mode

The `--cli` flag enables an interactive command-line interface similar to `valkey-cli`:

### Interactive Mode

```bash
./valkey-bench-rs --cli -h $HOST

# Output:
# Connecting to localhost:6379...
# Connected to Valkey localhost:6379 (8.2.0)
# Type 'help' for available commands, 'quit' or Ctrl-D to exit.
#
# localhost:6379> PING
# PONG
# localhost:6379> SET mykey "hello world"
# OK
# localhost:6379> GET mykey
# "hello world"
# localhost:6379> quit
```

### Non-Interactive Mode

Execute commands directly by appending them after the connection options:

```bash
# Simple commands
./valkey-bench-rs --cli -h $HOST PING
./valkey-bench-rs --cli -h $HOST INFO server
./valkey-bench-rs --cli -h $HOST DBSIZE

# Commands with arguments
./valkey-bench-rs --cli -h $HOST SET foo bar
./valkey-bench-rs --cli -h $HOST GET foo
./valkey-bench-rs --cli -h $HOST SCAN 0 COUNT 10

# Cluster commands
./valkey-bench-rs --cli -h $HOST CLUSTER INFO
./valkey-bench-rs --cli -h $HOST CLUSTER NODES

# Vector search commands
./valkey-bench-rs --cli -h $HOST FT._LIST
./valkey-bench-rs --cli -h $HOST FT.INFO idx
```

### CLI with TLS and Auth

```bash
# TLS connection
./valkey-bench-rs --cli -h secure-host --tls --tls-skip-verify PING

# With authentication
./valkey-bench-rs --cli -h $HOST -a mypassword INFO server
```

## Parameter Optimization

The optimizer automatically finds optimal parameter configurations by exploring the parameter space and converging on the best settings for your objective.

### Optimizer Options

| Option | Description | Default |
|--------|-------------|---------|
| `--optimize` | Enable optimization mode | `false` |
| `--objective <OBJ>` | Optimization goal(s), comma-separated | `maximize:qps` |
| `--tolerance <N>` | Equivalence tolerance for multi-goal (0.04 = 4%) | `0.04` |
| `--constraint <CONS>` | Add constraint (repeatable) | None |
| `--tune <PARAM>` | Parameter to tune (repeatable) | None |
| `--max-optimize-iterations <N>` | Maximum iterations | `50` |

### Objective Format

Single objective: `<direction>:<metric>` where:
- Direction: `maximize` or `minimize`
- Metrics: `qps`, `recall`, `p50_ms`, `p95_ms`, `p99_ms`, `p999_ms`, `mean_latency_ms`, `error_rate`

Multi-objective (ordered goals): `<goal1>,<goal2>,...`
- Goals are evaluated in order; configs within tolerance on goal N are compared by goal N+1
- Example: `maximize:qps,minimize:p99_ms` - maximize QPS, tiebreak on lowest p99

Bounded objective: `<direction>:<metric>:<op>:<value>`
- Find best value that also satisfies the bound
- Example: `maximize:qps:lt:1000000` - maximize QPS but must be < 1M req/s

### Constraint Format

`<metric>:<operator>:<value>` where:
- Operators: `gt` (>), `gte` (>=), `lt` (<), `lte` (<=), `eq` (=)
- Examples: `recall:gt:0.95`, `p99_ms:lt:0.1`, `qps:gte:100000`

### Parameter Format

`<name>:<min>:<max>:<step>` where:
- Names: `clients`, `threads`, `pipeline`, `ef_search`
- Examples: `clients:10:300:10`, `threads:1:32:1`, `ef_search:10:500:10`

### How It Works

The optimizer uses a three-phase approach:

1. **Feasibility Phase**: Tests maximum parameter values to establish an upper bound
2. **Exploration Phase**: Grid sampling with boundary values (min, 25%, 50%, 75%, max) to understand the parameter space
3. **Exploitation Phase**: Hill climbing with multiple step sizes (1x, 2x, 3x) in all directions to find the optimum

**Adaptive Duration**: Uses shorter runs (100K requests) during exploration and longer runs (500K requests) during exploitation for accuracy.

### Optimization Examples

```bash
# Maximize QPS for GET workload, tune clients and threads
./valkey-bench-rs -h $HOST --cluster -t get -n 100000 \
  --optimize --objective "maximize:qps" \
  --tune "clients:10:300:10" --tune "threads:1:32:1"

# Maximize QPS with p99 latency under 1ms
./valkey-bench-rs -h $HOST --cluster -t get -n 100000 \
  --optimize --objective "maximize:qps" --constraint "p99_ms:lt:1.0" \
  --tune "clients:10:200:10" --tune "threads:1:16:1"

# Multi-objective: maximize QPS, tiebreak on lowest p99 (4% tolerance)
./valkey-bench-rs -h $HOST --cluster -t get -n 100000 \
  --optimize --objective "maximize:qps,minimize:p99_ms" --tolerance 0.04 \
  --tune "clients:10:300:10" --tune "threads:1:32:1"

# Vector search: maximize QPS with recall above 95%
./valkey-bench-rs -h $HOST --cluster -t vec-query \
  --dataset vectors.bin --search-index idx -n 100000 \
  --optimize --objective "maximize:qps" --constraint "recall:gt:0.95" \
  --tune "ef_search:10:500:10" --tune "clients:10:100:10"

# Bounded objective: maximize QPS but must stay under 1M req/s
./valkey-bench-rs -h $HOST --cluster -t get -n 100000 \
  --optimize --objective "maximize:qps:lt:1000000" \
  --tune "clients:10:200:10"

# Minimize p99 latency while maintaining minimum throughput
./valkey-bench-rs -h $HOST --cluster -t get -n 100000 \
  --optimize --objective "minimize:p99_ms" --constraint "qps:gte:500000" \
  --tune "clients:10:200:10" --tune "pipeline:1:20:1"
```

### Optimizer Output

The optimizer prints one line per iteration for compact progress tracking:

```
=== OPTIMIZATION MODE ===

Objectives: maximize:qps, minimize:p99_ms
  Tolerance: 4.0% (configs within this range compared by secondary goals)
Constraints:
  - p99_ms < 1.0
Parameters to tune:
  - clients: 10 to 300 step 10
  - threads: 1 to 32 step 1
Max iterations: 50
Adaptive duration: 100K requests (exploration) -> 500K (exploitation)

[ 1] Feasibility | {clients=300, threads=32} | 892K req/s p99=0.52ms *BEST*
[ 2] Exploration | {clients=10, threads=1} | 245K req/s p99=0.12ms
[ 3] Exploration | {clients=10, threads=32} | 456K req/s p99=0.34ms
... (more iterations)
[25] Exploitation | {clients=275, threads=24} | 1.04M req/s p99=0.41ms *BEST*

=== Optimization Summary ===

Objectives: maximize:qps, minimize:p99_ms
  (tolerance: 4.0% - configs within this range are compared by secondary goals)
Constraints:
  - p99_ms < 1.0
Status: Converged (completed all phases)

=== Best Configuration ===
Config: {clients=275, threads=24}
qps: 1041234.5600

=== Recommended Command Line ===

./valkey-bench-rs -h $HOST --cluster -t get -c 275 --threads 24 -n 1000000

Expected performance: 1.04M req/sec, p99=0.41ms
```

If the optimizer hits the iteration limit without converging:

```
!!! OPTIMIZATION DID NOT CONVERGE !!!
The iteration limit (50) was reached before completing all phases.
The best result found may not be optimal.
```

## Vector Search Benchmarking

### Dataset Format

Vector datasets use a binary format with the following structure:

1. **Header** (128 bytes):
   - Magic number: `VDSET001`
   - Dimensions, vector count, query count, neighbor count
   - Data type (f32, f16, i8, u8, binary)
   - Distance metric (L2, Cosine, InnerProduct)

2. **Sections**:
   - Database vectors
   - Query vectors
   - Ground truth neighbor IDs (for recall computation)

### Dataset Options

| Option | Description | Default |
|--------|-------------|---------|
| `--schema <FILE>` | Schema YAML file (schema-driven format) | None |
| `--data <FILE>` | Binary data file (schema-driven format) | None |
| `--dataset <FILE>` | Legacy binary dataset file with embedded header | None |

**Schema-driven format (recommended):**
```bash
./valkey-bench-rs -h $HOST --cluster \
  --schema datasets/mnist.yaml --data datasets/mnist.bin \
  -t vec-load -n 60000 -c 100
```

**Legacy format:**
```bash
./valkey-bench-rs -h $HOST --cluster \
  --dataset datasets/legacy.bin \
  -t vec-load -n 60000 -c 100
```

### Vector Search Options

| Option | Description | Default |
| `--search-index <NAME>` | Vector index name | `idx` |
| `--search-prefix <PREFIX>` | Key prefix for vectors | `vec:` |
| `--search-vector-field <NAME>` | Vector field name in hash | `embedding` |
| `--search-algorithm <ALG>` | HNSW or FLAT | `HNSW` |
| `--search-distance <M>` | L2, COSINE, or IP | `L2` |
| `--ef-construction <N>` | HNSW build parameter | `200` |
| `--ef-search <N>` | HNSW search parameter (EF_RUNTIME) | `10` |
| `--hnsw-m <N>` | HNSW max connections | `16` |
| `-k, --search-k <N>` | Number of neighbors to return | `10` |
| `--nocontent` | Return only keys, not vector data | `false` |
| `--cleanup` | Delete index after benchmark | `false` |

### Tag and Attribute Options (Filtered Search)

| Option | Description | Default |
|--------|-------------|---------|
| `--tag-field <NAME>` | Tag field name in hash (for filtered search) | None |
| `--search-tags <DIST>` | Tag distribution for vec-load (see format below) | None |
| `--tag-filter <FILTER>` | Tag filter for vec-query FT.SEARCH | None |
| `--tag-max-len <N>` | Maximum tag field payload length | `128` |
| `--numeric-field <NAME>` | Simple numeric field (uses key_num as value) | None |
| `--numeric-field-config <CFG>` | Extended numeric field for vec-load (repeatable, see format below) | None |
| `--numeric-filter <FILTER>` | Numeric range filter for vec-query (repeatable, see format below) | None |

#### Tag Distribution Format

The `--search-tags` option specifies tag patterns and their selection probabilities:

```
pattern:probability,pattern:probability,...
```

- Each tag has an **independent probability** of being selected (0-100%)
- A vector may have 0, 1, or multiple tags based on the probabilities
- Pattern `__rand_int__` is replaced with a random integer (0-999999)

**Examples:**

```bash
# Single tag, always included
--search-tags "electronics:100"

# Multiple tags with different probabilities
--search-tags "electronics:30,clothing:25,home:20,sports:15,other:10"

# Dynamic tags with random suffixes
--search-tags "category_id_:50,tag__rand_int__:100"
```

#### Tag Filter Format

The `--tag-filter` option specifies the filter pattern for vec-query:

```bash
# Single tag filter
--tag-filter "electronics"

# Multiple tags (OR condition)
--tag-filter "electronics|clothing|home"
```

This generates FT.SEARCH queries with the filter prefix:
```
@tag_field:{electronics|clothing|home}=>[KNN 10 @embedding $BLOB]
```

#### Numeric Filter Format (Query-Side)

The `--numeric-filter` option adds numeric range constraints to vec-query FT.SEARCH queries. Can be repeated for multiple filters (AND logic).

**Format:** `field:[min,max]` or `field:(min,max)` for exclusive bounds

```bash
# Inclusive range [50, 100]
--numeric-filter "score:[50,100]"

# Exclusive bounds (50, 100) - excludes 50 and 100
--numeric-filter "price:(10,100)"

# Mixed bounds: > 0 and <= 100
--numeric-filter "rating:(0,100]"

# Unbounded ranges
--numeric-filter "rating:[-inf,4.5]"    # All values <= 4.5
--numeric-filter "count:[100,+inf)"     # All values >= 100

# Multiple filters (AND logic)
--numeric-filter "price:[10,100]" --numeric-filter "rating:[4.0,5.0]"
```

This generates FT.SEARCH queries with numeric filter prefixes:
```
@price:[10 100]=>[KNN 10 @embedding $BLOB]
(@tag:{electronics} @price:[10 100])=>[KNN 10 @embedding $BLOB]
```

#### Numeric Field Configuration Format (Load-Side)

The `--numeric-field-config` option enables adding numeric fields with various value types and distributions. Can be repeated for multiple fields.

**Format:** `name:type:distribution:params...`

**Value Types:**

| Type | Description | Example Output |
|------|-------------|----------------|
| `int` | Integer values | `42`, `1000` |
| `float` or `float:N` | Float with N decimal places (default 6) | `123.45` |
| `unix_timestamp` | Unix timestamp (seconds since epoch) | `1703001234` |
| `iso_datetime` | ISO 8601 datetime | `2024-12-19T15:30:45Z` |
| `date_only` | Date only | `2024-12-19` |

**Distributions:**

| Distribution | Format | Description |
|--------------|--------|-------------|
| `uniform` | `uniform:min:max` | Uniform random between min and max |
| `zipfian` | `zipfian:skew:min:max` | Power-law distribution (skew 0.5-2.0) |
| `normal` | `normal:mean:stddev` | Normal/Gaussian distribution |
| `sequential` | `sequential:start:step` | Sequential values |
| `constant` | `constant:value` | Fixed constant value |
| `key_based` | `key_based:min:max` | Derive from key number (deterministic) |

**Examples:**

```bash
# Price field: float, uniform distribution $0.99-$999.99, 2 decimals
--numeric-field-config "price:float:uniform:0.99:999.99:2"

# Quantity: integer, zipfian (most values low, few high)
--numeric-field-config "quantity:int:zipfian:1.5:1:1000"

# Rating: float, normal distribution centered at 4.0
--numeric-field-config "rating:float:normal:4.0:0.5:1"

# Creation timestamp: Unix timestamp, uniform over 2 years
--numeric-field-config "created_at:unix_timestamp:uniform:1672531200:1735689600"

# Sequential ID: starting at 0, incrementing by 1
--numeric-field-config "seq_id:int:sequential:0:1"

# Constant value
--numeric-field-config "version:int:constant:1"

# Multiple fields in one command
./valkey-bench-rs -h $HOST --cluster -t vec-load \
  --dataset vectors.bin --search-prefix "vec:" --search-index idx \
  --tag-field category --search-tags "electronics:40,clothing:30,books:30" \
  --numeric-field-config "price:float:uniform:0.99:999.99:2" \
  --numeric-field-config "quantity:int:zipfian:1.5:1:1000" \
  --numeric-field-config "rating:float:normal:4.0:0.5:1"
```

### Vector Search Examples

#### Basic Vector Operations

```bash
# Load vectors into database
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --search-prefix "doc:" \
  -t vec-load \
  -n 100000

# Run vector search queries with recall computation
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --search-prefix "doc:" \
  --search-algorithm HNSW \
  --ef-search 100 \
  -k 10 \
  -t vec-query \
  -n 10000
```

#### Filtered Vector Search

Load vectors with category tags, then run filtered queries:

```bash
# Step 1: Create index with TAG field
./valkey-bench-rs --cli -h $HOST \
  "FT.CREATE myindex ON HASH PREFIX 1 doc: SCHEMA embedding VECTOR HNSW 6 TYPE FLOAT32 DIM 128 DISTANCE_METRIC L2 category TAG"

# Step 2: Load vectors with tag distribution
# Each vector gets tags based on probability:
# - 30% get "electronics"
# - 25% get "clothing"
# - 20% get "home"
# - 15% get "sports"
# - 10% get "other"
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --search-prefix "doc:" \
  --tag-field category \
  --search-tags "electronics:30,clothing:25,home:20,sports:15,other:10" \
  -t vec-load \
  -n 100000

# Step 3: Query with single tag filter
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --search-prefix "doc:" \
  --tag-field category \
  --tag-filter "electronics" \
  -k 10 \
  -t vec-query \
  -n 10000

# Step 4: Query with multiple tag filter (OR condition)
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --search-prefix "doc:" \
  --tag-field category \
  --tag-filter "electronics|clothing|home" \
  -k 10 \
  -t vec-query \
  -n 10000

# Step 5: Query with numeric filter (requires index with NUMERIC field)
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --numeric-filter "price:[10,100]" \
  -k 10 \
  -t vec-query \
  -n 10000

# Step 6: Query with combined tag and numeric filters
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --tag-field category \
  --tag-filter "electronics" \
  --numeric-filter "price:[10,100]" \
  --numeric-filter "rating:[4.0,5.0]" \
  -k 10 \
  -t vec-query \
  -n 10000
```

#### High-Cardinality Tags

For scenarios with many unique tag values:

```bash
# Load vectors with random category IDs (high cardinality)
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --search-prefix "doc:" \
  --tag-field user_id \
  --search-tags "user__rand_int__:100" \
  -t vec-load \
  -n 100000
```

#### Vectors with Numeric Attributes

Add numeric fields with configurable distributions. The benchmark automatically creates the index if it doesn't exist:

```bash
# Load vectors with tag and numeric fields (index created automatically)
./valkey-bench-rs -h $HOST -p 6379 \
  --dataset vectors.bin \
  --search-index myindex \
  --search-prefix "doc:" \
  --tag-field category \
  --search-tags "electronics:40,clothing:30,books:30" \
  --numeric-field-config "price:float:uniform:9.99:499.99:2" \
  --numeric-field-config "rating:float:normal:4.0:0.5:1" \
  -t vec-load \
  -n 100000

# Verify data was loaded correctly
./valkey-bench-rs --cli -h $HOST KEYS "doc:*" | head -3
./valkey-bench-rs --cli -h $HOST HMGET doc:000000000001 category price rating
```

**Complete Example with E-commerce Data:**

```bash
# Simulating a product catalog with:
# - category (TAG): electronics 40%, accessories 30%, cables 30%
# - price (NUMERIC): uniform $9.99-$499.99
# - quantity (NUMERIC): zipfian distribution (most items low stock)
# - rating (NUMERIC): normal distribution centered at 4.0
# - created_at (NUMERIC): timestamps over 2 years
#
# The benchmark automatically creates the index with vector, tag, and numeric fields

./valkey-bench-rs -h $HOST --cluster -t vec-load \
  --dataset products.bin \
  --search-prefix "product:" \
  --search-index product_idx \
  --search-vector-field embedding \
  --tag-field category \
  --search-tags "electronics:40,accessories:30,cables:30" \
  --numeric-field-config "price:float:uniform:9.99:499.99:2" \
  --numeric-field-config "quantity:int:zipfian:1.5:1:1000" \
  --numeric-field-config "rating:float:normal:4.0:0.5:1" \
  --numeric-field-config "created_at:unix_timestamp:uniform:1672531200:1735689600" \
  -n 100000 -c 50 --threads 4

# Use --cleanup to drop existing index after benchmark
./valkey-bench-rs -h $HOST --cluster -t vec-load \
  --dataset products.bin \
  --search-index product_idx \
  --cleanup \
  ...
```

## Architecture

### Thread Model

```
┌─────────────────────────────────────────────────────────┐
│                    Orchestrator                         │
│  - Discovers cluster topology                           │
│  - Creates workers and distributes clients              │
│  - Collects and merges results                          │
└─────────────────────────────────────────────────────────┘
                           │
        ┌──────────────────┼──────────────────┐
        ▼                  ▼                  ▼
┌───────────────┐  ┌───────────────┐  ┌───────────────┐
│   Worker 0    │  │   Worker 1    │  │   Worker N    │
│ ┌───────────┐ │  │ ┌───────────┐ │  │ ┌───────────┐ │
│ │ Client 0  │ │  │ │ Client 0  │ │  │ │ Client 0  │ │
│ │ Client 1  │ │  │ │ Client 1  │ │  │ │ Client 1  │ │
│ │    ...    │ │  │ │    ...    │ │  │ │    ...    │ │
│ └───────────┘ │  │ └───────────┘ │  │ └───────────┘ │
│ Thread-local  │  │ Thread-local  │  │ Thread-local  │
│  histogram    │  │  histogram    │  │  histogram    │
└───────────────┘  └───────────────┘  └───────────────┘
```

### Cluster Support

- **Auto-discovery**: Connects to seed node and runs `CLUSTER NODES`
- **Slot routing**: CRC16-based slot calculation with hash tag support
- **Read-from-replica**: Distribute connections across replicas for read scaling
- **MOVED handling**: Automatic topology refresh on MOVED errors
- **ASK handling**: Redirect support for slot migration
- **CLUSTERDOWN**: Wait and retry on cluster failures

### Performance Optimizations

- **Lock-free counters**: Atomic operations for request claiming and progress
- **Thread-local histograms**: No contention during latency recording
- **Pre-computed command templates**: RESP encoding done once, placeholders filled at runtime
- **Pipeline batching**: Multiple commands per network round-trip
- **Memory-mapped datasets**: Zero-copy vector access for large datasets
- **Connection distribution**: Even distribution across nodes for replica reads

## Comparison with redis-benchmark

| Feature | valkey-bench-rs | redis-benchmark |
|---------|------------------------|-----------------|
| Vector search | Yes | No |
| Recall computation | Yes | No |
| Read-from-replica | Yes | No |
| CLI mode | Yes | No |
| Cluster MOVED handling | Yes (with refresh) | Limited |
| JSON output | Yes | Yes |
| TLS support | Yes | Yes |
| Custom workloads | Extensible | Limited |
| Language | Rust | C |

## Troubleshooting

### Connection Issues

```bash
# Test basic connectivity with CLI mode
./valkey-bench-rs --cli -h $HOST PING

# Test benchmark mode
./valkey-bench-rs -h $HOST -p 6379 -t ping -n 1

# Enable debug logging
RUST_LOG=debug ./valkey-bench-rs -h $HOST -p 6379 -t ping
```

### Cluster Issues

If you see MOVED errors in CLI mode:
- This is expected for key commands - CLI mode doesn't auto-redirect
- Use benchmark mode with `--cluster` for automatic slot handling

In benchmark mode:
- The benchmark automatically refreshes cluster topology
- Check that all cluster nodes are accessible
- Verify cluster is not resharding

### Performance Issues

- Increase pipeline depth (`-P 10` or higher)
- Increase client count (`-c 100`)
- Increase thread count (`--threads 8`)
- Enable read-from-replica for read workloads (`--rfr prefer-replica`)
- Use release build (`cargo build --release`)

