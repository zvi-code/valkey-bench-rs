# Schema-Driven Dataset Implementation Design

This document describes the implementation plan for converting the current hardcoded binary format to a unified schema-driven system, plus a Python command recorder for capturing Redis/Valkey commands.

## Part 1: System Modifications

### Current State

**Python Converter** (`prep_datasets/prepare_binary.py`):
- Converts HDF5 (vectors, queries, ground_truth) to hardcoded binary format
- Embeds header with magic, version, offsets in the binary file
- Vector-only: no support for text, tags, or other data types

**Rust Consumer** (`src/dataset/binary_dataset.rs`):
- Reads hardcoded 4KB header from binary file
- Parses `DatasetHeader` struct with fixed field positions
- Provides mmap access to vectors, queries, ground_truth sections

### Target State

**Schema + Data Separation**:
```
dataset.yaml  +  dataset.bin
    |                |
    v                v
[Structure]     [Raw Data]
```

**No embedded header** in binary file - all metadata in YAML schema.

---

## Part 1A: Modified Python Converter

### New: `prep_datasets/prepare_schema_binary.py`

```python
#!/usr/bin/env python3
"""
Schema-driven binary dataset generator.

Converts HDF5/Parquet/JSON sources to schema YAML + binary data file.
Replaces prepare_binary.py with unified approach.
"""

import numpy as np
import h5py
import yaml
import struct
from pathlib import Path
from dataclasses import dataclass, field
from typing import List, Dict, Any, Optional
import argparse


@dataclass
class FieldDef:
    """Field definition for schema generation."""
    name: str
    field_type: str  # vector, text, tag, numeric, blob
    dtype: Optional[str] = None  # float32, float16, int32, etc.
    dimensions: Optional[int] = None  # For vectors
    encoding: Optional[str] = None  # utf8, ascii
    length: Optional[str] = None  # fixed, variable
    max_bytes: Optional[int] = None  # For text/tag/blob

    def to_yaml_dict(self) -> Dict[str, Any]:
        """Convert to YAML-compatible dict."""
        d = {'name': self.name, 'type': self.field_type}
        if self.dtype:
            d['dtype'] = self.dtype
        if self.dimensions:
            d['dimensions'] = self.dimensions
        if self.encoding:
            d['encoding'] = self.encoding
        if self.length:
            d['length'] = self.length
        if self.max_bytes:
            d['max_bytes'] = self.max_bytes
        return d

    def byte_size(self) -> int:
        """Compute byte size for this field."""
        if self.field_type == 'vector':
            dtype_sizes = {'float32': 4, 'float16': 2, 'uint8': 1, 'int8': 1}
            return self.dimensions * dtype_sizes.get(self.dtype, 4)
        elif self.field_type in ('text', 'tag', 'blob'):
            if self.length == 'variable':
                return 4 + self.max_bytes  # u32 length prefix + data
            return self.max_bytes
        elif self.field_type == 'numeric':
            dtype_sizes = {'int32': 4, 'int64': 8, 'float32': 4, 'float64': 8, 'u32': 4, 'u64': 8}
            return dtype_sizes.get(self.dtype, 8)
        raise ValueError(f"Unknown field type: {self.field_type}")


@dataclass
class SchemaBuilder:
    """Builder for generating schema YAML and binary data."""
    name: str
    description: str = ""
    fields: List[FieldDef] = field(default_factory=list)

    # Section configurations
    record_count: int = 0
    keys_present: bool = False
    keys_pattern: Optional[str] = None
    keys_max_bytes: int = 64

    queries_present: bool = False
    query_count: int = 0
    query_fields: List[str] = field(default_factory=list)

    ground_truth_present: bool = False
    neighbors_per_query: int = 100
    gt_id_type: str = 'u64'

    # Field metadata
    field_metadata: Dict[str, Dict[str, Any]] = field(default_factory=dict)

    def add_vector_field(self, name: str, dimensions: int,
                         dtype: str = 'float32',
                         distance_metric: str = 'l2') -> 'SchemaBuilder':
        """Add a vector field."""
        self.fields.append(FieldDef(
            name=name,
            field_type='vector',
            dtype=dtype,
            dimensions=dimensions
        ))
        self.field_metadata[name] = {'distance_metric': distance_metric}
        return self

    def add_text_field(self, name: str, max_bytes: int,
                       length: str = 'fixed',
                       index_type: Optional[str] = None) -> 'SchemaBuilder':
        """Add a text field."""
        self.fields.append(FieldDef(
            name=name,
            field_type='text',
            encoding='utf8',
            length=length,
            max_bytes=max_bytes
        ))
        if index_type:
            self.field_metadata[name] = {'index_type': index_type}
        return self

    def add_tag_field(self, name: str, max_bytes: int) -> 'SchemaBuilder':
        """Add a tag field."""
        self.fields.append(FieldDef(
            name=name,
            field_type='tag',
            encoding='utf8',
            max_bytes=max_bytes
        ))
        self.field_metadata[name] = {'index_type': 'tag'}
        return self

    def add_numeric_field(self, name: str, dtype: str = 'float64') -> 'SchemaBuilder':
        """Add a numeric field."""
        self.fields.append(FieldDef(
            name=name,
            field_type='numeric',
            dtype=dtype
        ))
        return self

    def with_keys(self, pattern: Optional[str] = None,
                  max_bytes: int = 64) -> 'SchemaBuilder':
        """Configure key generation or storage."""
        if pattern:
            self.keys_present = False
            self.keys_pattern = pattern
        else:
            self.keys_present = True
        self.keys_max_bytes = max_bytes
        return self

    def with_queries(self, count: int, query_fields: List[str]) -> 'SchemaBuilder':
        """Configure query section."""
        self.queries_present = True
        self.query_count = count
        self.query_fields = query_fields
        return self

    def with_ground_truth(self, neighbors: int, id_type: str = 'u64') -> 'SchemaBuilder':
        """Configure ground truth section."""
        self.ground_truth_present = True
        self.neighbors_per_query = neighbors
        self.gt_id_type = id_type
        return self

    def compute_record_size(self) -> int:
        """Compute total record size in bytes."""
        return sum(f.byte_size() for f in self.fields)

    def compute_query_size(self) -> int:
        """Compute query record size (only query_fields)."""
        return sum(f.byte_size() for f in self.fields if f.name in self.query_fields)

    def generate_schema(self) -> Dict[str, Any]:
        """Generate complete schema as dict."""
        schema = {
            'version': 1,
            'metadata': {
                'name': self.name,
                'description': self.description
            },
            'record': {
                'fields': [f.to_yaml_dict() for f in self.fields]
            },
            'sections': {
                'records': {'count': self.record_count}
            }
        }

        # Keys section
        if self.keys_present:
            schema['sections']['keys'] = {
                'present': True,
                'encoding': 'utf8',
                'length': 'fixed',
                'max_bytes': self.keys_max_bytes
            }
        elif self.keys_pattern:
            schema['sections']['keys'] = {
                'present': False,
                'pattern': self.keys_pattern
            }

        # Queries section
        if self.queries_present:
            schema['sections']['queries'] = {
                'present': True,
                'count': self.query_count,
                'query_fields': self.query_fields
            }

        # Ground truth section
        if self.ground_truth_present:
            schema['sections']['ground_truth'] = {
                'present': True,
                'neighbors_per_query': self.neighbors_per_query,
                'id_type': self.gt_id_type
            }

        # Field metadata
        if self.field_metadata:
            schema['field_metadata'] = self.field_metadata

        return schema

    def write_schema(self, path: Path):
        """Write schema to YAML file."""
        schema = self.generate_schema()
        with open(path, 'w') as f:
            yaml.dump(schema, f, default_flow_style=False, sort_keys=False)
        print(f"Schema written to {path}")


class BinaryDataWriter:
    """Writer for binary data file (no header - schema-driven)."""

    def __init__(self, path: Path, schema_builder: SchemaBuilder):
        self.path = path
        self.schema = schema_builder
        self.file = open(path, 'wb')
        self.records_written = 0

        # Compute section offsets
        self.record_size = schema_builder.compute_record_size()
        self.records_offset = 0

        next_offset = schema_builder.record_count * self.record_size

        if schema_builder.keys_present:
            self.keys_offset = next_offset
            next_offset += schema_builder.record_count * schema_builder.keys_max_bytes
        else:
            self.keys_offset = None

        if schema_builder.queries_present:
            self.queries_offset = next_offset
            query_size = schema_builder.compute_query_size()
            next_offset += schema_builder.query_count * query_size
        else:
            self.queries_offset = None

        if schema_builder.ground_truth_present:
            self.ground_truth_offset = next_offset
            id_size = 8 if schema_builder.gt_id_type == 'u64' else 4
            next_offset += schema_builder.query_count * schema_builder.neighbors_per_query * id_size
        else:
            self.ground_truth_offset = None

        self.total_size = next_offset

    def write_record(self, record_data: Dict[str, Any]):
        """Write a single record."""
        offset = self.records_offset + (self.records_written * self.record_size)
        self.file.seek(offset)

        for field in self.schema.fields:
            value = record_data.get(field.name)
            self._write_field(field, value)

        self.records_written += 1

    def _write_field(self, field: FieldDef, value: Any):
        """Write a single field value."""
        if field.field_type == 'vector':
            arr = np.array(value, dtype=field.dtype)
            self.file.write(arr.tobytes())

        elif field.field_type in ('text', 'tag'):
            data = value.encode('utf-8') if isinstance(value, str) else (value or b'')
            max_bytes = field.max_bytes

            if field.length == 'variable':
                # Write length prefix
                self.file.write(struct.pack('<I', len(data)))
                self.file.write(data[:max_bytes])
                padding = max_bytes - min(len(data), max_bytes)
                self.file.write(b'\x00' * padding)
            else:
                # Fixed length
                self.file.write(data[:max_bytes])
                padding = max_bytes - min(len(data), max_bytes)
                self.file.write(b'\x00' * padding)

        elif field.field_type == 'numeric':
            dtype_fmt = {
                'int32': '<i', 'int64': '<q',
                'float32': '<f', 'float64': '<d',
                'u32': '<I', 'u64': '<Q'
            }
            self.file.write(struct.pack(dtype_fmt[field.dtype], value or 0))

    def write_keys(self, keys: List[str]):
        """Write keys section."""
        if self.keys_offset is None:
            raise ValueError("Keys section not configured in schema")

        self.file.seek(self.keys_offset)
        max_bytes = self.schema.keys_max_bytes

        for key in keys:
            data = key.encode('utf-8')
            self.file.write(data[:max_bytes])
            padding = max_bytes - min(len(data), max_bytes)
            self.file.write(b'\x00' * padding)

    def write_queries(self, queries: List[Dict[str, Any]]):
        """Write queries section."""
        if self.queries_offset is None:
            raise ValueError("Queries section not configured in schema")

        self.file.seek(self.queries_offset)

        for query in queries:
            for field in self.schema.fields:
                if field.name in self.schema.query_fields:
                    value = query.get(field.name)
                    self._write_field(field, value)

    def write_ground_truth(self, ground_truth: np.ndarray):
        """Write ground truth section."""
        if self.ground_truth_offset is None:
            raise ValueError("Ground truth section not configured in schema")

        self.file.seek(self.ground_truth_offset)

        if self.schema.gt_id_type == 'u64':
            gt = ground_truth.astype(np.int64)
        else:
            gt = ground_truth.astype(np.int32)

        gt.tofile(self.file)

    def close(self):
        """Close the file."""
        self.file.close()
        print(f"Binary data written to {self.path} ({self.total_size} bytes)")


def convert_hdf5_to_schema(h5_path: Path, output_base: Path,
                            dataset_name: str,
                            distance_metric: str = 'l2',
                            max_ground_truth: int = 100,
                            key_pattern: str = "vec:%012d"):
    """
    Convert HDF5 vector dataset to schema YAML + binary data.

    Replaces prepare_binary.py functionality.
    """
    print(f"Loading dataset from {h5_path}...")

    with h5py.File(h5_path, 'r') as f:
        # Load vectors
        if 'train' in f:
            vectors = np.array(f['train'], dtype=np.float32)
        elif 'database' in f:
            vectors = np.array(f['database'], dtype=np.float32)
        else:
            raise ValueError("No 'train' or 'database' key found")

        # Load queries
        if 'test' in f:
            queries = np.array(f['test'], dtype=np.float32)
        elif 'queries' in f:
            queries = np.array(f['queries'], dtype=np.float32)
        else:
            raise ValueError("No 'test' or 'queries' key found")

        # Load ground truth
        if 'neighbors' in f:
            ground_truth = np.array(f['neighbors'], dtype=np.int64)
        elif 'ground_truth' in f:
            ground_truth = np.array(f['ground_truth'], dtype=np.int64)
        else:
            print("Computing ground truth...")
            ground_truth = compute_ground_truth(vectors, queries, max_ground_truth)

        if ground_truth.shape[1] > max_ground_truth:
            ground_truth = ground_truth[:, :max_ground_truth]

    num_vectors, dim = vectors.shape
    num_queries = len(queries)
    num_neighbors = ground_truth.shape[1]

    print(f"Dataset: {num_vectors} vectors, {num_queries} queries, dim={dim}")

    # Build schema
    builder = SchemaBuilder(name=dataset_name)
    builder.add_vector_field('embedding', dimensions=dim,
                             dtype='float32', distance_metric=distance_metric)
    builder.record_count = num_vectors
    builder.with_keys(pattern=key_pattern)
    builder.with_queries(count=num_queries, query_fields=['embedding'])
    builder.with_ground_truth(neighbors=num_neighbors, id_type='u64')

    # Write schema
    schema_path = output_base.with_suffix('.yaml')
    builder.write_schema(schema_path)

    # Write binary data
    data_path = output_base.with_suffix('.bin')
    writer = BinaryDataWriter(data_path, builder)

    # Write records (vectors)
    print("Writing vectors...")
    for i, vec in enumerate(vectors):
        writer.write_record({'embedding': vec})
        if (i + 1) % 100000 == 0:
            print(f"  {i + 1}/{num_vectors}")

    # Write queries
    print("Writing queries...")
    query_records = [{'embedding': q} for q in queries]
    writer.write_queries(query_records)

    # Write ground truth
    print("Writing ground truth...")
    writer.write_ground_truth(ground_truth)

    writer.close()

    print(f"\nGenerated:")
    print(f"  Schema: {schema_path}")
    print(f"  Data:   {data_path}")


def compute_ground_truth(vectors: np.ndarray, queries: np.ndarray, k: int) -> np.ndarray:
    """Compute brute-force k-NN ground truth."""
    from scipy.spatial.distance import cdist
    print(f"Computing distances for {len(queries)} queries...")
    distances = cdist(queries, vectors, metric='euclidean')
    print(f"Finding top-{k} neighbors...")
    return np.argsort(distances, axis=1)[:, :k].astype(np.int64)


def main():
    parser = argparse.ArgumentParser(
        description='Convert HDF5 to schema-driven binary format'
    )
    parser.add_argument('input', help='Input HDF5 file')
    parser.add_argument('output', help='Output base path (without extension)')
    parser.add_argument('--name', help='Dataset name')
    parser.add_argument('--metric', choices=['l2', 'cosine', 'ip'], default='l2')
    parser.add_argument('--max-neighbors', type=int, default=100)
    parser.add_argument('--key-pattern', default='vec:%012d')

    args = parser.parse_args()

    name = args.name or Path(args.input).stem

    convert_hdf5_to_schema(
        Path(args.input),
        Path(args.output),
        name,
        distance_metric=args.metric,
        max_ground_truth=args.max_neighbors,
        key_pattern=args.key_pattern
    )


if __name__ == '__main__':
    main()
```

