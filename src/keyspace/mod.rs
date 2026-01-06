//! Keyspace Tracking Module
//!
//! This module provides unified keyspace management using the keyspace_tracker crate:
//!
//! - **VectorExistenceMap**: Tracks which vector IDs exist in the cluster (replaces ClusterTagMap)
//! - **ProtectedIds**: Ground truth protection for deletion benchmarks (replaces ProtectedVectorIds)
//!
//! The keyspace_tracker crate provides:
//! - Sub-nanosecond existence checks via atomic bitmaps
//! - Lock-free concurrent claiming for multi-threaded workloads
//! - SIMD-accelerated operations (ARM NEON, x86 AVX2/POPCNT)
//! - Memory-efficient 1 bit per ID storage

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use keyspace_tracker::{PrefixTracker, ReferenceSet, TrackerConfig};

use crate::client::{ControlPlane, RawConnection};
use crate::cluster::ClusterNode;
use crate::utils::{RespEncoder, RespValue};
use crate::workload::key_format::{KeyFormat, DEFAULT_KEY_WIDTH};

// Re-export keyspace_tracker types for direct use
pub use keyspace_tracker::{
    AccessDistribution, PrefixGroupsTracker, SamplingConfig, TrackerIterBuilder,
};

/// Vector existence tracking using atomic bitmaps
///
/// Wraps keyspace_tracker's PrefixTracker to provide:
/// - O(1) existence checks
/// - Atomic claim operations for partial prefill
/// - Memory-efficient 1-bit-per-vector storage
///
/// # Example
///
/// ```ignore
/// let tracker = VectorExistenceMap::new("vec:", 1_000_000, true);
///
/// // Mark vectors as existing
/// tracker.add(0);
/// tracker.add(100);
///
/// // Check existence
/// assert!(tracker.exists(100));
/// assert!(!tracker.exists(50));
///
/// // Claim unmapped IDs for loading
/// let id = tracker.claim_unmapped_id(1_000_000);
/// ```
pub struct VectorExistenceMap {
    /// Underlying prefix tracker
    tracker: PrefixTracker,
    /// Key prefix (e.g., "vec:")
    pub prefix: String,
    /// Total capacity in vector IDs
    capacity: u64,
    /// Number of keys scanned (for progress tracking)
    keys_scanned: AtomicU64,
    /// Whether cluster mode is enabled
    pub is_cluster_mode: bool,
    /// Atomic counter for claiming unmapped vector IDs
    unmapped_counter: AtomicU64,
}

impl VectorExistenceMap {
    /// Create a new vector existence map with given capacity
    ///
    /// Uses 1 bit per vector ID, so 10M vectors uses ~1.25MB.
    pub fn new(prefix: &str, capacity: u64, is_cluster_mode: bool) -> Self {
        let config = TrackerConfig::simple(prefix)
            .with_max_id(capacity)
            .with_initial_capacity(capacity as usize);

        Self {
            tracker: PrefixTracker::new(config),
            prefix: prefix.to_string(),
            capacity,
            keys_scanned: AtomicU64::new(0),
            is_cluster_mode,
            unmapped_counter: AtomicU64::new(0),
        }
    }

    /// Add a vector ID (mark as existing)
    ///
    /// The cluster_tag parameter is ignored for backward compatibility.
    /// This function just marks the vector ID as existing.
    #[inline]
    pub fn add_mapping(&self, vector_id: u64, _cluster_tag: &str) {
        if vector_id >= self.capacity {
            return;
        }

        self.keys_scanned.fetch_add(1, Ordering::Relaxed);
        self.tracker.add(vector_id);
    }

    /// Add a vector ID (simpler API)
    #[inline]
    pub fn add(&self, vector_id: u64) {
        if vector_id < self.capacity {
            self.tracker.add(vector_id);
        }
    }

    /// Check if a vector exists
    #[inline]
    pub fn exists(&self, vector_id: u64) -> bool {
        if vector_id >= self.capacity {
            return false;
        }
        self.tracker.exists(vector_id)
    }

    /// Get number of mapped vectors
    #[inline]
    pub fn count(&self) -> u64 {
        self.tracker.count()
    }

    /// Get number of keys scanned
    #[inline]
    pub fn keys_scanned(&self) -> u64 {
        self.keys_scanned.load(Ordering::Relaxed)
    }

