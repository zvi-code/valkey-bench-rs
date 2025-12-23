//! Index backfill progress tracking
//!
//! Provides functions to monitor index backfill progress across cluster nodes,
//! supporting both EC (ElastiCache Valkey) and MemoryDB engines.
//!
//! This module implements the same logic as the C code for accurate progress detection.

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};

use super::ft_info::{
    convert_ftinfo_to_lines, fields, get_field_or, parse_ftinfo_lines, validate_memdb_ftinfo,
    validate_search_info, EngineType, IndexStatus, ResponseFormat,
};
use crate::client::{ControlPlane, RawConnection};
use crate::cluster::ClusterNode;
use crate::utils::{RespEncoder, RespValue};

// ============================================================================
// Node Progress
// ============================================================================

/// Progress information for a single node
#[derive(Debug, Clone)]
pub struct NodeProgress {
    /// Node identifier
    pub node_id: String,
    /// Number of documents indexed
    pub num_docs: i64,
    /// Progress percentage (0-100)
    pub progress_percent: i32,
    /// Whether backfill is in progress
    pub in_progress: bool,
    /// Index status
    pub status: IndexStatus,
}

impl Default for NodeProgress {
    fn default() -> Self {
        Self {
            node_id: String::new(),
            num_docs: 0,
            progress_percent: 100,
            in_progress: false,
            status: IndexStatus::Available,
        }
    }
}

// ============================================================================
// EC (ElastiCache Valkey) Progress Detection
// ============================================================================

/// Get node progress for EC (ElastiCache Valkey)
///
/// Matches C code logic:
/// - Parse backfill_in_progress (default 0)
/// - Parse backfill_complete_percent (default 0.0)
/// - progress_percent = backfill_complete_percent * 100
/// - Return backfill_in_progress as in_progress flag
pub fn get_node_progress_ec(
    conn: &mut RawConnection,
    index_name: &str,
) -> Result<NodeProgress, String> {
    let mut encoder = RespEncoder::with_capacity(128);
    encoder.encode_command_str(&["FT.INFO", index_name]);

    let reply = conn
        .execute_encoded(&encoder)
        .map_err(|e| format!("FT.INFO failed: {}", e))?;

    let lines = convert_ftinfo_to_lines(&reply, ResponseFormat::ElastiCache);
    let info = parse_ftinfo_lines(&lines);

    // Extract fields with C-compatible defaults
    let backfill_in_progress: i32 =
        get_field_or(&info, fields::ec::BACKFILL_IN_PROGRESS, 0);
    let backfill_complete_percent: f64 =
        get_field_or(&info, fields::ec::BACKFILL_COMPLETE_PERCENT, 0.0);
    let num_docs: i64 = get_field_or(&info, fields::ec::NUM_DOCS, 0);

    // Determine progress based on backfill state
    // - If in_progress: progress = backfill_complete_percent * 100
    // - If complete: progress = 100
    let (in_progress, progress_percent) = if backfill_in_progress != 0 {
        (true, (backfill_complete_percent * 100.0) as i32)
    } else {
        (false, 100)
    };

    Ok(NodeProgress {
        node_id: String::new(),
        num_docs,
        progress_percent,
        in_progress,
        status: if in_progress {
            IndexStatus::Backfilling
        } else {
            IndexStatus::Available
        },
    })
}

// ============================================================================
// MemoryDB Progress Detection
// ============================================================================

/// Context for MemoryDB progress detection
pub struct MemoryDbProgressContext {
    /// Whether this node is a replica
    pub is_replica: bool,
}

impl Default for MemoryDbProgressContext {
    fn default() -> Self {
        Self { is_replica: false }
    }
}