---

## Part 1B: Modified Rust Consumer

### File Changes

**Delete/Replace**:
- `src/dataset/header.rs` - Hardcoded header parsing (delete entirely)
- `src/dataset/binary_dataset.rs` - Replace with schema-driven version

**New Files**:
```
src/dataset/
├── mod.rs              # Module exports
├── schema.rs           # Schema parsing (YAML)
├── layout.rs           # Layout computation from schema
├── context.rs          # Unified DatasetContext (replaces binary_dataset.rs)
├── source.rs           # Extended traits
└── error.rs            # Dataset errors
```

### New: `src/dataset/schema.rs`

```rust
//! Schema parsing from YAML files

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Schema version
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
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DType {
    Float32,
    Float16,
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
    pub fn byte_size(&self) -> usize {
        match self {
            DType::Float32 | DType::Int32 | DType::Uint32 => 4,
            DType::Float16 => 2,
            DType::Int64 | DType::Uint64 | DType::Float64 => 8,
            DType::Uint8 | DType::Int8 => 1,
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

/// Length specification
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LengthSpec {
    #[default]
    Fixed,
    Variable,
}

/// Field definition
#[derive(Debug, Clone, Deserialize)]
pub struct FieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    pub dtype: Option<DType>,
    pub dimensions: Option<u32>,
    pub encoding: Option<Encoding>,
    pub length: Option<LengthSpec>,
    pub max_bytes: Option<u32>,
}

impl FieldDef {
    /// Compute byte size for this field
    pub fn byte_size(&self) -> usize {
        match self.field_type {
            FieldType::Vector => {
                let dim = self.dimensions.unwrap_or(1) as usize;
                let dtype = self.dtype.unwrap_or(DType::Float32);
                dim * dtype.byte_size()
            }
            FieldType::Text | FieldType::Tag | FieldType::Blob => {
                let max_bytes = self.max_bytes.unwrap_or(64) as usize;
                let length = self.length.unwrap_or_default();
                if length == LengthSpec::Variable {
                    4 + max_bytes  // u32 length prefix + data
                } else {
                    max_bytes
                }
            }
            FieldType::Numeric => {
                self.dtype.unwrap_or(DType::Float64).byte_size()
            }
        }
    }
}

/// Record definition
#[derive(Debug, Clone, Deserialize)]
pub struct RecordDef {
    pub fields: Vec<FieldDef>,
}

/// Keys section config
#[derive(Debug, Clone, Deserialize, Default)]
pub struct KeysConfig {
    #[serde(default)]
    pub present: bool,
    pub pattern: Option<String>,
    pub encoding: Option<Encoding>,
    pub length: Option<LengthSpec>,
    pub max_bytes: Option<u32>,
}

/// Queries section config
#[derive(Debug, Clone, Deserialize, Default)]
pub struct QueriesConfig {
    #[serde(default)]
    pub present: bool,
    pub count: Option<u64>,
    pub query_fields: Option<Vec<String>>,
}

/// Ground truth section config
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GroundTruthConfig {
    #[serde(default)]
    pub present: bool,
    pub neighbors_per_query: Option<u32>,
    pub id_type: Option<String>,
}

/// Sections configuration
#[derive(Debug, Clone, Deserialize)]
pub struct SectionsConfig {
    pub records: RecordsConfig,
    #[serde(default)]
    pub keys: KeysConfig,
    #[serde(default)]
    pub queries: QueriesConfig,
    #[serde(default)]
    pub ground_truth: GroundTruthConfig,
}

/// Records section config
#[derive(Debug, Clone, Deserialize)]
pub struct RecordsConfig {
    pub count: u64,
}

/// Schema metadata
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SchemaMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Field metadata (distance metric, index type, etc.)
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FieldMetadata {
    pub distance_metric: Option<String>,
    pub index_type: Option<String>,
}

/// Complete schema definition
#[derive(Debug, Clone, Deserialize)]
pub struct DatasetSchema {
    pub version: u32,
    #[serde(default)]
    pub metadata: SchemaMetadata,
    pub record: RecordDef,
    pub sections: SectionsConfig,
    #[serde(default)]
    pub field_metadata: HashMap<String, FieldMetadata>,
}

impl DatasetSchema {
    /// Load schema from YAML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, SchemaError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| SchemaError::IoError(e.to_string()))?;

        let schema: DatasetSchema = serde_yaml::from_str(&content)
            .map_err(|e| SchemaError::ParseError(e.to_string()))?;

        if schema.version > SCHEMA_VERSION {
            return Err(SchemaError::UnsupportedVersion(schema.version));
        }

        Ok(schema)
    }

    /// Get distance metric for a field
    pub fn get_distance_metric(&self, field_name: &str) -> Option<&str> {
        self.field_metadata.get(field_name)
            .and_then(|m| m.distance_metric.as_deref())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Unsupported schema version: {0}")]
    UnsupportedVersion(u32),
}
```

