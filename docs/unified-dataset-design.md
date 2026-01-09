# Unified Schema-Driven Dataset System Design

## Overview

This document describes a unified dataset system that replaces the current vector-specific implementation with a schema-driven approach supporting arbitrary data types for benchmarking.

## Goals

1. **ONE Mechanism**: Single `DatasetContext` implementation for ALL data types - no special cases, no code duplication
2. **Schema-Driven**: YAML schema describes binary file structure at runtime
3. **Zero-Copy Efficiency**: Maintain mmap-based O(1) access patterns
4. **Extensible Field Types**: Vectors, text, tags, numeric, binary blobs - all handled uniformly
5. **Optional Components**: Keys can be generated or loaded from file
6. **Ground Truth Support**: Optional verification data for recall/accuracy metrics

**Non-Goals**:
- Backward compatibility with legacy formats (no users yet)
- Multiple code paths for different data types
- Embedded headers in binary files

## Current Architecture (To Be Replaced)

```
┌─────────────────────────────────────────────────────────────┐
│                    Binary Dataset File                       │
├─────────────────────────────────────────────────────────────┤
│ Header (4096 bytes)                                          │
│   - magic, version, dim, counts, offsets                     │
├─────────────────────────────────────────────────────────────┤
│ Database Vectors: N × dim × sizeof(f32)                      │
├─────────────────────────────────────────────────────────────┤
│ Query Vectors: Q × dim × sizeof(f32)                         │
├─────────────────────────────────────────────────────────────┤
│ Ground Truth: Q × K × sizeof(u64)                            │
└─────────────────────────────────────────────────────────────┘
```

**Limitations**:
- Hard-coded for vectors only
- Fixed header structure in binary file
- No support for multiple fields per record
- No text, tags, or arbitrary data support

## Proposed Architecture

### High-Level Design

```
┌──────────────────┐     ┌──────────────────┐
│  Schema File     │     │  Data File(s)    │
│  (YAML)          │     │  (Binary)        │
└────────┬─────────┘     └────────┬─────────┘
         │                        │
         ▼                        ▼
┌─────────────────────────────────────────────┐
│              SchemaLoader                    │
│  - Parses YAML schema                        │
│  - Validates structure                       │
│  - Computes field offsets                    │
└─────────────────────┬───────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────┐
│              DatasetContext                  │
│  - Memory-maps data file(s)                  │
│  - Provides zero-copy field access           │
│  - Implements DataSource trait               │
└─────────────────────────────────────────────┘
```

### Schema File Format (YAML)

```yaml
# Schema version for future compatibility
version: 1

# Dataset metadata
metadata:
  name: "my-dataset"
  description: "Full-text search benchmark dataset"

# Record structure - each record has these fields
record:
  # Total byte size of each record (computed if not specified)
  # size: 1024  # optional, computed from fields

  # Fields in order of appearance in binary file
  fields:
    - name: embedding
      type: vector
      dtype: float32      # float32, float16, uint8, binary
      dimensions: 768
      # Computed: offset=0, size=768*4=3072 bytes

    - name: content
      type: text
      encoding: utf8      # utf8, ascii
      length: fixed       # fixed or variable
      max_bytes: 512      # for fixed: exact size; for variable: max size
      # For fixed: offset=3072, size=512 bytes
      # For variable: offset=3072, size=4 (u32 length) + max_bytes

    - name: category
      type: tag
      encoding: utf8
      max_bytes: 64
      # offset=3584, size=64 bytes

    - name: score
      type: numeric
      dtype: float64      # int32, int64, float32, float64
      # offset=3648, size=8 bytes

    - name: payload
      type: blob
      length: fixed
      max_bytes: 256
      # offset=3656, size=256 bytes

# Data sections in the binary file
sections:
  # Primary data records
  records:
    count: 1000000        # Number of records
    # offset: 0           # Optional, default=0

  # Optional: keys for records (if not present, keys are generated)
  keys:
    present: true         # or false to generate keys
    encoding: utf8
    length: variable      # or fixed
    max_bytes: 128
    # If present, stored after records section

  # Optional: query records (for search benchmarks)
  queries:
    present: true
    count: 10000
    # Uses same field structure as records
    # Fields to use for queries (subset of record fields)
    query_fields:
      - embedding         # Only these fields present in query records

  # Optional: ground truth for recall verification
  ground_truth:
    present: true
    neighbors_per_query: 100
    id_type: u64          # u32 or u64

# Optional: field-specific metadata for workload generation
field_metadata:
  embedding:
    distance_metric: cosine  # l2, cosine, ip (inner product)
  content:
    index_type: fulltext     # fulltext, prefix, suffix
  category:
    index_type: tag
```

