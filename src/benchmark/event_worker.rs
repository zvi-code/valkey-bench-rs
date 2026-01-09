//! Event-driven benchmark worker using mio for non-blocking I/O
//!
//! This implementation matches the C valkey-benchmark's architecture:
//! - Non-blocking sockets
//! - Event loop (mio::Poll) similar to ae_event_loop
//! - Multiple clients multiplexed on single thread
//! - Write when writable, read when readable

use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use mio::event::Event;
use mio::net::TcpStream as MioTcpStream;
use mio::{Events, Interest, Poll, Token};

use super::counters::GlobalCounters;

/// Recall statistics for vector search
#[derive(Debug, Default)]
pub struct RecallStats {
    pub total_queries: u64,
    pub sum_recall: f64,
    pub min_recall: f64,
    pub max_recall: f64,
    pub perfect_count: u64, // recall == 1.0
    pub zero_count: u64,    // recall == 0.0
}

impl RecallStats {
    pub fn new() -> Self {
        Self {
            min_recall: f64::MAX,
            max_recall: f64::MIN,
            ..Default::default()
        }
    }

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
        if other.total_queries == 0 {
            return;
        }
        self.total_queries += other.total_queries;
        self.sum_recall += other.sum_recall;
        if other.min_recall < self.min_recall {
            self.min_recall = other.min_recall;
        }
        if other.max_recall > self.max_recall {
            self.max_recall = other.max_recall;
        }
        self.perfect_count += other.perfect_count;
        self.zero_count += other.zero_count;
    }
}

use crate::client::{CommandBuffer, PlaceholderType};
use crate::cluster::ClusterTopology;
use crate::config::BenchmarkConfig;
use crate::utils::RespValue;
use crate::workload::{WorkloadContext, WorkloadMetrics};
use std::sync::Arc;

/// CRC16 implementation for Redis cluster slot calculation (XMODEM)
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

/// Calculate slot for a cluster tag like {ABC}
fn slot_for_tag(tag: &[u8; 5]) -> u16 {
    // The content inside {} is: tag[1], tag[2], tag[3]
    crc16(&tag[1..4]) % 16384
}

/// Client state in the event loop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientState {
    /// Ready to send a new request
    Idle,
    /// Writing request to socket
    Writing,
    /// Waiting for response
    Reading,
}

/// Event-driven client
struct EventClient {
    /// Mio TCP stream (non-blocking)
    stream: MioTcpStream,
    /// Token for mio registry
    token: Token,
    /// Current state
    state: ClientState,
    /// Write buffer (command bytes)
    write_buf: Vec<u8>,
    /// Write position
    write_pos: usize,
    /// Read buffer
    read_buf: Vec<u8>,
    /// Read position
    read_pos: usize,
    /// Number of responses expected
    pending_responses: usize,
    /// Responses received
    responses: Vec<RespValue>,
    /// Request start time (for latency)
    start_time: Option<Instant>,
    /// Pipeline size
    pipeline: usize,
    /// Inflight dataset indices
    inflight_indices: VecDeque<u64>,
    /// Query indices for recall
    query_indices: VecDeque<u64>,
    /// Slots owned by this client's node (for cluster tag routing)
    /// Index is slot number (0-16383), value is true if owned
    owned_slots: Option<Box<[bool; 16384]>>,
}

impl EventClient {
    fn new(stream: MioTcpStream, token: Token, write_buf: Vec<u8>, pipeline: usize) -> Self {
        Self {
            stream,
            token,
            state: ClientState::Idle,
            write_buf,
            write_pos: 0,
            read_buf: vec![0u8; 65536],
            read_pos: 0,
            pending_responses: 0,
            responses: Vec::with_capacity(pipeline),
            start_time: None,
            pipeline,
            inflight_indices: VecDeque::with_capacity(pipeline),
            query_indices: VecDeque::with_capacity(pipeline),
            owned_slots: None,
        }
    }

    /// Set the slots owned by this client's node
    fn set_owned_slots(&mut self, slots: &[u16]) {
        let mut bitmap = Box::new([false; 16384]);
        for &slot in slots {
            if (slot as usize) < 16384 {
                bitmap[slot as usize] = true;
            }
        }
        self.owned_slots = Some(bitmap);
    }

    /// Check if a slot is owned by this client's node
    fn owns_slot(&self, slot: u16) -> bool {
        match &self.owned_slots {
            Some(bitmap) => bitmap.get(slot as usize).copied().unwrap_or(false),
            None => true, // Non-cluster mode: accept all
        }
    }

    /// Start a new request
    fn start_request(&mut self) {
        self.write_pos = 0;
        self.read_pos = 0;
        self.pending_responses = self.pipeline;
        self.responses.clear();
        // Note: DON'T clear query_indices here - they're filled BEFORE start_request()
        // and consumed AFTER responses are received
        self.start_time = Some(Instant::now());
        self.state = ClientState::Writing;
    }