### New: `src/dataset/layout.rs`

```rust
//! Layout computation from schema

use super::schema::{DatasetSchema, FieldDef, LengthSpec};
use std::collections::HashMap;

/// Computed field layout
#[derive(Debug, Clone)]
pub struct FieldLayout {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub data_offset: usize,  // Offset to actual data (after length prefix if variable)
    pub is_variable: bool,
}

/// Computed record layout
#[derive(Debug, Clone)]
pub struct RecordLayout {
    pub fields: Vec<FieldLayout>,
    pub field_by_name: HashMap<String, usize>,
    pub total_size: usize,
}

impl RecordLayout {
    /// Compute layout from field definitions
    pub fn from_fields(fields: &[FieldDef]) -> Self {
        let mut layout_fields = Vec::with_capacity(fields.len());
        let mut field_by_name = HashMap::new();
        let mut offset = 0;

        for (idx, field) in fields.iter().enumerate() {
            let size = field.byte_size();
            let is_variable = field.length == Some(LengthSpec::Variable);
            let data_offset = if is_variable { 4 } else { 0 };

            layout_fields.push(FieldLayout {
                name: field.name.clone(),
                offset,
                size,
                data_offset,
                is_variable,
            });

            field_by_name.insert(field.name.clone(), idx);
            offset += size;
        }

        Self {
            fields: layout_fields,
            field_by_name,
            total_size: offset,
        }
    }

    /// Get field index by name
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.field_by_name.get(name).copied()
    }
}

/// Computed section layout
#[derive(Debug, Clone)]
pub struct SectionLayout {
    pub records_offset: usize,
    pub records_size: usize,
    pub record_count: u64,

    pub keys_offset: Option<usize>,
    pub keys_entry_size: Option<usize>,

    pub queries_offset: Option<usize>,
    pub queries_size: Option<usize>,
    pub query_count: u64,
    pub query_record_size: usize,

    pub ground_truth_offset: Option<usize>,
    pub neighbors_per_query: usize,
    pub gt_id_size: usize,

    pub total_size: usize,
}

impl SectionLayout {
    /// Compute section layout from schema
    pub fn compute(schema: &DatasetSchema, record_layout: &RecordLayout) -> Self {
        let record_count = schema.sections.records.count;
        let records_size = record_count as usize * record_layout.total_size;

        let mut next_offset = records_size;

        // Keys section
        let (keys_offset, keys_entry_size) = if schema.sections.keys.present {
            let entry_size = schema.sections.keys.max_bytes.unwrap_or(64) as usize;
            let offset = next_offset;
            next_offset += record_count as usize * entry_size;
            (Some(offset), Some(entry_size))
        } else {
            (None, None)
        };

        // Queries section
        let query_fields = schema.sections.queries.query_fields.as_ref();
        let query_record_size = if let Some(qf) = query_fields {
            record_layout.fields.iter()
                .filter(|f| qf.contains(&f.name))
                .map(|f| f.size)
                .sum()
        } else {
            record_layout.total_size
        };

        let query_count = schema.sections.queries.count.unwrap_or(0);
        let (queries_offset, queries_size) = if schema.sections.queries.present && query_count > 0 {
            let size = query_count as usize * query_record_size;
            let offset = next_offset;
            next_offset += size;
            (Some(offset), Some(size))
        } else {
            (None, None)
        };

        // Ground truth section
        let neighbors_per_query = schema.sections.ground_truth.neighbors_per_query.unwrap_or(100) as usize;
        let gt_id_size = if schema.sections.ground_truth.id_type.as_deref() == Some("u32") { 4 } else { 8 };

        let ground_truth_offset = if schema.sections.ground_truth.present && query_count > 0 {
            let offset = next_offset;
            next_offset += query_count as usize * neighbors_per_query * gt_id_size;
            Some(offset)
        } else {
            None
        };

        Self {
            records_offset: 0,
            records_size,
            record_count,
            keys_offset,
            keys_entry_size,
            queries_offset,
            queries_size,
            query_count,
            query_record_size,
            ground_truth_offset,
            neighbors_per_query,
            gt_id_size,
            total_size: next_offset,
        }
    }
}
```

