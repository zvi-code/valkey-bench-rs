# Dataset Management Guide

Complete guide for downloading, converting, and managing datasets for benchmarking.

## Dataset Formats

valkey-bench-rs supports two dataset formats:

### Schema-Driven Format (Recommended)

The schema-driven format separates metadata (YAML) from data (binary):

```
dataset.yaml  # Schema describing structure, fields, and sections
dataset.bin   # Raw binary data (vectors, keys, values)
```

**Advantages:**
- Human-readable schema for easy inspection
- Zero-copy memory-mapped access
- Supports command replay (SET, HSET, etc.)
- Flexible field types: vector, tag, text, numeric, blob
- No embedded headers in binary file

**Usage:**
```bash
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/mnist.yaml \
  --data datasets/mnist.bin \
  -t vec-load -n 60000 -c 100
```

### Legacy Binary Format

Single binary file with embedded header:
```
dataset.bin  # Header + vectors + queries + ground truth
```

**Usage:**
```bash
./target/release/valkey-bench-rs -h $HOST --cluster \
  --dataset datasets/legacy.bin \
  -t vec-load -n 60000 -c 100
```

## Prerequisites

Before downloading datasets, set up the Python environment:

```bash
# Run the prereq script (one-time setup)
./prep_datasets/prereq-vectordbbench.sh
```

This installs VectorDBBench and dataset conversion dependencies in a virtual environment.

## Quick Start

### Using the Unified Dataset Manager (Recommended)

The simplest way to get datasets is using the shell wrapper:

```bash
# List all available datasets
./prep_datasets/dataset.sh list

# Filter by name
./prep_datasets/dataset.sh list cohere

# Download and convert in one command
./prep_datasets/dataset.sh get mnist                    # Small (60K vectors)
./prep_datasets/dataset.sh get sift-128                 # Medium (1M vectors)
./prep_datasets/dataset.sh get cohere-medium-1m         # Large (1M vectors, 768-dim)
./prep_datasets/dataset.sh get yfcc-10m                 # Extra large (10M vectors with metadata)

# Force re-download
./prep_datasets/dataset.sh get cohere-medium-1m --force

# Verify integrity
./prep_datasets/dataset.sh verify datasets/*.bin
```

The tool automatically:
1. Downloads source files
2. Converts to intermediate HDF5 format (cached)
3. Generates Valkey binary format
4. Verifies data integrity

## Available Datasets

### Small Datasets (< 100K vectors)

Perfect for quick testing and CI/CD:

| Dataset | Vectors | Dimensions | Metric | Size | Command |
|---------|---------|-----------|--------|------|---------|
| mnist | 60K | 784 | L2 | 180MB | `./prep_datasets/dataset.sh get mnist` |
| fashion-mnist | 60K | 784 | L2 | 180MB | `./prep_datasets/dataset.sh get fashion-mnist` |
| cohere-small-100k | 100K | 768 | COSINE | 290MB | `./prep_datasets/dataset.sh get cohere-small-100k` |

### Medium Datasets (1M vectors)

Good balance of scale and manageability:

| Dataset | Vectors | Dimensions | Metric | Size | Command |
|---------|---------|-----------|--------|------|---------|
| sift-128 | 1M | 128 | L2 | 500MB | `./prep_datasets/dataset.sh get sift-128` |
| gist-960 | 1M | 960 | L2 | 3.6GB | `./prep_datasets/dataset.sh get gist-960` |
| glove-25 | 1.18M | 25 | COSINE | 120MB | `./prep_datasets/dataset.sh get glove-25` |
| glove-50 | 1.18M | 50 | COSINE | 240MB | `./prep_datasets/dataset.sh get glove-50` |
| glove-100 | 1.18M | 100 | COSINE | 480MB | `./prep_datasets/dataset.sh get glove-100` |
| cohere-medium-1m | 1M | 768 | COSINE | 2.9GB | `./prep_datasets/dataset.sh get cohere-medium-1m` |

### Large Datasets (5-10M vectors)

Production-scale testing:

| Dataset | Vectors | Dimensions | Metric | Size | Command |
|---------|---------|-----------|--------|------|---------|
| deep-96 | 10M | 96 | COSINE | 3.6GB | `./prep_datasets/dataset.sh get deep-96` |
| bigann-10m | 10M | 128 | L2 | 5GB | `./prep_datasets/dataset.sh get bigann-10m` |
| **yfcc-10m** | 10M | 192 | L2 | 8.1GB | `./prep_datasets/dataset.sh get yfcc-10m` |
| cohere-large-10m | 10M | 768 | COSINE | 29GB | `./prep_datasets/dataset.sh get cohere-large-10m` |
| openai-medium-500k | 500K | 1536 | COSINE | 2.9GB | `./prep_datasets/dataset.sh get openai-medium-500k` |
| openai-large-5m | 5M | 1536 | COSINE | 29GB | `./prep_datasets/dataset.sh get openai-large-5m` |