    /// Try to write as much as possible (non-blocking)
    /// Returns: Ok(true) if write complete, Ok(false) if would block, Err on error
    fn try_write(&mut self) -> io::Result<bool> {
        while self.write_pos < self.write_buf.len() {
            match self.stream.write(&self.write_buf[self.write_pos..]) {
                Ok(0) => {
                    return Err(io::Error::new(ErrorKind::WriteZero, "Connection closed"));
                }
                Ok(n) => {
                    self.write_pos += n;
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    return Ok(false); // Would block, need to wait
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                    continue; // Retry
                }
                Err(e) => return Err(e),
            }
        }
        // Write complete, switch to reading
        self.state = ClientState::Reading;
        Ok(true)
    }

    /// Try to read responses (non-blocking)
    /// Returns: Ok(true) if all responses received, Ok(false) if need more data, Err on error
    fn try_read(&mut self) -> io::Result<bool> {
        // Read available data into buffer
        loop {
            // Ensure buffer has space
            if self.read_pos >= self.read_buf.len() {
                self.read_buf.resize(self.read_buf.len() * 2, 0);
            }

            match self.stream.read(&mut self.read_buf[self.read_pos..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        ErrorKind::UnexpectedEof,
                        "Connection closed",
                    ));
                }
                Ok(n) => {
                    self.read_pos += n;
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    break; // No more data available
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        // Try to parse responses
        self.parse_responses()
    }

    /// Parse RESP responses from read buffer
    fn parse_responses(&mut self) -> io::Result<bool> {
        let mut parse_pos = 0;

        while self.responses.len() < self.pending_responses {
            let data = &self.read_buf[parse_pos..self.read_pos];
            if data.is_empty() {
                break;
            }

            match parse_resp_value(data) {
                Ok((value, consumed)) => {
                    self.responses.push(value);
                    parse_pos += consumed;
                }
                Err(ParseError::Incomplete) => {
                    break; // Need more data
                }
                Err(ParseError::Invalid(msg)) => {
                    return Err(io::Error::new(ErrorKind::InvalidData, msg));
                }
            }
        }

        // Shift remaining data to front of buffer
        if parse_pos > 0 && parse_pos < self.read_pos {
            self.read_buf.copy_within(parse_pos..self.read_pos, 0);
            self.read_pos -= parse_pos;
        } else if parse_pos == self.read_pos {
            self.read_pos = 0;
        }

        // Check if all responses received
        if self.responses.len() >= self.pending_responses {
            self.state = ClientState::Idle;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get latency in microseconds
    fn latency_us(&self) -> u64 {
        self.start_time
            .map(|t| t.elapsed().as_micros() as u64)
            .unwrap_or(0)
    }
}

/// Parse error
enum ParseError {
    Incomplete,
    Invalid(String),
}

/// Parse a single RESP value from bytes
/// Returns (value, bytes_consumed) or error
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

fn find_crlf(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|w| w == b"\r\n")
}

fn parse_simple_string(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    let crlf = find_crlf(data).ok_or(ParseError::Incomplete)?;
    let s = String::from_utf8_lossy(&data[1..crlf]).to_string();
    Ok((RespValue::SimpleString(s), crlf + 2))
}

fn parse_error(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    let crlf = find_crlf(data).ok_or(ParseError::Incomplete)?;
    let s = String::from_utf8_lossy(&data[1..crlf]).to_string();
    Ok((RespValue::Error(s), crlf + 2))
}

fn parse_integer(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    let crlf = find_crlf(data).ok_or(ParseError::Incomplete)?;
    let s = std::str::from_utf8(&data[1..crlf])
        .map_err(|_| ParseError::Invalid("Invalid integer".to_string()))?;
    let n: i64 = s
        .parse()
        .map_err(|_| ParseError::Invalid("Invalid integer".to_string()))?;
    Ok((RespValue::Integer(n), crlf + 2))
}

fn parse_bulk_string(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    let crlf = find_crlf(data).ok_or(ParseError::Incomplete)?;
    let len_str = std::str::from_utf8(&data[1..crlf])
        .map_err(|_| ParseError::Invalid("Invalid bulk string length".to_string()))?;
    let len: i64 = len_str
        .parse()
        .map_err(|_| ParseError::Invalid("Invalid bulk string length".to_string()))?;

    if len < 0 {
        return Ok((RespValue::Null, crlf + 2));
    }

    let len = len as usize;
    let total_len = crlf + 2 + len + 2;

    if data.len() < total_len {
        return Err(ParseError::Incomplete);
    }

    let content = data[crlf + 2..crlf + 2 + len].to_vec();
    Ok((RespValue::BulkString(content), total_len))
}

fn parse_array(data: &[u8]) -> Result<(RespValue, usize), ParseError> {
    let crlf = find_crlf(data).ok_or(ParseError::Incomplete)?;
    let count_str = std::str::from_utf8(&data[1..crlf])
        .map_err(|_| ParseError::Invalid("Invalid array count".to_string()))?;
    let count: i64 = count_str
        .parse()
        .map_err(|_| ParseError::Invalid("Invalid array count".to_string()))?;

    if count < 0 {
        return Ok((RespValue::Null, crlf + 2));
    }

    let mut pos = crlf + 2;
    let mut elements = Vec::with_capacity(count as usize);

    for _ in 0..count {
        if pos >= data.len() {
            return Err(ParseError::Incomplete);
        }
        let (elem, consumed) = parse_resp_value(&data[pos..])?;
        elements.push(elem);
        pos += consumed;
    }

    Ok((RespValue::Array(elements), pos))
}

/// Result from event worker
pub struct EventWorkerResult {
    pub worker_id: usize,
    pub histogram: Histogram<u64>,
    pub recall_stats: RecallStats,
    pub redirect_count: u64,
    pub topology_refresh_count: u64,
    pub error_count: u64,
    pub requests_processed: u64,
}

/// Simple token bucket for rate limiting
struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    tokens_per_ms: f64,
    last_update: Instant,
}

impl TokenBucket {
    fn new(rps: u64) -> Self {
        let tokens_per_ms = rps as f64 / 1000.0;
        Self {
            tokens: 0.0,
            max_tokens: rps as f64, // 1 second burst
            tokens_per_ms,
            last_update: Instant::now(),
        }
    }