### New: `src/dataset/context.rs`

```rust
//! Unified schema-driven DatasetContext

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

use memmap2::Mmap;

use super::layout::{RecordLayout, SectionLayout};
use super::schema::{DatasetSchema, KeysConfig, LengthSpec};
use super::source::{DataSource, FieldDataSource, GroundTruthSource, VectorDataSource};
use crate::utils::DatasetError;

/// Key configuration for dataset
#[derive(Debug, Clone)]
pub enum KeyConfig {
    /// Keys stored in the data file
    FromFile {
        offset: usize,
        entry_size: usize,
    },
    /// Keys generated from pattern
    Generated {
        pattern: String,
    },
}

/// Unified schema-driven dataset context
pub struct DatasetContext {
    mmap: Mmap,
    schema: DatasetSchema,
    record_layout: RecordLayout,
    query_field_indices: Vec<usize>,  // Indices of query fields in record_layout
    section_layout: SectionLayout,
    key_config: KeyConfig,
}

impl DatasetContext {
    /// Load dataset from schema and data files
    pub fn load<P: AsRef<Path>>(schema_path: P, data_path: P) -> Result<Self, DatasetError> {
        let schema = DatasetSchema::load(schema_path.as_ref())
            .map_err(|e| DatasetError::SchemaError(e.to_string()))?;

        let record_layout = RecordLayout::from_fields(&schema.record.fields);
        let section_layout = SectionLayout::compute(&schema, &record_layout);

        // Map query fields to indices
        let query_field_indices = schema.sections.queries.query_fields.as_ref()
            .map(|qf| {
                qf.iter()
                    .filter_map(|name| record_layout.field_index(name))
                    .collect()
            })
            .unwrap_or_else(|| (0..record_layout.fields.len()).collect());

        // Determine key configuration
        let key_config = if schema.sections.keys.present {
            KeyConfig::FromFile {
                offset: section_layout.keys_offset.unwrap_or(0),
                entry_size: section_layout.keys_entry_size.unwrap_or(64),
            }
        } else {
            KeyConfig::Generated {
                pattern: schema.sections.keys.pattern.clone()
                    .unwrap_or_else(|| "key:%012d".to_string()),
            }
        };

        // Memory map the data file
        let file = File::open(data_path.as_ref())
            .map_err(DatasetError::OpenFailed)?;
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(DatasetError::OpenFailed)?;

        // Validate file size
        if mmap.len() < section_layout.total_size {
            return Err(DatasetError::FileTooSmall {
                size: mmap.len() as u64,
                minimum: section_layout.total_size as u64,
            });
        }

        Ok(Self {
            mmap,
            schema,
            record_layout,
            query_field_indices,
            section_layout,
            key_config,
        })
    }

    // === Record Access ===

    /// Get number of records
    #[inline]
    pub fn num_records(&self) -> u64 {
        self.section_layout.record_count
    }

    /// Get field bytes for a record by field name
    pub fn get_field_bytes(&self, record_idx: u64, field_name: &str) -> Option<&[u8]> {
        let field_idx = self.record_layout.field_index(field_name)?;
        self.get_field_bytes_by_index(record_idx, field_idx)
    }

    /// Get field bytes for a record by field index
    #[inline]
    pub fn get_field_bytes_by_index(&self, record_idx: u64, field_idx: usize) -> Option<&[u8]> {
        let field = self.record_layout.fields.get(field_idx)?;
        let record_offset = self.section_layout.records_offset
            + (record_idx as usize * self.record_layout.total_size);
        let field_offset = record_offset + field.offset;

        if field.is_variable {
            // Read length prefix
            let len_bytes = &self.mmap[field_offset..field_offset + 4];
            let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
            Some(&self.mmap[field_offset + 4..field_offset + 4 + len])
        } else {
            Some(&self.mmap[field_offset..field_offset + field.size])
        }
    }

    /// Get entire record as raw bytes
    pub fn get_record_bytes(&self, record_idx: u64) -> &[u8] {
        let offset = self.section_layout.records_offset
            + (record_idx as usize * self.record_layout.total_size);
        &self.mmap[offset..offset + self.record_layout.total_size]
    }

    // === Key Access ===

    /// Get key for a record
    pub fn get_key(&self, record_idx: u64) -> Cow<str> {
        match &self.key_config {
            KeyConfig::FromFile { offset, entry_size } => {
                let key_offset = offset + (record_idx as usize * entry_size);
                let key_bytes = &self.mmap[key_offset..key_offset + entry_size];
                // Find null terminator
                let len = key_bytes.iter().position(|&b| b == 0).unwrap_or(*entry_size);
                Cow::Borrowed(std::str::from_utf8(&key_bytes[..len]).unwrap_or(""))
            }
            KeyConfig::Generated { pattern } => {
                Cow::Owned(self.generate_key(pattern, record_idx))
            }
        }
    }

    fn generate_key(&self, pattern: &str, idx: u64) -> String {
        // Replace %012d style format with the index
        if let Some(pos) = pattern.find('%') {
            let end = pattern[pos..].find('d').map(|p| pos + p + 1).unwrap_or(pattern.len());
            let format_spec = &pattern[pos..end];
            // Parse width from %0Nd
            let width: usize = format_spec[1..format_spec.len()-1]
                .trim_start_matches('0')
                .parse()
                .unwrap_or(12);
            let formatted = format!("{:0width$}", idx, width = width);
            format!("{}{}{}", &pattern[..pos], formatted, &pattern[end..])
        } else {
            pattern.to_string()
        }
    }

    fn compute_cluster_tag(idx: u64) -> String {
        // Simple hash-based cluster tag
        let tag_chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let h = idx % (26 * 26 * 26);
        let c1 = (h / 676) as usize % 26;
        let c2 = (h / 26) as usize % 26;
        let c3 = h as usize % 26;
        format!("{}{}{}",
            tag_chars.chars().nth(c1).unwrap(),
            tag_chars.chars().nth(c2).unwrap(),
            tag_chars.chars().nth(c3).unwrap())
    }

    // === Query Access ===

    /// Get number of queries
    pub fn num_queries(&self) -> u64 {
        self.section_layout.query_count
    }

    /// Get query field bytes
    pub fn get_query_field_bytes(&self, query_idx: u64, field_name: &str) -> Option<&[u8]> {
        let field_idx = self.record_layout.field_index(field_name)?;
        self.get_query_field_bytes_by_index(query_idx, field_idx)
    }

    /// Get query field bytes by index (index into query_field_indices)
    pub fn get_query_field_bytes_by_index(&self, query_idx: u64, query_field_idx: usize) -> Option<&[u8]> {
        let queries_offset = self.section_layout.queries_offset?;

        // Compute offset within query record
        let mut field_offset_in_query = 0;
        for (i, &record_field_idx) in self.query_field_indices.iter().enumerate() {
            if i == query_field_idx {
                let field = &self.record_layout.fields[record_field_idx];
                let query_offset = queries_offset
                    + (query_idx as usize * self.section_layout.query_record_size)
                    + field_offset_in_query;

                if field.is_variable {
                    let len_bytes = &self.mmap[query_offset..query_offset + 4];
                    let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
                    return Some(&self.mmap[query_offset + 4..query_offset + 4 + len]);
                } else {
                    return Some(&self.mmap[query_offset..query_offset + field.size]);
                }
            }
            field_offset_in_query += self.record_layout.fields[record_field_idx].size;
        }
        None
    }

    // === Ground Truth ===

    /// Get ground truth neighbor IDs for a query
    pub fn get_neighbor_ids(&self, query_idx: u64) -> Option<&[u64]> {
        let gt_offset = self.section_layout.ground_truth_offset?;
        let num_neighbors = self.section_layout.neighbors_per_query;
        let id_size = self.section_layout.gt_id_size;

        if id_size != 8 {
            // u32 ground truth not yet supported in this accessor
            return None;
        }

        let offset = gt_offset + (query_idx as usize * num_neighbors * id_size);
        unsafe {
            Some(std::slice::from_raw_parts(
                self.mmap.as_ptr().add(offset) as *const u64,
                num_neighbors,
            ))
        }
    }

    /// Compute recall@k
    pub fn compute_recall(&self, query_idx: u64, result_ids: &[u64], k: usize) -> f64 {
        let Some(gt_ids) = self.get_neighbor_ids(query_idx) else {
            return 0.0;
        };

        let k = k.min(gt_ids.len()).min(result_ids.len());
        if k == 0 {
            return 0.0;
        }

        let matches = result_ids[..k].iter()
            .filter(|id| gt_ids[..k].contains(id))
            .count();

        matches as f64 / k as f64
    }

    /// Get all unique ground truth vector IDs
    pub fn get_ground_truth_vector_ids(&self) -> HashSet<u64> {
        let mut ids = HashSet::new();
        for query_idx in 0..self.num_queries() {
            if let Some(neighbors) = self.get_neighbor_ids(query_idx) {
                ids.extend(neighbors);
            }
        }
        ids
    }

    // === Accessors ===

    /// Get schema reference
    pub fn schema(&self) -> &DatasetSchema {
        &self.schema
    }

    /// Get record layout
    pub fn record_layout(&self) -> &RecordLayout {
        &self.record_layout
    }

    /// Get first vector field dimensions (for backward compatibility)
    pub fn dim(&self) -> usize {
        for field in &self.schema.record.fields {
            if field.field_type == super::schema::FieldType::Vector {
                return field.dimensions.unwrap_or(1) as usize;
            }
        }
        0
    }

    /// Get vector byte length (first vector field)
    pub fn vec_byte_len(&self) -> usize {
        for field in &self.record_layout.fields {
            if let Some(idx) = self.record_layout.field_index(&field.name) {
                let def = &self.schema.record.fields[idx];
                if def.field_type == super::schema::FieldType::Vector {
                    return field.size;
                }
            }
        }
        0
    }

    /// Get distance metric (first vector field)
    pub fn distance_metric(&self) -> &str {
        for field in &self.schema.record.fields {
            if field.field_type == super::schema::FieldType::Vector {
                return self.schema.get_distance_metric(&field.name).unwrap_or("l2");
            }
        }
        "l2"
    }

    /// Dataset summary
    pub fn summary(&self) -> String {
        format!(
            "Dataset: {} records, {} queries, {} fields",
            self.num_records(),
            self.num_queries(),
            self.record_layout.fields.len()
        )
    }
}

// Thread safety
unsafe impl Send for DatasetContext {}
unsafe impl Sync for DatasetContext {}
```

