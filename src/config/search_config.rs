//! Vector search configuration

use super::cli::{CliArgs, DistanceMetric, VectorAlgorithm};
use crate::workload::{NumericFieldConfig, NumericFieldSet, NumericValueType, TagDistributionSet};

/// Numeric bound for range filters
#[derive(Debug, Clone, PartialEq)]
pub enum NumericBound {
    /// Inclusive bound [x or x]
    Inclusive(f64),
    /// Exclusive bound (x or x)
    Exclusive(f64),
    /// Positive infinity (+inf)
    Inf,
    /// Negative infinity (-inf)
    NegInf,
}

impl NumericBound {
    /// Parse a bound from string
    ///
    /// Formats:
    /// - "[50" -> Inclusive(50)
    /// - "(50" -> Exclusive(50)
    /// - "50]" -> Inclusive(50)
    /// - "50)" -> Exclusive(50)
    /// - "+inf" or "inf" -> Inf
    /// - "-inf" -> NegInf
    pub fn parse_min(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(NumericBound::NegInf);
        }

        if s.eq_ignore_ascii_case("-inf") || s.eq_ignore_ascii_case("(-inf") {
            return Ok(NumericBound::NegInf);
        }

        if s.starts_with('[') {
            let num: f64 = s[1..].parse().map_err(|_| format!("Invalid bound: {}", s))?;
            Ok(NumericBound::Inclusive(num))
        } else if s.starts_with('(') {
            let num: f64 = s[1..].parse().map_err(|_| format!("Invalid bound: {}", s))?;
            Ok(NumericBound::Exclusive(num))
        } else {
            // Default to inclusive for plain numbers
            let num: f64 = s.parse().map_err(|_| format!("Invalid bound: {}", s))?;
            Ok(NumericBound::Inclusive(num))
        }
    }

    /// Parse an upper bound from string
    pub fn parse_max(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(NumericBound::Inf);
        }

        if s.eq_ignore_ascii_case("+inf") || s.eq_ignore_ascii_case("inf") || s.eq_ignore_ascii_case("+inf)") {
            return Ok(NumericBound::Inf);
        }

        if s.ends_with(']') {
            let num: f64 = s[..s.len()-1].parse().map_err(|_| format!("Invalid bound: {}", s))?;
            Ok(NumericBound::Inclusive(num))
        } else if s.ends_with(')') {
            let num: f64 = s[..s.len()-1].parse().map_err(|_| format!("Invalid bound: {}", s))?;
            Ok(NumericBound::Exclusive(num))
        } else {
            // Default to inclusive for plain numbers
            let num: f64 = s.parse().map_err(|_| format!("Invalid bound: {}", s))?;
            Ok(NumericBound::Inclusive(num))
        }
    }

    /// Format for FT.SEARCH query
    pub fn format(&self) -> String {
        match self {
            NumericBound::Inclusive(v) => format!("{}", v),
            NumericBound::Exclusive(v) => format!("({}", v),
            NumericBound::Inf => "+inf".to_string(),
            NumericBound::NegInf => "-inf".to_string(),
        }
    }
}

/// Numeric range filter for FT.SEARCH queries
#[derive(Debug, Clone)]
pub struct NumericFilter {
    /// Field name to filter on
    pub field: String,
    /// Minimum bound
    pub min: NumericBound,
    /// Maximum bound
    pub max: NumericBound,
}

impl NumericFilter {
    /// Create a new numeric filter
    pub fn new(field: &str, min: NumericBound, max: NumericBound) -> Self {
        Self {
            field: field.to_string(),
            min,
            max,
        }
    }

    /// Create an inclusive range filter [min, max]
    pub fn inclusive_range(field: &str, min: f64, max: f64) -> Self {
        Self::new(field, NumericBound::Inclusive(min), NumericBound::Inclusive(max))
    }

    /// Parse from CLI specification
    ///
    /// Format: "field:[min,max]" or "field:(min,max)" or "field:[min,max)"
    /// Examples:
    /// - "score:[50,100]" -> @score:[50 100]
    /// - "score:(0,100]" -> @score:[(0 100]
    /// - "score:[-inf,50]" -> @score:[-inf 50]
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();

        // Find the colon separator
        let colon_pos = spec.find(':')
            .ok_or_else(|| format!("Missing ':' in numeric filter: {}", spec))?;

        let field = &spec[..colon_pos];
        let range = &spec[colon_pos + 1..];

        if range.is_empty() {
            return Err("Empty range specification".to_string());
        }

