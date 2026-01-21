#!/usr/bin/env python3
"""
Create test datasets in schema-driven format (YAML schema + binary data file).
No embedded headers - all structure is defined by the YAML schema.
"""

import numpy as np
import struct
import yaml
from pathlib import Path
import argparse


def create_vector_dataset(output_dir: Path, name: str, num_vectors: int,
                          num_queries: int, dim: int, num_neighbors: int = 10):
    """Create a simple vector dataset for testing."""

    output_dir.mkdir(parents=True, exist_ok=True)

    # Generate random vectors
    np.random.seed(42)
    vectors = np.random.randn(num_vectors, dim).astype(np.float32)
    queries = np.random.randn(num_queries, dim).astype(np.float32)

    # Compute ground truth (brute force L2)
    print(f"Computing ground truth for {num_queries} queries...")
    ground_truth = np.zeros((num_queries, num_neighbors), dtype=np.uint64)
    for i in range(num_queries):
        distances = np.sum((vectors - queries[i]) ** 2, axis=1)
        ground_truth[i] = np.argsort(distances)[:num_neighbors]

    # Calculate sizes
    record_size = dim * 4  # float32
    records_size = num_vectors * record_size
    queries_size = num_queries * record_size
    gt_size = num_queries * num_neighbors * 8  # uint64

    # Write binary data (no header)
    data_path = output_dir / f"{name}.bin"
    with open(data_path, 'wb') as f:
        # Records section (vectors)
        vectors.tofile(f)

        # Queries section
        queries.tofile(f)

        # Ground truth section
        ground_truth.tofile(f)

    # Create schema YAML
    schema = {
        'version': 1,
        'metadata': {
            'name': name,
            'description': f'Test vector dataset: {num_vectors} vectors, {dim} dimensions'
        },
        'record': {
            'fields': [
                {
                    'name': 'embedding',
                    'type': 'vector',
                    'dtype': 'float32',
                    'dimensions': dim
                }
            ]
        },
        'sections': {
            'records': {
                'count': num_vectors
            },
            'keys': {
                'present': False,
                'pattern': 'vec:{id}'
            },
            'queries': {
                'present': True,
                'count': num_queries
            },
            'ground_truth': {
                'present': True,
                'neighbors_per_query': num_neighbors,
                'id_type': 'u64'
            }
        }
    }

    schema_path = output_dir / f"{name}.yaml"
    with open(schema_path, 'w') as f:
        yaml.dump(schema, f, default_flow_style=False, sort_keys=False)

    print(f"Created {data_path} ({data_path.stat().st_size} bytes)")
    print(f"Created {schema_path}")

    return schema_path, data_path


def create_hash_dataset(output_dir: Path, name: str, num_records: int):
    """Create a hash dataset with vector + category fields."""

    output_dir.mkdir(parents=True, exist_ok=True)

    np.random.seed(42)
    dim = 128
    categories = [b'electronics', b'clothing', b'food', b'books', b'toys']
    max_category_len = 32

    # Write binary data
    data_path = output_dir / f"{name}.bin"
    with open(data_path, 'wb') as f:
        for i in range(num_records):
            # Vector field (128 x float32 = 512 bytes)
            vec = np.random.randn(dim).astype(np.float32)
            vec.tofile(f)

            # Category field (fixed 32 bytes, null-padded)
            cat = categories[i % len(categories)]
            cat_padded = cat.ljust(max_category_len, b'\x00')
            f.write(cat_padded)

            # Price field (float64 = 8 bytes)
            price = np.float64(10.0 + i * 0.5)
            f.write(struct.pack('<d', price))

    # Create schema
    schema = {
        'version': 1,
        'metadata': {
            'name': name,
            'description': f'Test hash dataset with vector, category, and price fields'
        },
        'record': {
            'fields': [
                {
                    'name': 'embedding',
                    'type': 'vector',
                    'dtype': 'float32',
                    'dimensions': dim
                },
                {
                    'name': 'category',
                    'type': 'tag',
                    'encoding': 'utf8',
                    'length': 'fixed',
                    'max_bytes': max_category_len
                },
                {
                    'name': 'price',
                    'type': 'numeric',
                    'dtype': 'float64'
                }
            ]
        },
        'sections': {
            'records': {
                'count': num_records
            },
            'keys': {
                'present': False,
                'pattern': 'product:{id}'
            },
            'queries': {
                'present': False
            },
            'ground_truth': {
                'present': False
            }
        }
    }

    schema_path = output_dir / f"{name}.yaml"
    with open(schema_path, 'w') as f:
        yaml.dump(schema, f, default_flow_style=False, sort_keys=False)

    print(f"Created {data_path} ({data_path.stat().st_size} bytes)")
    print(f"Created {schema_path}")

    return schema_path, data_path


def create_small_test_dataset(output_dir: Path):
    """Create a minimal dataset for quick testing."""
    return create_vector_dataset(output_dir, "test_small",
                                 num_vectors=100, num_queries=10,
                                 dim=32, num_neighbors=5)


def create_mnist_like_dataset(output_dir: Path):
    """Create a dataset with MNIST-like dimensions (784-dim, 60K vectors)."""
    return create_vector_dataset(output_dir, "test_mnist",
                                 num_vectors=1000, num_queries=100,
                                 dim=784, num_neighbors=10)


def main():
    parser = argparse.ArgumentParser(description='Create test datasets')
    parser.add_argument('--output-dir', '-o', default='datasets',
                       help='Output directory (default: datasets)')
    parser.add_argument('--type', '-t', choices=['small', 'mnist', 'hash', 'all'],
                       default='all', help='Dataset type to create')

    args = parser.parse_args()
    output_dir = Path(args.output_dir)

    if args.type in ('small', 'all'):
        create_small_test_dataset(output_dir)

    if args.type in ('mnist', 'all'):
        create_mnist_like_dataset(output_dir)

    if args.type in ('hash', 'all'):
        create_hash_dataset(output_dir, "test_hash", num_records=100)

    print("\nDone! Created datasets in", output_dir)


if __name__ == '__main__':
    main()
