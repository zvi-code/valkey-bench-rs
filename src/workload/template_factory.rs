//! Template factory for creating command templates for all workload types
//!
//! Uses `AddressSpec` from addressable.rs for unified key/address handling.

use crate::config::SearchConfig;

use super::addressable::{AddressSpec, AddressType, DEFAULT_KEY_WIDTH};
use super::command_template::CommandTemplate;
use super::workload_type::WorkloadType;

/// Create command template for given workload type
///
/// # Arguments
/// * `workload` - The workload type
/// * `key_prefix` - Key prefix (e.g., "key:", "vec:")
/// * `data_size` - Size of data payload for SET-like commands
/// * `search_config` - Optional search configuration for vector workloads
/// * `address_spec` - Optional address specification for sub-key iteration
pub fn create_template(
    workload: WorkloadType,
    key_prefix: &str,
    data_size: usize,
    search_config: Option<&SearchConfig>,
    address_spec: Option<&AddressSpec>,
) -> CommandTemplate {
    // Use provided AddressSpec or create simple key-only spec
    let default_spec = AddressSpec::with_width(key_prefix, DEFAULT_KEY_WIDTH, u64::MAX);
    let spec = address_spec.unwrap_or(&default_spec);

    match workload {
        // === Simple commands ===
        WorkloadType::Ping => CommandTemplate::new("PING").arg_str("PING"),

        // === Key-value commands ===
        WorkloadType::Set => spec
            .add_key_to_template(CommandTemplate::new("SET").arg_str("SET"))
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Get => spec.add_key_to_template(CommandTemplate::new("GET").arg_str("GET")),

        WorkloadType::Incr => {
            spec.add_key_to_template(CommandTemplate::new("INCR").arg_str("INCR"))
        }

        // === List commands ===
        WorkloadType::Lpush => spec
            .add_key_to_template(CommandTemplate::new("LPUSH").arg_str("LPUSH"))
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Rpush => spec
            .add_key_to_template(CommandTemplate::new("RPUSH").arg_str("RPUSH"))
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Lpop => {
            spec.add_key_to_template(CommandTemplate::new("LPOP").arg_str("LPOP"))
        }

        WorkloadType::Rpop => {
            spec.add_key_to_template(CommandTemplate::new("RPOP").arg_str("RPOP"))
        }

        WorkloadType::Lrange100 => create_lrange_template(spec, 100),
        WorkloadType::Lrange300 => create_lrange_template(spec, 300),
        WorkloadType::Lrange500 => create_lrange_template(spec, 500),
        WorkloadType::Lrange600 => create_lrange_template(spec, 600),

        // === Set commands ===
        WorkloadType::Sadd => spec
            .add_key_to_template(CommandTemplate::new("SADD").arg_str("SADD"))
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Spop => {
            spec.add_key_to_template(CommandTemplate::new("SPOP").arg_str("SPOP"))
        }

        // === Hash commands ===
        WorkloadType::Hset => {
            let template =
                spec.add_key_to_template(CommandTemplate::new("HSET").arg_str("HSET"));
            // AddressSpec handles field placeholder if sub_key is set
            // If no sub_key, use default literal field name
            if spec.address_type() == AddressType::Key {
                template.arg_str("field").arg_literal(&vec![b'x'; data_size])
            } else {
                template.arg_literal(&vec![b'x'; data_size])
            }
        }

        // === Sorted set commands ===
        WorkloadType::Zadd => spec
            .add_key_to_template(CommandTemplate::new("ZADD").arg_str("ZADD"))
            .arg_rand_int(spec.key_width())
            .arg_literal(&vec![b'x'; data_size]),

        WorkloadType::Zpopmin => {
            spec.add_key_to_template(CommandTemplate::new("ZPOPMIN").arg_str("ZPOPMIN"))
        }

        // === Multi-key commands ===
        WorkloadType::Mset => create_mset_template(spec, data_size, 10),

        // === Vector search commands ===
        WorkloadType::VecLoad | WorkloadType::VecGtLoad | WorkloadType::VecUpdate => {
            let sc = search_config.expect("Vector workload requires search config");
            create_vec_load_template(sc)
        }

        WorkloadType::VecQuery => {
            let sc = search_config.expect("VecQuery requires search config");
            create_vec_query_template(sc)
        }
        WorkloadType::VecDel => {
            let sc = search_config.expect("VecDelProtected requires search config");
            // Same as VecDelete - just DEL with prefixed key
            // The GT protection is handled by DeleteContext
            CommandTemplate::new("DEL")
                .arg_str("DEL")
                .arg_prefixed_key(&sc.prefix, spec.key_width())
        }        
    }
}

