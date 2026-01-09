# Low Level Design: valkey-bench-rs (Rust Implementation)

**Version:** 1.0
**Purpose:** Detailed technical reference for the Rust implementation of valkey-bench-rs

---

## 1. Executive Summary

`valkey-bench-rs` is a high-performance, multi-threaded benchmarking tool for Valkey/Redis written in Rust. The implementation features zero external Redis client dependencies, using a custom mio-based event-driven architecture for maximum throughput and minimal latency.

### 1.1 Core Design Goals

1. **Zero-allocation hot path**: Pre-computed RESP templates with in-place placeholder replacement
2. **Cluster-native**: Full support for slot-based routing and per-node statistics
3. **Vector search native**: FT.SEARCH/FT.CREATE integration with recall verification
4. **Dataset-driven**: Memory-mapped binary dataset loading with ground truth comparison
5. **Optimization-aware**: Adaptive tuning of benchmark parameters to meet constraints

### 1.2 Key Differences from C Implementation

| Aspect | C Implementation | Rust Implementation |
|--------|------------------|---------------------|
| Event Loop | ae (Redis event library) | mio (polling abstraction) |
| Redis Client | libvalkey | Custom RESP codec |
| Threads | pthreads | std::thread |
| Memory | Manual allocation | Ownership + RAII |
| Strings | sds (dynamic strings) | String / Vec<u8> |
| Hash Tables | dict | HashMap |
| Histograms | hdrhistogram-c | hdrhistogram (Rust) |

---

## 2. Architectural Overview

### 2.1 High-Level Component Diagram

```
+------------------------------------------------------------------------------+
|                           valkey-bench-rs                             |
+------------------------------------------------------------------------------+
|                                                                              |
|  +---------------+   +--------------+   +-------------------------------+   |
|  |    Config     |   |  Optimizer   |   |       Metrics Collector       |   |
|  |   (clap CLI)  |   |   (Phased)   |   |  (HDR Histogram, Per-Node)    |   |
|  +-------+-------+   +------+-------+   +--------------+----------------+   |
|          |                  |                          |                     |
|          v                  v                          v                     |
|  +-----------------------------------------------------------------------+  |
|  |                       Benchmark Orchestrator                           |  |
|  |  - Workload sequencing (sequential/parallel scheduling)                |  |
|  |  - Thread pool management (std::thread)                                |  |
|  |  - Global counters (AtomicU64: requests_issued, requests_finished)     |  |
|  +----------------------------------+------------------------------------+  |
|                                     |                                        |
|          +--------------------------|------------------------+               |
|          v                          v                        v               |
|  +--------------+          +--------------+          +--------------+        |
|  |  Worker 0    |          |  Worker 1    |   ...    |  Worker N    |        |
|  | (mio::Poll)  |          | (mio::Poll)  |          | (mio::Poll)  |        |
|  +------+-------+          +------+-------+          +------+-------+        |
|         |                         |                         |                |
|         v                         v                         v                |
|  +-----------------------------------------------------------------------+  |
|  |                        Client Pool (EventClient)                       |  |
|  |  +----------+  +----------+  +----------+       +----------+          |  |
|  |  | Client 0 |  | Client 1 |  | Client 2 |  ...  | Client M |          |  |
|  |  | (node 0) |  | (node 1) |  | (node 2) |       | (node K) |          |  |
|  |  +----------+  +----------+  +----------+       +----------+          |  |
|  +-----------------------------------------------------------------------+  |
|                                     |                                        |
|          +--------------------------|------------------------+               |
|          v                          v                        v               |
|  +-----------------------------------------------------------------------+  |
|  |                      Cluster Topology Manager                          |  |
|  |  - Slot-to-node mapping (16384 slots via [Option<usize>; 16384])       |  |
|  |  - Node selection (primary/replica/all via ReadFromReplica)            |  |
|  |  - MOVED/ASK handling and topology refresh                             |  |
|  +-----------------------------------------------------------------------+  |
|                                     |                                        |
|                                     v                                        |
|  +-----------------------------------------------------------------------+  |
|  |                        Dataset Manager                                 |  |
|  |  - Memory-mapped binary dataset (memmap2: vectors, queries, GT)        |  |
|  |  - Vector ID to cluster tag mapping (ClusterTagMap)                    |  |
|  |  - Recall verification against ground truth                            |  |
|  +-----------------------------------------------------------------------+  |
|                                                                              |
+------------------------------------------------------------------------------+
```

### 2.2 Request Flow Diagram