### Binary File Format

The binary file structure is determined entirely by the schema:

```
┌─────────────────────────────────────────────────────────────┐
│ Section: Records                                             │
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ Record 0                                                 │ │
│ │   Field: embedding (3072 bytes)                          │ │
│ │   Field: content (512 bytes, fixed)                      │ │
│ │   Field: category (64 bytes)                             │ │
│ │   Field: score (8 bytes)                                 │ │
│ │   Field: payload (256 bytes)                             │ │
│ └─────────────────────────────────────────────────────────┘ │
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ Record 1 ...                                             │ │
│ └─────────────────────────────────────────────────────────┘ │
│ ... N records total                                          │
├─────────────────────────────────────────────────────────────┤
│ Section: Keys (optional)                                     │
│   Key 0: [4-byte len][key bytes...]                          │
│   Key 1: ...                                                 │
│   ... N keys total                                           │
├─────────────────────────────────────────────────────────────┤
│ Section: Queries (optional)                                  │
│   Query records (only query_fields, not full record)         │
│   ... Q query records                                        │
├─────────────────────────────────────────────────────────────┤
│ Section: Ground Truth (optional)                             │
│   Q × K neighbor IDs                                         │
└─────────────────────────────────────────────────────────────┘
```

### Variable-Length Fields

For variable-length fields (text, tags, blobs with `length: variable`):

```
┌──────────────┬─────────────────────────────────────┐
│ Length (u32) │ Data (up to max_bytes)              │
│ 4 bytes      │ length bytes + padding to max_bytes │
└──────────────┴─────────────────────────────────────┘
```

This maintains fixed record sizes for O(1) access while supporting variable content.

## Rust Implementation

### Core Types

```rust
// src/dataset/schema.rs

/// Field data types
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Vector,
    Text,
    Tag,
    Numeric,
    Blob,
}

/// Numeric/vector element types
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DType {
    Int32,
    Int64,
    Float16,
    Float32,
    Float64,
    Uint8,
    Binary,  // For binary vectors (bit-packed)
}

/// Text/tag encoding
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    Utf8,
    Ascii,
}

/// Length specification
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthSpec {
    Fixed,
    Variable,
}

/// Field definition from schema
#[derive(Debug, Clone, Deserialize)]
pub struct FieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,

    // Vector-specific
    pub dtype: Option<DType>,
    pub dimensions: Option<u32>,

    // Text/Tag/Blob-specific
    pub encoding: Option<Encoding>,
    pub length: Option<LengthSpec>,
    pub max_bytes: Option<u32>,
}

/// Computed field layout
#[derive(Debug, Clone)]
pub struct FieldLayout {
    pub def: FieldDef,
    pub offset: usize,      // Offset within record
    pub size: usize,        // Total size in bytes (including length prefix for variable)
    pub data_offset: usize, // Offset to actual data (after length prefix if variable)
}

/// Record layout computed from schema
#[derive(Debug, Clone)]
pub struct RecordLayout {
    pub fields: Vec<FieldLayout>,
    pub field_by_name: HashMap<String, usize>,
    pub total_size: usize,
}

/// Section configuration
#[derive(Debug, Clone, Deserialize)]
pub struct SectionConfig {
    pub count: Option<u64>,
    pub present: Option<bool>,
    pub encoding: Option<Encoding>,
    pub length: Option<LengthSpec>,
    pub max_bytes: Option<u32>,
    pub query_fields: Option<Vec<String>>,
    pub neighbors_per_query: Option<u32>,
    pub id_type: Option<String>,
}

/// Full schema definition
#[derive(Debug, Clone, Deserialize)]
pub struct DatasetSchema {
    pub version: u32,
    pub metadata: Option<SchemaMetadata>,
    pub record: RecordDef,
    pub sections: SectionsDef,
    pub field_metadata: Option<HashMap<String, FieldMetadata>>,
}

/// Computed section offsets
#[derive(Debug, Clone)]
pub struct SectionLayout {
    pub records_offset: usize,
    pub records_size: usize,
    pub keys_offset: Option<usize>,
    pub keys_size: Option<usize>,
    pub queries_offset: Option<usize>,
    pub queries_size: Option<usize>,
    pub ground_truth_offset: Option<usize>,
    pub ground_truth_size: Option<usize>,
    pub total_size: usize,
}
```

### DatasetContext (Unified)

