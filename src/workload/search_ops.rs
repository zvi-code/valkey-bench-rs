//! Vector search operations (FT.CREATE, FT.SEARCH, FT.DROPINDEX)
//!
//! This module provides higher-level operations for managing vector search indices
//! and executing search queries.

use crate::client::{ControlPlane, RawConnection};
use crate::config::SearchConfig;
use crate::metrics::ft_info::{EngineType, FtInfoResult};
use crate::utils::{RespEncoder, RespValue};

/// Create a vector search index using FT.CREATE
///
/// # Arguments
/// * `conn` - Connection to execute command on
/// * `config` - Search configuration with index parameters
/// * `overwrite` - If true, drop existing index first; if false, skip if exists
pub fn create_index(
    conn: &mut RawConnection,
    config: &SearchConfig,
    overwrite: bool,
) -> Result<(), String> {
    if overwrite {
        // Drop existing index first
        let _ = drop_index(conn, &config.index_name);
    } else {
        // Check if index already exists - if so, skip creation
        if index_exists(conn, &config.index_name) {
            return Ok(());
        }
    }

    // Build FT.CREATE command
    let mut encoder = RespEncoder::with_capacity(1024);

    // FT.CREATE index_name ON HASH PREFIX 1 prefix: SCHEMA field_name VECTOR algorithm TYPE FLOAT32 DIM dim DISTANCE_METRIC metric
    let mut args: Vec<&[u8]> = Vec::with_capacity(30);

    args.push(b"FT.CREATE");
    let index_bytes = config.index_name.as_bytes();
    args.push(index_bytes);
    args.push(b"ON");
    args.push(b"HASH");
    args.push(b"PREFIX");
    args.push(b"1");
    let prefix_bytes = config.prefix.as_bytes();
    args.push(prefix_bytes);
    args.push(b"SCHEMA");
    let field_bytes = config.vector_field.as_bytes();
    args.push(field_bytes);
    args.push(b"VECTOR");

    // Algorithm-specific parameters
    let algo_str = config.algorithm.as_str();
    args.push(algo_str.as_bytes());

    // Calculate number of attribute pairs (each pair = 2 args)
    // Base: TYPE + DIM + DISTANCE_METRIC = 3 pairs = 6 args
    // Optional HNSW params: EF_CONSTRUCTION (2 args), M (2 args)
    let mut num_attrs = 6; // TYPE + value + DIM + value + DISTANCE_METRIC + value
    if matches!(config.algorithm, crate::config::VectorAlgorithm::Hnsw) {
        if config.ef_construction.is_some() {
            num_attrs += 2;
        }
        if config.hnsw_m.is_some() {
            num_attrs += 2;
        }
    }
    let num_attrs_str = num_attrs.to_string();
    args.push(num_attrs_str.as_bytes());

    // Common vector params
    args.push(b"TYPE");
    args.push(b"FLOAT32");
    args.push(b"DIM");
    let dim_str = config.dim.to_string();
    args.push(dim_str.as_bytes());
    args.push(b"DISTANCE_METRIC");
    let metric_str = config.distance_metric.as_str();
    args.push(metric_str.as_bytes());

    // HNSW-specific parameters
    let ef_str: String;
    let m_str: String;
    if matches!(config.algorithm, crate::config::VectorAlgorithm::Hnsw) {
        if let Some(ef) = config.ef_construction {
            args.push(b"EF_CONSTRUCTION");
            ef_str = ef.to_string();
            args.push(ef_str.as_bytes());
        }
        if let Some(m) = config.hnsw_m {
            args.push(b"M");
            m_str = m.to_string();
            args.push(m_str.as_bytes());
        }
    }

    encoder.encode_command(&args);

    // Execute
    match conn.execute_encoded(&encoder) {
        Ok(RespValue::SimpleString(s)) if s == "OK" => Ok(()),
        Ok(RespValue::Error(e)) => Err(e),
        Ok(other) => Err(format!("Unexpected response: {:?}", other)),
        Err(e) => Err(format!("IO error: {}", e)),
    }
}