---

## Part 2: Python Command Recorder

A new Python library that captures Redis/Valkey commands and generates schema + binary files.

### File: `prep_datasets/command_recorder.py`

```python
#!/usr/bin/env python3
"""
Redis/Valkey Command Recorder

Captures redis-py style commands and generates schema YAML + binary data
that can be loaded by valkey-bench-rs to replay the exact commands.

Usage:
    from command_recorder import DatasetRecorder

    rec = DatasetRecorder()
    rec.set("key:1", "value1")
    rec.hset("user:1", {"name": "Alice", "age": "30"})
    rec.sadd("tags:1", "python", "rust")
    rec.zadd("scores:1", {"alice": 1.5, "bob": 2.5})

    rec.generate("output_dataset")
    # Creates: output_dataset.yaml, output_dataset.bin
"""

import yaml
import struct
import numpy as np
from pathlib import Path
from dataclasses import dataclass, field
from typing import List, Dict, Any, Optional, Set, Tuple, Union
from collections import defaultdict
from enum import Enum
import json


class CommandType(Enum):
    """Redis command types we support."""
    SET = "SET"
    HSET = "HSET"
    SADD = "SADD"
    RPUSH = "RPUSH"
    LPUSH = "LPUSH"
    ZADD = "ZADD"
    # Vector operations
    HSET_VECTOR = "HSET_VECTOR"  # HSET with vector data


@dataclass
class RecordedCommand:
    """A recorded Redis command."""
    cmd_type: CommandType
    key: str
    data: Any  # Type depends on command


@dataclass
class FieldSpec:
    """Inferred field specification."""
    name: str
    field_type: str  # text, numeric, vector, tag
    max_bytes: int = 0
    dtype: str = "float64"
    dimensions: int = 0
    is_variable: bool = False


class DatasetRecorder:
    """
    Records Redis-style commands and generates schema + binary dataset.

    Interface mirrors redis-py for familiarity.
    """

    def __init__(self, name: str = "recorded_dataset"):
        self.name = name
        self.commands: List[RecordedCommand] = []
        self.keys: List[str] = []
        self.key_set: Set[str] = set()

        # Track field statistics for schema inference
        self._field_stats: Dict[str, Dict[str, Any]] = defaultdict(lambda: {
            'max_len': 0,
            'values': [],
            'types': set(),
        })

        # Track collection sizes
        self._collection_stats: Dict[str, Dict[str, int]] = defaultdict(lambda: {
            'max_members': 0,
        })

    # === String Commands ===

    def set(self, key: str, value: str) -> None:
        """Record a SET command."""
        self._record_key(key)
        self.commands.append(RecordedCommand(
            cmd_type=CommandType.SET,
            key=key,
            data=value
        ))
        self._track_field('value', value, 'text')

    # === Hash Commands ===

    def hset(self, key: str, mapping: Dict[str, str] = None, **kwargs) -> None:
        """
        Record an HSET command.

        Usage:
            rec.hset("user:1", {"name": "Alice", "age": "30"})
            # or
            rec.hset("user:1", name="Alice", age="30")
        """
        if mapping is None:
            mapping = kwargs
        else:
            mapping = {**mapping, **kwargs}

        self._record_key(key)
        self.commands.append(RecordedCommand(
            cmd_type=CommandType.HSET,
            key=key,
            data=mapping
        ))

        for field_name, value in mapping.items():
            self._track_field(field_name, value, 'text')

    def hset_vector(self, key: str, field_name: str, vector: List[float],
                    **other_fields) -> None:
        """
        Record an HSET with vector data.

        Usage:
            rec.hset_vector("vec:1", "embedding", [0.1, 0.2, 0.3, 0.4],
                           category="electronics", price="99.99")
        """
        self._record_key(key)

        data = {field_name: np.array(vector, dtype=np.float32)}
        data.update(other_fields)

        self.commands.append(RecordedCommand(
            cmd_type=CommandType.HSET_VECTOR,
            key=key,
            data=data
        ))

        # Track vector field
        self._track_field(field_name, vector, 'vector')

        # Track other fields
        for fname, fvalue in other_fields.items():
            self._track_field(fname, fvalue, 'text')

    # === Set Commands ===

    def sadd(self, key: str, *members: str) -> None:
        """Record a SADD command."""
        self._record_key(key)
        self.commands.append(RecordedCommand(
            cmd_type=CommandType.SADD,
            key=key,
            data=list(members)
        ))

        # Track collection size
        self._collection_stats[key]['max_members'] = max(
            self._collection_stats[key]['max_members'],
            len(members)
        )

        # Track member size
        for member in members:
            self._track_field('_set_member', member, 'text')

    # === List Commands ===

    def rpush(self, key: str, *values: str) -> None:
        """Record an RPUSH command."""
        self._record_key(key)
        self.commands.append(RecordedCommand(
            cmd_type=CommandType.RPUSH,
            key=key,
            data=list(values)
        ))

        self._collection_stats[key]['max_members'] = max(
            self._collection_stats[key]['max_members'],
            len(values)
        )

        for value in values:
            self._track_field('_list_element', value, 'text')

    def lpush(self, key: str, *values: str) -> None:
        """Record an LPUSH command."""
        self._record_key(key)
        self.commands.append(RecordedCommand(
            cmd_type=CommandType.LPUSH,
            key=key,
            data=list(values)
        ))

        self._collection_stats[key]['max_members'] = max(
            self._collection_stats[key]['max_members'],
            len(values)
        )

        for value in values:
            self._track_field('_list_element', value, 'text')

    # === Sorted Set Commands ===

    def zadd(self, key: str, mapping: Dict[str, float] = None, **kwargs) -> None:
        """
        Record a ZADD command.

        Usage:
            rec.zadd("scores:1", {"alice": 1.5, "bob": 2.5})
            # or
            rec.zadd("scores:1", alice=1.5, bob=2.5)
        """
        if mapping is None:
            mapping = {k: float(v) for k, v in kwargs.items()}
        else:
            mapping = {**mapping, **{k: float(v) for k, v in kwargs.items()}}

        self._record_key(key)
        self.commands.append(RecordedCommand(
            cmd_type=CommandType.ZADD,
            key=key,
            data=mapping
        ))

        self._collection_stats[key]['max_members'] = max(
            self._collection_stats[key]['max_members'],
            len(mapping)
        )

        for member in mapping.keys():
            self._track_field('_zset_member', member, 'text')

    # === Internal Tracking ===

    def _record_key(self, key: str) -> None:
        """Track unique keys."""
        if key not in self.key_set:
            self.key_set.add(key)
            self.keys.append(key)

    def _track_field(self, name: str, value: Any, inferred_type: str) -> None:
        """Track field statistics for schema inference."""
        stats = self._field_stats[name]
        stats['types'].add(inferred_type)

        if inferred_type == 'text':
            if isinstance(value, str):
                stats['max_len'] = max(stats['max_len'], len(value.encode('utf-8')))
            stats['values'].append(value)
        elif inferred_type == 'vector':
            if isinstance(value, (list, np.ndarray)):
                arr = np.array(value)
                if 'dimensions' not in stats:
                    stats['dimensions'] = len(arr)
                stats['values'].append(arr)

    # === Schema Generation ===

    def _infer_schema(self) -> Dict[str, Any]:
        """Infer schema from recorded commands."""

        # Determine primary command type
        cmd_types = set(cmd.cmd_type for cmd in self.commands)

        # Build fields based on command type
        fields = []
        field_metadata = {}
        workload_type = 'hash'
        workload_command = 'HSET'

        if CommandType.SET in cmd_types:
            workload_type = 'string'
            workload_command = 'SET'
            stats = self._field_stats['value']
            max_bytes = max(stats['max_len'], 64)
            # Round up to power of 2 for alignment
            max_bytes = 2 ** (max_bytes - 1).bit_length() if max_bytes > 0 else 64
            fields.append({
                'name': 'value',
                'type': 'text',
                'encoding': 'utf8',
                'length': 'variable' if stats['max_len'] > 256 else 'fixed',
                'max_bytes': max_bytes
            })

        elif CommandType.HSET in cmd_types or CommandType.HSET_VECTOR in cmd_types:
            workload_type = 'hash'
            workload_command = 'HSET'

            # Collect all hash fields
            hash_fields = set()
            for cmd in self.commands:
                if cmd.cmd_type in (CommandType.HSET, CommandType.HSET_VECTOR):
                    hash_fields.update(cmd.data.keys())

            for fname in sorted(hash_fields):
                stats = self._field_stats[fname]

                if 'vector' in stats['types']:
                    dim = stats.get('dimensions', 768)
                    fields.append({
                        'name': fname,
                        'type': 'vector',
                        'dtype': 'float32',
                        'dimensions': dim
                    })
                    field_metadata[fname] = {'distance_metric': 'l2'}
                else:
                    max_bytes = max(stats['max_len'], 64)
                    max_bytes = 2 ** (max_bytes - 1).bit_length() if max_bytes > 0 else 64
                    fields.append({
                        'name': fname,
                        'type': 'text',
                        'encoding': 'utf8',
                        'length': 'fixed',
                        'max_bytes': max_bytes
                    })

        elif CommandType.SADD in cmd_types:
            workload_type = 'set'
            workload_command = 'SADD'

            max_members = max(
                (s['max_members'] for s in self._collection_stats.values()),
                default=10
            )
            member_stats = self._field_stats['_set_member']
            max_bytes = max(member_stats['max_len'], 64)
            max_bytes = 2 ** (max_bytes - 1).bit_length() if max_bytes > 0 else 64

            return self._build_collection_schema(
                'set', max_members, max_bytes, workload_type, workload_command
            )

        elif CommandType.RPUSH in cmd_types or CommandType.LPUSH in cmd_types:
            workload_type = 'list'
            workload_command = 'RPUSH' if CommandType.RPUSH in cmd_types else 'LPUSH'

            max_elements = max(
                (s['max_members'] for s in self._collection_stats.values()),
                default=10
            )
            element_stats = self._field_stats['_list_element']
            max_bytes = max(element_stats['max_len'], 64)
            max_bytes = 2 ** (max_bytes - 1).bit_length() if max_bytes > 0 else 64

            return self._build_collection_schema(
                'list', max_elements, max_bytes, workload_type, workload_command
            )

        elif CommandType.ZADD in cmd_types:
            workload_type = 'zset'
            workload_command = 'ZADD'

            max_members = max(
                (s['max_members'] for s in self._collection_stats.values()),
                default=10
            )
            member_stats = self._field_stats['_zset_member']
            max_bytes = max(member_stats['max_len'], 64)
            max_bytes = 2 ** (max_bytes - 1).bit_length() if max_bytes > 0 else 64

            return self._build_zset_schema(max_members, max_bytes)

        # Compute max key length
        max_key_len = max(len(k.encode('utf-8')) for k in self.keys) if self.keys else 64
        max_key_len = 2 ** (max_key_len - 1).bit_length() if max_key_len > 0 else 64

        schema = {
            'version': 1,
            'metadata': {
                'name': self.name,
                'description': f'Recorded dataset with {len(self.commands)} commands'
            },
            'workload': {
                'type': workload_type,
                'command': workload_command
            },
            'record': {
                'fields': fields
            },
            'sections': {
                'records': {'count': len(self.keys)},
                'keys': {
                    'present': True,
                    'encoding': 'utf8',
                    'length': 'fixed',
                    'max_bytes': max_key_len
                }
            }
        }

        if field_metadata:
            schema['field_metadata'] = field_metadata

        return schema

    def _build_collection_schema(self, coll_type: str, max_members: int,
                                  member_max_bytes: int,
                                  workload_type: str, workload_command: str) -> Dict:
        """Build schema for SET/LIST collections."""
        max_key_len = max(len(k.encode('utf-8')) for k in self.keys) if self.keys else 64
        max_key_len = 2 ** (max_key_len - 1).bit_length() if max_key_len > 0 else 64

        return {
            'version': 1,
            'metadata': {
                'name': self.name,
                'description': f'Recorded {coll_type} dataset'
            },
            'workload': {
                'type': workload_type,
                'command': workload_command
            },
            'record': {
                'collection': {
                    'type': coll_type,
                    'max_members': max_members,
                    'member': {
                        'type': 'text',
                        'encoding': 'utf8',
                        'length': 'fixed',
                        'max_bytes': member_max_bytes
                    }
                }
            },
            'sections': {
                'records': {'count': len(self.keys)},
                'keys': {
                    'present': True,
                    'encoding': 'utf8',
                    'length': 'fixed',
                    'max_bytes': max_key_len
                }
            }
        }

    def _build_zset_schema(self, max_members: int, member_max_bytes: int) -> Dict:
        """Build schema for ZSET collections."""
        max_key_len = max(len(k.encode('utf-8')) for k in self.keys) if self.keys else 64
        max_key_len = 2 ** (max_key_len - 1).bit_length() if max_key_len > 0 else 64

        return {
            'version': 1,
            'metadata': {
                'name': self.name,
                'description': 'Recorded zset dataset'
            },
            'workload': {
                'type': 'zset',
                'command': 'ZADD'
            },
            'record': {
                'collection': {
                    'type': 'zset',
                    'max_members': max_members,
                    'member': {
                        'fields': [
                            {'name': 'score', 'type': 'numeric', 'dtype': 'float64'},
                            {'name': 'value', 'type': 'text', 'encoding': 'utf8',
                             'length': 'fixed', 'max_bytes': member_max_bytes}
                        ]
                    }
                }
            },
            'sections': {
                'records': {'count': len(self.keys)},
                'keys': {
                    'present': True,
                    'encoding': 'utf8',
                    'length': 'fixed',
                    'max_bytes': max_key_len
                }
            }
        }

    # === Binary Generation ===

    def generate(self, output_base: Union[str, Path]) -> Tuple[Path, Path]:
        """
        Generate schema YAML and binary data files.

        Args:
            output_base: Base path without extension

        Returns:
            Tuple of (schema_path, data_path)
        """
        output_base = Path(output_base)
        schema_path = output_base.with_suffix('.yaml')
        data_path = output_base.with_suffix('.bin')

        # Infer schema
        schema = self._infer_schema()

        # Write schema
        with open(schema_path, 'w') as f:
            yaml.dump(schema, f, default_flow_style=False, sort_keys=False)
        print(f"Schema written to {schema_path}")

        # Write binary data
        self._write_binary(schema, data_path)
        print(f"Binary data written to {data_path}")

        return schema_path, data_path

    def _write_binary(self, schema: Dict, data_path: Path) -> None:
        """Write binary data file."""
        record_def = schema['record']
        sections = schema['sections']

        # Compute sizes
        if 'collection' in record_def:
            record_size = self._compute_collection_record_size(record_def['collection'])
        else:
            record_size = self._compute_field_record_size(record_def['fields'])

        num_records = sections['records']['count']
        key_size = sections['keys']['max_bytes']

        records_size = num_records * record_size
        keys_size = num_records * key_size

        with open(data_path, 'wb') as f:
            # Write records
            key_to_idx = {k: i for i, k in enumerate(self.keys)}
            record_data = [None] * num_records

            for cmd in self.commands:
                idx = key_to_idx.get(cmd.key)
                if idx is None:
                    continue

                if cmd.cmd_type == CommandType.SET:
                    record_data[idx] = {'value': cmd.data}
                elif cmd.cmd_type in (CommandType.HSET, CommandType.HSET_VECTOR):
                    if record_data[idx] is None:
                        record_data[idx] = {}
                    record_data[idx].update(cmd.data)
                elif cmd.cmd_type in (CommandType.SADD, CommandType.RPUSH, CommandType.LPUSH):
                    if record_data[idx] is None:
                        record_data[idx] = {'members': []}
                    record_data[idx]['members'].extend(cmd.data)
                elif cmd.cmd_type == CommandType.ZADD:
                    if record_data[idx] is None:
                        record_data[idx] = {'members': []}
                    for member, score in cmd.data.items():
                        record_data[idx]['members'].append((score, member))

            # Write each record
            for idx, data in enumerate(record_data):
                if data is None:
                    data = {}

                if 'collection' in record_def:
                    self._write_collection_record(f, record_def['collection'], data, record_size)
                else:
                    self._write_field_record(f, record_def['fields'], data, record_size)

            # Write keys
            for key in self.keys:
                key_bytes = key.encode('utf-8')
                f.write(key_bytes[:key_size])
                padding = key_size - min(len(key_bytes), key_size)
                f.write(b'\x00' * padding)

    def _compute_field_record_size(self, fields: List[Dict]) -> int:
        """Compute record size from field definitions."""
        size = 0
        for field in fields:
            ftype = field['type']
            if ftype == 'vector':
                dim = field['dimensions']
                dtype_size = {'float32': 4, 'float16': 2}.get(field.get('dtype', 'float32'), 4)
                size += dim * dtype_size
            elif ftype in ('text', 'tag', 'blob'):
                max_bytes = field['max_bytes']
                if field.get('length') == 'variable':
                    size += 4 + max_bytes
                else:
                    size += max_bytes
            elif ftype == 'numeric':
                dtype_size = {'float64': 8, 'float32': 4, 'int64': 8, 'int32': 4}.get(
                    field.get('dtype', 'float64'), 8)
                size += dtype_size
        return size

    def _compute_collection_record_size(self, collection: Dict) -> int:
        """Compute record size for collection types."""
        max_members = collection['max_members']
        member = collection['member']

        if 'fields' in member:
            # ZSET with score + value
            member_size = self._compute_field_record_size(member['fields'])
        else:
            # Simple member
            member_size = member['max_bytes']

        return 4 + (max_members * member_size)  # u32 count + members

    def _write_field_record(self, f, fields: List[Dict], data: Dict, record_size: int) -> None:
        """Write a field-based record."""
        start_pos = f.tell()

        for field in fields:
            fname = field['name']
            ftype = field['type']
            value = data.get(fname)

            if ftype == 'vector':
                if isinstance(value, np.ndarray):
                    f.write(value.astype(np.float32).tobytes())
                else:
                    dim = field['dimensions']
                    f.write(b'\x00' * (dim * 4))

            elif ftype in ('text', 'tag'):
                max_bytes = field['max_bytes']
                is_variable = field.get('length') == 'variable'

                if value is None:
                    value = ''
                value_bytes = value.encode('utf-8') if isinstance(value, str) else value

                if is_variable:
                    f.write(struct.pack('<I', len(value_bytes)))

                f.write(value_bytes[:max_bytes])
                padding = max_bytes - min(len(value_bytes), max_bytes)
                f.write(b'\x00' * padding)

            elif ftype == 'numeric':
                dtype = field.get('dtype', 'float64')
                fmt = {'float64': '<d', 'float32': '<f', 'int64': '<q', 'int32': '<i'}.get(dtype, '<d')
                f.write(struct.pack(fmt, float(value) if value else 0.0))

        # Ensure we wrote exactly record_size bytes
        written = f.tell() - start_pos
        if written < record_size:
            f.write(b'\x00' * (record_size - written))

    def _write_collection_record(self, f, collection: Dict, data: Dict, record_size: int) -> None:
        """Write a collection record (SET, LIST, ZSET)."""
        start_pos = f.tell()
        max_members = collection['max_members']
        member = collection['member']
        members = data.get('members', [])

        # Write member count
        f.write(struct.pack('<I', len(members)))

        if 'fields' in member:
            # ZSET: score + value pairs
            member_fields = member['fields']
            member_size = self._compute_field_record_size(member_fields)

            for i in range(max_members):
                if i < len(members):
                    score, value = members[i]
                    member_data = {'score': score, 'value': value}
                else:
                    member_data = {}
                self._write_field_record(f, member_fields, member_data, member_size)
        else:
            # Simple members
            max_bytes = member['max_bytes']

            for i in range(max_members):
                if i < len(members):
                    value = members[i]
                    value_bytes = value.encode('utf-8') if isinstance(value, str) else value
                    f.write(value_bytes[:max_bytes])
                    padding = max_bytes - min(len(value_bytes), max_bytes)
                    f.write(b'\x00' * padding)
                else:
                    f.write(b'\x00' * max_bytes)

        # Ensure we wrote exactly record_size bytes
        written = f.tell() - start_pos
        if written < record_size:
            f.write(b'\x00' * (record_size - written))


# === Example Usage ===

def example_string_dataset():
    """Example: Recording string SET commands."""
    rec = DatasetRecorder(name="string_benchmark")

    for i in range(1000):
        rec.set(f"key:{i:06d}", f"value-{i}-data-payload")

    rec.generate("datasets/string_benchmark")


def example_hash_dataset():
    """Example: Recording hash HSET commands."""
    rec = DatasetRecorder(name="user_profiles")

    for i in range(1000):
        rec.hset(f"user:{i:06d}", {
            "name": f"User {i}",
            "email": f"user{i}@example.com",
            "age": str(20 + (i % 50)),
            "city": ["NYC", "LA", "SF", "CHI", "BOS"][i % 5]
        })

    rec.generate("datasets/user_profiles")


def example_vector_dataset():
    """Example: Recording vector HSET commands."""
    rec = DatasetRecorder(name="product_vectors")

    np.random.seed(42)
    for i in range(1000):
        embedding = np.random.randn(128).astype(np.float32)
        rec.hset_vector(
            f"product:{i:06d}",
            "embedding", embedding.tolist(),
            category=["electronics", "clothing", "books", "home"][i % 4],
            price=str(10.0 + (i % 100))
        )

    rec.generate("datasets/product_vectors")


def example_set_dataset():
    """Example: Recording set SADD commands."""
    rec = DatasetRecorder(name="user_tags")

    tags = ["python", "rust", "javascript", "go", "java", "c++", "ruby", "php"]

    for i in range(1000):
        user_tags = [tags[j] for j in range(len(tags)) if (i + j) % 3 == 0]
        rec.sadd(f"user:{i:06d}:tags", *user_tags)

    rec.generate("datasets/user_tags")


def example_zset_dataset():
    """Example: Recording sorted set ZADD commands."""
    rec = DatasetRecorder(name="leaderboard")

    for i in range(100):
        scores = {f"player{j}": float(j * 100 + i) for j in range(10)}
        rec.zadd(f"game:{i:04d}:scores", scores)

    rec.generate("datasets/leaderboard")


if __name__ == '__main__':
    import sys

    if len(sys.argv) > 1:
        example = sys.argv[1]
        if example == 'string':
            example_string_dataset()
        elif example == 'hash':
            example_hash_dataset()
        elif example == 'vector':
            example_vector_dataset()
        elif example == 'set':
            example_set_dataset()
        elif example == 'zset':
            example_zset_dataset()
        else:
            print(f"Unknown example: {example}")
            print("Available: string, hash, vector, set, zset")
    else:
        print("Usage: python command_recorder.py <example>")
        print("Available examples: string, hash, vector, set, zset")
```