```rust
// src/dataset/context.rs

pub struct DatasetContext {
    // Memory-mapped data
    mmap: Mmap,

    // Schema and layouts
    schema: DatasetSchema,
    record_layout: RecordLayout,
    query_layout: Option<RecordLayout>,  // May be subset of record fields
    section_layout: SectionLayout,

    // Cached counts
    num_records: u64,
    num_queries: u64,
    num_neighbors: usize,

    // Key configuration
    key_config: KeyConfig,
}

pub enum KeyConfig {
    /// Keys are stored in the data file
    FromFile {
        offset: usize,
        encoding: Encoding,
        length_spec: LengthSpec,
        max_bytes: usize,
    },
    /// Keys are generated with a pattern
    Generated {
        pattern: String,  // e.g., "vec:%012d"
    },
}

impl DatasetContext {
    /// Load from schema file and data file
    pub fn load(schema_path: &Path, data_path: &Path) -> Result<Self> {
        let schema = Self::load_schema(schema_path)?;
        let record_layout = Self::compute_record_layout(&schema.record)?;
        let section_layout = Self::compute_section_layout(&schema, &record_layout)?;

        let file = File::open(data_path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        // Validate file size matches expected
        if mmap.len() < section_layout.total_size {
            return Err(Error::InvalidDataFile("File too small for schema"));
        }

        Ok(Self { /* ... */ })
    }

    /// Get raw bytes for a field from a record
    pub fn get_field_bytes(&self, record_idx: u64, field_name: &str) -> &[u8] {
        let field_idx = self.record_layout.field_by_name[field_name];
        let field = &self.record_layout.fields[field_idx];

        let record_offset = self.section_layout.records_offset
            + (record_idx as usize * self.record_layout.total_size);
        let field_offset = record_offset + field.offset;

        match field.def.length {
            Some(LengthSpec::Variable) => {
                // Read length prefix, return actual data
                let len_bytes = &self.mmap[field_offset..field_offset + 4];
                let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
                &self.mmap[field_offset + 4..field_offset + 4 + len]
            }
            _ => {
                // Fixed length, return full field
                &self.mmap[field_offset + field.data_offset..field_offset + field.size]
            }
        }
    }

    /// Get all fields for a record as a map
    pub fn get_record_fields(&self, record_idx: u64) -> HashMap<&str, &[u8]> {
        self.record_layout.fields.iter()
            .map(|f| (f.def.name.as_str(), self.get_field_bytes(record_idx, &f.def.name)))
            .collect()
    }

    /// Get key for a record
    pub fn get_key(&self, record_idx: u64) -> Cow<str> {
        match &self.key_config {
            KeyConfig::FromFile { offset, encoding, length_spec, max_bytes } => {
                // Read key from mmap
                let key_offset = self.compute_key_offset(record_idx);
                let key_bytes = self.read_key_bytes(key_offset);
                Cow::Borrowed(std::str::from_utf8(key_bytes).unwrap())
            }
            KeyConfig::Generated { pattern } => {
                // Generate key from pattern
                Cow::Owned(self.generate_key(pattern, record_idx))
            }
        }
    }

    /// Get query field bytes (for search operations)
    pub fn get_query_field_bytes(&self, query_idx: u64, field_name: &str) -> &[u8] {
        let query_layout = self.query_layout.as_ref()
            .expect("Schema has no query section");
        // Similar to get_field_bytes but uses query_layout and queries_offset
        // ...
    }

    /// Compute recall for vector queries
    pub fn compute_recall(&self, query_idx: u64, result_ids: &[u64], k: usize) -> f64 {
        if self.section_layout.ground_truth_offset.is_none() {
            return 0.0;  // No ground truth available
        }
        // Read ground truth and compute intersection
        // ...
    }
}
```

### Trait Implementation

```rust
// The unified DatasetContext implements existing traits

impl DataSource for DatasetContext {
    fn num_items(&self) -> u64 {
        self.num_records
    }

    fn get_item_bytes(&self, idx: u64) -> &[u8] {
        // Return first field's bytes (backward compat for simple cases)
        // Or: return entire record bytes
        let offset = self.section_layout.records_offset
            + (idx as usize * self.record_layout.total_size);
        &self.mmap[offset..offset + self.record_layout.total_size]
    }

    fn item_byte_len(&self) -> usize {
        self.record_layout.total_size
    }
}

impl VectorDataSource for DatasetContext {
    fn num_queries(&self) -> u64 {
        self.num_queries
    }

    fn get_query_bytes(&self, idx: u64) -> &[u8] {
        // Get the vector field from query record
        self.get_query_field_bytes(idx, "embedding")  // Or first vector field
    }

    fn compute_recall(&self, query_idx: u64, result_ids: &[u64], k: usize) -> f64 {
        self.compute_recall(query_idx, result_ids, k)
    }

    fn get_ground_truth_vector_ids(&self) -> HashSet<u64> {
        // ... implementation
    }
}
```