**Bold** = Includes metadata for filtered search

## Dataset Sources

Datasets come from multiple sources:

### VectorDBBench (COHERE, OPENAI, SIFT, GIST)

Modern embedding datasets from vectordb-bench library:

```bash
# Download using Python script
source venv/bin/activate
python prep_datasets/download_dataset.py COHERE 1000000
python prep_datasets/download_dataset.py OPENAI 5000000

# Or use unified manager
./prep_datasets/dataset.sh get cohere-medium-1m
./prep_datasets/dataset.sh get openai-large-5m
```

**Available sizes:**
- COHERE: 100K, 1M, 10M (768-dim, COSINE)
- OPENAI: 500K, 5M (1536-dim, COSINE)
- SIFT: 500K, 5M (128-dim, L2)
- GIST: 100K, 1M (960-dim, L2)

### ANN-Benchmarks (MNIST, Fashion-MNIST, GloVe)

Standard ML/NLP datasets:

```bash
./prep_datasets/dataset.sh get mnist
./prep_datasets/dataset.sh get glove-100
```

### BigANN (SIFT1B, Deep1B Subsets)

Billion-scale competition datasets:

```bash
./prep_datasets/dataset.sh get bigann-10m
./prep_datasets/dataset.sh get deep-10m
```

**Important**: BigANN subsets require **subset-specific** ground truth files, not the full 1B ground truth!

#### BigANN Ground Truth URLs

- **bigann-10M**: `https://dl.fbaipublicfiles.com/billion-scale-ann-benchmarks/GT_10M/bigann-10M`
- **bigann-100M**: `https://dl.fbaipublicfiles.com/billion-scale-ann-benchmarks/GT_100M/bigann-100M`
- **deep-10M**: `https://dl.fbaipublicfiles.com/billion-scale-ann-benchmarks/GT_10M/deep-10M`

### YFCC-10M (Metadata Filtering)

Special dataset with metadata tags for filtered search:

```bash
./prep_datasets/dataset.sh get yfcc-10m
```

**Metadata features:**
- 10M vectors (192-dim CLIP embeddings)
- 200,386 unique tags (vocabulary)
- 108M tag assignments (~11 tags per vector)
- 100K queries with predicates
- Tags include: years, months, cameras, countries

**Example tags:**
- `year_2015`, `month_April`
- `camera_Canon`, `camera_Nikon`
- `us_state_New_York`, `country_France`

## Manual Conversion Workflow

For custom datasets or understanding the pipeline:

### Step 1: Download Raw Data

```bash
# VectorDBBench datasets
source venv/bin/activate
python prep_datasets/download_dataset.py COHERE 1000000
```

Output: `/mnt/data/datasets/cohere/cohere_medium_1m/` (parquet files)

### Step 2: Convert Parquet to HDF5

```bash
python prep_datasets/convert_parquet_to_hdf5.py \
  /mnt/data/datasets/cohere/cohere_medium_1m \
  /mnt/data/datasets/cohere-medium-1m.hdf5 \
  --name cohere-medium-1m
```

Output: `/mnt/data/datasets/cohere-medium-1m.hdf5`

**HDF5 structure:**
```
/train        - Training vectors [N x dim] float32
/test         - Query vectors [Q x dim] float32
/neighbors    - Ground truth neighbors [Q x k] int32
/distances    - Ground truth distances [Q x k] float32
```

### Step 3: Convert HDF5 to Valkey Binary

```bash
python prep_datasets/prepare_binary.py \
  /mnt/data/datasets/cohere-medium-1m.hdf5 \
  /mnt/data/build-datasets/cohere-medium-1m.bin \
  --metric COSINE \
  --max-neighbors 100
```

Output: `/mnt/data/build-datasets/cohere-medium-1m.bin`

### Automated Conversion

For automatic conversion, use the unified dataset manager:

```bash
./prep_datasets/dataset.sh get cohere-medium-1m
```

This automatically handles all three steps (download, convert to HDF5, convert to binary).

## File Locations

Standard directory structure:

```
/mnt/data/datasets/                     # Raw downloads + HDF5 cache
├── cohere/
│   ├── cohere_small_100k/             # Parquet files
│   ├── cohere_medium_1m/
│   └── cohere_large_10m/
├── cohere-small-100k.hdf5             # Intermediate HDF5
└── cohere-medium-1m.hdf5

/mnt/data/build-datasets/              # Final binary format
├── cohere-small-100k.bin              # Valkey binary
├── cohere-medium-1m.bin
└── cohere-large-10m.bin

datasets/                               # Local symlinks or small datasets
├── mnist.bin
└── ...
```