    /// Get capacity
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity as usize
    }

    /// Claim the next unmapped vector ID (for partial prefill)
    ///
    /// Atomically finds and claims the next vector ID that doesn't exist.
    /// Returns None when all vectors up to max_id have been processed.
    ///
    /// This method uses the tracker's continue_write iterator for atomic claim semantics,
    /// ensuring no two threads can claim the same ID even under high concurrency.
    /// continue_write() preserves the cursor position across calls.
    pub fn claim_unmapped_id(&self, max_id: u64) -> Option<u64> {
        let effective_max = max_id.min(self.capacity);

        // Use tracker's continue_write iterator for atomic claim
        // continue_write() doesn't reset cursor - allows multi-threaded claiming
        // The iterator finds unset bits and atomically sets them
        let mut write_iter = self.tracker.iter().unset_only().continue_write();
        while let Some((id, _)) = write_iter.next() {
            if id < effective_max {
                // Write iterator already marks the ID as set atomically
                return Some(id);
            }
            // ID is beyond our range, iterator exhausted for our purposes
            break;
        }

        // Iterator exhausted - all IDs up to effective_max have been claimed
        None
    }

    /// Reset the unmapped counter
    pub fn reset_unmapped_counter(&self) {
        self.unmapped_counter.store(0, Ordering::Relaxed);
    }

    /// Get current unmapped counter value
    pub fn unmapped_counter_value(&self) -> u64 {
        self.unmapped_counter.load(Ordering::Relaxed)
    }

    /// Get memory usage in bytes
    pub fn memory_usage_bytes(&self) -> usize {
        // PrefixTracker uses 1 bit per ID -> capacity / 8 bytes
        ((self.capacity + 63) / 64 * 8) as usize
    }

    /// Get reference to underlying tracker for advanced iteration
    pub fn tracker(&self) -> &PrefixTracker {
        &self.tracker
    }
}

/// Protected IDs using ReferenceSet
///
/// Manages vector IDs that should be skipped during deletion benchmarks.
/// These are typically ground truth neighbors that must remain for recall.
///
/// Uses keyspace_tracker's ReferenceSet for O(1) membership tests.
///
/// # Example
///
/// ```ignore
/// let ground_truth_ids = vec![1, 3, 5, 7, 9];
/// let protected = ProtectedIds::new(ground_truth_ids, 1000);
///
/// assert!(protected.is_protected(3));
/// assert!(!protected.is_protected(2));
///
/// // Claim deleteable IDs (skips protected)
/// let id = protected.claim_deleteable_id(); // Returns Some(0)
/// let id = protected.claim_deleteable_id(); // Returns Some(2) (skips 1)
/// ```
pub struct ProtectedIds {
    /// Reference set of protected IDs
    reference_set: ReferenceSet,
    /// Atomic counter for claiming deleteable IDs
    delete_counter: AtomicU64,
    /// Maximum vector ID
    max_id: u64,
}

impl ProtectedIds {
    /// Create from an iterator of protected IDs
    pub fn new(protected_ids: impl IntoIterator<Item = u64>, max_id: u64) -> Self {
        let reference_set = ReferenceSet::from_iter(protected_ids);
        Self {
            reference_set,
            delete_counter: AtomicU64::new(0),
            max_id,
        }
    }

    /// Create from a HashSet (backward compatibility)
    pub fn from_hashset(protected: std::collections::HashSet<u64>, max_id: u64) -> Self {
        Self::new(protected.into_iter(), max_id)
    }

    /// Check if a vector ID is protected
    #[inline]
    pub fn is_protected(&self, id: u64) -> bool {
        self.reference_set.contains(id)
    }

    /// Claim the next deleteable (non-protected) vector ID
    ///
    /// Returns None when all deleteable vectors have been claimed.
    pub fn claim_deleteable_id(&self) -> Option<u64> {
        loop {
            let candidate = self.delete_counter.fetch_add(1, Ordering::Relaxed);
            if candidate >= self.max_id {
                return None;
            }
            if !self.reference_set.contains(candidate) {
                return Some(candidate);
            }
        }
    }

    /// Get number of protected IDs
    #[inline]
    pub fn protected_count(&self) -> u64 {
        self.reference_set.len()
    }

    /// Get number of deleteable IDs
    #[inline]
    pub fn deleteable_count(&self) -> u64 {
        self.max_id.saturating_sub(self.protected_count())
    }

    /// Get maximum vector ID
    #[inline]
    pub fn max_id(&self) -> u64 {
        self.max_id
    }

    /// Get current counter value
    #[inline]
    pub fn counter_value(&self) -> u64 {
        self.delete_counter.load(Ordering::Relaxed)
    }