    fn acquire(&mut self, count: u32) -> Option<Duration> {
        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed_ms = now.duration_since(self.last_update).as_secs_f64() * 1000.0;
        self.tokens = (self.tokens + elapsed_ms * self.tokens_per_ms).min(self.max_tokens);
        self.last_update = now;

        let needed = count as f64;
        if self.tokens >= needed {
            self.tokens -= needed;
            None // Can proceed immediately
        } else {
            // Calculate wait time
            let deficit = needed - self.tokens;
            let wait_ms = deficit / self.tokens_per_ms;
            Some(Duration::from_secs_f64(wait_ms / 1000.0))
        }
    }
}

/// Weighted command templates for parallel workloads
#[derive(Clone)]
pub struct WeightedTemplates {
    /// Command templates (one per workload type)
    templates: Vec<CommandBuffer>,
    /// Cumulative weights for O(1) selection [0.3, 0.8, 1.0] for 30%, 50%, 20%
    cumulative_weights: Vec<f64>,
}

impl WeightedTemplates {
    /// Create from a single template (non-parallel mode)
    pub fn single(template: CommandBuffer) -> Self {
        Self {
            templates: vec![template],
            cumulative_weights: vec![1.0],
        }
    }

    /// Create from multiple templates with weights
    pub fn weighted(templates: Vec<(CommandBuffer, f64)>) -> Self {
        if templates.is_empty() {
            panic!("WeightedTemplates requires at least one template");
        }

        let total: f64 = templates.iter().map(|(_, w)| w).sum();
        let mut cumulative = Vec::with_capacity(templates.len());
        let mut sum = 0.0;

        let (buffers, weights): (Vec<_>, Vec<_>) = templates.into_iter().unzip();

        for w in weights {
            sum += w / total;
            cumulative.push(sum);
        }

        // Ensure last element is exactly 1.0
        if let Some(last) = cumulative.last_mut() {
            *last = 1.0;
        }

        Self {
            templates: buffers,
            cumulative_weights: cumulative,
        }
    }

    /// Select a template index based on a random value in [0, 1)
    #[inline]
    pub fn select_index(&self, random: f64) -> usize {
        if self.templates.len() == 1 {
            return 0;
        }
        match self
            .cumulative_weights
            .binary_search_by(|w| w.partial_cmp(&random).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(idx) => idx.min(self.templates.len() - 1),
            Err(idx) => idx.min(self.templates.len() - 1),
        }
    }

    /// Get template at index
    #[inline]
    pub fn get(&self, index: usize) -> &CommandBuffer {
        &self.templates[index]
    }

    /// Get the first/primary template (for single-template mode)
    #[inline]
    pub fn primary(&self) -> &CommandBuffer {
        &self.templates[0]
    }

    /// Check if this is parallel mode (multiple templates)
    #[inline]
    pub fn is_parallel(&self) -> bool {
        self.templates.len() > 1
    }