### Extended Traits for Multi-Field Access

```rust
// src/dataset/source.rs - extended traits

/// Multi-field data source
pub trait FieldDataSource: DataSource {
    /// Get list of available fields
    fn field_names(&self) -> &[String];

    /// Get a specific field's bytes for a record
    fn get_field(&self, record_idx: u64, field_name: &str) -> &[u8];

    /// Get key for a record (generated or from file)
    fn get_key(&self, record_idx: u64) -> Cow<str>;

    /// Get all fields for a record
    fn get_record(&self, record_idx: u64) -> HashMap<&str, &[u8]>;
}

/// Query data source (extends FieldDataSource for search operations)
pub trait QueryDataSource: FieldDataSource {
    /// Number of query records
    fn num_queries(&self) -> u64;

    /// Get query field bytes
    fn get_query_field(&self, query_idx: u64, field_name: &str) -> &[u8];

    /// Fields available for queries
    fn query_field_names(&self) -> &[String];
}

/// Ground truth support for recall/accuracy metrics
pub trait GroundTruthSource {
    /// Check if ground truth is available
    fn has_ground_truth(&self) -> bool;

    /// Get ground truth neighbor IDs for a query
    fn get_ground_truth(&self, query_idx: u64) -> &[u64];

    /// Compute recall given result IDs
    fn compute_recall(&self, query_idx: u64, result_ids: &[u64], k: usize) -> f64;
}
```

## Usage Examples

### Vector-Only Dataset (Current Use Case)

Schema file (`mnist.yaml`):
```yaml
version: 1
metadata:
  name: "mnist"
  description: "MNIST handwritten digits, 784-dim vectors"

record:
  fields:
    - name: embedding
      type: vector
      dtype: float32
      dimensions: 784

sections:
  records:
    count: 60000
  keys:
    present: false  # Generate keys
  queries:
    present: true
    count: 10000
    query_fields: [embedding]
  ground_truth:
    present: true
    neighbors_per_query: 100
    id_type: u64

field_metadata:
  embedding:
    distance_metric: l2
```

### Full-Text Search Dataset

Schema file (`wiki-articles.yaml`):
```yaml
version: 1
metadata:
  name: "wikipedia-articles"
  description: "Wikipedia articles for full-text search benchmarking"

record:
  fields:
    - name: title
      type: text
      encoding: utf8
      length: variable
      max_bytes: 256

    - name: content
      type: text
      encoding: utf8
      length: variable
      max_bytes: 65536  # 64KB max per article

    - name: categories
      type: tag
      encoding: utf8
      max_bytes: 512    # Comma-separated tags

sections:
  records:
    count: 1000000
  keys:
    present: true       # Article IDs from file
    encoding: utf8
    length: variable
    max_bytes: 64
  queries:
    present: true
    count: 10000
    query_fields: [content]  # Query text for FT.SEARCH
  ground_truth:
    present: true
    neighbors_per_query: 100
    id_type: u64

field_metadata:
  content:
    index_type: fulltext
  categories:
    index_type: tag
```

### Hybrid Vector + Metadata

Schema file (`products.yaml`):
```yaml
version: 1
metadata:
  name: "product-catalog"

record:
  fields:
    - name: embedding
      type: vector
      dtype: float32
      dimensions: 768

    - name: description
      type: text
      encoding: utf8
      length: variable
      max_bytes: 4096

    - name: category
      type: tag
      encoding: utf8
      max_bytes: 64

    - name: price
      type: numeric
      dtype: float64

    - name: rating
      type: numeric
      dtype: float32

sections:
  records:
    count: 5000000
  keys:
    present: false  # Generate product IDs
  queries:
    present: true
    count: 10000
    query_fields: [embedding]  # Vector search
  ground_truth:
    present: true
    neighbors_per_query: 100

field_metadata:
  embedding:
    distance_metric: cosine
  description:
    index_type: fulltext
  category:
    index_type: tag
```

## Command Line Integration