/// Get node progress for MemoryDB
///
/// Matches C code logic exactly:
/// 1. Send FT.INFO and INFO SEARCH commands
/// 2. If FT.INFO is empty:
///    - If replica: return in_progress=true, progress=0
///    - Otherwise: error
/// 3. Validate required fields exist
/// 4. Determine progress based on status and degradation
pub fn get_node_progress_memorydb(
    conn: &mut RawConnection,
    index_name: &str,
    ctx: &MemoryDbProgressContext,
) -> Result<NodeProgress, String> {
    // Send FT.INFO command
    let mut encoder = RespEncoder::with_capacity(128);
    encoder.encode_command_str(&["FT.INFO", index_name]);

    let ft_info_reply = conn
        .execute_encoded(&encoder)
        .map_err(|e| format!("FT.INFO failed: {}", e))?;

    let ft_info_lines = convert_ftinfo_to_lines(&ft_info_reply, ResponseFormat::MemoryDb);
    let ft_info = parse_ftinfo_lines(&ft_info_lines);

    // Send INFO SEARCH command
    let mut encoder = RespEncoder::with_capacity(64);
    encoder.encode_command_str(&["INFO", "SEARCH"]);

    let search_info_reply = conn
        .execute_encoded(&encoder)
        .map_err(|e| format!("INFO SEARCH failed: {}", e))?;

    let search_info_lines = match &search_info_reply {
        RespValue::BulkString(data) => String::from_utf8_lossy(data.as_slice()).to_string(),
        _ => convert_ftinfo_to_lines(&search_info_reply, ResponseFormat::MemoryDb),
    };
    let search_info = parse_ftinfo_lines(&search_info_lines);

    // Handle empty FT.INFO response (C code logic)
    if ft_info_lines.is_empty() {
        if ctx.is_replica {
            // Replica might be lagging, return in progress
            return Ok(NodeProgress {
                node_id: String::new(),
                num_docs: 0,
                progress_percent: 0,
                in_progress: true,
                status: IndexStatus::Backfilling,
            });
        } else {
            return Err("Empty FT.INFO response from primary node".to_string());
        }
    }

    // Validate required fields (C code asserts these exist)
    validate_memdb_ftinfo(&ft_info).map_err(|e| e.message)?;

    // Parse search_num_active_backfills (required)
    let active_backfills: i32 =
        get_field_or(&search_info, fields::search_info::NUM_ACTIVE_BACKFILLS, -1);
    if active_backfills < 0 {
        return Err(format!(
            "Missing {} in INFO SEARCH response",
            fields::search_info::NUM_ACTIVE_BACKFILLS
        ));
    }

    // Validate search_current_backfill_progress_percentage conditionally
    validate_search_info(&search_info, active_backfills).map_err(|e| e.message)?;

    // Parse FT.INFO fields
    let status_str = ft_info
        .get(fields::memdb::INDEX_STATUS)
        .map(|s| s.as_str())
        .unwrap_or("AVAILABLE");
    let status = IndexStatus::from_str(status_str);

    let degradation: i32 =
        get_field_or(&ft_info, fields::memdb::INDEX_DEGRADATION_PERCENTAGE, 0);
    let num_docs: i64 = get_field_or(&ft_info, fields::memdb::NUM_INDEXED_VECTORS, 0);

    // Determine progress (matching C code logic exactly)
    let (in_progress, progress_percent) =
        calculate_memdb_progress(&status, degradation, &search_info)?;

    // C code asserts status is AVAILABLE if not in backfilling state
    if !in_progress && !status.is_available() {
        return Err(format!(
            "Unexpected status '{:?}' when not backfilling",
            status
        ));
    }

    Ok(NodeProgress {
        node_id: String::new(),
        num_docs,
        progress_percent,
        in_progress,
        status,
    })
}

/// Calculate MemoryDB progress matching C code logic
///
/// Priority order (matching pseudocode):
/// 1. BACKFILLING: return in_progress=true, progress=search_backfill_progress
/// 2. QUEUED: return in_progress=true, progress=0 (regardless of degradation)
/// 3. degradation > 0: return in_progress=true, progress=100-degradation
/// 4. AVAILABLE: return in_progress=false, progress=100
fn calculate_memdb_progress(
    status: &IndexStatus,
    degradation: i32,
    search_info: &HashMap<String, String>,
) -> Result<(bool, i32), String> {
    // Check BACKFILLING first - use search progress
    if matches!(status, IndexStatus::Backfilling) {
        let progress = get_field_or(
            search_info,
            fields::search_info::BACKFILL_PROGRESS_PERCENTAGE,
            0,
        );
        return Ok((true, progress));
    }

    // Check QUEUED second - returns 0 regardless of degradation
    if matches!(status, IndexStatus::Queued) {
        return Ok((true, 0));
    }

    // Check degradation third
    if degradation > 0 {
        return Ok((true, 100 - degradation));
    }

    // AVAILABLE with no degradation
    Ok((false, 100))
}

/// Get node progress for MemoryDB (simple API without context)
pub fn get_node_progress_memorydb_simple(
    conn: &mut RawConnection,
    index_name: &str,
) -> Result<NodeProgress, String> {
    get_node_progress_memorydb(conn, index_name, &MemoryDbProgressContext::default())
}

