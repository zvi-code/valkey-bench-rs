//! FT.INFO response parsing
//!
//! Parses FT.INFO responses from different engine types:
//! - EC (ElastiCache Valkey) - uses flat key-value pairs
//! - MemoryDB - uses nested RESP3 structures
//!
//! This module provides a unified trait-based approach for handling
//! engine-specific differences in backfill progress detection.

use std::collections::HashMap;

use crate::utils::RespValue;

// ============================================================================
// Engine Type
// ============================================================================

/// Engine type for determining response format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    /// Unknown engine type
    Unknown,
    /// Open Source Valkey
    OssValkey,
    /// ElastiCache Valkey (provisioned)
    ElasticacheValkey,
    /// ElastiCache Serverless
    ElasticacheServerless,
    /// Amazon MemoryDB
    MemoryDb,
}

impl EngineType {
    /// Detect engine type from server info
    pub fn detect(info_response: &str) -> Self {
        if info_response.contains("memorydb") || info_response.contains("MemoryDB") {
            return EngineType::MemoryDb;
        }
        if info_response.contains("elasticache") || info_response.contains("ElastiCache") {
            if info_response.contains("serverless") {
                return EngineType::ElasticacheServerless;
            }
            return EngineType::ElasticacheValkey;
        }
        if info_response.contains("valkey-search") || info_response.contains("search_") {
            return EngineType::ElasticacheValkey;
        }
        EngineType::OssValkey
    }

    pub fn is_memorydb(&self) -> bool {
        matches!(self, EngineType::MemoryDb)
    }
}

// ============================================================================
// Response Format
// ============================================================================

/// Response format for FT.INFO parsing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    /// EC format: flat key-value pairs with nested arrays
    ElastiCache,
    /// MemoryDB format: strict key-value pairs (step by 2)
    MemoryDb,
}

impl From<EngineType> for ResponseFormat {
    fn from(engine: EngineType) -> Self {
        match engine {
            EngineType::MemoryDb => ResponseFormat::MemoryDb,
            _ => ResponseFormat::ElastiCache,
        }
    }
}

// ============================================================================
// Field Definitions (Centralized)
// ============================================================================

/// Centralized field definitions for FT.INFO
pub mod fields {
    /// EC field names
    pub mod ec {
        pub const INDEX_NAME: &str = "index_name";
        pub const STATE: &str = "state";
        pub const NUM_DOCS: &str = "num_docs";
        pub const NUM_INDEXED_VECTORS: &str = "num_indexed_vectors";
        pub const BACKFILL_IN_PROGRESS: &str = "backfill_in_progress";
        pub const BACKFILL_COMPLETE_PERCENT: &str = "backfill_complete_percent";
        pub const SPACE_USAGE: &str = "space_usage";
        pub const VECTOR_SPACE_USAGE: &str = "vector_space_usage";
    }

    /// MemoryDB field names
    pub mod memdb {
        pub const INDEX_NAME: &str = "index_name";
        pub const INDEX_STATUS: &str = "index_status";
        pub const NUM_DOCS: &str = "num_docs";
        pub const NUM_INDEXED_VECTORS: &str = "num_indexed_vectors";
        pub const INDEX_DEGRADATION_PERCENTAGE: &str = "index_degradation_percentage";
        pub const SPACE_USAGE: &str = "space_usage";
        pub const VECTOR_SPACE_USAGE: &str = "vector_space_usage";
        pub const CURRENT_LAG: &str = "current_lag";
    }

    /// INFO SEARCH field names (MemoryDB)
    pub mod search_info {
        pub const NUM_ACTIVE_BACKFILLS: &str = "search_num_active_backfills";
        pub const BACKFILL_PROGRESS_PERCENTAGE: &str = "search_current_backfill_progress_percentage";
    }

    /// Required fields for MemoryDB FT.INFO validation
    pub const REQUIRED_MEMDB_FTINFO: &[&str] = &[
        memdb::INDEX_STATUS,
        memdb::INDEX_DEGRADATION_PERCENTAGE,
        memdb::NUM_INDEXED_VECTORS,
    ];
}