```bash
# Load with schema-driven dataset
./target/release/valkey-bench-rs \
  -h cluster.example.com --cluster \
  -t hset-load \
  --schema datasets/products.yaml \
  --data datasets/products.bin \
  --prefix "product:" \
  -n 5000000 -c 100

# Vector query with multi-field dataset
./target/release/valkey-bench-rs \
  -h cluster.example.com --cluster \
  -t vec-query \
  --schema datasets/products.yaml \
  --data datasets/products.bin \
  --index-name products_idx \
  --query-field embedding \
  -k 10 -n 10000

# Full-text search
./target/release/valkey-bench-rs \
  -h cluster.example.com --cluster \
  -t ft-search \
  --schema datasets/wiki-articles.yaml \
  --data datasets/wiki-articles.bin \
  --index-name wiki_idx \
  --query-field content \
  -n 10000
```

## Migration Path

### Phase 1: Schema Infrastructure
1. Implement schema parsing (`schema.rs`)
2. Implement layout computation
3. Create new `DatasetContext` with schema support

### Phase 2: Unified Loading
1. Replace current `binary_dataset.rs` with schema-driven version
2. Update workload types to use `FieldDataSource` trait
3. Update CLI to accept `--schema` and `--data` parameters

### Phase 3: New Workload Types
1. Add `hset-load` workload for multi-field HSET
2. Add `ft-search` workload for full-text queries
3. Add `ft-aggregate` workload for aggregation queries

### Phase 4: Dataset Tools
1. Create `valkey-dataset-gen` tool for creating datasets
2. Support conversion from common formats (JSON, CSV, Parquet)
3. Add validation and inspection commands

## File Organization

```
src/dataset/
├── mod.rs           # Module exports
├── schema.rs        # Schema parsing and validation
├── layout.rs        # Field and section layout computation
├── context.rs       # Unified DatasetContext
├── source.rs        # DataSource, FieldDataSource, QueryDataSource traits
├── key.rs           # Key generation and formatting
└── error.rs         # Dataset-specific errors
```

## Workload Integration

### Placeholder Types for Schema Fields

The command template system uses `PlaceholderType` to mark positions for runtime fill. For schema-driven datasets, we extend this:

```rust
// src/client/mod.rs - extended PlaceholderType

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderType {
    // Existing types
    Key,
    Vector,
    QueryVector,
    ClusterTag,
    RandInt,
    Tag,
    Numeric,
    NumericField(usize),
    Field,
    JsonPath,

    // New: Schema-driven field types
    /// Field from schema by name (resolved at template build time to index)
    SchemaField(usize),     // Index into record_layout.fields
    /// Query field from schema (for search operations)
    SchemaQueryField(usize),
    /// Key from dataset (when keys are in the data file)
    DatasetKey,
}
```

### Template Factory Extension

```rust
// src/workload/template_factory.rs - schema-driven template creation

/// Create HSET template from schema definition
///
/// Generates: HSET key field1 value1 field2 value2 ...
pub fn create_schema_hset_template(
    schema: &DatasetSchema,
    layout: &RecordLayout,
    key_prefix: &str,
    key_width: usize,
    cluster_mode: bool,
) -> CommandTemplate {
    let mut template = CommandTemplate::new("HSET")
        .arg_str("HSET");

    // Add key (generated or from dataset)
    if schema.sections.keys.as_ref().map_or(true, |k| !k.present.unwrap_or(false)) {
        // Generate keys
        template = if cluster_mode {
            template.arg_prefixed_key_with_cluster_tag(key_prefix, key_width)
        } else {
            template.arg_prefixed_key(key_prefix, key_width)
        };
    } else {
        // Keys from dataset file
        template = template.arg_dataset_key(schema.sections.keys.as_ref().unwrap().max_bytes.unwrap_or(128));
    }

    // Add all fields from schema
    for (idx, field) in layout.fields.iter().enumerate() {
        template = template
            .arg_str(&field.def.name)
            .arg_schema_field(idx, field.size);
    }

    template
}

/// Create FT.SEARCH template from schema definition
pub fn create_schema_search_template(
    schema: &DatasetSchema,
    layout: &RecordLayout,
    query_field: &str,
    index_name: &str,
    k: usize,
) -> CommandTemplate {
    let query_field_idx = layout.field_by_name.get(query_field)
        .expect("Query field not found in schema");

    let query = format!("*=>[KNN {} @{} $BLOB]", k, query_field);

    CommandTemplate::new("FT.SEARCH")
        .arg_str("FT.SEARCH")
        .arg_str(index_name)
        .arg_str(&query)
        .arg_str("PARAMS")
        .arg_str("2")
        .arg_str("BLOB")
        .arg_schema_query_field(*query_field_idx, layout.fields[*query_field_idx].size)
        .arg_str("DIALECT")
        .arg_str("2")
}
```