/// Drop a vector search index
pub fn drop_index(conn: &mut RawConnection, index_name: &str) -> Result<(), String> {
    let mut encoder = RespEncoder::with_capacity(64);
    encoder.encode_command_str(&["FT.DROPINDEX", index_name]);

    match conn.execute_encoded(&encoder) {
        Ok(RespValue::SimpleString(s)) if s == "OK" => Ok(()),
        Ok(RespValue::Error(e)) => {
            // Ignore "Unknown Index name" errors
            if e.contains("Unknown Index name") || e.contains("Unknown index name") {
                Ok(())
            } else {
                Err(e)
            }
        }
        Ok(_) => Ok(()),
        Err(e) => Err(format!("IO error: {}", e)),
    }
}

/// Check if an index exists using FT.INFO
pub fn index_exists(conn: &mut RawConnection, index_name: &str) -> bool {
    let mut encoder = RespEncoder::with_capacity(64);
    encoder.encode_command_str(&["FT.INFO", index_name]);

    match conn.execute_encoded(&encoder) {
        Ok(RespValue::Error(_)) => false, // Index doesn't exist
        Ok(_) => true,                    // Got a valid response, index exists
        Err(_) => false,                  // IO error, assume doesn't exist
    }
}

/// Get index information using FT.INFO
/// Uses engine-aware parsing from metrics::ft_info module
pub fn get_index_info(conn: &mut RawConnection, index_name: &str) -> Result<IndexInfo, String> {
    // First detect engine type from INFO SEARCH
    let engine_type = detect_engine_type(conn);

    // Get FT.INFO
    let mut encoder = RespEncoder::with_capacity(64);
    encoder.encode_command_str(&["FT.INFO", index_name]);

    match conn.execute_encoded(&encoder) {
        Ok(RespValue::Error(e)) => Err(e),
        Ok(reply) => {
            // Use proper engine-aware parsing
            let ft_info = FtInfoResult::from_response(&reply, engine_type);

            // Determine if indexing is in progress based on engine-specific fields
            let indexing = ft_info.backfill_in_progress || !ft_info.is_ready();
            let percent_indexed = ft_info.backfill_complete_percent;

            Ok(IndexInfo {
                index_name: ft_info.index_name.clone().unwrap_or_default(),
                num_docs: ft_info.num_docs as u64,
                max_doc_id: 0, // Not always available
                num_records: ft_info.num_indexed_vectors as u64,
                indexing,
                percent_indexed,
            })
        }
        Err(e) => Err(format!("IO error: {}", e)),
    }
}

/// Detect engine type from INFO SEARCH response
fn detect_engine_type(conn: &mut RawConnection) -> EngineType {
    let mut encoder = RespEncoder::with_capacity(64);
    encoder.encode_command_str(&["INFO", "SEARCH"]);

    match conn.execute_encoded(&encoder) {
        Ok(RespValue::BulkString(data)) => {
            let info_str = String::from_utf8_lossy(&data);
            EngineType::detect(&info_str)
        }
        _ => EngineType::Unknown,
    }
}

/// Parsed index information
#[derive(Debug, Default)]
pub struct IndexInfo {
    pub index_name: String,
    pub num_docs: u64,
    pub max_doc_id: u64,
    pub num_records: u64,
    pub indexing: bool,
    pub percent_indexed: f64,
}