```
+------------------------------------------------------------------------------+
|                           REQUEST LIFECYCLE                                   |
+------------------------------------------------------------------------------+
|                                                                              |
|  STARTUP PHASE (once per benchmark run):                                     |
|  +------------------+     +------------------+     +------------------+      |
|  | Parse CLI args   |---->| Create command   |---->| Scan template    |      |
|  | via clap         |     | template         |     | for placeholders |      |
|  +------------------+     +------------------+     +--------+---------+      |
|                                                            |                 |
|                                                +-----------v-----------+     |
|                                                | Build PlaceholderOffset|     |
|                                                | Vec for each cmd       |     |
|                                                +-----------+-----------+     |
|                                                            |                 |
|  RUNTIME PHASE (per request batch):                        v                 |
|  +------------------+     +------------------+     +------------------+      |
|  | EventWorker      |---->| fill_placeholders|---->| try_write()      |      |
|  | poll ready       |     | (in-place)       |     | (non-blocking)   |      |
|  +------------------+     +--------+---------+     +--------+---------+      |
|                                    |                        |                |
|          +-------------------------+------------------------+                |
|          |                                                                   |
|          |  PLACEHOLDER REPLACEMENT (PlaceholderType enum):                  |
|          |  +---------------------------------------------------------------+|
|          |  | Key:        claim_next_id() -> format as fixed-width decimal  ||
|          |  | Vector:     get_vector_bytes(idx) -> memcpy from mmap         ||
|          |  | QueryVector: get_query_bytes(idx) -> memcpy from mmap         ||
|          |  | ClusterTag: compute slot -> format as {XXX}                   ||
|          |  | Tag:        fill_tag_placeholder() -> probabilistic tags      ||
|          |  | NumericField: fill_numeric_field() -> distribution-based      ||
|          |  +---------------------------------------------------------------+|
|          |                                                                   |
|  RESPONSE PHASE:                                                             |
|  +------------------+     +------------------+     +------------------+      |
|  | try_read()       |---->| parse_resp_value |---->| Workload-specific|      |
|  | (non-blocking)   |     | (streaming RESP) |     | processing       |      |
|  +------------------+     +------------------+     +--------+---------+      |
|                                                            |                 |
|                          +----------------------------------+                 |
|                          v                                                   |
|  +-----------------------------------------------------------------------+  |
|  | WORKLOAD-SPECIFIC RESPONSE HANDLERS (via WorkloadContext trait):       |  |
|  |                                                                         |  |
|  | VectorQueryContext (FT.SEARCH):                                         |  |
|  |   1. Dequeue query_idx from client's tracking VecDeque                  |  |
|  |   2. parse_search_response() -> extract returned document IDs           |  |
|  |   3. datasetGetNeighbors(query_idx) -> load ground truth                |  |
|  |   4. Compute recall = |intersection| / k                                |  |
|  |   5. Update thread-local RecallStats                                    |  |
|  |                                                                         |  |
|  | VectorLoadContext (HSET):                                               |  |
|  |   1. Dequeue inflight dataset_idx from client VecDeque                  |  |
|  |   2. On error: log and count                                            |  |
|  |   3. On success: ClusterTagMap updated during scan phase                |  |
|  |                                                                         |  |
|  | MOVED/ASK errors:                                                        |  |
|  |   1. RespValue::is_moved() / is_ask() detection                         |  |
|  |   2. parse_redirect() -> (slot, host, port)                             |  |
|  |   3. TopologyManager triggers refresh                                   |  |
|  +-----------------------------------------------------------------------+  |
|                                                                              |
+------------------------------------------------------------------------------+
```

---

## 3. Core Data Structures

### 3.1 Configuration (`BenchmarkConfig` struct)

Located in `src/config/benchmark_config.rs`:

```rust
/// Main benchmark configuration
pub struct BenchmarkConfig {
    // Connection
    pub hosts: Vec<ServerAddress>,       // Multiple seed hosts supported
    pub auth: Option<AuthConfig>,        // Username + password
    pub tls: Option<TlsConfig>,          // Certificate-based TLS
    pub dbnum: u8,                        // Database number

    // Cluster Mode
    pub cluster_mode: bool,               // Enable cluster support
    pub read_from_replica: ReadFromReplica,  // RFR strategy

    // Concurrency
    pub clients: usize,                   // Total simulated connections
    pub threads: usize,                   // Worker thread count
    pub pipeline: usize,                  // Commands per pipeline batch

    // Request Control
    pub requests: u64,                    // Total requests to issue
    pub keyspace: u64,                    // Key space size [0, keyspace)
    pub data_size: usize,                 // Value size for SET/HSET
    pub rps: Option<u64>,                 // Rate limit (requests/second)

    // Key Generation
    pub sequential: bool,                 // Sequential vs random keys
    pub seed: u64,                        // RNG seed for reproducibility

    // Iteration Strategy
    pub iteration: IterationStrategy,     // Key selection pattern
    pub address_type: AddressableSpace,   // Key/hash/json addressing

    // Vector Search
    pub search: SearchConfig,             // Index config, ef_search, etc.
    pub dataset: Option<PathBuf>,         // Binary dataset file

    // Output
    pub output_format: OutputFormat,      // Text, JSON, CSV
    pub quiet: bool,                      // Suppress progress bar
    pub verbose: bool,                    // Debug output
}

/// Server address with host:port
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

/// Authentication configuration
pub struct AuthConfig {
    pub username: Option<String>,
    pub password: String,
}

/// Read-from-replica strategy
pub enum ReadFromReplica {
    Primary,           // Only primaries
    PreferReplica,     // Replicas preferred, fallback to primary
    RoundRobin,        // Distribute across all nodes
}
```

### 3.2 Client Structure (`EventClient` struct)

Located in `src/benchmark/event_worker.rs`:

```rust
/// Event-driven client in the mio event loop
struct EventClient {
    /// Mio TCP stream (non-blocking)
    stream: MioTcpStream,

    /// Token for mio registry (identifies client in events)
    token: Token,

    /// Current state machine state
    state: ClientState,  // Idle, Writing, Reading

    /// Write buffer (RESP-encoded command template)
    write_buf: Vec<u8>,

    /// Write position (bytes already sent)
    write_pos: usize,

    /// Read buffer (for response parsing)
    read_buf: Vec<u8>,  // Default 64KB, grows as needed

    /// Read position (bytes received)
    read_pos: usize,

    /// Pipeline state
    pending_responses: usize,

    /// Parsed responses (for batch processing)
    responses: Vec<RespValue>,

    /// Request start time (for latency calculation)
    start_time: Option<Instant>,

    /// Pipeline depth
    pipeline: usize,

    /// Dataset indices in flight (for retry on error)
    inflight_indices: VecDeque<u64>,

    /// Query indices (for recall verification)
    query_indices: VecDeque<u64>,

    /// Slot ownership bitmap (cluster mode)
    owned_slots: Option<Box<[bool; 16384]>>,
}

/// Client state machine
enum ClientState {
    Idle,      // Ready to send new request
    Writing,   // Sending command to socket
    Reading,   // Waiting for response
}
```

### 3.3 Worker Thread (`EventWorker` struct)

Located in `src/benchmark/event_worker.rs`:

```rust
/// Event-driven worker thread
pub struct EventWorker {
    /// Worker thread ID
    id: usize,

    /// mio Poll instance (epoll/kqueue abstraction)
    poll: Poll,

    /// Event buffer
    events: Events,

    /// All clients owned by this worker
    clients: Vec<EventClient>,

    /// Shared global counters (atomics)
    counters: Arc<GlobalCounters>,

    /// Thread-local HDR histogram
    histogram: Histogram<u64>,

    /// Thread-local recall statistics
    recall_stats: RecallStats,

    /// Thread-local RNG (fastrand)
    rng: fastrand::Rng,

    /// Workload context (handles ID claiming, response processing)
    context: Box<dyn WorkloadContext>,

    /// Command template with placeholder info
    template: CommandBuffer,

    /// Cluster topology (for slot routing)
    topology: Option<Arc<ClusterTopology>>,

    /// Configuration reference
    config: Arc<BenchmarkConfig>,
}
```