### WorkloadContext Extension

```rust
// src/workload/context.rs - extended for schema-driven access

impl WorkloadContext {
    /// Get field bytes for a record from schema-driven dataset
    pub fn get_schema_field_bytes(&self, record_idx: u64, field_idx: usize) -> Option<&[u8]> {
        self.dataset.as_ref()
            .and_then(|d| d.get_field_by_index(record_idx, field_idx))
    }

    /// Get query field bytes from schema-driven dataset
    pub fn get_schema_query_field_bytes(&self, query_idx: u64, field_idx: usize) -> Option<&[u8]> {
        self.dataset.as_ref()
            .and_then(|d| d.get_query_field_by_index(query_idx, field_idx))
    }

    /// Get key from dataset (when keys are stored in data file)
    pub fn get_dataset_key(&self, record_idx: u64) -> Option<Cow<str>> {
        self.dataset.as_ref()
            .and_then(|d| d.get_key(record_idx))
    }

    /// Fill schema field placeholder
    pub fn fill_schema_field(&self, record_idx: u64, field_idx: usize, buf: &mut [u8]) {
        if let Some(bytes) = self.get_schema_field_bytes(record_idx, field_idx) {
            // For variable-length fields, we might need padding
            let copy_len = bytes.len().min(buf.len());
            buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
            // Pad remaining with appropriate padding (spaces for text, zeros for binary)
            if copy_len < buf.len() {
                buf[copy_len..].fill(b' '); // Or context-appropriate padding
            }
        }
    }
}
```

### Event Worker Placeholder Handling

```rust
// src/benchmark/event_worker.rs - extended placeholder filling

match ph.placeholder_type {
    // ... existing handlers ...

    PlaceholderType::SchemaField(field_idx) => {
        // Fill from schema-driven dataset
        let buf = &mut self.clients[client_idx].write_buf[offset..offset + ph.len];
        self.workload_ctx.fill_schema_field(key_num, field_idx, buf);
    }

    PlaceholderType::SchemaQueryField(field_idx) => {
        let num_queries = self.workload_ctx.num_queries();
        if num_queries > 0 {
            let idx = self.rng.u64(0..num_queries);
            if let Some(bytes) = self.workload_ctx.get_schema_query_field_bytes(idx, field_idx) {
                self.clients[client_idx].write_buf[offset..offset + bytes.len()]
                    .copy_from_slice(bytes);
                self.clients[client_idx].query_indices.push_back(idx);
            }
        }
    }

    PlaceholderType::DatasetKey => {
        if let Some(key) = self.workload_ctx.get_dataset_key(key_num) {
            let key_bytes = key.as_bytes();
            let copy_len = key_bytes.len().min(ph.len);
            self.clients[client_idx].write_buf[offset..offset + copy_len]
                .copy_from_slice(&key_bytes[..copy_len]);
            // Pad with spaces if key is shorter
            if copy_len < ph.len {
                self.clients[client_idx].write_buf[offset + copy_len..offset + ph.len].fill(b' ');
            }
        }
    }
}
```

### New Workload Types

```rust
// src/workload/workload_type.rs - extended

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadType {
    // Existing types
    Ping, Set, Get, Incr,
    Lpush, Rpush, Lpop, Rpop,
    Lrange100, Lrange300, Lrange500, Lrange600,
    Sadd, Spop, Hset, Zadd, Zpopmin, Mset,
    VecLoad, VecQuery, VecDel, VecUpdate,
    Custom,

    // New: Schema-driven workloads
    /// Load records from schema-driven dataset
    SchemaLoad,
    /// Full-text search using schema query field
    FtSearch,
    /// Full-text aggregation query
    FtAggregate,
}

impl FromStr for WorkloadType {
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            // ... existing patterns ...
            "schema-load" | "schemaload" => Ok(WorkloadType::SchemaLoad),
            "ft-search" | "ftsearch" => Ok(WorkloadType::FtSearch),
            "ft-aggregate" | "ftaggregate" => Ok(WorkloadType::FtAggregate),
            _ => Err(format!("Unknown workload type: {}", s)),
        }
    }
}
```

## CLI Integration

### New Command Line Options

