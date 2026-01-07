//! Template factory for creating command templates for all workload types

use crate::config::SearchConfig;

use super::addressable::AddressType;
use super::command_template::CommandTemplate;
use super::key_format::DEFAULT_KEY_WIDTH;
use super::workload_type::WorkloadType;

/// Configuration for address-based templates (hash fields, JSON paths)
#[derive(Debug, Clone)]
pub struct AddressConfig {
    /// Type of address space being used
    pub address_type: AddressType,
    /// Maximum field name length (for hash fields)
    pub max_field_len: usize,
    /// Maximum JSON path length (for JSON paths)
    pub max_path_len: usize,
}

impl Default for AddressConfig {
    fn default() -> Self {
        Self {
            address_type: AddressType::Key,
            max_field_len: 32,
            max_path_len: 64,
        }
    }
}

impl AddressConfig {
    /// Create config for hash field addressing
    pub fn hash_field(max_field_len: usize) -> Self {
        Self {
            address_type: AddressType::HashField,
            max_field_len,
            max_path_len: 64,
        }
    }

    /// Create config for JSON path addressing
    pub fn json_path(max_path_len: usize) -> Self {
        Self {
            address_type: AddressType::JsonPath,
            max_field_len: 32,
            max_path_len,
        }
    }
}

/// Create command template for given workload type
///
/// The `cluster_mode` parameter is now ignored - all keys use simple format
/// (e.g., `key:000000000001`). The cluster hash tag injection has been removed.
pub fn create_template(
    workload: WorkloadType,
    key_prefix: &str,
    data_size: usize,
    search_config: Option<&SearchConfig>,
    _cluster_mode: bool, // Ignored: cluster hash tags removed
) -> CommandTemplate {
    create_template_with_address(workload, key_prefix, data_size, search_config, false, None)
}

/// Create command template with optional address configuration
///
/// This variant allows specifying an AddressConfig to enable hash field or JSON path iteration.
/// When address_config is provided with HashField type, HSET will use a Field placeholder.
///
/// Note: The `cluster_mode` parameter is now ignored. All keys use simple format.
pub fn create_template_with_address(
    workload: WorkloadType,
    key_prefix: &str,
    data_size: usize,
    search_config: Option<&SearchConfig>,
    _cluster_mode: bool, // Ignored: cluster hash tags removed
    address_config: Option<&AddressConfig>,
) -> CommandTemplate {
    let key_width = DEFAULT_KEY_WIDTH;

    // All keys now use simple prefixed format (no cluster hash tags)
    let add_key = |template: CommandTemplate| -> CommandTemplate {
        template.arg_prefixed_key(key_prefix, key_width)
    };

    match workload {
        // === Simple commands ===
        WorkloadType::Ping => CommandTemplate::new("PING").arg_str("PING"),

        // === Key-value commands ===
        WorkloadType::Set => add_key(CommandTemplate::new("SET").arg_str("SET"))
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Get => add_key(CommandTemplate::new("GET").arg_str("GET")),

        WorkloadType::Incr => add_key(CommandTemplate::new("INCR").arg_str("INCR")),

        // === List commands ===
        WorkloadType::Lpush => add_key(CommandTemplate::new("LPUSH").arg_str("LPUSH"))
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Rpush => add_key(CommandTemplate::new("RPUSH").arg_str("RPUSH"))
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Lpop => add_key(CommandTemplate::new("LPOP").arg_str("LPOP")),

        WorkloadType::Rpop => add_key(CommandTemplate::new("RPOP").arg_str("RPOP")),

        WorkloadType::Lrange100 => create_lrange_template(key_prefix, key_width, 100),
        WorkloadType::Lrange300 => create_lrange_template(key_prefix, key_width, 300),
        WorkloadType::Lrange500 => create_lrange_template(key_prefix, key_width, 500),
        WorkloadType::Lrange600 => create_lrange_template(key_prefix, key_width, 600),

        // === Set commands ===
        WorkloadType::Sadd => add_key(CommandTemplate::new("SADD").arg_str("SADD"))
            .arg_rand_int(key_width),

        WorkloadType::Spop => add_key(CommandTemplate::new("SPOP").arg_str("SPOP")),

        // === Hash commands ===
        WorkloadType::Hset => {
            let template = add_key(CommandTemplate::new("HSET").arg_str("HSET"));
            // Use Field placeholder when address_config specifies hash field iteration
            if let Some(cfg) = address_config {
                if cfg.address_type == AddressType::HashField {
                    return template
                        .arg_field(cfg.max_field_len)
                        .arg_literal(&vec![b'x'; data_size]);
                }
            }
            // Default: literal field name
            template
                .arg_str("field")
                .arg_literal(&vec![b'x'; data_size])
        }

        // === Sorted set commands ===
        WorkloadType::Zadd => add_key(CommandTemplate::new("ZADD").arg_str("ZADD"))
            .arg_rand_int(key_width) // score
            .arg_str("member"),

        WorkloadType::Zpopmin => add_key(CommandTemplate::new("ZPOPMIN").arg_str("ZPOPMIN")),

        // === Multi-key commands ===
        WorkloadType::Mset => create_mset_template(key_prefix, key_width, data_size, 10),

        // === Vector search commands (simple key format) ===
        WorkloadType::VecLoad => {
            let sc = search_config.expect("VecLoad requires search config");
            create_vec_load_template(sc, key_width)
        }

        WorkloadType::VecQuery => {
            let sc = search_config.expect("VecQuery requires search config");
            create_vec_query_template(sc)
        }

        WorkloadType::VecDelete => {
            let sc = search_config.expect("VecDelete requires search config");
            // Simple key format: prefix + zero-padded ID
            CommandTemplate::new("DEL")
                .arg_str("DEL")
                .arg_prefixed_key(&sc.prefix, key_width)
        }

        WorkloadType::VecDelProtected => {
            let sc = search_config.expect("VecDelProtected requires search config");
            // Same as VecDelete - just DEL with prefixed key
            // The GT protection is handled by ProtectedDeleteContext
            CommandTemplate::new("DEL")
                .arg_str("DEL")
                .arg_prefixed_key(&sc.prefix, key_width)
        }

        WorkloadType::VecUpdate => {
            let sc = search_config.expect("VecUpdate requires search config");
            create_vec_load_template(sc, key_width)
        }

        WorkloadType::Custom => {
            // Custom commands should be handled separately
            CommandTemplate::new("CUSTOM").arg_str("PING")
        }
    }
}

