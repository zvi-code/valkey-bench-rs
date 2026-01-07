#!/usr/bin/env python3
"""
Schema-Driven Binary Dataset Generator

Converts HDF5/Parquet/JSON sources to schema YAML + binary data file.
This is the schema-driven replacement for prepare_binary.py.

The output format is:
  - dataset.yaml: Schema describing structure
  - dataset.bin: Raw binary data (no embedded header)

Usage:
    # Convert HDF5 vector dataset
    python prepare_schema_binary.py hdf5 input.hdf5 output_base --name mnist

    # Convert with custom settings
    python prepare_schema_binary.py hdf5 input.hdf5 output_base \\
        --name my_dataset \\
        --metric cosine \\
        --max-neighbors 100 \\
        --key-pattern "vec:%012d"
"""

import numpy as np
import struct
import yaml
import argparse
from pathlib import Path
from dataclasses import dataclass, field
from typing import List, Dict, Any, Optional


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
            length='fixed',
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

    def with_queries(self, count: int, query_fields: List[str] = None) -> 'SchemaBuilder':
        """Configure query section."""
        self.queries_present = True
        self.query_count = count
        self.query_fields = query_fields or [f.name for f in self.fields if f.field_type == 'vector']
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
            if value is not None:
                arr = np.array(value, dtype=field.dtype)
                self.file.write(arr.tobytes())
            else:
                self.file.write(b'\x00' * field.byte_size())

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

    def write_vectors_bulk(self, vectors: np.ndarray, vector_field: str = 'embedding'):
        """Write all vectors at once (optimized for large datasets)."""
        # Find the vector field
        vector_field_def = None
        field_offset = 0
        for f in self.schema.fields:
            if f.name == vector_field:
                vector_field_def = f
                break
            field_offset += f.byte_size()

        if vector_field_def is None:
            raise ValueError(f"Vector field '{vector_field}' not found in schema")

        # Write all vectors
        vectors = vectors.astype(np.float32)
        for i, vec in enumerate(vectors):
            offset = self.records_offset + (i * self.record_size) + field_offset
            self.file.seek(offset)
            self.file.write(vec.tobytes())

        self.records_written = max(self.records_written, len(vectors))

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

    def write_queries(self, queries: np.ndarray):
        """Write queries section (for vector datasets)."""
        if self.queries_offset is None:
            raise ValueError("Queries section not configured in schema")

        self.file.seek(self.queries_offset)
        queries = queries.astype(np.float32)
        self.file.write(queries.tobytes())

    def write_ground_truth(self, ground_truth: np.ndarray):
        """Write ground truth section."""
        if self.ground_truth_offset is None:
            raise ValueError("Ground truth section not configured in schema")

        self.file.seek(self.ground_truth_offset)

        if self.schema.gt_id_type == 'u64':
            gt = ground_truth.astype(np.int64)
        else:
            gt = ground_truth.astype(np.int32)

        self.file.write(gt.tobytes())

    def close(self):
        """Close the file."""
        self.file.close()
        print(f"Binary data written to {self.path} ({self.total_size:,} bytes)")


def compute_ground_truth(vectors: np.ndarray, queries: np.ndarray, k: int) -> np.ndarray:
    """Compute brute-force k-NN ground truth."""
    print(f"Computing ground truth for {len(queries)} queries, k={k}...")

    try:
        from scipy.spatial.distance import cdist
        print("  Using scipy.spatial.distance.cdist")
        distances = cdist(queries, vectors, metric='euclidean')
        return np.argsort(distances, axis=1)[:, :k].astype(np.int64)
    except ImportError:
        print("  scipy not available, using numpy (slower)")
        ground_truth = np.zeros((len(queries), k), dtype=np.int64)
        for i, query in enumerate(queries):
            distances = np.linalg.norm(vectors - query, axis=1)
            ground_truth[i] = np.argsort(distances)[:k]
            if (i + 1) % 100 == 0:
                print(f"    Processed {i + 1}/{len(queries)} queries")
        return ground_truth