        // Find the range bounds
        // Expected format: [min,max] or (min,max) or [min,max) or (min,max]
        let first_char = range.chars().next().unwrap();
        let last_char = range.chars().last().unwrap();

        if first_char != '[' && first_char != '(' {
            return Err(format!("Range must start with '[' or '(': {}", range));
        }
        if last_char != ']' && last_char != ')' {
            return Err(format!("Range must end with ']' or ')': {}", range));
        }

        // Extract min and max parts
        let inner = &range[1..range.len()-1];
        let comma_pos = inner.find(',')
            .ok_or_else(|| format!("Missing ',' in range: {}", range))?;

        let min_str = inner[..comma_pos].trim();
        let max_str = inner[comma_pos + 1..].trim();

        // Parse bounds with bracket info
        let min = if first_char == '[' {
            if min_str.eq_ignore_ascii_case("-inf") {
                NumericBound::NegInf
            } else {
                NumericBound::Inclusive(min_str.parse().map_err(|_| format!("Invalid min: {}", min_str))?)
            }
        } else if min_str.eq_ignore_ascii_case("-inf") {
            NumericBound::NegInf
        } else {
            NumericBound::Exclusive(min_str.parse().map_err(|_| format!("Invalid min: {}", min_str))?)
        };

        let max = if last_char == ']' {
            if max_str.eq_ignore_ascii_case("+inf") || max_str.eq_ignore_ascii_case("inf") {
                NumericBound::Inf
            } else {
                NumericBound::Inclusive(max_str.parse().map_err(|_| format!("Invalid max: {}", max_str))?)
            }
        } else if max_str.eq_ignore_ascii_case("+inf") || max_str.eq_ignore_ascii_case("inf") {
            NumericBound::Inf
        } else {
            NumericBound::Exclusive(max_str.parse().map_err(|_| format!("Invalid max: {}", max_str))?)
        };

        Ok(Self::new(field, min, max))
    }

    /// Format for FT.SEARCH query
    ///
    /// Output: "@field:[min max]"
    pub fn format_query(&self) -> String {
        format!("@{}:[{} {}]", self.field, self.min.format(), self.max.format())
    }
}

/// Vector search configuration
#[derive(Debug, Clone)]
pub struct SearchConfig {
    pub index_name: String,
    pub vector_field: String,
    pub prefix: String,
    pub algorithm: VectorAlgorithm,
    pub distance_metric: DistanceMetric,
    pub dim: u32,
    pub k: u32,
    pub ef_construction: Option<u32>,
    pub hnsw_m: Option<u32>,
    pub ef_search: Option<u32>,
    pub nocontent: bool,
    /// Tag field name (optional, for filtered search)
    pub tag_field: Option<String>,
    /// Tag distribution set for generating tags during vec-load
    pub tag_distributions: Option<TagDistributionSet>,
    /// Tag filter pattern for vec-query (e.g., "tag1|tag2")
    pub tag_filter: Option<String>,
    /// Maximum tag payload length
    pub tag_max_len: usize,
    /// Numeric field name (optional, for filtered search) - backward compatibility
    pub numeric_field: Option<String>,
    /// Extended numeric field configurations with types and distributions
    pub numeric_fields: NumericFieldSet,
    /// Numeric range filters for FT.SEARCH queries (query-side filtering)
    pub numeric_filters: Vec<NumericFilter>,
}

impl SearchConfig {
    pub fn from_cli(args: &CliArgs) -> Self {
        // Parse tag distributions if provided
        let tag_distributions = args.search_tags.as_ref().and_then(|tags_str| {
            match TagDistributionSet::parse(tags_str) {
                Ok(set) => Some(set.with_max_len(args.tag_max_len)),
                Err(e) => {
                    eprintln!("Warning: Failed to parse --search-tags: {}", e);
                    None
                }
            }
        });

        // Parse extended numeric field configurations
        let mut numeric_fields = NumericFieldSet::new();
        for config_str in &args.numeric_field_configs {
            match NumericFieldConfig::parse(config_str) {
                Ok(config) => numeric_fields.add(config),
                Err(e) => {
                    eprintln!("Warning: Failed to parse --numeric-field-config '{}': {}", config_str, e);
                }
            }
        }

        // If no extended configs but simple --numeric-field is set, create a key-based config
        if numeric_fields.is_empty() {
            if let Some(ref field_name) = args.numeric_field {
                // Create a simple key-based numeric field for backward compatibility
                let config = NumericFieldConfig::new_key_based(field_name, 0.0, f64::MAX);
                numeric_fields.add(config);
            }
        }

        // Parse numeric filters for query-side filtering
        let mut numeric_filters = Vec::new();
        for filter_str in &args.numeric_filters {
            match NumericFilter::parse(filter_str) {
                Ok(filter) => numeric_filters.push(filter),
                Err(e) => {
                    eprintln!("Warning: Failed to parse --numeric-filter '{}': {}", filter_str, e);
                }
            }
        }

        Self {
            index_name: args.search_index.clone(),
            vector_field: args.search_vector_field.clone(),
            prefix: args.search_prefix.clone(),
            algorithm: args.search_algorithm,
            distance_metric: args.search_distance,
            dim: args.vector_dim,
            k: args.search_k,
            ef_construction: args.ef_construction,
            hnsw_m: args.hnsw_m,
            ef_search: args.ef_search,
            nocontent: args.nocontent,
            tag_field: args.tag_field.clone(),
            tag_distributions,
            tag_filter: args.tag_filter.clone(),
            tag_max_len: args.tag_max_len,
            numeric_field: args.numeric_field.clone(),
            numeric_fields,
            numeric_filters,
        }
    }