### 3.4 Command Template System

Located in `src/workload/command_template.rs` and `src/client/benchmark_client.rs`:

```rust
/// Template argument variants
pub enum TemplateArg {
    /// Literal bytes (copied as-is)
    Literal(Vec<u8>),

    /// Placeholder with type and reserved length
    Placeholder {
        ph_type: PlaceholderType,
        len: usize,
    },

    /// Prefixed placeholder (prefix + placeholder in single RESP arg)
    PrefixedPlaceholder {
        prefix: Vec<u8>,
        ph_type: PlaceholderType,
        len: usize,
    },

    /// Key with cluster tag: prefix + {tag} + ":" + key
    PrefixedKeyWithClusterTag {
        prefix: Vec<u8>,
        key_width: usize,  // Fixed 12 digits
    },
}

/// Placeholder types for runtime replacement
pub enum PlaceholderType {
    Key,              // Random/sequential key (fixed-width decimal)
    Vector,           // Database vector (binary blob, HSET)
    QueryVector,      // Query vector (binary blob, FT.SEARCH)
    ClusterTag,       // Cluster routing tag {XXX}
    RandInt,          // Random integer
    Tag,              // Tag field value (variable length, padded)
    Numeric,          // Numeric field (backward compat)
    NumericField(usize),  // Indexed numeric field with distribution
    Field,            // Hash field name (AddressableSpace)
    JsonPath,         // JSON path (AddressableSpace)
}

/// Pre-computed command buffer with placeholder offsets
pub struct CommandBuffer {
    /// RESP-encoded command bytes (template)
    pub bytes: Vec<u8>,

    /// Placeholder offsets per command in pipeline
    /// placeholders[cmd_idx] = Vec of PlaceholderOffset
    pub placeholders: Vec<Vec<PlaceholderOffset>>,

    /// Number of commands in pipeline
    pub pipeline_size: usize,

    /// Bytes per single command
    pub command_len: usize,
}

/// Single placeholder offset
pub struct PlaceholderOffset {
    pub offset: usize,           // Byte offset in write buffer
    pub len: usize,              // Reserved length
    pub placeholder_type: PlaceholderType,
}
```

### 3.5 Cluster Topology

Located in `src/cluster/topology.rs`:

```rust
/// Cluster topology snapshot
pub struct ClusterTopology {
    /// All nodes in the cluster
    pub nodes: Vec<ClusterNode>,

    /// Slot to node index mapping (O(1) lookup)
    slot_map: [Option<usize>; 16384],

    /// Primary node indices
    primary_indices: Vec<usize>,

    /// Replica nodes grouped by primary ID
    replica_map: HashMap<String, Vec<usize>>,
}

/// Single cluster node
pub struct ClusterNode {
    pub id: String,           // Node ID from CLUSTER NODES
    pub host: String,         // IP address
    pub port: u16,            // Port
    pub is_primary: bool,     // Primary vs replica
    pub is_replica: bool,     // Replica flag
    pub slots: Vec<u16>,      // Owned slots (primaries only)
    pub primary_id: Option<String>,  // Primary's ID (replicas only)
    pub shard_id: Option<u16>,       // Shard number (1-based)
    pub shard_index: Option<u16>,    // Index within shard
}
```

### 3.6 Dataset Structures

Located in `src/dataset/`:

```rust
/// Dataset header (128 bytes, matches binary format)
#[repr(C, packed)]
pub struct DatasetHeader {
    pub magic: [u8; 8],           // "VDSET001"
    pub dimensions: u32,          // Vector dimensionality
    pub num_vectors: u64,         // Database size
    pub num_queries: u64,         // Query set size
    pub num_neighbors: u32,       // Ground truth k
    pub data_type: DataType,      // Float32, Float16, etc.
    pub distance_metric: DistanceMetric,  // L2, Cosine, IP
    pub vectors_offset: u64,      // Offset to vector data
    pub queries_offset: u64,      // Offset to query vectors
    pub ground_truth_offset: u64, // Offset to neighbor IDs
    // ... padding to 128 bytes
}

/// Memory-mapped dataset context
pub struct DatasetContext {
    /// Memory-mapped file
    mmap: Mmap,

    /// Parsed header
    header: DatasetHeader,

    /// Direct pointers into mmap'd region (zero-copy)
    vectors: *const f32,
    queries: *const f32,
    ground_truth: *const u32,

    /// Precomputed byte offsets
    vector_stride: usize,  // dim * sizeof(f32)
    query_stride: usize,
}

// Implements Send + Sync via raw pointers (mmap is immutable)
unsafe impl Send for DatasetContext {}
unsafe impl Sync for DatasetContext {}
```

---

## 4. Threading Model

### 4.1 Overview

```
+------------------------------------------------------------------------+
|                         THREAD HIERARCHY                                |
+------------------------------------------------------------------------+
|                                                                        |
|  MAIN THREAD                                                           |
|  +------------------------------------------------------------------+  |
|  | - Parse CLI arguments (clap)                                      |  |
|  | - Discover cluster topology (ControlPlane trait)                  |  |
|  | - Load dataset (memmap2)                                          |  |
|  | - Build command templates (CommandTemplate)                       |  |
|  | - Spawn worker threads (std::thread::spawn)                       |  |
|  | - Display progress bar (indicatif)                                |  |
|  | - Collect and merge results (join handles)                        |  |
|  +------------------------------------------------------------------+  |
|           |                                                            |
|           v spawns N worker threads                                    |
|  +------------------------------------------------------------------+  |
|  |                     WORKER THREADS (N)                            |  |
|  |                                                                    |  |
|  |  Each EventWorker:                                                 |  |
|  |  - Owns M/N clients exclusively (no sharing, no locks)            |  |
|  |  - Runs its own mio::Poll event loop                              |  |
|  |  - Maintains thread-local Histogram<u64>                          |  |
|  |  - Uses thread-local fastrand::Rng                                |  |
|  |                                                                    |  |
|  |  Synchronization (Arc<GlobalCounters> with atomics):              |  |
|  |  - requests_issued: claim quota before sending (fetch_add)        |  |
|  |  - requests_finished: increment after response (fetch_add)        |  |
|  |  - dataset_counter: claim unique vector indices (fetch_add)       |  |
|  |  - key_counter: sequential key generation (fetch_add)             |  |
|  |                                                                    |  |
|  +------------------------------------------------------------------+  |
|                                                                        |
+------------------------------------------------------------------------+
```