def convert_hdf5_to_schema(h5_path: Path, output_base: Path,
                           dataset_name: str,
                           distance_metric: str = 'l2',
                           max_ground_truth: int = 100,
                           key_pattern: str = "vec:%012d"):
    """
    Convert HDF5 vector dataset to schema YAML + binary data.

    Replaces prepare_binary.py functionality.
    """
    try:
        import h5py
    except ImportError:
        print("Error: h5py required. Install with: pip install h5py")
        return

    print(f"Loading dataset from {h5_path}...")

    with h5py.File(h5_path, 'r') as f:
        # Show available keys
        print(f"  Available keys: {list(f.keys())}")

        # Load vectors
        if 'train' in f:
            vectors = np.array(f['train'], dtype=np.float32)
            print(f"  Using 'train' for vectors")
        elif 'database' in f:
            vectors = np.array(f['database'], dtype=np.float32)
            print(f"  Using 'database' for vectors")
        else:
            raise ValueError("No 'train' or 'database' key found")

        # Load queries
        if 'test' in f:
            queries = np.array(f['test'], dtype=np.float32)
            print(f"  Using 'test' for queries")
        elif 'queries' in f:
            queries = np.array(f['queries'], dtype=np.float32)
            print(f"  Using 'queries' for queries")
        else:
            raise ValueError("No 'test' or 'queries' key found")

        # Load ground truth
        if 'neighbors' in f:
            ground_truth = np.array(f['neighbors'], dtype=np.int64)
            print(f"  Using 'neighbors' for ground truth")
        elif 'ground_truth' in f:
            ground_truth = np.array(f['ground_truth'], dtype=np.int64)
            print(f"  Using 'ground_truth' for ground truth")
        else:
            print("  No ground truth found, computing...")
            ground_truth = compute_ground_truth(vectors, queries, max_ground_truth)

        if ground_truth.shape[1] > max_ground_truth:
            ground_truth = ground_truth[:, :max_ground_truth]

    num_vectors, dim = vectors.shape
    num_queries = len(queries)
    num_neighbors = ground_truth.shape[1]

    print(f"\nDataset summary:")
    print(f"  Vectors: {num_vectors:,} x {dim}")
    print(f"  Queries: {num_queries:,} x {dim}")
    print(f"  Ground truth: {num_neighbors} neighbors per query")

    # Build schema
    builder = SchemaBuilder(
        name=dataset_name,
        description=f'Converted from {h5_path.name}: {num_vectors:,} vectors, {dim} dimensions'
    )
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

    print("\nWriting binary data...")
    print("  Writing vectors...")
    writer.write_vectors_bulk(vectors, 'embedding')

    print("  Writing queries...")
    writer.write_queries(queries)

    print("  Writing ground truth...")
    writer.write_ground_truth(ground_truth)

    writer.close()

    print(f"\nGenerated:")
    print(f"  Schema: {schema_path}")
    print(f"  Data:   {data_path}")


def main():
    parser = argparse.ArgumentParser(
        description='Convert datasets to schema-driven binary format',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Convert HDF5 vector dataset
  python prepare_schema_binary.py hdf5 mnist.hdf5 datasets/mnist

  # With custom settings
  python prepare_schema_binary.py hdf5 glove.hdf5 datasets/glove \\
      --name glove-100 --metric cosine --max-neighbors 50
"""
    )

    subparsers = parser.add_subparsers(dest='command', required=True)

    # HDF5 subcommand
    hdf5_parser = subparsers.add_parser('hdf5', help='Convert HDF5 file')
    hdf5_parser.add_argument('input', type=Path, help='Input HDF5 file')
    hdf5_parser.add_argument('output', type=Path, help='Output base path (without extension)')
    hdf5_parser.add_argument('--name', help='Dataset name (default: input filename)')
    hdf5_parser.add_argument('--metric', choices=['l2', 'cosine', 'ip'], default='l2',
                             help='Distance metric')
    hdf5_parser.add_argument('--max-neighbors', type=int, default=100,
                             help='Max ground truth neighbors')
    hdf5_parser.add_argument('--key-pattern', default='vec:%012d',
                             help='Key generation pattern')

    args = parser.parse_args()

    if args.command == 'hdf5':
        name = args.name or args.input.stem
        convert_hdf5_to_schema(
            args.input,
            args.output,
            name,
            distance_metric=args.metric,
            max_ground_truth=args.max_neighbors,
            key_pattern=args.key_pattern
        )


if __name__ == '__main__':
    main()
