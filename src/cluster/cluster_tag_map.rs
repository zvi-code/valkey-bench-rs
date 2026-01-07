//! Vector ID Existence Mapping
//!
//! Tracks which vector IDs exist in the cluster for recall validation
//! and partial prefill support using a compact bitmap representation.
//!
//! Key format: `prefix + vector_id`
//! Example: `vec:000001` where `vec:` is prefix
//!
//! Note: Cluster hash tag injection has been removed. Keys are now
//! distributed across cluster slots based on their natural hash.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::client::{ControlPlane, RawConnection};
use crate::cluster::ClusterNode;
use crate::utils::{RespEncoder, RespValue};

/// Thread-safe bitmap-based existence tracking table
/// 
/// Uses 1 bit per vector ID (8x more memory efficient than previous 1-byte-per-entry).
/// For 10M vectors, requires ~1.25MB instead of ~10MB.
pub struct ClusterTagMap {
    /// Key prefix (e.g., "vec:")
    pub prefix: String,
    /// Bitmap: each bit represents existence of a vector ID
    /// bit index = vector_id, bit value 1 = exists
    bitmap: Vec<AtomicU64>,
    /// Total capacity in vector IDs
    capacity: u64,
    /// Number of valid mappings (vectors that exist)
    count: AtomicU64,
    /// Total keys scanned (for progress tracking)
    keys_scanned: AtomicU64,
    /// Whether cluster mode is enabled
    pub is_cluster_mode: bool,
    /// Mutex for concurrent updates (only needed for count updates)
    update_mutex: Mutex<()>,
    /// Atomic counter for claiming unmapped vector IDs (for partial prefill)
    unmapped_counter: AtomicU64,
}

impl ClusterTagMap {
    /// Create a new cluster tag map with given capacity
    /// 
    /// The bitmap uses 1 bit per vector ID, so capacity of 1M vectors uses ~125KB.
    pub fn new(prefix: &str, capacity: u64, is_cluster_mode: bool) -> Self {
        // Calculate number of u64 words needed (64 bits per word)
        let num_words = ((capacity + 63) / 64) as usize;
        let bitmap: Vec<AtomicU64> = (0..num_words)
            .map(|_| AtomicU64::new(0))
            .collect();
        
        Self {
            prefix: prefix.to_string(),
            bitmap,
            capacity,
            count: AtomicU64::new(0),
            keys_scanned: AtomicU64::new(0),
            is_cluster_mode,
            update_mutex: Mutex::new(()),
            unmapped_counter: AtomicU64::new(0),
        }
    }