// ============================================================================
// Unified Progress Detection
// ============================================================================

/// Get node progress based on engine type
pub fn get_node_progress(
    conn: &mut RawConnection,
    index_name: &str,
    engine_type: EngineType,
) -> Result<NodeProgress, String> {
    match engine_type {
        EngineType::MemoryDb => get_node_progress_memorydb_simple(conn, index_name),
        _ => get_node_progress_ec(conn, index_name),
    }
}

/// Get node progress with replica context (for MemoryDB)
pub fn get_node_progress_with_context(
    conn: &mut RawConnection,
    index_name: &str,
    engine_type: EngineType,
    is_replica: bool,
) -> Result<NodeProgress, String> {
    match engine_type {
        EngineType::MemoryDb => {
            let ctx = MemoryDbProgressContext { is_replica };
            get_node_progress_memorydb(conn, index_name, &ctx)
        }
        _ => get_node_progress_ec(conn, index_name),
    }
}

// ============================================================================
// Cluster-wide Progress
// ============================================================================

/// Cluster-wide backfill progress
#[derive(Debug, Clone)]
pub struct ClusterBackfillProgress {
    /// Total documents across all nodes
    pub total_docs: i64,
    /// Number of nodes still backfilling
    pub nodes_in_progress: usize,
    /// Average progress percentage
    pub progress_percent: i32,
    /// Per-node progress
    pub node_progress: Vec<NodeProgress>,
}

/// Configuration for waiting on backfill completion
#[derive(Debug, Clone)]
pub struct BackfillWaitConfig {
    /// Poll interval
    pub poll_interval: Duration,
    /// Initial delay before first check
    pub initial_delay: Duration,
    /// Maximum wait time (None for unlimited)
    pub max_wait: Option<Duration>,
    /// Whether to show progress bar
    pub show_progress: bool,
}

impl Default for BackfillWaitConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            initial_delay: Duration::from_secs(2),
            max_wait: None,
            show_progress: true,
        }
    }
}