/// Create LRANGE template with specified count
fn create_lrange_template(key_prefix: &str, key_width: usize, count: i32) -> CommandTemplate {
    CommandTemplate::new(&format!("LRANGE_{}", count))
        .arg_str("LRANGE")
        .arg_prefixed_key(key_prefix, key_width)
        .arg_str("0")
        .arg_str(&(count - 1).to_string())
}

/// Create MSET template with multiple keys
fn create_mset_template(
    key_prefix: &str,
    key_width: usize,
    data_size: usize,
    num_keys: usize,
) -> CommandTemplate {
    let mut template = CommandTemplate::new("MSET").arg_str("MSET");

    for _ in 0..num_keys {
        template = template.arg_prefixed_key(key_prefix, key_width);
        template = template.arg_literal(&vec![b'x'; data_size]);
    }

    template
}

/// Create HSET template for vector loading
/// Key format: prefix + zero-padded ID (e.g., "zvec_:000000055083")
///
/// Fields added:
/// - vector_field: <vector data> (always)
/// - tag_field: <tag value> (if search_config.tag_field is set)
/// - numeric_field(s): <numeric value> (from search_config.numeric_fields)
fn create_vec_load_template(search_config: &SearchConfig, key_width: usize) -> CommandTemplate {
    // Simple key format: prefix + zero-padded vector ID
    let mut template = CommandTemplate::new("HSET")
        .arg_str("HSET")
        .arg_prefixed_key(&search_config.prefix, key_width)
        .arg_str(&search_config.vector_field)
        .arg_vector(search_config.vec_byte_len());

    // Add tag field if configured (this is INDEX tag field, NOT cluster hash tag)
    if let Some(ref tag_field) = search_config.tag_field {
        template = template
            .arg_str(tag_field)
            .arg_tag_placeholder(search_config.tag_max_len);
    }

    // Add numeric fields from the NumericFieldSet
    for (idx, field_config) in search_config.numeric_fields.iter().enumerate() {
        template = template
            .arg_str(&field_config.name)
            .arg_numeric_field(idx, field_config.max_byte_len());
    }

    template
}