    /// Number of templates
    pub fn len(&self) -> usize {
        self.templates.len()
    }
}

/// Event-driven benchmark worker
pub struct EventWorker {
    id: usize,
    poll: Poll,
    clients: Vec<EventClient>,
    events: Events,
    rng: fastrand::Rng,
    histogram: Histogram<u64>,
    pipeline: usize,
    /// Seed for deterministic random key generation
    seed: u64,
    /// Command templates (single or weighted for parallel mode)
    templates: WeightedTemplates,
    error_count: u64,
    requests_processed: u64,
    /// Workload-specific context (handles ID claiming, recall computation, etc.)
    workload_ctx: Box<dyn WorkloadContext>,
    /// Slot to node address mapping (built from topology)
    slot_to_node: Option<Box<[(String, u16); 16384]>>,
    /// Key prefix for regular keys (for slot calculation)
    regular_key_prefix: String,
    /// Ready queues per node - clients ready to accept new requests (O(1) pop/push)
    node_ready_queues: std::collections::HashMap<(String, u16), VecDeque<usize>>,
    /// Global ready queue for standalone mode or non-slot-aware workloads
    global_ready_queue: VecDeque<usize>,
    /// Rate limiter (optional, for --rps flag)
    rate_limiter: Option<TokenBucket>,
}

impl EventWorker {
    /// Create new event-driven worker
    ///
    /// # Arguments
    /// * `id` - Worker ID
    /// * `addresses` - List of (host, port) to connect to
    /// * `clients_per_addr` - Number of clients per address
    /// * `config` - Benchmark configuration
    /// * `command_template` - Pre-built command template
    /// * `topology` - Optional cluster topology for slot-aware routing
    /// * `workload_ctx` - Workload-specific context for ID claiming, recall, etc.
    pub fn new(
        id: usize,
        addresses: Vec<(String, u16)>,
        clients_per_addr: usize,
        config: &BenchmarkConfig,
        command_template: CommandBuffer,
        topology: Option<&ClusterTopology>,
        workload_ctx: Box<dyn WorkloadContext>,
    ) -> io::Result<Self> {
        Self::with_templates(
            id,
            addresses,
            clients_per_addr,
            config,
            WeightedTemplates::single(command_template),
            topology,
            workload_ctx,
        )
    }

    /// Create new event-driven worker with weighted templates
    ///
    /// Used for parallel workload execution with weighted traffic distribution.
    pub fn with_templates(
        id: usize,
        addresses: Vec<(String, u16)>,
        clients_per_addr: usize,
        config: &BenchmarkConfig,
        templates: WeightedTemplates,
        topology: Option<&ClusterTopology>,
        workload_ctx: Box<dyn WorkloadContext>,
    ) -> io::Result<Self> {
        let poll = Poll::new()?;
        let mut clients = Vec::new();
        let mut token_counter = 0usize;
        let mut node_ready_queues: std::collections::HashMap<(String, u16), VecDeque<usize>> =
            std::collections::HashMap::new();

        // Build slot_to_node mapping from topology
        let slot_to_node: Option<Box<[(String, u16); 16384]>> = if let Some(topo) = topology {
            // Create array with default empty values
            let mut mapping: Box<[(String, u16); 16384]> =
                vec![(String::new(), 0u16); 16384].into_boxed_slice().try_into().unwrap();
            
            for node in topo.primaries() {
                for &slot in &node.slots {
                    mapping[slot as usize] = (node.host.clone(), node.port);
                }
            }
            Some(mapping)
        } else {
            None
        };

        // Build a map of (host, port) -> slots for cluster mode
        let slot_map: std::collections::HashMap<(String, u16), Vec<u16>> = if let Some(topo) = topology {
            topo.primaries()
                .map(|n| ((n.host.clone(), n.port), n.slots.clone()))
                .collect()
        } else {
            std::collections::HashMap::new()
        };

        // Create clients distributed across addresses
        for (host, port) in addresses.iter() {
            // Get slots for this node (if cluster mode)
            let node_slots = slot_map.get(&(host.clone(), *port));

            for _ in 0..clients_per_addr {
                let addr = format!("{}:{}", host, port);
                let std_stream = std::net::TcpStream::connect(&addr)?;
                std_stream.set_nonblocking(true)?;
                std_stream.set_nodelay(true)?;

                let mut mio_stream = MioTcpStream::from_std(std_stream);
                let token = Token(token_counter);
                let client_idx = token_counter;
                token_counter += 1;

                // Register with poll - initially interested in WRITABLE
                poll.registry().register(
                    &mut mio_stream,
                    token,
                    Interest::READABLE | Interest::WRITABLE,
                )?;

                let mut client = EventClient::new(
                    mio_stream,
                    token,
                    templates.primary().bytes.clone(),
                    config.pipeline as usize,
                );

                // Set slot ownership for cluster mode
                if let Some(slots) = node_slots {
                    client.set_owned_slots(slots);
                }

                clients.push(client);

                // Track which clients belong to which node (add to ready queue)
                node_ready_queues
                    .entry((host.clone(), *port))
                    .or_default()
                    .push_back(client_idx);
            }
        }

        // Initialize RNG
        let seed = if config.seed == 0 {
            fastrand::u64(..)
        } else {
            config.seed.wrapping_add(id as u64)
        };
        let rng = fastrand::Rng::with_seed(seed);

        // Histogram
        let histogram =
            Histogram::new_with_bounds(1, 3_600_000_000, 3).expect("Failed to create histogram");

        // Build global ready queue with all client indices
        let global_ready_queue: VecDeque<usize> = (0..clients.len()).collect();

        // Rate limiter (if --rps specified)
        let rate_limiter = if config.requests_per_second > 0 {
            // Divide RPS among threads
            let per_thread_rps = config.requests_per_second / config.threads.max(1) as u64;
            Some(TokenBucket::new(per_thread_rps.max(1)))
        } else {
            None
        };

        Ok(Self {
            id,
            poll,
            clients,
            events: Events::with_capacity(1024),
            rng,
            histogram,
            pipeline: config.pipeline as usize,
            seed: config.seed,
            templates,
            error_count: 0,
            requests_processed: 0,
            workload_ctx,
            slot_to_node,
            regular_key_prefix: config.key_prefix.clone(),
            node_ready_queues,
            global_ready_queue,
            rate_limiter,
        })
    }