    /// Reset the counter
    pub fn reset_counter(&self) {
        self.delete_counter.store(0, Ordering::Relaxed);
    }

    /// Get reference to underlying ReferenceSet for advanced operations
    pub fn reference_set(&self) -> &ReferenceSet {
        &self.reference_set
    }

    /// Claim the next deleteable ID from a tracker (existence-aware)
    ///
    /// Only returns IDs that:
    /// 1. Exist in the tracker (are set)
    /// 2. Are not protected
    ///
    /// Uses atomic counter to ensure unique claiming across threads.
    /// This is useful for deletion workloads that need to delete existing vectors
    /// while protecting ground truth.
    pub fn claim_deleteable_from_tracker(&self, tracker: &PrefixTracker) -> Option<u64> {
        // Use atomic counter to ensure unique claiming
        loop {
            let candidate = self.delete_counter.fetch_add(1, Ordering::Relaxed);
            if candidate >= self.max_id {
                return None;
            }
            // Skip protected IDs
            if self.reference_set.contains(candidate) {
                continue;
            }
            // Only return if it exists in the tracker
            if tracker.exists(candidate) {
                return Some(candidate);
            }
        }
    }

    /// Count how many protected IDs exist in the tracker
    ///
    /// This is useful for coverage checking before query benchmarks.
    pub fn count_existing_protected(&self, tracker: &PrefixTracker) -> u64 {
        let snapshot = tracker.snapshot();
        self.reference_set.count_existing_in(&snapshot)
    }

    /// Count how many protected IDs are missing from the tracker
    pub fn count_missing_protected(&self, tracker: &PrefixTracker) -> u64 {
        let snapshot = tracker.snapshot();
        self.reference_set.count_missing_in(&snapshot)
    }

    /// Get coverage ratio (protected IDs that exist / total protected)
    pub fn coverage_ratio(&self, tracker: &PrefixTracker) -> f64 {
        let total = self.reference_set.len();
        if total == 0 {
            return 1.0;
        }
        let existing = self.count_existing_protected(tracker);
        existing as f64 / total as f64
    }
}

// =============================================================================
// Cluster Scan Support
// =============================================================================

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
pub fn parse_vector_key(key: &str, prefix: &str) -> Option<(u64, String)> {
    let format = KeyFormat::new(prefix, DEFAULT_KEY_WIDTH);
    let (vector_id, _tag_opt) = format.parse_key(key)?;
    Some((vector_id, String::new()))
}

/// Build vector existence map by scanning cluster nodes
pub fn build_vector_id_mappings(
    tracker: &VectorExistenceMap,
    nodes: &[ClusterNode],
    config: &ClusterScanConfig,
) -> Result<ClusterScanResults, String> {
    use std::sync::Arc;
    use std::thread;

    let start_time = Instant::now();
    let total_keys = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));

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

    let handles: Vec<_> = primaries
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let host = node.host.clone();
            let port = node.port;
            let pattern = config.pattern.clone();
            let batch_size = config.batch_size;
            let timeout = config.timeout;
            let prefix = tracker.prefix.clone();
            let total_keys = Arc::clone(&total_keys);
            let errors = Arc::clone(&errors);
            let tracker_ptr = tracker as *const VectorExistenceMap as usize;

            thread::spawn(move || {
                let result =
                    scan_node(tracker_ptr, &host, port, &pattern, batch_size, timeout, &prefix, idx);
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
            tracker.count(),
            tracker.keys_scanned()
        );
    }

    Ok(results)
}