    /// Add a vector ID mapping (marks as existing)
    ///
    /// The cluster_tag parameter is ignored - cluster tags are no longer stored.
    /// This function just marks the vector ID as existing in the bitmap.
    pub fn add_mapping(&self, vector_id: u64, _cluster_tag: &str) {
        if vector_id >= self.capacity {
            return;
        }

        self.keys_scanned.fetch_add(1, Ordering::Relaxed);

        let word_idx = (vector_id / 64) as usize;
        let bit_idx = (vector_id % 64) as u32;
        let bit_mask = 1u64 << bit_idx;

        // Atomically set the bit
        let old_word = self.bitmap[word_idx].fetch_or(bit_mask, Ordering::Relaxed);
        
        // If bit wasn't already set, increment count
        if (old_word & bit_mask) == 0 {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Check if a vector exists in the cluster
    pub fn vector_exists(&self, vector_id: u64) -> bool {
        if vector_id >= self.capacity {
            return false;
        }

        let word_idx = (vector_id / 64) as usize;
        let bit_idx = (vector_id % 64) as u32;
        let bit_mask = 1u64 << bit_idx;

        (self.bitmap[word_idx].load(Ordering::Relaxed) & bit_mask) != 0
    }

    /// Get number of mapped vectors
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get number of keys scanned
    pub fn keys_scanned(&self) -> u64 {
        self.keys_scanned.load(Ordering::Relaxed)
    }

    /// Get capacity
    pub fn capacity(&self) -> usize {
        self.capacity as usize
    }

    /// Claim the next unmapped vector ID (for partial prefill support)
    ///
    /// Atomically finds and claims the next vector ID that doesn't exist in the map.
    /// This allows multiple workers to efficiently skip existing vectors without
    /// duplicate work.
    ///
    /// Returns None when all vectors up to max_id have been processed or mapped.
    pub fn claim_unmapped_id(&self, max_id: u64) -> Option<u64> {
        loop {
            let candidate = self.unmapped_counter.fetch_add(1, Ordering::Relaxed);
            if candidate >= max_id {
                return None; // All vectors processed
            }
            if !self.vector_exists(candidate) {
                return Some(candidate);
            }
            // Vector already exists, try next one
        }
    }

    /// Reset the unmapped counter (call before starting vec-load)
    pub fn reset_unmapped_counter(&self) {
        self.unmapped_counter.store(0, Ordering::Relaxed);
    }

    /// Get current unmapped counter value (for progress tracking)
    pub fn unmapped_counter_value(&self) -> u64 {
        self.unmapped_counter.load(Ordering::Relaxed)
    }
    
    /// Get memory usage in bytes
    pub fn memory_usage_bytes(&self) -> usize {
        self.bitmap.len() * std::mem::size_of::<AtomicU64>()
    }
}

/// Configuration for cluster scan
pub struct ClusterScanConfig {
    /// Key pattern to scan for (e.g., "vec:*")
    pub pattern: String,
    /// SCAN batch size
    pub batch_size: usize,
    /// Connection timeout
    pub timeout: Duration,
    /// Whether to show progress
    pub show_progress: bool,
}

impl Default for ClusterScanConfig {
    fn default() -> Self {
        Self {
            pattern: "*".to_string(),
            batch_size: 1000,
            timeout: Duration::from_secs(5),
            show_progress: true,
        }
    }
}

/// Results from cluster scan
#[derive(Debug, Clone)]
pub struct ClusterScanResults {
    /// Total keys processed
    pub total_keys: u64,
    /// Total scan time in milliseconds
    pub total_time_ms: u64,
    /// Number of nodes scanned
    pub nodes_scanned: usize,
    /// Errors encountered
    pub errors: usize,
    /// Keys per second
    pub keys_per_second: f64,
}

/// Extract vector ID from a key
///
/// Uses the unified key format from workload::key_format module.
/// Key format: `prefix + vector_id`
/// Example: `vec:000123`
///
/// Returns (vector_id, empty_string) for backward compatibility.
/// The cluster_tag return value is always empty since tags are no longer used.
pub fn parse_vector_key(key: &str, prefix: &str) -> Option<(u64, String)> {
    use crate::workload::key_format::{KeyFormat, DEFAULT_KEY_WIDTH};

    let format = KeyFormat::new(prefix, DEFAULT_KEY_WIDTH);
    let (vector_id, _tag_opt) = format.parse_key(key)?;

    // Return empty string for cluster_tag (backward compat)
    Some((vector_id, String::new()))
}

/// Build vector ID mappings by scanning cluster nodes
///
/// This function scans all primary nodes in parallel to discover existing keys
/// and build a mapping from vector_id to cluster_tag.
pub fn build_vector_id_mappings(
    tag_map: &ClusterTagMap,
    nodes: &[ClusterNode],
    config: &ClusterScanConfig,
) -> Result<ClusterScanResults, String> {
    use std::sync::Arc;
    use std::thread;

    let start_time = Instant::now();
    let total_keys = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));

    // Filter to primary nodes only
    let primaries: Vec<_> = nodes.iter().filter(|n| n.is_primary).collect();

    if primaries.is_empty() {
        return Err("No primary nodes found".to_string());
    }

    if config.show_progress {
        println!(
            "[CLUSTER-SCAN] Scanning {} primary nodes for pattern '{}'",
            primaries.len(),
            config.pattern
        );
    }