    /// Calculate slot for a key (using CRC16)
    fn slot_for_key(&self, key: &[u8]) -> u16 {
        // Check for hash tag {xxx}
        if let Some(start) = key.iter().position(|&b| b == b'{') {
            if let Some(end) = key[start + 1..].iter().position(|&b| b == b'}') {
                if end > 0 {
                    // Hash only the content inside {}
                    return crc16(&key[start + 1..start + 1 + end]) % 16384;
                }
            }
        }
        // Hash the entire key
        crc16(key) % 16384
    }

    /// Build a key and return its bytes for slot calculation
    fn build_key_bytes(&self, key_num: u64) -> Vec<u8> {
        let mut key_bytes = Vec::with_capacity(self.regular_key_prefix.len() + 12);
        key_bytes.extend_from_slice(self.regular_key_prefix.as_bytes());
        // Format as zero-padded 12-digit number
        write!(std::io::Write::by_ref(&mut key_bytes), "{:012}", key_num).ok();
        key_bytes
    }

    /// Pop a ready client for a given slot (cluster mode) - O(1)
    /// Returns client index or None if no ready client for this slot's node
    fn pop_ready_client_for_slot(&mut self, slot: u16) -> Option<usize> {
        if let Some(ref slot_to_node) = self.slot_to_node {
            // Cluster mode: get client from the node owning this slot
            let node_addr = slot_to_node[slot as usize].clone();
            if node_addr.0.is_empty() {
                // Slot not mapped, try any node
                return self.pop_any_ready_client();
            }
            
            // Pop from node's ready queue
            if let Some(queue) = self.node_ready_queues.get_mut(&node_addr) {
                queue.pop_front()
            } else {
                None
            }
        } else {
            // Standalone mode: pop from global queue
            self.global_ready_queue.pop_front()
        }
    }

    /// Pop any ready client - tries all node queues in cluster mode, global queue in standalone
    fn pop_any_ready_client(&mut self) -> Option<usize> {
        if self.slot_to_node.is_some() {
            // Cluster mode: try all node queues
            for queue in self.node_ready_queues.values_mut() {
                if let Some(idx) = queue.pop_front() {
                    return Some(idx);
                }
            }
            None
        } else {
            // Standalone mode: use global queue
            self.global_ready_queue.pop_front()
        }
    }

    /// Return a client to the ready queue after completing a request
    fn return_client_to_ready(&mut self, client_idx: usize) {
        if self.slot_to_node.is_some() {
            // Cluster mode: add to node-specific queue only
            if let Some(ref owned) = self.clients[client_idx].owned_slots {
                // Find first slot owned by this client to determine node
                for slot in 0..16384u16 {
                    if owned[slot as usize] {
                        if let Some(ref slot_to_node) = self.slot_to_node {
                            let node_addr = slot_to_node[slot as usize].clone();
                            if !node_addr.0.is_empty() {
                                if let Some(queue) = self.node_ready_queues.get_mut(&node_addr) {
                                    queue.push_back(client_idx);
                                }
                            }
                        }
                        break;
                    }
                }
            }
        } else {
            // Standalone mode: add to global queue
            self.global_ready_queue.push_back(client_idx);
        }
    }

    /// Check if workload needs slot-aware routing
    /// 
    /// Currently returns false as cluster mode uses standard key hashing.
    /// Slot-aware routing is reserved for future features like node-balanced load.
    fn needs_slot_routing(&self) -> bool {
        false
    }

