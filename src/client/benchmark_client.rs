//! Benchmark client with pre-allocated buffers
//!
//! This client is optimized for benchmark traffic with:
//! - Pre-allocated write buffer (command template)
//! - Pre-allocated read buffer (responses)
//! - In-place placeholder replacement
//! - Pipeline support

use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use super::raw_connection::RawConnection;
use crate::utils::RespValue;

/// Placeholder offset information
#[derive(Debug, Clone)]
pub struct PlaceholderOffset {
    /// Byte offset in write buffer
    pub offset: usize,
    /// Length of placeholder region
    pub len: usize,
    /// Type of placeholder
    pub placeholder_type: PlaceholderType,
}

/// Types of placeholders
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderType {
    /// Random or sequential key (fixed-width decimal)
    Key,
    /// Vector data (binary blob) - for database vectors (HSET)
    Vector,
    /// Query vector data (binary blob) - for FT.SEARCH queries
    QueryVector,
    /// Random integer
    RandInt,
    /// Tag field value (variable-length string, padded to max length)
    Tag,
    /// Numeric field value (fixed-width decimal)
    Numeric,
    /// Indexed numeric field with configurable type/distribution
    /// The usize is the index into the NumericFieldSet
    NumericField(usize),
    /// Hash field name (from AddressableSpace, padded to max length)
    Field,
    /// JSON path (from AddressableSpace, padded to max length)
    JsonPath,
}

/// Pre-computed command template with placeholder offsets
#[derive(Debug, Clone)]
pub struct CommandBuffer {
    /// RESP-encoded command bytes (template)
    pub bytes: Vec<u8>,
    /// Placeholder offsets for each command in pipeline
    pub placeholders: Vec<Vec<PlaceholderOffset>>,
    /// Number of commands in pipeline
    pub pipeline_size: usize,
    /// Bytes per single command (for pipeline offset calculation)
    pub command_len: usize,
}

impl CommandBuffer {
    /// Create a new command buffer from template bytes
    pub fn new(template_bytes: Vec<u8>, pipeline_size: usize) -> Self {
        let command_len = template_bytes.len() / pipeline_size.max(1);
        Self {
            bytes: template_bytes,
            placeholders: Vec::new(),
            pipeline_size,
            command_len,
        }
    }

    /// Register placeholder offset for command at index
    pub fn add_placeholder(&mut self, cmd_idx: usize, offset: PlaceholderOffset) {
        while self.placeholders.len() <= cmd_idx {
            self.placeholders.push(Vec::new());
        }
        self.placeholders[cmd_idx].push(offset);
    }

    /// Get absolute offset for placeholder in command at index
    #[inline]
    pub fn absolute_offset(&self, cmd_idx: usize, relative_offset: usize) -> usize {
        cmd_idx * self.command_len + relative_offset
    }
}

/// Batch response from pipeline execution
#[derive(Debug)]
pub struct BatchResponse {
    /// Response values
    pub values: Vec<RespValue>,
    /// Latency for the batch in microseconds
    pub latency_us: u64,
    /// Dataset indices for retry (if insert failed)
    pub inflight_indices: Vec<u64>,
    /// Query indices for recall verification
    pub query_indices: Vec<u64>,
}

/// High-performance benchmark client
pub struct BenchmarkClient {
    /// Underlying connection
    conn: RawConnection,

    /// Pre-allocated command buffer (write)
    write_buf: CommandBuffer,

    /// Pipeline depth
    pipeline: usize,

    /// Pending responses to read
    pending: usize,

    /// Dataset indices currently in flight
    inflight_indices: VecDeque<u64>,

    /// Query indices for recall verification
    query_indices: VecDeque<u64>,

    /// Assigned node (for cluster routing)
    assigned_node: Option<(String, u16)>,
}

impl BenchmarkClient {
    /// Create new benchmark client from connection and command template
    pub fn new(conn: RawConnection, command_template: CommandBuffer, pipeline: usize) -> Self {
        Self {
            conn,
            write_buf: command_template,
            pipeline,
            pending: 0,
            inflight_indices: VecDeque::with_capacity(pipeline),
            query_indices: VecDeque::with_capacity(pipeline),
            assigned_node: None,
        }
    }

    /// Set assigned node for this client
    pub fn set_assigned_node(&mut self, host: String, port: u16) {
        self.assigned_node = Some((host, port));
    }

    /// Get assigned node
    pub fn assigned_node(&self) -> Option<&(String, u16)> {
        self.assigned_node.as_ref()
    }

    /// Clear tracking state for new batch
    pub fn clear_batch_state(&mut self) {
        self.inflight_indices.clear();
        self.query_indices.clear();
    }

    /// Replace key placeholder at command index with fixed-width value
    ///
    /// # Arguments
    /// * `cmd_idx` - Index of command in pipeline (0-based)
    /// * `value` - Key value to write
    /// * `ph_offset` - Placeholder offset information
    #[inline]
    pub fn replace_key(&mut self, cmd_idx: usize, value: u64, ph_offset: &PlaceholderOffset) {
        let offset = self.write_buf.absolute_offset(cmd_idx, ph_offset.offset);
        write_fixed_width_u64(&mut self.write_buf.bytes, offset, value, ph_offset.len);
    }