## Binary Format Specification

Valkey uses a custom memory-mapped binary format:

```
+------------------+
| Header (128 B)   |  magic, dimensions, counts, offsets
+------------------+
| Database Vectors |  N x dim x sizeof(f32)
+------------------+
| Query Vectors    |  Q x dim x sizeof(f32)
+------------------+
| Ground Truth     |  Q x K neighbor IDs
+------------------+
```

**Header structure:**
- Magic: `VDSET001`
- Dimensions, vector count, query count, neighbor count
- Data type (f32, f16, i8, u8, binary)
- Distance metric (L2, Cosine, InnerProduct)

**Memory layout:**
- Training vectors: `num_vectors x dim x float32` (64-byte aligned)
- Query vectors: `num_queries x dim x float32` (64-byte aligned)
- Ground truth neighbors: `num_queries x num_neighbors x int64`

**Alignment:**
- Header: 128 bytes
- Vectors: 64 bytes (cache line aligned)
- Ground truth: Natural alignment

## Storage Management

### Check Space

```bash
# Check available space
df -h /mnt/data

# Check dataset sizes
du -sh /mnt/data/datasets/*
du -sh /mnt/data/build-datasets/*
```

### Clean Cache

```bash
# Remove HDF5 cache (can regenerate)
rm /mnt/data/datasets/*.hdf5

# Remove specific dataset
rm /mnt/data/build-datasets/cohere-large-10m.bin
rm -rf /mnt/data/datasets/cohere/cohere_large_10m
```

### Manage Datasets

```bash
# List all binary datasets
ls -lh /mnt/data/build-datasets/

# Verify dataset integrity
./prep_datasets/dataset.sh verify /mnt/data/build-datasets/*.bin

# Clean all datasets
./prep_datasets/dataset.sh clean
```

## Performance Considerations

### Memory Mapping

Binary files use mmap for zero-copy access:
- Training vectors: Sequential prefill with `MADV_WILLNEED`
- Query vectors: Efficient random access pattern
- Ground truth: Pre-computed for fast recall validation

### Cache Efficiency

- **64-byte alignment**: Optimal for CPU cache lines
- **Sequential layout**: Minimizes page faults

### Storage Recommendations

| Dataset Size | Storage Type | Notes |
|-------------|-------------|-------|
| < 1GB | Local SSD | Fast enough |
| 1-10GB | NVMe | Recommended |
| > 10GB | NVMe | Required for good performance |

## Troubleshooting

### Download Issues

**Error: `ModuleNotFoundError: No module named 'vectordb_bench'`**

```bash
source venv/bin/activate
pip install vectordb-bench==1.0.10
```

**Error: Connection timeout**

```bash
# Retry with longer timeout
export DATASET_TIMEOUT=300
./prep_datasets/dataset.sh get cohere-medium-1m
```

### Conversion Issues

**Error: `OutOfMemoryError` during conversion**

```bash
# Use memory-efficient converter
python prep_datasets/convert_parquet_to_hdf5.py \
  --chunk-size 10000  # Smaller chunks
```

**Error: Invalid binary format**

```bash
# Verify magic number
hexdump -C dataset.bin | head -20
# Should show: VDSET001
```

### Space Issues

**Error: No space left on device**

```bash
# Use NVMe storage (see INSTALLATION.md)
# Or clean up old files
./prep_datasets/dataset.sh clean
```

## Custom Dataset Integration

### Method 1: CommandRecorder API (Recommended)

Use the Python CommandRecorder API to create schema-driven datasets:

```python
from prep_datasets.command_recorder import CommandRecorder, Vector, Tag, Blob, Numeric
import numpy as np

# Create recorder
rec = CommandRecorder(name="my_products")

# Declare schema upfront (optional but recommended)
rec.declare_field("embedding", "vector", dim=128, dtype="float32")
rec.declare_field("category", "tag", max_bytes=32)
rec.declare_field("price", "numeric", dtype="float64")

# Record HSET commands
for i in range(10000):
    vec = np.random.rand(128).astype(np.float32)
    rec.record("HSET", f"product:{i:06d}",
               "embedding", Vector(vec),
               "category", Tag("electronics"),
               "price", Numeric(99.99))

# Generate schema + binary
rec.generate("datasets/my_products")  # Creates .yaml and .bin
```

**For key-value datasets:**

```python
rec = CommandRecorder(name="my_kv")
rec.declare_field("_arg0", "blob", max_bytes=500)

for i in range(3000000):
    value = np.random.bytes(500)
    rec.record("SET", f"key:{i:012d}", Blob(value))

rec.generate("datasets/my_kv")
```