/// Wait for index to be fully indexed (background indexing complete)
///
/// Backfill always runs when an index is created - it scans all existing keys
/// matching the prefix. For an empty database, it completes instantly.
/// After backfill completes, new inserts are synchronous (no additional backfill).
///
/// Uses engine-aware status checking:
/// - EC/Valkey: waits for state=ready AND backfill_in_progress=false
/// - MemoryDB: waits for index_status=AVAILABLE AND degradation_percentage=0
pub fn wait_for_indexing(
    conn: &mut RawConnection,
    index_name: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    use indicatif::{ProgressBar, ProgressStyle};
    use std::time::{Duration, Instant};

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(500);

    // Detect engine type once
    let engine_type = detect_engine_type(conn);

    // Create progress bar (100 units = 100%)
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}% ({msg})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message("initializing...");

    let mut last_docs = 0i64;
    let mut last_time = Instant::now();

    loop {
        // Get FT.INFO with engine-aware parsing
        let mut encoder = RespEncoder::with_capacity(64);
        encoder.encode_command_str(&["FT.INFO", index_name]);

        let ft_info = match conn.execute_encoded(&encoder) {
            Ok(RespValue::Error(e)) => {
                pb.finish_and_clear();
                return Err(e);
            }
            Ok(reply) => FtInfoResult::from_response(&reply, engine_type),
            Err(e) => {
                pb.finish_and_clear();
                return Err(format!("IO error: {}", e));
            }
        };

        // Calculate progress percentage
        let progress_pct = if ft_info.backfill_in_progress {
            (ft_info.backfill_complete_percent * 100.0) as u64
        } else if ft_info.is_ready() {
            100
        } else {
            0
        };

        // Calculate docs/sec rate
        let now = Instant::now();
        let elapsed = now.duration_since(last_time).as_secs_f64();
        let docs_delta = (ft_info.num_docs - last_docs).max(0);
        let docs_per_sec = if elapsed > 0.1 {
            docs_delta as f64 / elapsed
        } else {
            0.0
        };

        // Update for next iteration
        if elapsed > 0.5 {
            last_docs = ft_info.num_docs;
            last_time = now;
        }

        // Update progress bar
        pb.set_position(progress_pct);
        if ft_info.backfill_in_progress {
            pb.set_message(format!(
                "{} docs, {:.0} docs/sec",
                ft_info.num_docs, docs_per_sec
            ));
        } else if ft_info.is_ready() {
            pb.set_message(format!("{} docs indexed", ft_info.num_docs));
        } else {
            pb.set_message(format!("state: {:?}", ft_info.status));
        }

        // Engine-specific readiness check
        // For EC: state=ready AND backfill_in_progress=false
        // For MemoryDB: status=AVAILABLE AND degradation=0
        if ft_info.is_ready() {
            pb.finish_with_message(format!("complete - {} docs indexed", ft_info.num_docs));
            return Ok(());
        }

        if start.elapsed() > timeout {
            pb.finish_and_clear();
            return Err(format!(
                "Timeout waiting for index {} to complete ({}% indexed, {} docs, backfill_in_progress={})",
                index_name,
                ft_info.backfill_complete_percent * 100.0,
                ft_info.num_docs,
                ft_info.backfill_in_progress
            ));
        }

        std::thread::sleep(poll_interval);
    }
}

/// Parse FT.SEARCH response to extract document IDs
///
/// Handles both response formats:
/// - Without NOCONTENT: [count, docID1, [fields...], docID2, [fields...], ...]
/// - With NOCONTENT: [count, docID1, docID2, docID3, ...]
pub fn parse_search_response(response: &RespValue) -> Vec<String> {
    let mut doc_ids = Vec::new();

    if let RespValue::Array(arr) = response {
        // First element is total count
        let mut i = 1;
        while i < arr.len() {
            if let RespValue::BulkString(doc_id) = &arr[i] {
                doc_ids.push(String::from_utf8_lossy(doc_id).to_string());
            }
            i += 1;
            // Skip fields array if present (when NOCONTENT is not used)
            if i < arr.len() {
                if let RespValue::Array(_) = &arr[i] {
                    i += 1;
                }
            }
        }
    }

    doc_ids
}