// ============================================================================
// RESP Response Parsing
// ============================================================================

fn resp_to_string(value: &RespValue) -> Option<String> {
    match value {
        RespValue::SimpleString(s) => Some(s.clone()),
        RespValue::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        _ => None,
    }
}

/// Convert FT.INFO RESP response to key:value lines
pub fn convert_ftinfo_to_lines(reply: &RespValue, format: ResponseFormat) -> String {
    let mut lines = String::new();
    convert_recursive(reply, None, &mut lines, format);
    lines
}

fn convert_recursive(
    reply: &RespValue,
    prefix: Option<&str>,
    lines: &mut String,
    format: ResponseFormat,
) {
    match reply {
        RespValue::SimpleString(s) => {
            if let Some(p) = prefix {
                lines.push_str(&format!("{}:{}\n", p, s));
            }
        }
        RespValue::BulkString(bytes) => {
            if let Some(p) = prefix {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    lines.push_str(&format!("{}:{}\n", p, s));
                }
            }
        }
        RespValue::Integer(i) => {
            if let Some(p) = prefix {
                lines.push_str(&format!("{}:{}\n", p, i));
            }
        }
        RespValue::Array(elements) => match format {
            ResponseFormat::ElastiCache => convert_ec_array(elements, prefix, lines, format),
            ResponseFormat::MemoryDb => convert_memdb_array(elements, prefix, lines, format),
        },
        _ => {}
    }
}

fn convert_ec_array(
    elements: &[RespValue],
    prefix: Option<&str>,
    lines: &mut String,
    format: ResponseFormat,
) {
    let mut i = 0;
    while i < elements.len() {
        let element = &elements[i];

        if let RespValue::Array(_) = element {
            convert_recursive(element, prefix, lines, format);
            i += 1;
            continue;
        }

        if i == elements.len() - 1 {
            if let Some(s) = resp_to_string(element) {
                if let Some(p) = prefix {
                    lines.push_str(&format!("{}:{}\n", p, s));
                }
            } else if let RespValue::Integer(n) = element {
                if let Some(p) = prefix {
                    lines.push_str(&format!("{}:{}\n", p, n));
                }
            }
            break;
        }

        let key_name = match resp_to_string(element) {
            Some(s) => s,
            None => {
                i += 1;
                continue;
            }
        };

        let full_key = match prefix {
            Some(p) => format!("{}.{}", p, key_name),
            None => key_name,
        };

        i += 1;
        if i >= elements.len() {
            break;
        }

        let value = &elements[i];
        if let Some(s) = resp_to_string(value) {
            lines.push_str(&format!("{}:{}\n", full_key, s));
        } else if let RespValue::Integer(n) = value {
            lines.push_str(&format!("{}:{}\n", full_key, n));
        } else if let RespValue::Array(_) = value {
            convert_recursive(value, Some(&full_key), lines, format);
        }

        i += 1;
    }
}

fn convert_memdb_array(
    elements: &[RespValue],
    prefix: Option<&str>,
    lines: &mut String,
    format: ResponseFormat,
) {
    let mut i = 0;
    while i + 1 < elements.len() {
        let key_elem = &elements[i];
        let val_elem = &elements[i + 1];

        let key_name = match resp_to_string(key_elem) {
            Some(s) => s,
            None => {
                i += 2;
                continue;
            }
        };

        let full_key = match prefix {
            Some(p) => format!("{}.{}", p, key_name),
            None => key_name,
        };

        if let Some(s) = resp_to_string(val_elem) {
            lines.push_str(&format!("{}:{}\n", full_key, s));
        } else if let RespValue::Integer(n) = val_elem {
            lines.push_str(&format!("{}:{}\n", full_key, n));
        } else if let RespValue::Array(sub_elements) = val_elem {
            if sub_elements.is_empty() {
                i += 2;
                continue;
            }

            let first = &sub_elements[0];
            match first {
                RespValue::Array(_) => {
                    for sub in sub_elements {
                        convert_recursive(sub, Some(&full_key), lines, format);
                    }
                }
                RespValue::SimpleString(_) | RespValue::BulkString(_)
                    if sub_elements.len() % 2 == 0 =>
                {
                    convert_recursive(val_elem, Some(&full_key), lines, format);
                }
                _ => {
                    if let Some(s) = resp_to_string(first) {
                        lines.push_str(&format!("{}:{}\n", full_key, s));
                    } else if let RespValue::Integer(n) = first {
                        lines.push_str(&format!("{}:{}\n", full_key, n));
                    }
                }
            }
        }

        i += 2;
    }
}