/// Create LRANGE template with specified count
fn create_lrange_template(spec: &AddressSpec, count: i32) -> CommandTemplate {
    spec.add_key_to_template(
        CommandTemplate::new(&format!("LRANGE_{}", count)).arg_str("LRANGE"),
    )
    .arg_str("0")
    .arg_str(&(count - 1).to_string())
}

/// Create MSET template with multiple keys
fn create_mset_template(spec: &AddressSpec, data_size: usize, num_keys: usize) -> CommandTemplate {
    let mut template = CommandTemplate::new("MSET").arg_str("MSET");

    for _ in 0..num_keys {
        template = template.arg_prefixed_key(spec.prefix(), spec.key_width());
        template = template.arg_literal(&vec![b'x'; data_size]);
    }

    template
}

/// Create HSET template for vector loading
///
/// Fields added:
/// - vector_field: <vector data> (always)
/// - tag_field: <tag value> (if search_config.tag_field is set)
/// - numeric_field(s): <numeric value> (from search_config.numeric_fields)
fn create_vec_load_template(search_config: &SearchConfig) -> CommandTemplate {
    let spec = AddressSpec::with_width(&search_config.prefix, DEFAULT_KEY_WIDTH, u64::MAX);

    let mut template = spec
        .add_key_to_template(CommandTemplate::new("HSET").arg_str("HSET"))
        .arg_str(&search_config.vector_field)
        .arg_vector(search_config.vec_byte_len());

    // Add tag field if configured
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
        let template = create_template(WorkloadType::Ping, "key:", 3, None, None);
        let buf = template.build(1);
        assert!(buf.placeholders[0].is_empty());
    }

    #[test]
    fn test_create_set_template() {
        let template = create_template(WorkloadType::Set, "key:", 100, None, None);
        let buf = template.build(1);
        assert_eq!(buf.placeholders[0].len(), 1); // Key placeholder
    }

    #[test]
    fn test_create_set_with_address_spec() {
        let spec = AddressSpec::key("mykey:", 1_000_000);
        let template = create_template(WorkloadType::Set, "key:", 100, None, Some(&spec));
        let buf = template.build(1);
        assert_eq!(buf.placeholders[0].len(), 1); // Key placeholder
    }

    #[test]
    fn test_create_get_template() {
        let template = create_template(WorkloadType::Get, "key:", 3, None, None);
        let buf = template.build(1);
        assert_eq!(buf.placeholders[0].len(), 1); // Key placeholder
    }

    #[test]
    fn test_create_hset_with_field_iteration() {
        let spec = AddressSpec::hash_fields(
            "obj:",
            100_000,
            vec!["f1".to_string(), "f2".to_string()],
        );
        let template = create_template(WorkloadType::Hset, "obj:", 50, None, Some(&spec));
        let buf = template.build(1);
        // Key placeholder + Field placeholder = 2 placeholders
        assert_eq!(buf.placeholders[0].len(), 2);
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

        let template = create_template(WorkloadType::VecLoad, "key:", 3, Some(&search_config), None);
        let buf = template.build(1);

        // VecLoad: 1 key placeholder + 1 vector placeholder = 2 total
        assert_eq!(buf.placeholders[0].len(), 2);
    }
}