### 4.2 Global Counters

Located in `src/benchmark/counters.rs`:

```rust
/// Shared counters for cross-thread synchronization
pub struct GlobalCounters {
    /// Requests issued (claimed quota)
    pub requests_issued: AtomicU64,

    /// Requests finished (responses received)
    pub requests_finished: AtomicU64,

    /// Dataset insert counter (for vec-load)
    pub dataset_counter: AtomicU64,

    /// Query counter (for vec-query)
    pub query_counter: AtomicU64,

    /// Sequential key counter
    pub key_counter: AtomicU64,

    /// Error counter
    pub error_count: AtomicU64,
}

impl GlobalCounters {
    /// Claim request quota (returns true if quota available)
    pub fn claim_requests(&self, count: u64, limit: u64) -> bool {
        let current = self.requests_issued.fetch_add(count, Ordering::SeqCst);
        current < limit
    }

    /// Claim dataset index (returns unique index)
    pub fn claim_dataset_index(&self) -> u64 {
        self.dataset_counter.fetch_add(1, Ordering::SeqCst)
    }
}
```

### 4.3 Thread-Local Resources

Each worker thread maintains:

| Resource | Type | Purpose |
|----------|------|---------|
| `histogram` | `Histogram<u64>` | Latency distribution (HDR) |
| `recall_stats` | `RecallStats` | Vector search recall |
| `rng` | `fastrand::Rng` | Fast random number generation |
| `context` | `Box<dyn WorkloadContext>` | Workload-specific logic |

**Design Decision:** Thread-local histograms are merged only at benchmark completion (via `histogram.add()`), avoiding runtime synchronization.

---

## 5. Event Loop Implementation

### 5.1 mio-based Event Loop

Located in `src/benchmark/event_worker.rs`:

```rust
impl EventWorker {
    /// Main event loop (runs on worker thread)
    pub fn run(&mut self) -> EventWorkerResult {
        loop {
            // Check termination condition
            if self.counters.requests_finished.load(Ordering::SeqCst) >= self.target_requests {
                break;
            }

            // Poll for events (10ms timeout)
            self.poll.poll(&mut self.events, Some(Duration::from_millis(10)))?;

            // Process events
            for event in self.events.iter() {
                let token = event.token();
                let client_idx = token.0;

                if event.is_writable() {
                    self.handle_writable(client_idx)?;
                }
                if event.is_readable() {
                    self.handle_readable(client_idx)?;
                }
            }

            // Check idle clients (start new requests)
            self.check_idle_clients()?;
        }

        // Return thread-local results for merging
        Ok(EventWorkerResult {
            histogram: std::mem::take(&mut self.histogram),
            recall_stats: std::mem::take(&mut self.recall_stats),
            error_count: self.local_error_count,
        })
    }
}
```

### 5.2 Write Handler

```rust
fn handle_writable(&mut self, client_idx: usize) -> io::Result<()> {
    let client = &mut self.clients[client_idx];

    match client.state {
        ClientState::Idle => {
            // Claim request quota
            if !self.counters.claim_requests(self.pipeline as u64, self.target_requests) {
                return Ok(()); // Quota exhausted
            }

            // Fill placeholders
            self.fill_placeholders(client_idx)?;

            // Start request timing
            client.start_request();
        }
        ClientState::Writing => {}
        ClientState::Reading => return Ok(()),
    }

    // Non-blocking write
    match client.try_write() {
        Ok(true) => {
            // Write complete, switch to read mode
            self.poll.registry().reregister(
                &mut client.stream,
                client.token,
                Interest::READABLE,
            )?;
        }
        Ok(false) => {
            // Would block, keep waiting
        }
        Err(e) => {
            self.handle_client_error(client_idx, e)?;
        }
    }

    Ok(())
}
```

### 5.3 Read Handler

```rust
fn handle_readable(&mut self, client_idx: usize) -> io::Result<()> {
    let client = &mut self.clients[client_idx];

    // Non-blocking read
    match client.try_read() {
        Ok(true) => {
            // All responses received
            let latency_us = client.latency_us();

            // Record latency
            self.histogram.record(latency_us)?;

            // Process responses (workload-specific)
            for (i, response) in client.responses.iter().enumerate() {
                if response.is_error() {
                    self.local_error_count += 1;

                    // Check for MOVED/ASK
                    if response.is_moved() || response.is_ask() {
                        // Trigger topology refresh
                        self.topology_stale.store(true, Ordering::SeqCst);
                    }
                } else if let Some(query_idx) = client.query_indices.pop_front() {
                    // Recall verification for vector search
                    self.context.compute_and_record_recall(query_idx, response);
                }
            }

            // Update finished counter
            self.counters.requests_finished.fetch_add(
                client.pipeline as u64,
                Ordering::SeqCst,
            );

            // Reset client for next request
            client.state = ClientState::Idle;

            // Re-register for write
            self.poll.registry().reregister(
                &mut client.stream,
                client.token,
                Interest::WRITABLE,
            )?;
        }
        Ok(false) => {
            // Need more data
        }
        Err(e) => {
            self.handle_client_error(client_idx, e)?;
        }
    }

    Ok(())
}
```

---

## 6. Template and Placeholder System

### 6.1 Template Creation

```rust
/// Build command template for SET workload
pub fn create_set_template(
    config: &BenchmarkConfig,
    cluster_mode: bool,
) -> CommandTemplate {
    let mut template = CommandTemplate::new("SET");

    // Command name
    template = template.arg_str("SET");

    // Key with optional cluster tag
    if cluster_mode {
        template = template.arg_prefixed_key_with_cluster_tag(
            config.prefix.as_bytes(),
            KEY_WIDTH,  // 12 digits
        );
    } else {
        template = template.arg_prefixed_placeholder(
            config.prefix.as_bytes(),
            PlaceholderType::Key,
            KEY_WIDTH,
        );
    }

    // Value (random data placeholder)
    template = template.arg_placeholder(
        PlaceholderType::RandInt,
        config.data_size,
    );

    template
}
```