**Usage:**
```bash
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/my_products.yaml \
  --data datasets/my_products.bin \
  -t vec-load -n 10000 -c 50
```

See `examples/create_kv_dataset.py` for a complete working example.

### Method 2: HDF5 Conversion (Vector Datasets)

For standard vector datasets in HDF5 format:

1. **Prepare HDF5** with required structure:
   ```python
   import h5py
   with h5py.File('custom.hdf5', 'w') as f:
       f.create_dataset('train', data=train_vectors)  # [N, dim]
       f.create_dataset('test', data=test_vectors)    # [Q, dim]
       f.create_dataset('neighbors', data=gt_neighbors)  # [Q, k]
       f.create_dataset('distances', data=gt_distances)  # [Q, k]
   ```

2. **Convert to schema-driven format**:
   ```bash
   python prep_datasets/prepare_schema_binary.py hdf5 \
     custom.hdf5 datasets/custom \
     --name custom --metric L2 --max-neighbors 100
   ```

3. **Verify**:
   ```bash
   ./prep_datasets/dataset.sh verify datasets/custom.yaml
   ```

## Schema Format Reference

### Complete Schema Structure

```yaml
version: 1

metadata:
  name: dataset_name                    # Required
  description: Human-readable description

# Command replay (for SET, HSET commands)
replay:
  command: HSET                         # SET or HSET

# Record structure
record:
  fields:
  - name: field_name
    type: vector|tag|text|numeric|blob
    # Type-specific options below

# Data sections
sections:
  records:
    count: 60000                        # Number of records
  keys:
    present: true|false                 # Keys stored in binary?
    pattern: "vec:%012d"                # Key pattern for generated keys
    encoding: utf8
    max_bytes: 32
  queries:
    present: true|false
    count: 10000
  ground_truth:
    present: true|false
    neighbors_per_query: 100
    id_type: u64|u32

# Optional field metadata (for index creation hints)
field_metadata:
  embedding:
    distance_metric: l2|cosine|ip
  category:
    index_type: tag
```

### Field Types

| Type | Options | Binary Format |
|------|---------|---------------|
| `vector` | `dtype: float32\|float16`, `dimensions: N` | N x sizeof(dtype) |
| `tag` | `max_bytes: N`, `encoding: utf8` | Fixed N bytes |
| `text` | `max_bytes: N`, `encoding: utf8` | Fixed N bytes |
| `numeric` | `dtype: float64\|int64\|int32` | sizeof(dtype) |
| `blob` | `max_bytes: N` | Fixed N bytes |

### Key Patterns

Use patterns for automatic key generation:

| Pattern | Example | Description |
|---------|---------|-------------|
| `%012d` | `000000000001` | Zero-padded record ID |
| `prefix:%06d` | `vec:000001` | Prefix with formatted ID |

Example: `vec:%012d` generates keys like `vec:000000000042`

### Example Schemas

**Vector Search Dataset:**
```yaml
version: 1
metadata:
  name: mnist
  description: 'MNIST: 60,000 vectors, 784 dimensions'
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
    present: false
    pattern: vec:%012d
  queries:
    present: true
    count: 10000
  ground_truth:
    present: true
    neighbors_per_query: 100
field_metadata:
  embedding:
    distance_metric: l2
```

**Key-Value Dataset:**
```yaml
version: 1
metadata:
  name: kv_3m
  description: '3M SET commands'
replay:
  command: SET
record:
  fields:
  - name: _arg0
    type: blob
    max_bytes: 500
sections:
  records:
    count: 3000000
  keys:
    present: true
    max_bytes: 32
```

**E-commerce Product Catalog:**
```yaml
version: 1
metadata:
  name: product_catalog
  description: 'Product embeddings with metadata'
replay:
  command: HSET
record:
  fields:
  - name: embedding
    type: vector
    dtype: float32
    dimensions: 128
  - name: category
    type: tag
    max_bytes: 32
  - name: price
    type: numeric
    dtype: float64
  - name: description
    type: text
    max_bytes: 256
sections:
  records:
    count: 100000
  keys:
    present: true
    max_bytes: 32
field_metadata:
  embedding:
    distance_metric: cosine
  category:
    index_type: tag
```

## Next Steps

- **Example Datasets**: See [examples/](examples/) for sample datasets and Python scripts
- **Running Benchmarks**: See [BENCHMARKING.md](BENCHMARKING.md)
- **Advanced Features**: See [ADVANCED.md](ADVANCED.md) for metadata filtering
- **Examples**: See [EXAMPLES.md](EXAMPLES.md) for comprehensive benchmark examples
- **Installation**: See [INSTALLATION.md](INSTALLATION.md) for environment setup