/// Extract numeric IDs from document keys
/// Uses the unified key format from key_format module
///
/// Handles both formats:
/// - Simple: "vec:000000000123" -> 123
/// - With cluster tag: "vec:{ABC}:000000000123" -> 123
pub fn extract_numeric_ids(doc_ids: &[String], prefix: &str) -> Vec<u64> {
    super::key_format::extract_numeric_ids_from_keys(doc_ids, prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_search_response_nocontent() {
        // NOCONTENT format: [count, docID1, docID2, ...]
        let response = RespValue::Array(vec![
            RespValue::Integer(3), // total count
            RespValue::BulkString(b"vec:000000000001".to_vec()),
            RespValue::BulkString(b"vec:000000000002".to_vec()),
            RespValue::BulkString(b"vec:000000000003".to_vec()),
        ]);

        let doc_ids = parse_search_response(&response);
        assert_eq!(doc_ids.len(), 3);
        assert_eq!(doc_ids[0], "vec:000000000001");
        assert_eq!(doc_ids[1], "vec:000000000002");
        assert_eq!(doc_ids[2], "vec:000000000003");
    }

    #[test]
    fn test_parse_search_response_with_fields() {
        // Non-NOCONTENT format: [count, docID1, [fields...], docID2, [fields...], ...]
        let response = RespValue::Array(vec![
            RespValue::Integer(2), // total count
            RespValue::BulkString(b"vec:000000000001".to_vec()),
            RespValue::Array(vec![
                RespValue::BulkString(b"field1".to_vec()),
                RespValue::BulkString(b"value1".to_vec()),
            ]),
            RespValue::BulkString(b"vec:000000000002".to_vec()),
            RespValue::Array(vec![
                RespValue::BulkString(b"field1".to_vec()),
                RespValue::BulkString(b"value2".to_vec()),
            ]),
        ]);

        let doc_ids = parse_search_response(&response);
        assert_eq!(doc_ids.len(), 2);
        assert_eq!(doc_ids[0], "vec:000000000001");
        assert_eq!(doc_ids[1], "vec:000000000002");
    }

    #[test]
    fn test_parse_search_response_empty() {
        let response = RespValue::Array(vec![
            RespValue::Integer(0), // total count = 0
        ]);

        let doc_ids = parse_search_response(&response);
        assert!(doc_ids.is_empty());
    }

    #[test]
    fn test_extract_numeric_ids() {
        let doc_ids = vec![
            "vec:000000000001".to_string(),
            "vec:000000000042".to_string(),
            "vec:000000000100".to_string(),
        ];

        let ids = extract_numeric_ids(&doc_ids, "vec:");
        assert_eq!(ids, vec![1, 42, 100]);
    }

    #[test]
    fn test_extract_numeric_ids_empty() {
        let doc_ids: Vec<String> = vec![];
        let ids = extract_numeric_ids(&doc_ids, "vec:");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_extract_numeric_ids_with_zero() {
        let doc_ids = vec![
            "vec:000000000000".to_string(),
            "vec:000000000001".to_string(),
        ];

        let ids = extract_numeric_ids(&doc_ids, "vec:");
        assert_eq!(ids, vec![0, 1]);
    }

    #[test]
    fn test_extract_numeric_ids_with_cluster_tag() {
        let doc_ids = vec![
            "zvec_:{ABC}:000000000001".to_string(),
            "zvec_:{XYZ}:000000000042".to_string(),
            "zvec_:{DEF}:000000000100".to_string(),
        ];

        let ids = extract_numeric_ids(&doc_ids, "zvec_:");
        assert_eq!(ids, vec![1, 42, 100]);
    }

    #[test]
    fn test_extract_numeric_ids_mixed_formats() {
        let doc_ids = vec![
            "vec:000000000001".to_string(),         // Simple format
            "vec:{ABC}:000000000042".to_string(),   // Cluster tag format
        ];

        let ids = extract_numeric_ids(&doc_ids, "vec:");
        assert_eq!(ids, vec![1, 42]);
    }
}