### 6.2 RESP Encoding

```rust
impl CommandTemplate {
    /// Encode template to RESP with placeholder tracking
    pub fn encode(&self, pipeline: usize) -> CommandBuffer {
        let mut encoder = RespEncoder::with_capacity(4096);
        let mut placeholders = Vec::new();

        for cmd_idx in 0..pipeline {
            let mut cmd_placeholders = Vec::new();
            let cmd_start = encoder.as_bytes().len();

            // Array header
            encoder.buffer_mut().push(b'*');
            encoder.write_int(self.args.len() as i64);
            encoder.buffer_mut().extend_from_slice(b"\r\n");

            for arg in &self.args {
                match arg {
                    TemplateArg::Literal(bytes) => {
                        encoder.encode_bulk_string(bytes);
                    }
                    TemplateArg::Placeholder { ph_type, len } => {
                        // Write placeholder: $<len>\r\n<zeros>\r\n
                        let offset = encoder.as_bytes().len();
                        encoder.buffer_mut().push(b'$');
                        encoder.write_int(*len as i64);
                        encoder.buffer_mut().extend_from_slice(b"\r\n");

                        let data_offset = encoder.as_bytes().len();
                        encoder.buffer_mut().resize(
                            encoder.as_bytes().len() + len,
                            b'0',
                        );
                        encoder.buffer_mut().extend_from_slice(b"\r\n");

                        cmd_placeholders.push(PlaceholderOffset {
                            offset: data_offset - cmd_start,
                            len: *len,
                            placeholder_type: *ph_type,
                        });
                    }
                    // ... other variants
                }
            }

            placeholders.push(cmd_placeholders);
        }

        CommandBuffer::new(encoder.into_bytes(), pipeline)
    }
}
```

### 6.3 Runtime Placeholder Replacement

```rust
fn fill_placeholders(&mut self, client_idx: usize) -> Result<()> {
    let client = &mut self.clients[client_idx];
    let buf = &mut client.write_buf;

    for cmd_idx in 0..self.pipeline {
        for ph in &self.template.placeholders[cmd_idx] {
            let offset = self.template.absolute_offset(cmd_idx, ph.offset);
            let slice = &mut buf[offset..offset + ph.len];

            match ph.placeholder_type {
                PlaceholderType::Key => {
                    // Format key as fixed-width decimal
                    let key_num = if self.config.sequential {
                        self.counters.key_counter.fetch_add(1, Ordering::SeqCst)
                    } else {
                        self.rng.u64(..) % self.config.keyspace
                    };
                    write_fixed_decimal(slice, key_num, ph.len);
                }

                PlaceholderType::Vector => {
                    // Copy vector bytes from mmap'd dataset
                    let idx = self.context.next_dataset_idx(&self.counters)?;
                    if let Some(vec_bytes) = self.context.get_vector_bytes(idx) {
                        slice.copy_from_slice(vec_bytes);
                    }
                    client.inflight_indices.push_back(idx);
                }

                PlaceholderType::QueryVector => {
                    // Copy query vector from dataset
                    let idx = self.counters.query_counter.fetch_add(1, Ordering::SeqCst)
                        % self.context.num_queries();
                    if let Some(query_bytes) = self.context.get_query_bytes(idx) {
                        slice.copy_from_slice(query_bytes);
                    }
                    client.query_indices.push_back(idx);
                }

                PlaceholderType::ClusterTag => {
                    // Compute slot-based tag
                    let tag = self.compute_cluster_tag(key_num)?;
                    slice.copy_from_slice(&tag);
                }

                PlaceholderType::Tag => {
                    // Fill with probabilistic tags
                    self.context.fill_tag_placeholder(key_num, slice);
                }

                PlaceholderType::NumericField(field_idx) => {
                    // Generate numeric value based on distribution
                    let seq = self.counters.key_counter.load(Ordering::SeqCst);
                    self.context.fill_numeric_field(field_idx, key_num, seq, slice);
                }

                // ... other types
            }
        }
    }

    Ok(())
}

/// Format u64 as fixed-width decimal (zero-padded)
fn write_fixed_decimal(buf: &mut [u8], value: u64, width: usize) {
    let mut v = value;
    for i in (0..width).rev() {
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
}
```

---

## 7. RESP Protocol Implementation

Located in `src/utils/resp.rs`:

### 7.1 RESP Value Type

```rust
/// RESP value types (Redis Serialization Protocol)
pub enum RespValue {
    SimpleString(String),   // +OK\r\n
    Error(String),          // -ERR message\r\n
    Integer(i64),           // :1000\r\n
    BulkString(Vec<u8>),    // $6\r\nfoobar\r\n
    Null,                   // $-1\r\n
    Array(Vec<RespValue>),  // *2\r\n...
}

impl RespValue {
    pub fn is_error(&self) -> bool { ... }
    pub fn is_moved(&self) -> bool { ... }
    pub fn is_ask(&self) -> bool { ... }
    pub fn parse_redirect(&self) -> Option<(u16, String, u16)> { ... }
    pub fn as_str(&self) -> Option<&str> { ... }
    pub fn as_bytes(&self) -> Option<&[u8]> { ... }
    pub fn as_i64(&self) -> Option<i64> { ... }
    pub fn as_array(&self) -> Option<&[RespValue]> { ... }
}
```

### 7.2 Streaming Parser