---

## Part 3: Integration Summary

### File Changes Overview

```
prep_datasets/
├── prepare_binary.py              # KEEP: Legacy HDF5 converter (deprecated)
├── prepare_schema_binary.py       # NEW: Schema-driven HDF5 converter
├── command_recorder.py            # NEW: Redis command recorder
└── README.md                      # UPDATE: Document new tools

src/dataset/
├── mod.rs                         # UPDATE: New module structure
├── header.rs                      # DELETE: Hardcoded header
├── binary_dataset.rs              # DELETE: Old implementation
├── schema.rs                      # NEW: YAML schema parsing
├── layout.rs                      # NEW: Layout computation
├── context.rs                     # NEW: Unified DatasetContext
├── source.rs                      # UPDATE: Extended traits
└── error.rs                       # NEW: Dataset errors

docs/
├── unified-dataset-design.md      # EXISTING: Design doc
├── dataset-binary-format.md       # EXISTING: Binary format spec
├── dataset-examples.md            # EXISTING: Schema examples
└── schema-driven-implementation.md # NEW: This document
```

### CLI Changes

```bash
# Old (deprecated):
./target/release/valkey-bench-rs --dataset mnist.bin

# New (schema-driven):
./target/release/valkey-bench-rs --schema mnist.yaml --data mnist.bin

# The --dataset flag becomes shorthand for finding .yaml + .bin pair:
./target/release/valkey-bench-rs --dataset datasets/mnist
# Looks for: datasets/mnist.yaml + datasets/mnist.bin
```

### Migration Path

1. **Phase 1**: Add new schema infrastructure alongside existing code
2. **Phase 2**: Add `--schema`/`--data` CLI options
3. **Phase 3**: Deprecate `--dataset` single-file mode
4. **Phase 4**: Remove old hardcoded header code

### Backward Compatibility

During transition, support both modes:
- Old: `--dataset file.bin` (reads embedded header)
- New: `--schema file.yaml --data file.bin` (schema-driven)

Eventually deprecate and remove old mode.

---

## Summary

This design provides:

1. **Modified Python Converter** (`prepare_schema_binary.py`):
   - Generates YAML schema + binary data from HDF5
   - No embedded header in binary file
   - Extensible field types

2. **Modified Rust Consumer** (`src/dataset/`):
   - `schema.rs`: Parses YAML schema
   - `layout.rs`: Computes field/section offsets
   - `context.rs`: Unified mmap-based access

3. **Python Command Recorder** (`command_recorder.py`):
   - Redis-py compatible interface
   - Records SET, HSET, SADD, RPUSH, ZADD, etc.
   - Generates schema + binary on `generate()`
   - Enables exact command replay via benchmark

Key principle: **Schema describes structure, binary contains raw data.**