```rust
// src/config/cli.rs - new options

/// Dataset configuration for schema-driven workloads
#[derive(Debug, Clone)]
pub struct SchemaDatasetConfig {
    /// Path to YAML schema file
    pub schema_path: PathBuf,
    /// Path to binary data file
    pub data_path: PathBuf,
    /// Query field for search workloads (defaults to first vector field)
    pub query_field: Option<String>,
}

// CLI argument additions:
// --schema <path>      YAML schema file
// --data <path>        Binary data file
// --query-field <name> Field to use for queries
```

## Design Decisions

### Why YAML Schema (Not Embedded in Binary)?

1. **Human Readable**: Easy to inspect and modify
2. **Version Control**: Schema changes are trackable
3. **Tooling**: Standard YAML tools for validation
4. **Flexibility**: Same binary data, different interpretations
5. **Separation of Concerns**: Data vs. metadata clearly separated

### Why Fixed-Size Records with Variable-Length Fields?

1. **O(1) Access**: `record_offset = base + (idx * record_size)`
2. **Memory Mapping**: No need to scan for record boundaries
3. **Predictable Memory**: Known buffer sizes
4. **Trade-off**: Some wasted space for padding, acceptable for benchmarking

### Why Single DatasetContext?

1. **No Duplication**: One implementation to maintain
2. **Consistent Interface**: Same API for all data types
3. **Composable**: Traits allow workloads to use what they need
4. **Testable**: One implementation to test thoroughly

## Dataset Generation Tool

A companion tool for creating binary datasets from source files:

### Usage

```bash
# Convert JSON lines to binary dataset
valkey-dataset-gen \
  --schema products.yaml \
  --input products.jsonl \
  --output products.bin

# Convert CSV with header
valkey-dataset-gen \
  --schema wiki-articles.yaml \
  --input articles.csv \
  --format csv \
  --output wiki-articles.bin

# Combine multiple sources
valkey-dataset-gen \
  --schema hybrid.yaml \
  --vectors embeddings.fvecs \     # Vector field
  --text content.txt \              # Text field (one per line)
  --keys keys.txt \                 # Keys (one per line)
  --ground-truth gt.ivecs \         # Ground truth
  --output hybrid.bin

# Verify dataset
valkey-dataset-gen verify \
  --schema products.yaml \
  --data products.bin
```

### Implementation

```rust
// tools/dataset_gen/main.rs

use clap::{Parser, Subcommand};
use valkey_bench_rs::dataset::{DatasetSchema, RecordLayout, SectionLayout};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate binary dataset from source files
    Generate {
        #[arg(long)]
        schema: PathBuf,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        format: Option<String>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify dataset against schema
    Verify {
        #[arg(long)]
        schema: PathBuf,
        #[arg(long)]
        data: PathBuf,
    },
    /// Inspect dataset contents
    Inspect {
        #[arg(long)]
        schema: PathBuf,
        #[arg(long)]
        data: PathBuf,
        #[arg(long, default_value = "10")]
        limit: usize,
    },
}

fn generate(schema_path: &Path, input: &Path, format: &str, output: &Path) -> Result<()> {
    let schema = DatasetSchema::load(schema_path)?;
    let layout = RecordLayout::from_schema(&schema.record)?;
    let section_layout = SectionLayout::compute(&schema, &layout)?;

    let mut writer = BinaryDatasetWriter::new(output, &layout, &section_layout)?;

    match format {
        "jsonl" => {
            for line in BufReader::new(File::open(input)?).lines() {
                let record: serde_json::Value = serde_json::from_str(&line?)?;
                writer.write_record(&record, &layout)?;
            }
        }
        "csv" => {
            let mut rdr = csv::Reader::from_path(input)?;
            for result in rdr.deserialize() {
                let record: HashMap<String, String> = result?;
                writer.write_record_map(&record, &layout)?;
            }
        }
        _ => return Err(Error::UnsupportedFormat(format.to_string())),
    }

    writer.finalize()?;
    Ok(())
}
```

### Python Script Alternative