```rust
/// Parse a single RESP value from bytes (streaming)
fn parse_resp_value(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    if data.is_empty() {
        return Err(ParseError::Incomplete);
    }

    match data[0] {
        b'+' => parse_simple_string(data),
        b'-' => parse_error(data),
        b':' => parse_integer(data),
        b'$' => parse_bulk_string(data),
        b'*' => parse_array(data),
        _ => Err(ParseError::Invalid(format!(
            "Invalid RESP type byte: {}",
            data[0]
        ))),
    }
}

fn parse_bulk_string(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    let crlf = find_crlf(data).ok_or(ParseError::Incomplete)?;
    let len: i64 = parse_int(&data[1..crlf])?;

    if len < 0 {
        return Ok((RespValue::Null, crlf + 2));
    }

    let len = len as usize;
    let total = crlf + 2 + len + 2;  // $<len>\r\n<data>\r\n

    if data.len() < total {
        return Err(ParseError::Incomplete);
    }

    let content = data[crlf + 2..crlf + 2 + len].to_vec();
    Ok((RespValue::BulkString(content), total))
}

fn parse_array(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    let crlf = find_crlf(data).ok_or(ParseError::Incomplete)?;
    let count: i64 = parse_int(&data[1..crlf])?;

    if count < 0 {
        return Ok((RespValue::Null, crlf + 2));
    }

    let mut elements = Vec::with_capacity(count as usize);
    let mut pos = crlf + 2;

    for _ in 0..count {
        let (value, consumed) = parse_resp_value(&data[pos..])?;
        elements.push(value);
        pos += consumed;
    }

    Ok((RespValue::Array(elements), pos))
}
```

---

## 8. Workload System

### 8.1 Workload Context Trait

Located in `src/workload/context.rs`:

```rust
/// Trait for workload-specific behavior
pub trait WorkloadContext: Send {
    /// Claim the next key/item ID
    fn claim_next_id(&self, counters: &GlobalCounters) -> Option<u64>;

    /// Get dataset index for vector operations
    fn next_dataset_idx(&self, counters: &GlobalCounters) -> Option<u64>;

    /// Get query vector bytes
    fn get_query_bytes(&self, idx: u64) -> Option<&[u8]>;

    /// Get database vector bytes
    fn get_vector_bytes(&self, idx: u64) -> Option<&[u8]>;

    /// Fill tag placeholder
    fn fill_tag_placeholder(&self, key_num: u64, buf: &mut [u8]);

    /// Fill numeric field placeholder
    fn fill_numeric_field(&self, field_idx: usize, key_num: u64, seq: u64, buf: &mut [u8]);

    /// Compute and record recall
    fn compute_and_record_recall(&mut self, query_idx: u64, response: &RespValue);

    /// Take accumulated metrics
    fn take_metrics(&mut self) -> WorkloadMetrics;

    /// Get item count (for modulo)
    fn num_items(&self) -> u64;

    /// Get query count
    fn num_queries(&self) -> u64;
}
```

### 8.2 Workload Context Implementations

```rust
/// Simple workload context (GET, SET, PING, etc.)
pub struct SimpleContext {
    keyspace: u64,
    iteration: IterationStrategy,
    address_space: AddressableSpace,
}

/// Vector query context (FT.SEARCH)
pub struct VectorQueryContext {
    dataset: Arc<DatasetContext>,
    tag_map: Option<Arc<ClusterTagMap>>,
    recall_stats: RecallStats,
    k: usize,
}

/// Vector load context (HSET with vectors)
pub struct VectorLoadContext {
    dataset: Arc<DatasetContext>,
    tag_map: Option<Arc<ClusterTagMap>>,
    tag_distribution: Option<TagDistributionSet>,
    numeric_fields: Option<NumericFieldSet>,
}

/// Vector delete context
pub struct DeleteContext {
    protected_ids: Arc<ProtectedVectorIds>,
    tag_map: Arc<ClusterTagMap>,
}
```

### 8.3 Parallel Workloads

Located in `src/workload/parallel.rs`:

```rust
/// Parallel workload with weighted traffic distribution
pub struct ParallelWorkload {
    /// Components and their weights
    components: Vec<ParallelComponent>,

    /// Cumulative weights for O(1) selection
    cumulative_weights: Vec<f64>,
}

impl ParallelWorkload {
    /// Select workload based on random value
    pub fn select(&self, rand: f64) -> &ParallelComponent {
        let idx = self.cumulative_weights
            .iter()
            .position(|&w| rand < w)
            .unwrap_or(self.components.len() - 1);
        &self.components[idx]
    }
}

/// Usage in EventWorker:
// let rand = self.rng.f64();
// let component = parallel_workload.select(rand);
// let template = &component.template;
```

### 8.4 Iteration Strategies

Located in `src/workload/iteration.rs`:

```rust
/// Key/vector iteration strategy
pub enum IterationStrategy {
    /// Sequential: 0, 1, 2, ... N-1
    Sequential,

    /// Uniform random with optional seed
    Random { seed: Option<u64> },

    /// Subset: only keys in [start, end)
    Subset { start: u64, end: u64 },

    /// Zipfian: power-law distribution
    Zipfian { skew: f64, seed: Option<u64> },
}

impl IterationStrategy {
    /// Get next key based on strategy
    pub fn next_key(&self, counter: u64, keyspace: u64, rng: &mut fastrand::Rng) -> u64 {
        match self {
            Self::Sequential => counter % keyspace,
            Self::Random { .. } => rng.u64(..) % keyspace,
            Self::Subset { start, end } => {
                start + (rng.u64(..) % (end - start))
            }
            Self::Zipfian { skew, .. } => {
                zipfian_sample(keyspace, *skew, rng)
            }
        }
    }
}
```

---

## 9. Cluster Management

### 9.1 Topology Discovery

Located in `src/cluster/topology.rs`:

```rust
impl ClusterTopology {
    /// Parse CLUSTER NODES response
    pub fn from_cluster_nodes(response: &str) -> Result<Self, String> {
        let mut nodes = Vec::new();
        let mut slot_map = [None; 16384];
        let mut primary_indices = Vec::new();
        let mut replica_map: HashMap<String, Vec<usize>> = HashMap::new();

        for line in response.lines() {
            if let Some(node) = parse_cluster_node_line(line) {
                let idx = nodes.len();

                if node.is_primary {
                    primary_indices.push(idx);
                    for &slot in &node.slots {
                        slot_map[slot as usize] = Some(idx);
                    }
                }

                nodes.push(node);
            }
        }

        // Map replicas to primaries
        for (idx, node) in nodes.iter().enumerate() {
            if node.is_replica {
                if let Some(primary_id) = &node.primary_id {
                    if let Some(replicas) = replica_map.get_mut(primary_id) {
                        replicas.push(idx);
                    }
                }
            }
        }

        Ok(Self { nodes, slot_map, primary_indices, replica_map })
    }

    /// Get node for slot
    pub fn node_for_slot(&self, slot: u16) -> Option<&ClusterNode> {
        self.slot_map[slot as usize].map(|idx| &self.nodes[idx])
    }

    /// Select nodes based on RFR strategy
    pub fn select_nodes(&self, rfr: ReadFromReplica) -> Vec<&ClusterNode> {
        match rfr {
            ReadFromReplica::Primary => {
                self.primary_indices.iter().map(|&i| &self.nodes[i]).collect()
            }
            ReadFromReplica::PreferReplica => {
                // Replicas first, then primaries as fallback
                ...
            }
            ReadFromReplica::RoundRobin => {
                // All nodes
                self.nodes.iter().collect()
            }
        }
    }
}
```