    /// Generate next key number (sequential or random)
    ///
    /// Try to start a request with slot-aware routing
    /// Returns true if a request was started, false if no idle client available or iterator exhausted
    fn try_start_slot_aware_request(
        &mut self,
        _counters: &GlobalCounters,
        iterator_exhausted: &mut bool,
    ) -> bool {
        // First try to get any ready client
        // Try the slot-specific path first (most likely to succeed)

        // OPTIMIZATION: Check if any node queues have clients before generating key
        let has_ready_clients = self.node_ready_queues.values().any(|q| !q.is_empty());
        if !has_ready_clients {
            return false;
        }

        // Generate key number from iterator
        let key_num = match self.workload_ctx.claim_next_id() {
            Some(id) => id,
            None => {
                *iterator_exhausted = true;
                return false;
            }
        };

        // Build key and calculate slot
        let key_bytes = self.build_key_bytes(key_num);
        let slot = self.slot_for_key(&key_bytes);

        // Try to get client for this slot's node first
        if let Some(client_idx) = self.pop_ready_client_for_slot(slot) {
            self.fill_placeholders_with_key(client_idx, key_num);
            self.clients[client_idx].start_request();
            let _ = self.clients[client_idx].try_write();
            return true;
        }

        // No client for this specific slot's node, try any available client
        // This may cause a MOVED redirect, but it's better than losing work
        if let Some(client_idx) = self.pop_any_ready_client() {
            self.fill_placeholders_with_key(client_idx, key_num);
            self.clients[client_idx].start_request();
            let _ = self.clients[client_idx].try_write();
            return true;
        }

        // This shouldn't happen if has_ready_clients was true, but handle it
        // The key_num is lost in this case, but it's a race condition edge case
        false
    }

    /// Fill placeholders with a pre-determined key (for slot-aware routing)
    fn fill_placeholders_with_key(
        &mut self,
        client_idx: usize,
        key_num: u64,
    ) {
        // Select template: random weighted selection for parallel mode, primary for single
        let template = if self.templates.is_parallel() {
            let random = self.rng.f64();
            let idx = self.templates.select_index(random);
            let selected = self.templates.get(idx);
            // Copy selected template bytes into write buffer (different commands have different structures)
            let buf = &mut self.clients[client_idx].write_buf;
            if buf.len() != selected.bytes.len() {
                buf.resize(selected.bytes.len(), 0);
            }
            buf.copy_from_slice(&selected.bytes);
            selected
        } else {
            self.templates.primary()
        };
        for (cmd_idx, phs) in template.placeholders.iter().enumerate() {
            for ph in phs {
                let offset = template.absolute_offset(cmd_idx, ph.offset);
                match ph.placeholder_type {
                    PlaceholderType::Key => {
                        // For VecDel: key is directly the claimed ID
                        // For dataset workloads: key is set by Vector handler
                        // For simple workloads: key is the key_num
                        if self.workload_ctx.key_is_claimed_id() {
                            write_fixed_width_u64(&mut self.clients[client_idx].write_buf, offset, key_num, ph.len);
                        } else if self.workload_ctx.uses_dataset_keys() {
                            continue; // Key will be set by Vector handler
                        } else {
                            write_fixed_width_u64(&mut self.clients[client_idx].write_buf, offset, key_num, ph.len);
                        }
                    }
                    PlaceholderType::Vector => {
                        // Use key_num as the vector index - it was already claimed via claim_next_id
                        if let Some(vec_bytes) = self.workload_ctx.get_vector_bytes(key_num) {
                            self.clients[client_idx].write_buf[offset..offset + vec_bytes.len()]
                                .copy_from_slice(vec_bytes);
                            // Also set the Key placeholder with the vector ID
                            for ph2 in phs {
                                if matches!(ph2.placeholder_type, PlaceholderType::Key) {
                                    let key_offset = template.absolute_offset(cmd_idx, ph2.offset);
                                    write_fixed_width_u64(&mut self.clients[client_idx].write_buf, key_offset, key_num, ph2.len);
                                    break;
                                }
                            }
                        }
                    }
                    PlaceholderType::QueryVector => {
                        let num_queries = self.workload_ctx.num_queries();
                        if num_queries > 0 {
                            let idx = self.rng.u64(0..num_queries);
                            if let Some(vec_bytes) = self.workload_ctx.get_query_bytes(idx) {
                                self.clients[client_idx].write_buf[offset..offset + vec_bytes.len()]
                                    .copy_from_slice(vec_bytes);
                                self.clients[client_idx].query_indices.push_back(idx);
                            }
                        }
                    }
                    PlaceholderType::RandInt => {
                        let value = self.rng.u64(..);
                        write_fixed_width_u64(&mut self.clients[client_idx].write_buf, offset, value, ph.len);
                    }
                    PlaceholderType::Tag => {
                        // Delegate tag generation to workload context
                        let buf = &mut self.clients[client_idx].write_buf[offset..offset + ph.len];
                        self.workload_ctx.fill_tag_placeholder(key_num, buf);
                    }
                    PlaceholderType::Numeric => {
                        write_fixed_width_u64(
                            &mut self.clients[client_idx].write_buf,
                            offset,
                            key_num,
                            ph.len,
                        );
                    }
                    PlaceholderType::NumericField(field_idx) => {
                        // Delegate numeric field generation to workload context
                        let buf = &mut self.clients[client_idx].write_buf[offset..offset + ph.len];
                        // Use key_num as both key-based seed and seq_counter for now
                        self.workload_ctx.fill_numeric_field(field_idx, key_num, key_num, buf);
                    }
                    PlaceholderType::Field => {
                        // Delegate field name filling to workload context (for hash field iteration)
                        let buf = &mut self.clients[client_idx].write_buf[offset..offset + ph.len];
                        self.workload_ctx.fill_field_placeholder(buf);
                    }
                    PlaceholderType::JsonPath => {
                        // Delegate JSON path filling to workload context (for JSON path iteration)
                        let buf = &mut self.clients[client_idx].write_buf[offset..offset + ph.len];
                        self.workload_ctx.fill_json_path_placeholder(buf);
                    }
                }
            }
        }
    }