    // Scan each node in a separate thread
    let handles: Vec<_> = primaries
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let host = node.host.clone();
            let port = node.port;
            let pattern = config.pattern.clone();
            let batch_size = config.batch_size;
            let timeout = config.timeout;
            let prefix = tag_map.prefix.clone();
            let total_keys = Arc::clone(&total_keys);
            let errors = Arc::clone(&errors);

            // We need to pass the tag_map reference carefully
            // Since ClusterTagMap uses interior mutability, we can share it across threads
            let tag_map_ptr = tag_map as *const ClusterTagMap as usize;

            thread::spawn(move || {
                let result = scan_node(tag_map_ptr, &host, port, &pattern, batch_size, timeout, &prefix, idx);
                match result {
                    Ok(keys) => {
                        total_keys.fetch_add(keys, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("[CLUSTER-SCAN] Worker {}: Error: {}", idx, e);
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        let _ = handle.join();
    }

    let elapsed = start_time.elapsed();
    let total_time_ms = elapsed.as_millis() as u64;
    let total_keys_count = total_keys.load(Ordering::Relaxed);
    let keys_per_second = if total_time_ms > 0 {
        (total_keys_count as f64 * 1000.0) / total_time_ms as f64
    } else {
        0.0
    };

    let results = ClusterScanResults {
        total_keys: total_keys_count,
        total_time_ms,
        nodes_scanned: primaries.len(),
        errors: errors.load(Ordering::Relaxed) as usize,
        keys_per_second,
    };

    if config.show_progress {
        println!(
            "[CLUSTER-SCAN] Complete: {} keys in {}ms ({:.1} keys/sec)",
            results.total_keys, results.total_time_ms, results.keys_per_second
        );
        println!(
            "[CLUSTER-SCAN] Mapped {} vectors from {} scanned keys",
            tag_map.count(),
            tag_map.keys_scanned()
        );
    }

    Ok(results)
}

/// Scan a single node for keys matching pattern
fn scan_node(
    tag_map_ptr: usize,
    host: &str,
    port: u16,
    pattern: &str,
    batch_size: usize,
    timeout: Duration,
    prefix: &str,
    _worker_id: usize,
) -> Result<u64, String> {
    // Reconstruct tag_map reference
    let tag_map = unsafe { &*(tag_map_ptr as *const ClusterTagMap) };

    // Connect to node
    let mut conn = RawConnection::connect_tcp(host, port, timeout)
        .map_err(|e| format!("Connection failed: {}", e))?;

    let mut cursor: u64 = 0;
    let mut keys_processed: u64 = 0;

    loop {
        // Build SCAN command
        let mut encoder = RespEncoder::with_capacity(128);
        encoder.encode_command_str(&[
            "SCAN",
            &cursor.to_string(),
            "MATCH",
            pattern,
            "COUNT",
            &batch_size.to_string(),
        ]);

        let reply = conn.execute_encoded(&encoder).map_err(|e| format!("SCAN failed: {}", e))?;

        // Parse response: [cursor, [keys...]]
        let (new_cursor, keys) = match reply {
            RespValue::Array(arr) if arr.len() == 2 => {
                let cur = match &arr[0] {
                    RespValue::BulkString(s) => {
                        String::from_utf8_lossy(s).parse::<u64>().unwrap_or(0)
                    }
                    RespValue::Integer(i) => *i as u64,
                    _ => 0,
                };

                let key_list = match &arr[1] {
                    RespValue::Array(keys) => keys
                        .iter()
                        .filter_map(|k| match k {
                            RespValue::BulkString(s) => {
                                String::from_utf8(s.clone()).ok()
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                    _ => vec![],
                };

                (cur, key_list)
            }
            _ => return Err("Invalid SCAN response".to_string()),
        };

        cursor = new_cursor;

        // Process keys
        for key in keys {
            if let Some((vector_id, cluster_tag)) = parse_vector_key(&key, prefix) {
                tag_map.add_mapping(vector_id, &cluster_tag);
            }
            keys_processed += 1;
        }

        // Done when cursor returns to 0
        if cursor == 0 {
            break;
        }
    }

    Ok(keys_processed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vector_key() {
        // Simple format: prefix + id (no cluster tags)
        let result = parse_vector_key("vec:000123", "vec:");
        assert_eq!(result, Some((123, String::new())));

        // With more digits
        let result = parse_vector_key("vec:000456", "vec:");
        assert_eq!(result, Some((456, String::new())));

        // Wrong prefix
        let result = parse_vector_key("other:000123", "vec:");
        assert!(result.is_none());
    }

    #[test]
    fn test_cluster_tag_map_bitmap() {
        let map = ClusterTagMap::new("vec:", 1000, true);

        // Tags are ignored, just marks existence
        map.add_mapping(0, "");
        map.add_mapping(1, "");
        map.add_mapping(999, "");

        assert!(map.vector_exists(0));
        assert!(map.vector_exists(1));
        assert!(map.vector_exists(999));
        assert!(!map.vector_exists(2));
        assert!(!map.vector_exists(500));

        assert_eq!(map.count(), 3);
    }

    #[test]
    fn test_bitmap_memory_efficiency() {
        // 1M vectors should use ~128KB (1M / 8 bytes)
        let map = ClusterTagMap::new("vec:", 1_000_000, true);
        
        // Each u64 holds 64 bits, so 1M vectors needs ~15625 u64s
        // 15625 * 8 bytes = 125KB
        let expected_bytes = ((1_000_000 + 63) / 64) * 8;
        assert_eq!(map.memory_usage_bytes(), expected_bytes as usize);
    }

    #[test]
    fn test_bitmap_boundary_cases() {
        let map = ClusterTagMap::new("vec:", 128, true);

        // Test at word boundaries (every 64 bits)
        map.add_mapping(0, "");
        map.add_mapping(63, "");
        map.add_mapping(64, "");
        map.add_mapping(127, "");

        assert!(map.vector_exists(0));
        assert!(map.vector_exists(63));
        assert!(map.vector_exists(64));
        assert!(map.vector_exists(127));
        assert!(!map.vector_exists(1));
        assert!(!map.vector_exists(65));

        assert_eq!(map.count(), 4);
    }

    #[test]
    fn test_duplicate_add() {
        let map = ClusterTagMap::new("vec:", 100, true);

        // Adding same ID twice should only count once
        map.add_mapping(42, "");
        map.add_mapping(42, "");
        map.add_mapping(42, "");

        assert!(map.vector_exists(42));
        assert_eq!(map.count(), 1);
        assert_eq!(map.keys_scanned(), 3);
    }

    #[test]
    fn test_out_of_bounds() {
        let map = ClusterTagMap::new("vec:", 100, true);

        // Should safely ignore out-of-bounds IDs
        map.add_mapping(100, "");
        map.add_mapping(1000, "");

        assert!(!map.vector_exists(100));
        assert!(!map.vector_exists(1000));
        assert_eq!(map.count(), 0);
    }

    #[test]
    fn test_claim_unmapped_id() {
        let map = ClusterTagMap::new("vec:", 10, true);

        // Mark some as existing
        map.add_mapping(0, "");
        map.add_mapping(2, "");
        map.add_mapping(4, "");

        // Should skip existing IDs
        assert_eq!(map.claim_unmapped_id(10), Some(1));
        assert_eq!(map.claim_unmapped_id(10), Some(3));
        assert_eq!(map.claim_unmapped_id(10), Some(5));
        assert_eq!(map.claim_unmapped_id(10), Some(6));
    }

    #[test]
    fn test_non_cluster_mode() {
        let map = ClusterTagMap::new("vec:", 1000, false);

        map.add_mapping(0, "");
        
        // Works the same in non-cluster mode
        assert!(map.vector_exists(0));
        assert!(!map.vector_exists(1));
    }
}