### 9.2 Slot Calculation

```rust
/// CRC16 implementation (XMODEM polynomial)
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Calculate slot for key (with hash tag support)
fn slot_for_key(key: &[u8]) -> u16 {
    // Find hash tag {xxx}
    if let Some(start) = key.iter().position(|&b| b == b'{') {
        if let Some(end) = key[start+1..].iter().position(|&b| b == b'}') {
            if end > 0 {
                return crc16(&key[start+1..start+1+end]) % 16384;
            }
        }
    }
    crc16(key) % 16384
}
```

### 9.3 Cluster Tag Map

Located in `src/cluster/cluster_tag_map.rs`:

```rust
/// Maps vector IDs to their cluster tags (for query routing)
pub struct ClusterTagMap {
    /// Sparse map: vector_id -> cluster_tag
    map: HashMap<u64, [u8; 5]>,

    /// Prefix for key scanning
    prefix: String,

    /// Mutex for thread-safe updates
    lock: RwLock<()>,
}

impl ClusterTagMap {
    /// Build mapping by scanning cluster
    pub fn build_from_cluster(
        topology: &ClusterTopology,
        prefix: &str,
        conn_factory: &dyn ConnectionFactory,
    ) -> Result<Self> {
        let mut map = HashMap::new();

        // Parallel SCAN across all nodes
        for node in topology.primary_nodes() {
            let mut cursor = 0;
            loop {
                let (next_cursor, keys) = scan_node(
                    node,
                    &format!("{}*", prefix),
                    cursor,
                    conn_factory,
                )?;

                for key in keys {
                    if let Some((vector_id, tag)) = parse_key_and_tag(&key) {
                        map.insert(vector_id, tag);
                    }
                }

                cursor = next_cursor;
                if cursor == 0 {
                    break;
                }
            }
        }

        Ok(Self { map, prefix: prefix.to_string(), lock: RwLock::new(()) })
    }

    /// Get cluster tag for vector ID
    pub fn get_tag(&self, vector_id: u64) -> Option<[u8; 5]> {
        let _guard = self.lock.read();
        self.map.get(&vector_id).copied()
    }
}
```

---

## 10. Dataset Integration

### 10.1 Memory-Mapped Dataset

Located in `src/dataset/binary_dataset.rs`:

```rust
impl DatasetContext {
    /// Load dataset from file
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        // Parse header
        let header: DatasetHeader = unsafe {
            std::ptr::read(mmap.as_ptr() as *const DatasetHeader)
        };

        // Validate magic
        if &header.magic != b"VDSET001" {
            return Err(DatasetError::InvalidMagic);
        }

        // Calculate pointers
        let vectors = unsafe {
            mmap.as_ptr().add(header.vectors_offset as usize) as *const f32
        };
        let queries = unsafe {
            mmap.as_ptr().add(header.queries_offset as usize) as *const f32
        };
        let ground_truth = unsafe {
            mmap.as_ptr().add(header.ground_truth_offset as usize) as *const u32
        };

        let vector_stride = header.dimensions as usize * std::mem::size_of::<f32>();

        Ok(Self {
            mmap,
            header,
            vectors,
            queries,
            ground_truth,
            vector_stride,
            query_stride: vector_stride,
        })
    }

    /// Get vector bytes (zero-copy from mmap)
    pub fn get_vector(&self, idx: u64) -> &[u8] {
        let offset = idx as usize * self.vector_stride;
        unsafe {
            std::slice::from_raw_parts(
                (self.vectors as *const u8).add(offset),
                self.vector_stride,
            )
        }
    }

    /// Get ground truth neighbors
    pub fn get_neighbors(&self, query_idx: u64) -> &[u32] {
        let offset = query_idx as usize * self.header.num_neighbors as usize;
        unsafe {
            std::slice::from_raw_parts(
                self.ground_truth.add(offset),
                self.header.num_neighbors as usize,
            )
        }
    }
}
```

### 10.2 Recall Computation

```rust
impl VectorQueryContext {
    fn compute_and_record_recall(&mut self, query_idx: u64, response: &RespValue) {
        // Parse FT.SEARCH response
        let result_ids = parse_search_response(response);

        // Get ground truth
        let gt_ids = self.dataset.get_neighbors(query_idx);

        // Convert to sets for intersection
        let result_set: HashSet<u64> = result_ids.iter().copied().collect();
        let gt_set: HashSet<u64> = gt_ids.iter().map(|&id| id as u64).collect();

        // Compute recall
        let intersection = result_set.intersection(&gt_set).count();
        let recall = intersection as f64 / self.k.min(gt_set.len()) as f64;

        // Record
        self.recall_stats.record(recall);
    }
}

/// Recall statistics (thread-local, merged at end)
pub struct RecallStats {
    pub total_queries: u64,
    pub sum_recall: f64,
    pub min_recall: f64,
    pub max_recall: f64,
    pub perfect_count: u64,  // recall == 1.0
    pub zero_count: u64,     // recall == 0.0
}

impl RecallStats {
    pub fn record(&mut self, recall: f64) {
        self.total_queries += 1;
        self.sum_recall += recall;
        self.min_recall = self.min_recall.min(recall);
        self.max_recall = self.max_recall.max(recall);

        if (recall - 1.0).abs() < f64::EPSILON {
            self.perfect_count += 1;
        }
        if recall < f64::EPSILON {
            self.zero_count += 1;
        }
    }

    pub fn average(&self) -> f64 {
        if self.total_queries > 0 {
            self.sum_recall / self.total_queries as f64
        } else {
            0.0
        }
    }

    pub fn merge(&mut self, other: &RecallStats) {
        self.total_queries += other.total_queries;
        self.sum_recall += other.sum_recall;
        self.min_recall = self.min_recall.min(other.min_recall);
        self.max_recall = self.max_recall.max(other.max_recall);
        self.perfect_count += other.perfect_count;
        self.zero_count += other.zero_count;
    }
}
```