```python
#!/usr/bin/env python3
"""dataset_gen.py - Generate binary datasets from source files"""

import yaml
import struct
import numpy as np
from pathlib import Path

def compute_field_size(field: dict) -> int:
    """Compute byte size for a field"""
    field_type = field['type']

    if field_type == 'vector':
        dim = field['dimensions']
        dtype = field.get('dtype', 'float32')
        dtype_size = {'float32': 4, 'float16': 2, 'uint8': 1}[dtype]
        return dim * dtype_size

    elif field_type in ('text', 'tag', 'blob'):
        length = field.get('length', 'fixed')
        max_bytes = field['max_bytes']
        if length == 'variable':
            return 4 + max_bytes  # u32 length prefix + data
        return max_bytes

    elif field_type == 'numeric':
        dtype = field.get('dtype', 'float64')
        return {'int32': 4, 'int64': 8, 'float32': 4, 'float64': 8}[dtype]

    raise ValueError(f"Unknown field type: {field_type}")

def write_field(f, field: dict, value, offset: int) -> int:
    """Write a field value to file at offset"""
    field_type = field['type']
    size = compute_field_size(field)

    if field_type == 'vector':
        arr = np.array(value, dtype=field.get('dtype', 'float32'))
        f.seek(offset)
        f.write(arr.tobytes())

    elif field_type in ('text', 'tag'):
        data = value.encode('utf-8') if isinstance(value, str) else value
        length = field.get('length', 'fixed')
        max_bytes = field['max_bytes']

        f.seek(offset)
        if length == 'variable':
            f.write(struct.pack('<I', len(data)))
            f.write(data[:max_bytes])
            # Pad to max_bytes
            if len(data) < max_bytes:
                f.write(b'\x00' * (max_bytes - len(data)))
        else:
            f.write(data[:max_bytes].ljust(max_bytes, b'\x00'))

    elif field_type == 'numeric':
        dtype = field.get('dtype', 'float64')
        fmt = {'int32': '<i', 'int64': '<q', 'float32': '<f', 'float64': '<d'}[dtype]
        f.seek(offset)
        f.write(struct.pack(fmt, value))

    return size

def generate_dataset(schema_path: Path, input_path: Path, output_path: Path):
    """Generate binary dataset from schema and input"""
    with open(schema_path) as f:
        schema = yaml.safe_load(f)

    # Compute layout
    fields = schema['record']['fields']
    field_offsets = []
    offset = 0
    for field in fields:
        field_offsets.append(offset)
        offset += compute_field_size(field)
    record_size = offset

    # Read input and write binary
    import json
    records = [json.loads(line) for line in open(input_path)]
    num_records = len(records)

    with open(output_path, 'wb') as f:
        for rec_idx, record in enumerate(records):
            rec_offset = rec_idx * record_size
            for field, field_offset in zip(fields, field_offsets):
                value = record.get(field['name'])
                if value is not None:
                    write_field(f, field, value, rec_offset + field_offset)

if __name__ == '__main__':
    import sys
    generate_dataset(Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]))
```

## Summary

This design provides a unified, schema-driven dataset system with **ONE mechanism** for all data ingestion:

1. **Single Implementation**: One `DatasetContext` serves vectors, text, tags, numeric, blobs - no special cases
2. **Schema + Data**: Every dataset is a pair: `schema.yaml` + `data.bin`
3. **Zero-Copy mmap**: O(1) random access via computed offsets
4. **No Legacy Code Paths**: Existing vector datasets must provide a schema file (easy to create)
5. **Extensible**: New field types add to schema spec, not new code paths

### The One Mechanism

```
                     ┌─────────────────┐
                     │  schema.yaml    │  Describes structure
                     └────────┬────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────┐
│                  DatasetContext                      │  ONE implementation
│  - load(schema_path, data_path)                      │
│  - get_field(record_idx, field_name) -> &[u8]        │
│  - get_key(record_idx) -> Cow<str>                   │
│  - get_query_field(query_idx, field_name) -> &[u8]   │
│  - compute_recall(query_idx, results, k) -> f64      │
└─────────────────────────────────────────────────────┘
                              │
                              ▼
                     ┌─────────────────┐
                     │    data.bin     │  Flat binary, mmap'd
                     └─────────────────┘
```

### What Gets Deleted

- `DatasetHeader` struct (embedded header format)
- `DataType`, `DistanceMetricId` enums (replaced by schema)
- Legacy offset computation in `binary_dataset.rs`
- Hardcoded vector-only assumptions

### What Stays (Unified)

- mmap-based zero-copy access pattern
- `DataSource` trait (generalized)
- Placeholder-based template filling
- Ground truth and recall computation

### Implementation Priority

1. **Phase 1**: Schema infrastructure (`schema.rs`, `layout.rs`) - parse YAML, compute offsets
2. **Phase 2**: Unified `DatasetContext` - replace current implementation entirely
3. **Phase 3**: Template factory - `create_schema_hset_template()`, `create_schema_search_template()`
4. **Phase 4**: Workload types - `SchemaLoad`, `FtSearch`, `FtAggregate`
5. **Phase 5**: Dataset tooling - `valkey-dataset-gen` for creating binary files
