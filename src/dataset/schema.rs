//! Schema parsing from YAML files
//!
//! This module provides schema definitions and parsing for the unified
//! schema-driven dataset system. Schemas describe the structure of binary
//! data files, enabling flexible dataset formats without code changes.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Current schema version
pub const SCHEMA_VERSION: u32 = 1;

/// Field type enum
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Vector,
    Text,
    Tag,
    Numeric,
    Blob,
}

/// Data type for vectors and numerics
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DType {
    #[default]
    Float32,
    Float16,
    Float64,
    Int32,
    Int64,
    #[serde(alias = "u32")]
    Uint32,
    #[serde(alias = "u64")]
    Uint64,
    Uint8,
    Int8,
}

impl DType {
    /// Get byte size for this data type
    pub fn byte_size(&self) -> usize {
        match self {
            DType::Float32 | DType::Int32 | DType::Uint32 => 4,
            DType::Float16 => 2,
            DType::Float64 | DType::Int64 | DType::Uint64 => 8,
            DType::Uint8 | DType::Int8 => 1,
        }
    }

    /// Get string representation for display
    pub fn as_str(&self) -> &'static str {
        match self {
            DType::Float32 => "float32",
            DType::Float16 => "float16",
            DType::Float64 => "float64",
            DType::Int32 => "int32",
            DType::Int64 => "int64",
            DType::Uint32 => "u32",
            DType::Uint64 => "u64",
            DType::Uint8 => "uint8",
            DType::Int8 => "int8",
        }
    }
}

/// Text encoding
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Utf8,
    Ascii,
}

/// Length specification for variable-length fields
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LengthSpec {
    #[default]
    Fixed,
    Variable,
}

/// Field definition from schema
#[derive(Debug, Clone, Deserialize)]
pub struct FieldDef {
    /// Field name (used for HSET field names)
    pub name: String,

    /// Field type
    #[serde(rename = "type")]
    pub field_type: FieldType,

    /// Data type (for vectors and numerics)
    pub dtype: Option<DType>,

    /// Vector dimensions
    pub dimensions: Option<u32>,

    /// Text encoding
    pub encoding: Option<Encoding>,

    /// Length specification (fixed or variable)
    pub length: Option<LengthSpec>,

    /// Maximum bytes for text/tag/blob fields
    pub max_bytes: Option<u32>,
}

impl FieldDef {
    /// Compute byte size for this field
    pub fn byte_size(&self) -> usize {
        match self.field_type {
            FieldType::Vector => {
                let dim = self.dimensions.unwrap_or(1) as usize;
                let dtype = self.dtype.unwrap_or_default();
                dim * dtype.byte_size()
            }
            FieldType::Text | FieldType::Tag | FieldType::Blob => {
                let max_bytes = self.max_bytes.unwrap_or(64) as usize;
                let length = self.length.unwrap_or_default();
                if length == LengthSpec::Variable {
                    4 + max_bytes // u32 length prefix + data
                } else {
                    max_bytes
                }
            }
            FieldType::Numeric => self.dtype.unwrap_or(DType::Float64).byte_size(),
        }
    }

    /// Check if this is a vector field
    pub fn is_vector(&self) -> bool {
        self.field_type == FieldType::Vector
    }
}

/// Collection member definition (for SET, LIST, ZSET)
#[derive(Debug, Clone, Deserialize)]
pub struct MemberDef {
    /// Simple member type
    #[serde(rename = "type")]
    pub member_type: Option<FieldType>,

    /// Encoding for simple members
    pub encoding: Option<Encoding>,

    /// Length spec for simple members
    pub length: Option<LengthSpec>,

    /// Max bytes for simple members
    pub max_bytes: Option<u32>,

    /// Compound member fields (for ZSET: score + value)
    pub fields: Option<Vec<FieldDef>>,
}

impl MemberDef {
    /// Compute byte size for one member
    pub fn byte_size(&self) -> usize {
        if let Some(ref fields) = self.fields {
            // Compound member (e.g., ZSET with score + value)
            fields.iter().map(|f| f.byte_size()).sum()
        } else {
            // Simple member
            let max_bytes = self.max_bytes.unwrap_or(64) as usize;
            let length = self.length.unwrap_or_default();
            if length == LengthSpec::Variable {
                4 + max_bytes
            } else {
                max_bytes
            }
        }
    }
}