---

## 11. Metrics Collection

### 11.1 HDR Histogram Integration

```rust
/// Worker result with histogram for merging
pub struct EventWorkerResult {
    /// Thread-local histogram
    pub histogram: Histogram<u64>,
    /// Thread-local recall stats
    pub recall_stats: RecallStats,
    /// Error count
    pub error_count: u64,
}

/// Orchestrator merges results from all workers
impl Orchestrator {
    fn merge_results(&self, results: Vec<EventWorkerResult>) -> BenchmarkResult {
        // Merge histograms
        let mut merged_histogram = Histogram::new(3)?;
        for result in &results {
            merged_histogram.add(&result.histogram)?;
        }

        // Merge recall stats
        let mut merged_recall = RecallStats::new();
        for result in &results {
            merged_recall.merge(&result.recall_stats);
        }

        // Sum errors
        let total_errors: u64 = results.iter().map(|r| r.error_count).sum();

        BenchmarkResult {
            histogram: merged_histogram,
            recall_stats: merged_recall,
            error_count: total_errors,
            // ...
        }
    }
}
```

### 11.2 Per-Node Metrics

Located in `src/metrics/node_metrics.rs`:

```rust
/// Per-node metrics tracking
pub struct NodeMetrics {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub ops_completed: AtomicU64,
    pub errors: AtomicU64,
    pub histogram: Mutex<Histogram<u64>>,
}

/// Snapshot for reporting
pub struct NodeMetricsSnapshot {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub ops: u64,
    pub errors: u64,
    pub latency_avg_us: f64,
    pub latency_p99_us: u64,
}
```

---

## 12. Load Optimizer

Located in `src/optimizer/optimizer.rs`:

### 12.1 Optimizer Phases

```rust
/// Optimization phase state machine
pub enum OptimizerPhase {
    /// Initial configuration test
    Feasibility,
    /// Grid sampling of parameter space
    Exploration,
    /// Hill climbing toward optimum
    Exploitation,
    /// Final fine-tuning
    Refinement,
    /// Optimization complete
    Converged,
}

/// Parameter to tune
pub struct TunableParameter {
    pub name: String,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub current: i64,
}

/// Optimizer configuration
pub struct Optimizer {
    /// Parameters being tuned
    parameters: Vec<TunableParameter>,
    /// Objective function(s)
    objectives: Vec<Objective>,
    /// Constraints
    constraints: Vec<Constraint>,
    /// Current phase
    phase: OptimizerPhase,
    /// Best configuration found
    best_config: Option<BestConfig>,
    /// History for convergence detection
    history: Vec<Measurement>,
}
```

### 12.2 Objective and Constraint Format

```rust
/// Optimization objective
pub enum Objective {
    Maximize { metric: Metric, bound: Option<Bound> },
    Minimize { metric: Metric, bound: Option<Bound> },
}

/// Constraint on metrics
pub struct Constraint {
    pub metric: Metric,
    pub operator: Operator,  // Gt, Gte, Lt, Lte, Eq
    pub value: f64,
}

/// Available metrics
pub enum Metric {
    Qps,
    Recall,
    P50Ms,
    P95Ms,
    P99Ms,
    P999Ms,
    MeanLatencyMs,
    ErrorRate,
}
```

---

## 13. Error Handling

Located in `src/utils/error.rs`:

```rust
/// Top-level error type
#[derive(Error, Debug)]
pub enum BenchmarkError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection error: {0}")]
    Connection(#[from] ConnectionError),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Dataset error: {0}")]
    Dataset(#[from] DatasetError),

    #[error("Cluster error: {0}")]
    Cluster(#[from] ClusterError),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Worker error: {0}")]
    Worker(String),
}

/// Connection-specific errors
#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("Failed to connect to {host}:{port}: {source}")]
    ConnectFailed { host: String, port: u16, source: io::Error },

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("TLS handshake failed: {0}")]
    TlsFailed(String),

    #[error("Connection closed unexpectedly")]
    Closed,
}

/// Result type alias
pub type Result<T> = std::result::Result<T, BenchmarkError>;
```

---

## 14. Build and Dependencies

### 14.1 Cargo.toml Overview

```toml
[dependencies]
# CLI
clap = { version = "4", features = ["derive"] }

# Event-driven I/O
mio = { version = "1", features = ["os-poll", "net"] }

# Metrics
hdrhistogram = "7"

# Dataset
memmap2 = "0.9"

# Random
fastrand = "2"

# Progress
indicatif = "0.17"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Error handling
thiserror = "1"
anyhow = "1"

# TLS (optional)
native-tls = { version = "0.2", optional = true }
rustls = { version = "0.23", optional = true }

[features]
default = ["native-tls-backend"]
native-tls-backend = ["native-tls"]
rustls-backend = ["rustls", "rustls-pemfile", "webpki-roots"]
```

### 14.2 Build Commands

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# With rustls instead of native-tls
cargo build --release --no-default-features --features rustls-backend

# Run tests
cargo test

# Run with logging
RUST_LOG=debug ./target/release/valkey-bench-rs ...
```

---

## 15. Summary

This LLD documents the Rust implementation of `valkey-bench-rs`, covering:

1. **Architecture**: Multi-threaded, mio-based event-driven design
2. **Zero-allocation**: Template-based RESP encoding with in-place placeholder replacement
3. **Cluster support**: Full topology discovery, slot routing, MOVED/ASK handling
4. **Dataset integration**: Memory-mapped binary datasets with ground truth verification
5. **Workload system**: WorkloadContext trait for extensible workload types
6. **Parallel/composite**: Mixed traffic and sequential phase execution
7. **Optimization**: Multi-objective parameter tuning with constraints
8. **Metrics**: HDR histograms, recall statistics, per-node analysis

The Rust implementation provides equivalent functionality to the original C version while leveraging Rust's ownership model for memory safety, type system for correctness, and zero-cost abstractions for performance.