    /// Apply rate limiting if configured
    fn apply_rate_limit(&mut self) {
        if let Some(ref mut limiter) = self.rate_limiter {
            if let Some(wait) = limiter.acquire(self.pipeline as u32) {
                std::thread::sleep(wait);
            }
        }
    }

    /// Run the event loop
    pub fn run(
        mut self,
        counters: Arc<GlobalCounters>,
    ) -> EventWorkerResult {
        let batch_size = self.pipeline as u64;
        let slot_aware = self.needs_slot_routing();
        let mut iterator_exhausted = false;

        // Start initial batch: drain ready queues
        if slot_aware {
            // For slot-aware: try to start as many requests as possible
            loop {
                if counters.is_duration_exceeded() {
                    break;
                }
                self.apply_rate_limit();
                if !self.try_start_slot_aware_request(&counters, &mut iterator_exhausted) {
                    break;
                }
                counters.record_issued(batch_size);
            }
        } else {
            // For non-slot-aware: pop from global ready queue
            while let Some(client_idx) = self.pop_any_ready_client() {
                if counters.is_duration_exceeded() {
                    self.return_client_to_ready(client_idx);
                    break;
                }
                self.apply_rate_limit();
                if !self.fill_placeholders_for(client_idx) {
                    // Iterator exhausted
                    iterator_exhausted = true;
                    self.return_client_to_ready(client_idx);
                    break;
                }
                counters.record_issued(batch_size);
                self.clients[client_idx].start_request();
            }
        }

        // Event loop - poll for I/O readiness
        loop {
            if counters.is_shutdown() || counters.is_duration_exceeded() {
                break;
            }

            // Check if iterator exhausted and all clients are idle
            if iterator_exhausted {
                let all_idle = self
                    .clients
                    .iter()
                    .all(|c| c.state == ClientState::Idle);
                if all_idle {
                    break;
                }
            }

            // Poll for events - use short timeout to also handle idle clients
            if self
                .poll
                .poll(&mut self.events, Some(Duration::from_micros(100)))
                .is_err()
            {
                continue;
            }

            // Process all clients - not just those with events
            // This is more like C's approach where we try operations optimistically
            for client_idx in 0..self.clients.len() {
                if counters.is_duration_exceeded() {
                    break;
                }

                let state = self.clients[client_idx].state;

                match state {
                    ClientState::Idle => {
                        // Idle clients are in the ready queue, nothing to do here
                    }
                    ClientState::Writing => {
                        // Try to write (optimistically)
                        match self.clients[client_idx].try_write() {
                            Ok(true) => {
                                // Write complete, now in Reading state
                            }
                            Ok(false) => {
                                // Would block, will try again next iteration
                            }
                            Err(_e) => {
                                self.error_count += batch_size;
                                self.clients[client_idx].state = ClientState::Idle;
                                self.return_client_to_ready(client_idx);
                            }
                        }
                    }
                    ClientState::Reading => {
                        // Try to read (optimistically)
                        match self.clients[client_idx].try_read() {
                            Ok(true) => {
                                // All responses received
                                let latency = self.clients[client_idx].latency_us();
                                self.histogram.record(latency).ok();
                                self.requests_processed +=
                                    self.clients[client_idx].responses.len() as u64;
                                counters.record_finished(
                                    self.clients[client_idx].responses.len() as u64,
                                );

                                // Compute recall for vector queries (if query indices present)
                                if !self.clients[client_idx].query_indices.is_empty() {
                                    // Collect query indices first to avoid borrow conflict
                                    let query_indices: Vec<u64> = self.clients[client_idx]
                                        .query_indices
                                        .drain(..)
                                        .collect();

                                    for (response, query_idx) in self.clients[client_idx]
                                        .responses
                                        .iter()
                                        .zip(query_indices.iter())
                                    {
                                        self.workload_ctx.compute_and_record_recall(*query_idx, response);
                                    }
                                }

                                // Start next request immediately on this client
                                // For non-slot-aware: reuse same client
                                // For slot-aware: return to queue (new request may need different node)
                                if slot_aware {
                                    // Return to ready queue, will be picked up for correct slot
                                    self.clients[client_idx].state = ClientState::Idle;
                                    self.return_client_to_ready(client_idx);
                                } else {
                                    // Non-slot-aware: reuse client directly
                                    if !iterator_exhausted && !counters.is_duration_exceeded() {
                                        self.apply_rate_limit();
                                        if self.fill_placeholders_for(client_idx) {
                                            counters.record_issued(batch_size);
                                            self.clients[client_idx].start_request();
                                            // Immediately try to write (socket likely writable)
                                            let _ = self.clients[client_idx].try_write();
                                        } else {
                                            iterator_exhausted = true;
                                            self.clients[client_idx].state = ClientState::Idle;
                                            self.return_client_to_ready(client_idx);
                                        }
                                    } else {
                                        self.clients[client_idx].state = ClientState::Idle;
                                        self.return_client_to_ready(client_idx);
                                    }
                                }
                            }
                            Ok(false) => {
                                // Need more data, will try again next iteration
                            }
                            Err(_e) => {
                                self.error_count += batch_size;
                                self.clients[client_idx].state = ClientState::Idle;
                                self.return_client_to_ready(client_idx);
                            }
                        }
                    }
                }
            }

            // Start new requests from ready queue (if iterator not exhausted)
            if !iterator_exhausted {
                if slot_aware {
                    // Slot-aware: try to start requests with proper routing
                    loop {
                        if counters.is_duration_exceeded() {
                            break;
                        }
                        self.apply_rate_limit();
                        if !self.try_start_slot_aware_request(&counters, &mut iterator_exhausted) {
                            break;
                        }
                        counters.record_issued(batch_size);
                    }
                } else {
                    // Non-slot-aware: pop from global ready queue
                    while let Some(client_idx) = self.pop_any_ready_client() {
                        if counters.is_duration_exceeded() {
                            self.return_client_to_ready(client_idx);
                            break;
                        }
                        self.apply_rate_limit();
                        if !self.fill_placeholders_for(client_idx) {
                            iterator_exhausted = true;
                            self.return_client_to_ready(client_idx);
                            break;
                        }
                        counters.record_issued(batch_size);
                        self.clients[client_idx].start_request();
                        let _ = self.clients[client_idx].try_write();
                    }
                }
            }
        }

        // Get recall stats from workload context
        let recall_stats = match self.workload_ctx.take_metrics() {
            WorkloadMetrics::Recall(stats) => stats,
            WorkloadMetrics::None => RecallStats::new(),
        };

        EventWorkerResult {
            worker_id: self.id,
            histogram: self.histogram,
            recall_stats,
            redirect_count: 0,
            topology_refresh_count: 0,
            error_count: self.error_count,
            requests_processed: self.requests_processed,
        }
    }