/// Collection definition (for SET, LIST, ZSET)
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionDef {
    /// Collection type
    #[serde(rename = "type")]
    pub collection_type: String,

    /// Maximum members per collection
    pub max_members: u32,

    /// Member definition
    pub member: MemberDef,
}

impl CollectionDef {
    /// Compute total byte size for this collection record
    pub fn byte_size(&self) -> usize {
        // u32 member count + max_members * member_size
        4 + (self.max_members as usize * self.member.byte_size())
    }
}

/// Record definition (either fields or collection)
#[derive(Debug, Clone, Deserialize)]
pub struct RecordDef {
    /// Field-based record (for HASH, STRING with fields)
    pub fields: Option<Vec<FieldDef>>,

    /// Collection-based record (for SET, LIST, ZSET)
    pub collection: Option<CollectionDef>,
}

impl RecordDef {
    /// Get fields if this is a field-based record
    pub fn get_fields(&self) -> Option<&[FieldDef]> {
        self.fields.as_deref()
    }

    /// Get collection if this is a collection-based record
    pub fn get_collection(&self) -> Option<&CollectionDef> {
        self.collection.as_ref()
    }

    /// Compute total record size
    pub fn byte_size(&self) -> usize {
        if let Some(ref collection) = self.collection {
            collection.byte_size()
        } else if let Some(ref fields) = self.fields {
            fields.iter().map(|f| f.byte_size()).sum()
        } else {
            0
        }
    }
}

/// Records section configuration
#[derive(Debug, Clone, Deserialize)]
pub struct RecordsConfig {
    /// Number of records
    pub count: u64,
}

/// Keys section configuration
#[derive(Debug, Clone, Deserialize, Default)]
pub struct KeysConfig {
    /// Whether keys are present in the data file
    #[serde(default)]
    pub present: bool,

    /// Key generation pattern (when not present in file)
    pub pattern: Option<String>,

    /// Key encoding
    pub encoding: Option<Encoding>,

    /// Key length specification
    pub length: Option<LengthSpec>,

    /// Maximum key bytes
    pub max_bytes: Option<u32>,
}

/// Queries section configuration
#[derive(Debug, Clone, Deserialize, Default)]
pub struct QueriesConfig {
    /// Whether queries are present
    #[serde(default)]
    pub present: bool,

    /// Number of queries
    pub count: Option<u64>,

    /// Fields to include in query records
    pub query_fields: Option<Vec<String>>,
}

/// Ground truth section configuration
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GroundTruthConfig {
    /// Whether ground truth is present
    #[serde(default)]
    pub present: bool,

    /// Number of neighbors per query
    pub neighbors_per_query: Option<u32>,

    /// ID type (u32 or u64)
    pub id_type: Option<String>,
}

/// All sections configuration
#[derive(Debug, Clone, Deserialize)]
pub struct SectionsConfig {
    /// Records section
    pub records: RecordsConfig,

    /// Keys section
    #[serde(default)]
    pub keys: KeysConfig,

    /// Queries section
    #[serde(default)]
    pub queries: QueriesConfig,

    /// Ground truth section
    #[serde(default)]
    pub ground_truth: GroundTruthConfig,
}

/// Schema metadata
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SchemaMetadata {
    /// Dataset name
    pub name: Option<String>,

    /// Dataset description
    pub description: Option<String>,
}

/// Field-specific metadata (index type, distance metric, etc.)
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FieldMetadata {
    /// Distance metric for vector fields
    pub distance_metric: Option<String>,

    /// Index type for text/tag fields
    pub index_type: Option<String>,
}

/// Workload hints
#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkloadConfig {
    /// Workload type (string, hash, set, list, zset, vector)
    #[serde(rename = "type")]
    pub workload_type: Option<String>,

    /// Primary command (SET, HSET, SADD, etc.)
    pub command: Option<String>,
}

/// Complete dataset schema
#[derive(Debug, Clone, Deserialize)]
pub struct DatasetSchema {
    /// Schema version
    pub version: u32,

    /// Metadata
    #[serde(default)]
    pub metadata: SchemaMetadata,