/// Create FT.SEARCH template for vector queries
///
/// Supports both tag filters and numeric filters:
///   No filters:    "*=>[KNN $K @embedding $BLOB]"
///   Tag filter:    "@tag_field:{filter}=>[KNN $K @embedding $BLOB]"
///   Numeric:       "@score:[50 100]=>[KNN $K @embedding $BLOB]"
///   Combined:      "(@tag_field:{filter} @score:[50 100])=>[KNN $K @embedding $BLOB]"
fn create_vec_query_template(search_config: &SearchConfig) -> CommandTemplate {
    let mut template = CommandTemplate::new("FT.SEARCH")
        .arg_str("FT.SEARCH")
        .arg_str(&search_config.index_name);

    // Collect all filter parts
    let mut filter_parts: Vec<String> = Vec::new();

    // Add tag filter if present
    if let (Some(ref tag_field), Some(ref tag_filter)) =
        (&search_config.tag_field, &search_config.tag_filter)
    {
        filter_parts.push(format!("@{}:{{{}}}", tag_field, tag_filter));
    }

    // Add numeric filters
    for filter in &search_config.numeric_filters {
        filter_parts.push(filter.format_query());
    }

    // Build filter prefix
    let filter_prefix = if filter_parts.is_empty() {
        "*".to_string()
    } else if filter_parts.len() == 1 {
        filter_parts.into_iter().next().unwrap()
    } else {
        // Multiple filters: combine with space inside parentheses
        format!("({})", filter_parts.join(" "))
    };

    // Build query string based on config
    let query = format!(
        "{}=>[KNN {} @{} $BLOB{}]",
        filter_prefix,
        search_config.k,
        search_config.vector_field,
        if let Some(ef) = search_config.ef_search {
            format!(" EF_RUNTIME {}", ef)
        } else {
            String::new()
        }
    );

    template = template
        .arg_str(&query)
        .arg_str("PARAMS")
        .arg_str("2") // 2 parameters
        .arg_str("BLOB")
        .arg_query_vector(search_config.vec_byte_len()) // Use query vector for FT.SEARCH
        .arg_str("DIALECT")
        .arg_str("2") // DIALECT 2 required for KNN queries
        .arg_str("LIMIT")
        .arg_str("0") // offset
        .arg_str(&search_config.k.to_string()); // count - must match KNN k to get all results

    if search_config.nocontent {
        template = template.arg_str("NOCONTENT");
    }

    template
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DistanceMetric, VectorAlgorithm};
    use crate::workload::NumericFieldSet;

    #[test]
    fn test_create_ping_template() {
        let template = create_template(WorkloadType::Ping, "key:", 3, None, false);
        let buf = template.build(1);
        assert!(buf.placeholders[0].is_empty());
    }

    #[test]
    fn test_create_set_template() {
        let template = create_template(WorkloadType::Set, "key:", 100, None, false);
        let buf = template.build(1);
        assert_eq!(buf.placeholders[0].len(), 1); // Key placeholder
    }

    #[test]
    fn test_create_set_template_cluster_mode() {
        // Note: cluster_mode parameter is now ignored (cluster hash tags removed)
        let template = create_template(WorkloadType::Set, "key:", 100, None, true);
        let buf = template.build(1);
        // All keys now use simple format: just 1 key placeholder
        assert_eq!(buf.placeholders[0].len(), 1);
    }

    #[test]
    fn test_create_get_template() {
        let template = create_template(WorkloadType::Get, "key:", 3, None, false);
        let buf = template.build(1);
        assert_eq!(buf.placeholders[0].len(), 1); // Key placeholder
    }

    #[test]
    fn test_create_vec_load_template() {
        let search_config = SearchConfig {
            index_name: "idx".to_string(),
            vector_field: "embedding".to_string(),
            prefix: "vec:".to_string(),
            algorithm: VectorAlgorithm::Hnsw,
            distance_metric: DistanceMetric::L2,
            dim: 128,
            k: 10,
            ef_construction: None,
            hnsw_m: None,
            ef_search: None,
            nocontent: false,
            tag_field: None,
            tag_distributions: None,
            tag_filter: None,
            tag_max_len: 128,
            numeric_field: None,
            numeric_fields: NumericFieldSet::new(),
            numeric_filters: Vec::new(),
        };

        let template = create_template(WorkloadType::VecLoad, "key:", 3, Some(&search_config), false);
        let buf = template.build(1);

        // VecLoad now uses simple key format:
        // 1 key placeholder + 1 vector placeholder = 2 total
        assert_eq!(buf.placeholders[0].len(), 2);
    }
}