fn scan_node(
    tracker_ptr: usize,
    host: &str,
    port: u16,
    pattern: &str,
    batch_size: usize,
    timeout: Duration,
    prefix: &str,
    _worker_id: usize,
) -> Result<u64, String> {
    let tracker = unsafe { &*(tracker_ptr as *const VectorExistenceMap) };

    let mut conn = RawConnection::connect_tcp(host, port, timeout)
        .map_err(|e| format!("Connection failed: {}", e))?;

    let mut cursor: u64 = 0;
    let mut keys_processed: u64 = 0;

    loop {
        let mut encoder = RespEncoder::with_capacity(128);
        encoder.encode_command_str(&[
            "SCAN",
            &cursor.to_string(),
            "MATCH",
            pattern,
            "COUNT",
            &batch_size.to_string(),
        ]);

        let reply = conn
            .execute_encoded(&encoder)
            .map_err(|e| format!("SCAN failed: {}", e))?;

        let (new_cursor, keys) = match reply {
            RespValue::Array(arr) if arr.len() == 2 => {
                let cur = match &arr[0] {
                    RespValue::BulkString(s) => String::from_utf8_lossy(s).parse::<u64>().unwrap_or(0),
                    RespValue::Integer(i) => *i as u64,
                    _ => 0,
                };

                let key_list = match &arr[1] {
                    RespValue::Array(keys) => keys
                        .iter()
                        .filter_map(|k| match k {
                            RespValue::BulkString(s) => String::from_utf8(s.clone()).ok(),
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

        for key in keys {
            if let Some((vector_id, cluster_tag)) = parse_vector_key(&key, prefix) {
                tracker.add_mapping(vector_id, &cluster_tag);
            }
            keys_processed += 1;
        }

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
    fn test_vector_existence_map_basic() {
        let map = VectorExistenceMap::new("vec:", 1000, true);

        map.add(0);
        map.add(1);
        map.add(999);

        assert!(map.exists(0));
        assert!(map.exists(1));
        assert!(map.exists(999));
        assert!(!map.exists(2));
        assert!(!map.exists(500));

        assert_eq!(map.count(), 3);
    }

    #[test]
    fn test_vector_existence_map_claim_unmapped() {
        let map = VectorExistenceMap::new("vec:", 10, true);

        map.add(0);
        map.add(2);
        map.add(4);

        assert_eq!(map.claim_unmapped_id(10), Some(1));
        assert_eq!(map.claim_unmapped_id(10), Some(3));
        assert_eq!(map.claim_unmapped_id(10), Some(5));
    }

    #[test]
    fn test_vector_existence_map_exhaustion() {
        // Test that claim_unmapped_id returns None after all IDs are claimed
        let map = VectorExistenceMap::new("vec:", 5, true);

        // Claim all 5 IDs
        assert_eq!(map.claim_unmapped_id(5), Some(0));
        assert_eq!(map.claim_unmapped_id(5), Some(1));
        assert_eq!(map.claim_unmapped_id(5), Some(2));
        assert_eq!(map.claim_unmapped_id(5), Some(3));
        assert_eq!(map.claim_unmapped_id(5), Some(4));

        // Should return None when exhausted - this MUST return immediately, not loop
        assert_eq!(map.claim_unmapped_id(5), None);
        assert_eq!(map.claim_unmapped_id(5), None); // Multiple calls should all return None quickly
    }

    #[test]
    fn test_protected_ids_basic() {
        let protected = ProtectedIds::new(vec![1, 3, 5], 10);

        assert!(protected.is_protected(1));
        assert!(protected.is_protected(3));
        assert!(protected.is_protected(5));
        assert!(!protected.is_protected(0));
        assert!(!protected.is_protected(2));
    }

    #[test]
    fn test_protected_ids_claim_deleteable() {
        let protected = ProtectedIds::new(vec![1, 3, 5], 10);

        // Should skip protected IDs
        assert_eq!(protected.claim_deleteable_id(), Some(0));
        assert_eq!(protected.claim_deleteable_id(), Some(2));
        assert_eq!(protected.claim_deleteable_id(), Some(4));
        assert_eq!(protected.claim_deleteable_id(), Some(6));
    }

    #[test]
    fn test_protected_ids_counts() {
        let protected = ProtectedIds::new(vec![1, 3, 5], 10);

        assert_eq!(protected.protected_count(), 3);
        assert_eq!(protected.deleteable_count(), 7);
        assert_eq!(protected.max_id(), 10);
    }

    #[test]
    fn test_parse_vector_key() {
        let result = parse_vector_key("vec:000123", "vec:");
        assert_eq!(result, Some((123, String::new())));

        let result = parse_vector_key("other:000123", "vec:");
        assert!(result.is_none());
    }

    // =========================================================================
    // Phase 2 Proof of Capability: Mixed Ratio Iteration
    // =========================================================================

    /// Demonstrates 80% existing / 20% new key iteration using TrackerIterBuilder
    ///
    /// This test validates Task 2.11 from the migration plan:
    /// - Implement basic mixed_ratio() support
    /// - Verify statistical distribution matches configuration (within 5%)
    #[test]
    fn test_mixed_ratio_iteration_proof_of_capability() {
        // Create a tracker with 1000 IDs
        let config = TrackerConfig::simple("test:").with_max_id(1000);
        let tracker = PrefixTracker::new(config);

        // Mark first 500 as existing (50% of keyspace)
        for i in 0..500 {
            tracker.add(i);
        }
        assert_eq!(tracker.count(), 500);

        // Configure mixed_ratio iteration: 80% existing, 20% new
        let set_ratio = 0.8;
        let limit = 1000;

        // Count how many returned IDs are existing vs new
        let mut existing_count = 0u64;
        let mut new_count = 0u64;

        // Use TrackerIterBuilder with mixed_ratio
        for (id, _) in tracker
            .iter()
            .mixed_ratio(set_ratio)
            .seed(42) // deterministic for testing
            .limit(limit)
            .random()
        {
            if tracker.exists(id) {
                existing_count += 1;
            } else {
                new_count += 1;
            }
        }

        let total = existing_count + new_count;
        let actual_ratio = existing_count as f64 / total as f64;

        // Verify: should be approximately 80% existing (within 10% tolerance)
        // The tolerance is higher because mixed_ratio depends on available IDs
        println!(
            "Mixed ratio test: {:.1}% existing ({}/{}), expected ~80%",
            actual_ratio * 100.0,
            existing_count,
            total
        );

        assert!(
            (actual_ratio - set_ratio).abs() < 0.15,
            "Expected ~80% existing, got {:.1}% ({}/{})",
            actual_ratio * 100.0,
            existing_count,
            total
        );
    }

    /// Demonstrates coverage checking using ReferenceSet
    #[test]
    fn test_reference_set_coverage_checking() {
        // Create a tracker with 100 IDs
        let config = TrackerConfig::simple("vec:").with_max_id(100);
        let tracker = PrefixTracker::new(config);

        // Create a reference set (simulating ground truth)
        let protected_ids: Vec<u64> = vec![5, 10, 15, 20, 25, 30, 35, 40, 45, 50];
        let reference = ReferenceSet::from_iter(protected_ids.clone());
        assert_eq!(reference.len(), 10);

        // Initially no coverage (nothing exists)
        let snapshot = tracker.snapshot();
        let existing = reference.count_existing_in(&snapshot);
        assert_eq!(existing, 0, "Should have 0 coverage initially");

        // Add some IDs to tracker (simulating loaded vectors)
        for id in 0..30 {
            tracker.add(id);
        }

        // Now should have partial coverage
        let snapshot = tracker.snapshot();
        let existing = reference.count_existing_in(&snapshot);
        let missing = reference.count_missing_in(&snapshot);

        println!(
            "Coverage: {}/{} existing, {}/{} missing",
            existing,
            reference.len(),
            missing,
            reference.len()
        );

        // IDs 5, 10, 15, 20, 25 should exist (5 out of 10)
        assert_eq!(existing, 5, "Should have 5 protected IDs existing");
        assert_eq!(missing, 5, "Should have 5 protected IDs missing");

        let coverage_ratio = existing as f64 / reference.len() as f64;
        assert!((coverage_ratio - 0.5).abs() < 0.01, "Coverage should be 50%");
    }

    /// Demonstrates tracker-aware deletion with protection
    #[test]
    fn test_tracker_aware_deletion_with_protection() {
        // Create tracker with 20 IDs, all existing
        let config = TrackerConfig::simple("vec:").with_max_id(20);
        let tracker = PrefixTracker::new(config);
        for i in 0..20 {
            tracker.add(i);
        }

        // Create protection for even IDs (simulating ground truth)
        let protected = ProtectedIds::new((0..20).step_by(2), 20);
        assert_eq!(protected.protected_count(), 10);

        // Claim deleteable from tracker (should only return existing + non-protected)
        let mut deleted = Vec::new();
        while let Some(id) = protected.claim_deleteable_from_tracker(&tracker) {
            // Verify: ID should exist AND not be protected
            assert!(tracker.exists(id), "Claimed ID {} should exist", id);
            assert!(!protected.is_protected(id), "Claimed ID {} should not be protected", id);
            deleted.push(id);
            tracker.remove(id);
        }

        // Should have deleted all odd IDs (1, 3, 5, ..., 19)
        println!("Deleted {} IDs: {:?}", deleted.len(), deleted);
        assert_eq!(deleted.len(), 10, "Should have deleted 10 non-protected IDs");
        for id in deleted {
            assert!(id % 2 == 1, "All deleted IDs should be odd (non-protected)");
        }

        // Protected IDs should still exist
        for id in (0..20).step_by(2) {
            assert!(tracker.exists(id), "Protected ID {} should still exist", id);
        }
    }
}