    /// Workload hints
    #[serde(default)]
    pub workload: WorkloadConfig,

    /// Record definition
    pub record: RecordDef,

    /// Sections configuration
    pub sections: SectionsConfig,

    /// Field-specific metadata
    #[serde(default)]
    pub field_metadata: HashMap<String, FieldMetadata>,
}

impl DatasetSchema {
    /// Load schema from YAML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, SchemaError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| SchemaError::IoError(e.to_string()))?;

        Self::from_yaml(&content)
    }

    /// Parse schema from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, SchemaError> {
        let schema: DatasetSchema =
            serde_yaml::from_str(yaml).map_err(|e| SchemaError::ParseError(e.to_string()))?;

        // Validate version
        if schema.version > SCHEMA_VERSION {
            return Err(SchemaError::UnsupportedVersion(schema.version));
        }

        // Validate record definition
        if schema.record.fields.is_none() && schema.record.collection.is_none() {
            return Err(SchemaError::InvalidSchema(
                "Record must have either 'fields' or 'collection'".to_string(),
            ));
        }

        Ok(schema)
    }

    /// Get dataset name
    pub fn name(&self) -> &str {
        self.metadata.name.as_deref().unwrap_or("unnamed")
    }

    /// Get distance metric for a field
    pub fn get_distance_metric(&self, field_name: &str) -> Option<&str> {
        self.field_metadata
            .get(field_name)
            .and_then(|m| m.distance_metric.as_deref())
    }

    /// Get the first vector field definition
    pub fn first_vector_field(&self) -> Option<&FieldDef> {
        self.record
            .fields
            .as_ref()?
            .iter()
            .find(|f| f.is_vector())
    }

    /// Get vector dimensions from first vector field
    pub fn vector_dimensions(&self) -> Option<u32> {
        self.first_vector_field()?.dimensions
    }

    /// Get number of records
    pub fn record_count(&self) -> u64 {
        self.sections.records.count
    }

    /// Get number of queries
    pub fn query_count(&self) -> u64 {
        if self.sections.queries.present {
            self.sections.queries.count.unwrap_or(0)
        } else {
            0
        }
    }

    /// Get number of ground truth neighbors per query
    pub fn neighbors_per_query(&self) -> usize {
        self.sections
            .ground_truth
            .neighbors_per_query
            .unwrap_or(100) as usize
    }

    /// Check if ground truth is present
    pub fn has_ground_truth(&self) -> bool {
        self.sections.ground_truth.present
    }

    /// Get ground truth ID size in bytes
    pub fn ground_truth_id_size(&self) -> usize {
        if self.sections.ground_truth.id_type.as_deref() == Some("u32") {
            4
        } else {
            8
        }
    }

    /// Get names of all tag fields in the schema
    pub fn tag_field_names(&self) -> Vec<&str> {
        self.record
            .fields
            .as_ref()
            .map(|fields| {
                fields
                    .iter()
                    .filter(|f| f.field_type == FieldType::Tag)
                    .map(|f| f.name.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get names of all numeric fields in the schema
    pub fn numeric_field_names(&self) -> Vec<&str> {
        self.record
            .fields
            .as_ref()
            .map(|fields| {
                fields
                    .iter()
                    .filter(|f| f.field_type == FieldType::Numeric)
                    .map(|f| f.name.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the first tag field name (for simple filtered search)
    pub fn first_tag_field(&self) -> Option<&str> {
        self.tag_field_names().into_iter().next()
    }

    /// Get the first numeric field name (for simple filtered search)
    pub fn first_numeric_field(&self) -> Option<&str> {
        self.numeric_field_names().into_iter().next()
    }
}

/// Schema parsing error
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("IO error: {0}")]
    IoError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Unsupported schema version: {0}")]
    UnsupportedVersion(u32),

    #[error("Invalid schema: {0}")]
    InvalidSchema(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTOR_SCHEMA_YAML: &str = r#"
version: 1
metadata:
  name: "test-vectors"
record:
  fields:
    - name: embedding
      type: vector
      dtype: float32
      dimensions: 128
sections:
  records:
    count: 1000
  keys:
    present: false
    pattern: "vec:%012d"
  queries:
    present: true
    count: 100
    query_fields:
      - embedding
  ground_truth:
    present: true
    neighbors_per_query: 10
    id_type: u64
field_metadata:
  embedding:
    distance_metric: l2
"#;

    const HASH_SCHEMA_YAML: &str = r#"
version: 1
metadata:
  name: "test-hash"
record:
  fields:
    - name: field1
      type: text
      encoding: utf8
      length: fixed
      max_bytes: 64
    - name: field2
      type: numeric
      dtype: float64
sections:
  records:
    count: 500
  keys:
    present: true
    encoding: utf8
    length: fixed
    max_bytes: 32
"#;

    #[test]
    fn test_parse_vector_schema() {
        let schema = DatasetSchema::from_yaml(VECTOR_SCHEMA_YAML).unwrap();

        assert_eq!(schema.version, 1);
        assert_eq!(schema.name(), "test-vectors");
        assert_eq!(schema.record_count(), 1000);
        assert_eq!(schema.query_count(), 100);
        assert!(schema.has_ground_truth());
        assert_eq!(schema.neighbors_per_query(), 10);

        let vec_field = schema.first_vector_field().unwrap();
        assert_eq!(vec_field.name, "embedding");
        assert_eq!(vec_field.dimensions, Some(128));
        assert_eq!(vec_field.dtype, Some(DType::Float32));
        assert_eq!(vec_field.byte_size(), 128 * 4); // 512 bytes

        assert_eq!(schema.get_distance_metric("embedding"), Some("l2"));
    }

    #[test]
    fn test_parse_hash_schema() {
        let schema = DatasetSchema::from_yaml(HASH_SCHEMA_YAML).unwrap();

        assert_eq!(schema.name(), "test-hash");
        assert_eq!(schema.record_count(), 500);
        assert_eq!(schema.query_count(), 0);
        assert!(!schema.has_ground_truth());

        let fields = schema.record.get_fields().unwrap();
        assert_eq!(fields.len(), 2);

        assert_eq!(fields[0].name, "field1");
        assert_eq!(fields[0].byte_size(), 64);

        assert_eq!(fields[1].name, "field2");
        assert_eq!(fields[1].byte_size(), 8);

        // Total record size
        assert_eq!(schema.record.byte_size(), 64 + 8);
    }

    #[test]
    fn test_field_sizes() {
        // Vector field
        let vec_field = FieldDef {
            name: "vec".to_string(),
            field_type: FieldType::Vector,
            dtype: Some(DType::Float32),
            dimensions: Some(768),
            encoding: None,
            length: None,
            max_bytes: None,
        };
        assert_eq!(vec_field.byte_size(), 768 * 4);

        // Fixed text field
        let text_field = FieldDef {
            name: "text".to_string(),
            field_type: FieldType::Text,
            dtype: None,
            dimensions: None,
            encoding: Some(Encoding::Utf8),
            length: Some(LengthSpec::Fixed),
            max_bytes: Some(256),
        };
        assert_eq!(text_field.byte_size(), 256);

        // Variable text field
        let var_text_field = FieldDef {
            name: "var_text".to_string(),
            field_type: FieldType::Text,
            dtype: None,
            dimensions: None,
            encoding: Some(Encoding::Utf8),
            length: Some(LengthSpec::Variable),
            max_bytes: Some(256),
        };
        assert_eq!(var_text_field.byte_size(), 4 + 256); // u32 prefix + data

        // Numeric field
        let num_field = FieldDef {
            name: "num".to_string(),
            field_type: FieldType::Numeric,
            dtype: Some(DType::Int64),
            dimensions: None,
            encoding: None,
            length: None,
            max_bytes: None,
        };
        assert_eq!(num_field.byte_size(), 8);
    }

    #[test]
    fn test_invalid_schema() {
        // Missing both fields and collection
        let bad_yaml = r#"
version: 1
record: {}
sections:
  records:
    count: 100
"#;
        let result = DatasetSchema::from_yaml(bad_yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_unsupported_version() {
        let future_yaml = r#"
version: 999
record:
  fields:
    - name: test
      type: text
      max_bytes: 32
sections:
  records:
    count: 100
"#;
        let result = DatasetSchema::from_yaml(future_yaml);
        assert!(matches!(result, Err(SchemaError::UnsupportedVersion(999))));
    }
}