/// Wait for index backfill to complete on all nodes
pub fn wait_for_index_backfill_complete<F>(
    engine_type: EngineType,
    index_names: &[&str],
    mut get_connection: F,
    node_count: usize,
    config: &BackfillWaitConfig,
) -> Result<ClusterBackfillProgress, String>
where
    F: FnMut(usize) -> Option<RawConnection>,
{
    if index_names.is_empty() {
        return Ok(ClusterBackfillProgress {
            total_docs: 0,
            nodes_in_progress: 0,
            progress_percent: 100,
            node_progress: vec![],
        });
    }

    let engine_name = match engine_type {
        EngineType::MemoryDb => "MemoryDB",
        _ => "ValkeySearch",
    };

    let index_list = index_names.join(", ");
    println!(
        "{}: Waiting for index{}: '{}' backfill to complete on all nodes...",
        engine_name,
        if index_names.len() > 1 { "es" } else { "" },
        index_list
    );

    thread::sleep(config.initial_delay);

    let progress_bar = if config.show_progress {
        let pb = ProgressBar::new(100);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}% | {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    let start_time = Instant::now();
    let mut last_progress = ClusterBackfillProgress {
        total_docs: 0,
        nodes_in_progress: node_count,
        progress_percent: 0,
        node_progress: vec![],
    };

    loop {
        if let Some(max_wait) = config.max_wait {
            if start_time.elapsed() > max_wait {
                if let Some(pb) = &progress_bar {
                    pb.finish_with_message("Timeout");
                }
                return Err("Backfill wait timeout".to_string());
            }
        }

        let mut total_docs = 0i64;
        let mut nodes_in_progress = 0usize;
        let mut total_progress = 0i32;
        let mut node_progress = Vec::with_capacity(node_count * index_names.len());

        for node_idx in 0..node_count {
            let Some(mut conn) = get_connection(node_idx) else {
                continue;
            };

            for index_name in index_names {
                match get_node_progress(&mut conn, index_name, engine_type) {
                    Ok(mut progress) => {
                        progress.node_id = format!("node-{}", node_idx);

                        if progress.in_progress {
                            nodes_in_progress += 1;
                        }

                        total_docs += progress.num_docs;
                        total_progress += progress.progress_percent;
                        node_progress.push(progress);
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to get progress from node {}: {}", node_idx, e);
                        nodes_in_progress += 1;
                    }
                }
            }
        }

        let entry_count = node_count * index_names.len();
        let avg_progress = if entry_count > 0 {
            total_progress / entry_count as i32
        } else {
            100
        };

        last_progress = ClusterBackfillProgress {
            total_docs,
            nodes_in_progress,
            progress_percent: avg_progress,
            node_progress,
        };

        if let Some(pb) = &progress_bar {
            pb.set_position(avg_progress as u64);
            pb.set_message(format!(
                "{} docs, {} nodes in progress",
                total_docs, nodes_in_progress
            ));
        }

        if nodes_in_progress == 0 {
            break;
        }

        thread::sleep(config.poll_interval);
    }

    if let Some(pb) = &progress_bar {
        pb.set_position(100);
        pb.finish_with_message(format!("Complete - {} docs", last_progress.total_docs));
    }

    println!(
        "{} Index{} backfill process has completed on all nodes. Total docs indexed: {}",
        index_names.len(),
        if index_names.len() > 1 { "es" } else { "" },
        last_progress.total_docs
    );

    Ok(last_progress)
}

/// Simple callback-based backfill wait for use with ClusterTopology
pub fn wait_for_backfill<'a>(
    engine_type: EngineType,
    index_names: &[&str],
    nodes: &'a [ClusterNode],
    create_connection: impl Fn(&'a ClusterNode) -> Option<RawConnection>,
    config: &BackfillWaitConfig,
) -> Result<ClusterBackfillProgress, String> {
    let node_connections: Vec<_> = nodes
        .iter()
        .filter_map(|node| {
            if node.is_primary {
                create_connection(node).map(|conn| (node.id.clone(), conn))
            } else {
                None
            }
        })
        .collect();

    let node_count = node_connections.len();
    let mut connections: Vec<Option<RawConnection>> =
        node_connections.into_iter().map(|(_, c)| Some(c)).collect();

    wait_for_index_backfill_complete(
        engine_type,
        index_names,
        |idx| {
            if idx < connections.len() {
                connections[idx].take()
            } else {
                None
            }
        },
        node_count,
        config,
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backfill_wait_config_default() {
        let config = BackfillWaitConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(1));
        assert_eq!(config.initial_delay, Duration::from_secs(2));
        assert!(config.max_wait.is_none());
        assert!(config.show_progress);
    }

    #[test]
    fn test_cluster_progress_empty() {
        let progress = ClusterBackfillProgress {
            total_docs: 0,
            nodes_in_progress: 0,
            progress_percent: 100,
            node_progress: vec![],
        };
        assert_eq!(progress.progress_percent, 100);
    }

    #[test]
    fn test_memdb_progress_calculation() {
        let search_info = HashMap::new();

        // Available with no degradation
        let (in_progress, pct) =
            calculate_memdb_progress(&IndexStatus::Available, 0, &search_info).unwrap();
        assert!(!in_progress);
        assert_eq!(pct, 100);

        // Degraded
        let (in_progress, pct) =
            calculate_memdb_progress(&IndexStatus::Available, 30, &search_info).unwrap();
        assert!(in_progress);
        assert_eq!(pct, 70);

        // Queued (no degradation)
        let (in_progress, pct) =
            calculate_memdb_progress(&IndexStatus::Queued, 0, &search_info).unwrap();
        assert!(in_progress);
        assert_eq!(pct, 0);

        // Queued with degradation - should still be 0 (QUEUED takes priority)
        let (in_progress, pct) =
            calculate_memdb_progress(&IndexStatus::Queued, 30, &search_info).unwrap();
        assert!(in_progress);
        assert_eq!(pct, 0);

        // Backfilling with progress
        let mut search_info_with_progress = HashMap::new();
        search_info_with_progress.insert(
            "search_current_backfill_progress_percentage".to_string(),
            "45".to_string(),
        );
        let (in_progress, pct) =
            calculate_memdb_progress(&IndexStatus::Backfilling, 0, &search_info_with_progress)
                .unwrap();
        assert!(in_progress);
        assert_eq!(pct, 45);
    }

    #[test]
    fn test_node_progress_default() {
        let progress = NodeProgress::default();
        assert_eq!(progress.progress_percent, 100);
        assert!(!progress.in_progress);
        assert_eq!(progress.status, IndexStatus::Available);
    }
}