    /// Fill placeholders for a specific client by index
    /// Fill placeholders for a client. Returns true if an ID was claimed, false if exhausted.
    fn fill_placeholders_for(
        &mut self,
        client_idx: usize,
    ) -> bool {
        // Get key number from workload context (claim once, use consistently)
        let key_num = match self.workload_ctx.claim_next_id() {
            Some(id) => id,
            None => return false, // No more IDs available (iterator exhausted)
        };

        // Delegate to the unified implementation
        self.fill_placeholders_with_key(client_idx, key_num);
        true
    }
}

/// Write u64 as fixed-width decimal (zero-padded)
#[inline]
fn write_fixed_width_u64(buf: &mut [u8], offset: usize, value: u64, width: usize) {
    let mut v = value;
    for i in (0..width).rev() {
        buf[offset + i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recall_stats() {
        let mut stats = RecallStats::new();

        stats.record(1.0);
        stats.record(0.5);
        stats.record(0.0);

        assert_eq!(stats.total_queries, 3);
        assert!((stats.average() - 0.5).abs() < 0.001);
        assert_eq!(stats.perfect_count, 1);
        assert_eq!(stats.zero_count, 1);
    }

    #[test]
    fn test_recall_stats_merge() {
        let mut stats1 = RecallStats::new();
        stats1.record(1.0);
        stats1.record(0.8);

        let mut stats2 = RecallStats::new();
        stats2.record(0.5);
        stats2.record(0.0);

        stats1.merge(&stats2);

        assert_eq!(stats1.total_queries, 4);
        assert!((stats1.average() - 0.575).abs() < 0.001); // (1.0 + 0.8 + 0.5 + 0.0) / 4
        assert_eq!(stats1.perfect_count, 1);
        assert_eq!(stats1.zero_count, 1);
    }

    #[test]
    fn test_token_bucket() {
        let mut bucket = TokenBucket::new(1000); // 1000 RPS

        // Should be able to acquire immediately after some time
        std::thread::sleep(Duration::from_millis(10));
        assert!(bucket.acquire(5).is_none());
    }

    #[test]
    fn test_token_bucket_wait() {
        let mut bucket = TokenBucket::new(100); // 100 RPS

        // Try to acquire more than accumulated
        let wait = bucket.acquire(200);
        assert!(wait.is_some());
        // Should need to wait approximately 2 seconds for 200 tokens at 100 RPS
        // But since some time may have passed, just verify we need to wait
        if let Some(duration) = wait {
            assert!(duration.as_secs_f64() > 0.1);
        }
    }
}