    /// Replace vector placeholder with raw bytes (zero-copy from mmap)
    ///
    /// # Arguments
    /// * `cmd_idx` - Index of command in pipeline
    /// * `vector_bytes` - Raw vector bytes (directly from mmap)
    /// * `ph_offset` - Placeholder offset information
    #[inline]
    pub fn replace_vector(
        &mut self,
        cmd_idx: usize,
        vector_bytes: &[u8],
        ph_offset: &PlaceholderOffset,
    ) {
        let offset = self.write_buf.absolute_offset(cmd_idx, ph_offset.offset);
        self.write_buf.bytes[offset..offset + vector_bytes.len()].copy_from_slice(vector_bytes);
    }

    /// Track inflight dataset index for retry
    #[inline]
    pub fn track_inflight(&mut self, idx: u64) {
        self.inflight_indices.push_back(idx);
    }

    /// Track query index for recall verification
    #[inline]
    pub fn track_query(&mut self, idx: u64) {
        self.query_indices.push_back(idx);
    }

    /// Send the pre-built command buffer
    pub fn send(&mut self) -> io::Result<()> {
        self.conn.write_all(&self.write_buf.bytes)?;
        self.conn.flush()?;
        self.pending = self.pipeline;
        Ok(())
    }

    /// Receive responses for pending commands
    pub fn recv(&mut self) -> io::Result<BatchResponse> {
        let start = Instant::now();

        let values = self.conn.read_responses(self.pending)?;

        let latency_us = start.elapsed().as_micros() as u64;
        self.pending = 0;

        Ok(BatchResponse {
            values,
            latency_us,
            inflight_indices: self.inflight_indices.drain(..).collect(),
            query_indices: self.query_indices.drain(..).collect(),
        })
    }

    /// Execute batch: send and receive
    pub fn execute_batch(&mut self) -> io::Result<BatchResponse> {
        let start = Instant::now();

        self.send()?;
        let mut response = self.recv()?;

        // Use total round-trip time
        response.latency_us = start.elapsed().as_micros() as u64;

        Ok(response)
    }

    /// Get mutable reference to write buffer for manual manipulation
    pub fn write_buffer_mut(&mut self) -> &mut CommandBuffer {
        &mut self.write_buf
    }

    /// Get reference to write buffer
    pub fn write_buffer(&self) -> &CommandBuffer {
        &self.write_buf
    }

    /// Get reference to the underlying connection
    pub fn connection(&self) -> &RawConnection {
        &self.conn
    }

    /// Get mutable reference to the underlying connection
    pub fn connection_mut(&mut self) -> &mut RawConnection {
        &mut self.conn
    }
}

/// Write u64 as fixed-width decimal string (zero-padded)
///
/// # Arguments
/// * `buf` - Target buffer
/// * `offset` - Starting offset in buffer
/// * `value` - Value to write
/// * `width` - Fixed width (will be zero-padded)
#[inline]
pub fn write_fixed_width_u64(buf: &mut [u8], offset: usize, value: u64, width: usize) {
    let mut v = value;
    for i in (0..width).rev() {
        buf[offset + i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
}

/// Write bytes from source to buffer at offset (single memcpy)
#[inline]
pub fn write_bytes(buf: &mut [u8], offset: usize, src: &[u8]) {
    buf[offset..offset + src.len()].copy_from_slice(src);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_fixed_width_u64() {
        let mut buf = vec![b'X'; 20];
        write_fixed_width_u64(&mut buf, 5, 12345, 10);
        assert_eq!(&buf[5..15], b"0000012345");
    }

    #[test]
    fn test_write_fixed_width_zero() {
        let mut buf = vec![b'X'; 10];
        write_fixed_width_u64(&mut buf, 0, 0, 5);
        assert_eq!(&buf[0..5], b"00000");
    }

    #[test]
    fn test_write_fixed_width_max() {
        let mut buf = vec![b'X'; 20];
        write_fixed_width_u64(&mut buf, 0, 999999999999, 12);
        assert_eq!(&buf[0..12], b"999999999999");
    }

    #[test]
    fn test_command_buffer_absolute_offset() {
        let buf = CommandBuffer::new(vec![0u8; 300], 3);
        // Each command is 100 bytes
        assert_eq!(buf.absolute_offset(0, 10), 10);
        assert_eq!(buf.absolute_offset(1, 10), 110);
        assert_eq!(buf.absolute_offset(2, 10), 210);
    }

    #[test]
    fn test_write_bytes() {
        let mut buf = vec![b'X'; 20];
        write_bytes(&mut buf, 5, b"hello");
        assert_eq!(&buf[5..10], b"hello");
        assert_eq!(&buf[0..5], b"XXXXX");
    }
}