    /// Update dimension from dataset
    pub fn set_dim(&mut self, dim: u32) {
        self.dim = dim;
    }

    /// Update distance metric
    pub fn set_distance_metric(&mut self, metric: DistanceMetric) {
        self.distance_metric = metric;
    }

    /// Get vector byte length (dim * sizeof(f32))
    pub fn vec_byte_len(&self) -> usize {
        self.dim as usize * std::mem::size_of::<f32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_filter_parse_inclusive() {
        let filter = NumericFilter::parse("score:[50,100]").unwrap();
        assert_eq!(filter.field, "score");
        assert_eq!(filter.min, NumericBound::Inclusive(50.0));
        assert_eq!(filter.max, NumericBound::Inclusive(100.0));
        assert_eq!(filter.format_query(), "@score:[50 100]");
    }

    #[test]
    fn test_numeric_filter_parse_exclusive() {
        let filter = NumericFilter::parse("score:(0,100)").unwrap();
        assert_eq!(filter.field, "score");
        assert_eq!(filter.min, NumericBound::Exclusive(0.0));
        assert_eq!(filter.max, NumericBound::Exclusive(100.0));
        assert_eq!(filter.format_query(), "@score:[(0 (100]");
    }

    #[test]
    fn test_numeric_filter_parse_mixed() {
        let filter = NumericFilter::parse("price:(10,50]").unwrap();
        assert_eq!(filter.field, "price");
        assert_eq!(filter.min, NumericBound::Exclusive(10.0));
        assert_eq!(filter.max, NumericBound::Inclusive(50.0));
        assert_eq!(filter.format_query(), "@price:[(10 50]");
    }

    #[test]
    fn test_numeric_filter_parse_inf() {
        let filter = NumericFilter::parse("rating:[-inf,4.5]").unwrap();
        assert_eq!(filter.field, "rating");
        assert_eq!(filter.min, NumericBound::NegInf);
        assert_eq!(filter.max, NumericBound::Inclusive(4.5));
        assert_eq!(filter.format_query(), "@rating:[-inf 4.5]");

        let filter = NumericFilter::parse("count:[100,+inf)").unwrap();
        assert_eq!(filter.field, "count");
        assert_eq!(filter.min, NumericBound::Inclusive(100.0));
        assert_eq!(filter.max, NumericBound::Inf);
        assert_eq!(filter.format_query(), "@count:[100 +inf]");
    }

    #[test]
    fn test_numeric_filter_parse_float() {
        let filter = NumericFilter::parse("temp:[98.6,102.5]").unwrap();
        assert_eq!(filter.field, "temp");
        assert_eq!(filter.min, NumericBound::Inclusive(98.6));
        assert_eq!(filter.max, NumericBound::Inclusive(102.5));
    }

    #[test]
    fn test_numeric_filter_parse_errors() {
        // Missing colon
        assert!(NumericFilter::parse("score50,100]").is_err());
        // Missing brackets
        assert!(NumericFilter::parse("score:50,100").is_err());
        // Missing comma
        assert!(NumericFilter::parse("score:[50100]").is_err());
        // Empty range
        assert!(NumericFilter::parse("score:").is_err());
    }

    #[test]
    fn test_numeric_bound_format() {
        assert_eq!(NumericBound::Inclusive(50.0).format(), "50");
        assert_eq!(NumericBound::Exclusive(50.0).format(), "(50");
        assert_eq!(NumericBound::Inf.format(), "+inf");
        assert_eq!(NumericBound::NegInf.format(), "-inf");
    }
}