/// Parse FT.INFO lines into a HashMap
pub fn parse_ftinfo_lines(lines: &str) -> HashMap<String, String> {
    lines
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Extract a typed field value from parsed info
pub fn get_field<T: std::str::FromStr>(info: &HashMap<String, String>, field: &str) -> Option<T> {
    info.get(field)?.parse().ok()
}

/// Extract a typed field with default value
pub fn get_field_or<T: std::str::FromStr>(
    info: &HashMap<String, String>,
    field: &str,
    default: T,
) -> T {
    get_field(info, field).unwrap_or(default)
}

// ============================================================================
// Index Status
// ============================================================================

/// Index status from FT.INFO
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexStatus {
    Available,
    Backfilling,
    Queued,
    Unknown(String),
}

impl IndexStatus {
    pub fn from_str(s: &str) -> Self {
        let upper = s.to_uppercase();
        match upper.as_str() {
            "AVAILABLE" | "READY" => IndexStatus::Available,
            "BACKFILLING" | "BACKFILL_IN_PROGRESS" => IndexStatus::Backfilling,
            s if s.contains("QUEUED") => IndexStatus::Queued,
            _ => IndexStatus::Unknown(s.to_string()),
        }
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(self, IndexStatus::Backfilling | IndexStatus::Queued)
    }

    pub fn is_available(&self) -> bool {
        matches!(self, IndexStatus::Available)
    }
}

// ============================================================================
// Validation
// ============================================================================

/// Validation error for FT.INFO response
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub message: String,
    pub missing_field: Option<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Validate FT.INFO response has required fields for MemoryDB
pub fn validate_memdb_ftinfo(raw: &HashMap<String, String>) -> Result<(), ValidationError> {
    for &field in fields::REQUIRED_MEMDB_FTINFO {
        if !raw.contains_key(field) {
            return Err(ValidationError {
                message: format!("Missing required field '{}' in FT.INFO response", field),
                missing_field: Some(field.to_string()),
            });
        }
    }
    Ok(())
}

/// Validate INFO SEARCH response has required fields
pub fn validate_search_info(
    search_info: &HashMap<String, String>,
    active_backfills: i32,
) -> Result<(), ValidationError> {
    if !search_info.contains_key(fields::search_info::NUM_ACTIVE_BACKFILLS) {
        return Err(ValidationError {
            message: format!(
                "Missing required field '{}' in INFO SEARCH response",
                fields::search_info::NUM_ACTIVE_BACKFILLS
            ),
            missing_field: Some(fields::search_info::NUM_ACTIVE_BACKFILLS.to_string()),
        });
    }

    if active_backfills > 0
        && !search_info.contains_key(fields::search_info::BACKFILL_PROGRESS_PERCENTAGE)
    {
        return Err(ValidationError {
            message: format!(
                "Missing '{}' with {} active backfills",
                fields::search_info::BACKFILL_PROGRESS_PERCENTAGE,
                active_backfills
            ),
            missing_field: Some(fields::search_info::BACKFILL_PROGRESS_PERCENTAGE.to_string()),
        });
    }

    Ok(())
}

// ============================================================================
// Parsed FT.INFO Result
// ============================================================================

/// Parsed FT.INFO result (unified for all engine types)
#[derive(Debug, Clone)]
pub struct FtInfoResult {
    pub index_name: Option<String>,
    pub num_docs: i64,
    pub num_indexed_vectors: i64,
    pub status: IndexStatus,
    pub degradation_percentage: i32,
    pub backfill_in_progress: bool,
    pub backfill_complete_percent: f64,
    pub space_usage: i64,
    pub vector_space_usage: i64,
    pub current_lag: i64,
    pub raw: HashMap<String, String>,
}

impl FtInfoResult {
    /// Parse from EC format response
    pub fn from_ec_response(reply: &RespValue) -> Self {
        let lines = convert_ftinfo_to_lines(reply, ResponseFormat::ElastiCache);
        let raw = parse_ftinfo_lines(&lines);

        // C code defaults backfill_complete_percent to 0.0 when field is missing
        let backfill_in_progress: i32 = get_field_or(&raw, fields::ec::BACKFILL_IN_PROGRESS, 0);
        let backfill_complete_percent: f64 =
            get_field_or(&raw, fields::ec::BACKFILL_COMPLETE_PERCENT, 0.0);

        Self {
            index_name: raw.get(fields::ec::INDEX_NAME).cloned(),
            num_docs: get_field_or(&raw, fields::ec::NUM_DOCS, 0),
            num_indexed_vectors: get_field_or(&raw, fields::ec::NUM_INDEXED_VECTORS, 0),
            status: raw
                .get(fields::ec::STATE)
                .map(|s| IndexStatus::from_str(s))
                .unwrap_or(IndexStatus::Available),
            degradation_percentage: 0,
            backfill_in_progress: backfill_in_progress != 0,
            backfill_complete_percent,
            space_usage: get_field_or(&raw, fields::ec::SPACE_USAGE, 0),
            vector_space_usage: get_field_or(&raw, fields::ec::VECTOR_SPACE_USAGE, 0),
            current_lag: 0,
            raw,
        }
    }

    /// Parse from MemoryDB format response
    pub fn from_memdb_response(reply: &RespValue) -> Self {
        let lines = convert_ftinfo_to_lines(reply, ResponseFormat::MemoryDb);
        let raw = parse_ftinfo_lines(&lines);

        Self {
            index_name: raw.get(fields::memdb::INDEX_NAME).cloned(),
            num_docs: get_field_or(&raw, fields::memdb::NUM_DOCS, 0),
            num_indexed_vectors: get_field_or(&raw, fields::memdb::NUM_INDEXED_VECTORS, 0),
            status: raw
                .get(fields::memdb::INDEX_STATUS)
                .map(|s| IndexStatus::from_str(s))
                .unwrap_or(IndexStatus::Available),
            degradation_percentage: get_field_or(
                &raw,
                fields::memdb::INDEX_DEGRADATION_PERCENTAGE,
                0,
            ),
            backfill_in_progress: false,
            backfill_complete_percent: 100.0,
            space_usage: get_field_or(&raw, fields::memdb::SPACE_USAGE, 0),
            vector_space_usage: get_field_or(&raw, fields::memdb::VECTOR_SPACE_USAGE, 0),
            current_lag: get_field_or(&raw, fields::memdb::CURRENT_LAG, 0),
            raw,
        }
    }

    /// Parse from response based on engine type
    pub fn from_response(reply: &RespValue, engine_type: EngineType) -> Self {
        match engine_type {
            EngineType::MemoryDb => Self::from_memdb_response(reply),
            _ => Self::from_ec_response(reply),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn is_ready(&self) -> bool {
        self.status == IndexStatus::Available
            && self.degradation_percentage == 0
            && !self.backfill_in_progress
    }

    /// Get progress percentage for EC format (0-100)
    pub fn ec_progress_percent(&self) -> i32 {
        (self.backfill_complete_percent * 100.0) as i32
    }
}

// ============================================================================
// Legacy compatibility (deprecated)
// ============================================================================

#[deprecated(note = "Use convert_ftinfo_to_lines with ResponseFormat::ElastiCache")]
pub fn convert_ftinfo_to_lines_legacy(reply: &RespValue, _prefix: Option<&str>) -> String {
    convert_ftinfo_to_lines(reply, ResponseFormat::ElastiCache)
}

#[deprecated(note = "Use convert_ftinfo_to_lines with ResponseFormat::MemoryDb")]
pub fn convert_memdb_ftinfo_to_lines(reply: &RespValue, _prefix: Option<&str>) -> String {
    convert_ftinfo_to_lines(reply, ResponseFormat::MemoryDb)
}

#[deprecated(note = "Use get_field instead")]
pub fn get_ftinfo_field<T: std::str::FromStr>(
    info: &HashMap<String, String>,
    field: &str,
) -> Option<T> {
    get_field(info, field)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_ftinfo_simple() {
        let reply = RespValue::Array(vec![
            RespValue::BulkString(b"index_name".to_vec()),
            RespValue::BulkString(b"my-index".to_vec()),
            RespValue::BulkString(b"num_docs".to_vec()),
            RespValue::Integer(1000),
        ]);

        let lines = convert_ftinfo_to_lines(&reply, ResponseFormat::ElastiCache);
        assert!(lines.contains("index_name:my-index"));
        assert!(lines.contains("num_docs:1000"));
    }

    #[test]
    fn test_convert_ftinfo_nested() {
        let reply = RespValue::Array(vec![
            RespValue::BulkString(b"attributes".to_vec()),
            RespValue::Array(vec![
                RespValue::BulkString(b"dim".to_vec()),
                RespValue::Integer(128),
            ]),
        ]);

        let lines = convert_ftinfo_to_lines(&reply, ResponseFormat::ElastiCache);
        assert!(lines.contains("attributes.dim:128"));
    }

    #[test]
    fn test_index_status() {
        assert_eq!(IndexStatus::from_str("AVAILABLE"), IndexStatus::Available);
        assert_eq!(IndexStatus::from_str("BACKFILLING"), IndexStatus::Backfilling);
        assert!(IndexStatus::from_str("QUEUED_FOR_BACKFILL").is_in_progress());
    }

    #[test]
    fn test_engine_detection() {
        assert_eq!(
            EngineType::detect("# Server\r\nvalkey_version:7.2.0\r\n"),
            EngineType::OssValkey
        );
        assert_eq!(
            EngineType::detect("# Modules\r\nmodule:name=valkey-search\r\n"),
            EngineType::ElasticacheValkey
        );
        assert_eq!(
            EngineType::detect("# Server\r\nmemorydb_version:7.1.0\r\n"),
            EngineType::MemoryDb
        );
    }

    #[test]
    fn test_validation_memdb() {
        let mut raw = HashMap::new();
        raw.insert("index_status".to_string(), "AVAILABLE".to_string());
        raw.insert("index_degradation_percentage".to_string(), "0".to_string());
        raw.insert("num_indexed_vectors".to_string(), "1000".to_string());

        assert!(validate_memdb_ftinfo(&raw).is_ok());

        let incomplete = HashMap::new();
        assert!(validate_memdb_ftinfo(&incomplete).is_err());
    }

    #[test]
    fn test_validation_search_info() {
        let mut search_info = HashMap::new();
        search_info.insert("search_num_active_backfills".to_string(), "0".to_string());

        assert!(validate_search_info(&search_info, 0).is_ok());
        assert!(validate_search_info(&search_info, 1).is_err());

        search_info.insert(
            "search_current_backfill_progress_percentage".to_string(),
            "50".to_string(),
        );
        assert!(validate_search_info(&search_info, 1).is_ok());
    }

    #[test]
    fn test_ec_backfill_default() {
        let reply = RespValue::Array(vec![]);
        let result = FtInfoResult::from_ec_response(&reply);
        assert_eq!(result.backfill_complete_percent, 0.0);
        assert_eq!(result.ec_progress_percent(), 0);
    }
}
