# Advanced Features Guide

Guide to advanced benchmarking features including optimizer internals, metadata filtering, and data format specifications.

## Table of Contents

1. [Optimizer Internals](#optimizer-internals)
2. [Metadata Filtering](#metadata-filtering)
3. [Binary Format Specification](#binary-format-specification)
4. [Extending the Benchmark](#extending-the-benchmark)

---

## Optimizer Internals

The optimizer uses a multi-phase approach to find optimal configurations automatically.

### Optimization Phases

#### Phase 1: Feasibility

Tests maximum parameter values to establish an upper bound for the search space.

**Algorithm:**
```
Test with all parameters at maximum values
Establish baseline for best possible performance
Verify constraints can be satisfied
```

#### Phase 2: Exploration (Grid Search)

Samples the parameter space at key points (min, 25%, 50%, 75%, max) to understand the response surface.

**Algorithm:**
```
For each parameter:
    Test at boundary values: min, 25%, 50%, 75%, max
    Track best configuration seen

Total iterations: ~5 x number_of_parameters
```

#### Phase 3: Exploitation (Hill Climbing)

Fine-tunes all parameters using gradient-based optimization with multiple step sizes.

**Algorithm:**
```
current_config = best from exploration
step_sizes = [1x, 2x, 3x] * base_step

while not converged:
    for each step_size in step_sizes:
        for each direction in [+step, -step]:
            test neighbor

    if improvement found:
        move to best neighbor
    else:
        converged
```

**Typical iterations:** 10-15 until convergence

### Objective and Constraint System

**Single Objective:**
```bash
--objective "maximize:qps"
--objective "minimize:p99_ms"
```

**Multi-Objective (ordered goals):**
```bash
# Max QPS first, then minimize p99 as tiebreaker
--objective "maximize:qps,minimize:p99_ms" --tolerance 0.04
```
Configs within tolerance (4%) on primary goal are compared by secondary goals.

**Bounded Objective:**
```bash
# Max QPS but must stay under 1M req/s
--objective "maximize:qps:lt:1000000"
```

**Constraints:**
```bash
--constraint "recall:gt:0.95"       # recall > 95%
--constraint "p99_ms:lt:1.0"        # p99 latency < 1ms
--constraint "qps:gte:100000"       # QPS >= 100K
```

### Tunable Parameters

| Parameter | Typical Range | Effect |
|-----------|---------------|--------|
| `clients` | 10-500 | Parallelism |
| `threads` | 1-32 | Worker threads |
| `pipeline` | 1-100 | Commands per batch |
| `ef_search` | 10-500 | Search quality vs speed |

**Parameter format:** `name:min:max:step`

**Examples:**
```bash
--tune "clients:10:300:10"
--tune "threads:1:32:1"
--tune "ef_search:10:500:10"
--tune "pipeline:1:20:1"
```

### Optimizer Output

The optimizer prints one line per iteration:

```
=== OPTIMIZATION MODE ===

Objectives: maximize:qps, minimize:p99_ms
  Tolerance: 4.0%
Constraints:
  - p99_ms < 1.0
Parameters to tune:
  - clients: 10 to 300 step 10
  - threads: 1 to 32 step 1
Max iterations: 50

[ 1] Feasibility   | {clients=300, threads=32} | 892K req/s p99=0.52ms *BEST*
[ 2] Exploration   | {clients=10, threads=1}   | 245K req/s p99=0.12ms
[ 3] Exploration   | {clients=155, threads=16} | 756K req/s p99=0.38ms
...
[25] Exploitation  | {clients=275, threads=24} | 1.04M req/s p99=0.41ms *BEST*

=== Best Configuration ===
Config: {clients=275, threads=24}
qps: 1041234.56

=== Recommended Command Line ===
./target/release/valkey-bench-rs -h $HOST --cluster -t get -c 275 --threads 24 -n 1000000
```

---

## Metadata Filtering

Support for filtered vector search with metadata tags (e.g., BigANN YFCC-10M format).

### Concepts

**Vector Metadata (Tags):**
- Each vector has 0+ tags from a vocabulary
- Example: Vector #5234 has tags: `[camera:Canon, year:2015, country:USA]`

**Query Predicates:**
- Each query has 1-2 tags that MUST match
- Example: Query #42 has predicates: `[year:2015, country:USA]`

**Filtering Logic:**
```python
# Find k-NN among vectors matching ALL predicates
matching_vectors = [
    i for i in range(num_vectors)
    if all(tag in vector_tags[i] for tag in query_predicates)
]
# Then search only within matching_vectors
```

### YFCC-10M Dataset

Special dataset with rich metadata:

- **10M vectors** (192-dim CLIP embeddings)
- **200K unique tags** (vocabulary)
- **108M tag assignments** (~11 tags per vector average)
- **100K queries** with 1-2 predicates each

**Tag categories:**
- Years: `year_2009`, `year_2010`, ...
- Months: `month_January`, `month_April`, ...
- Cameras: `camera_Canon`, `camera_Nikon`, ...
- Countries: `us_state_New_York`, `country_France`, ...

### Download and Convert

```bash
# Download YFCC-10M with metadata
./prep_datasets/dataset.sh get yfcc-10m
```

### Tag-Based Benchmarking

#### Loading Vectors with Tags

```bash
# Load vectors with tag distribution
./target/release/valkey-bench-rs -h $HOST --cluster -t vec-load \
  --dataset datasets/vectors.bin \
  --search-prefix "doc:" \
  --search-index myindex \
  --tag-field category \
  --search-tags "electronics:30,clothing:25,home:20,sports:15,other:10" \
  -n 100000 -c 10
```

**Tag distribution format:** `tag:probability,...`
- Probability is 0-100 (percentage)
- Each tag is selected independently

#### Querying with Filters

```bash
# Single tag filter
./target/release/valkey-bench-rs -h $HOST --cluster -t vec-query \
  --dataset datasets/vectors.bin \
  --search-index myindex \
  --tag-field category \
  --tag-filter "electronics" \
  -k 10 -n 10000

# Multiple tags (OR condition)
./target/release/valkey-bench-rs -h $HOST --cluster -t vec-query \
  --dataset datasets/vectors.bin \
  --search-index myindex \
  --tag-field category \
  --tag-filter "electronics|clothing|home" \
  -k 10 -n 10000
```

### Numeric Field Filtering

#### Loading with Numeric Fields

```bash
./target/release/valkey-bench-rs -h $HOST --cluster -t vec-load \
  --dataset datasets/vectors.bin \
  --search-index myindex \
  --tag-field category \
  --search-tags "electronics:40,clothing:30" \
  --numeric-field-config "price:float:uniform:9.99:499.99:2" \
  --numeric-field-config "rating:float:normal:4.0:0.5:1" \
  -n 100000 -c 10
```

**Numeric field format:** `name:type:distribution:params`

| Type | Description |
|------|-------------|
| `int` | Integer values |
| `float` or `float:N` | Float with N decimals |
| `unix_timestamp` | Unix timestamp |
| `iso_datetime` | ISO 8601 datetime |
| `date_only` | Date only |

| Distribution | Format |
|--------------|--------|
| `uniform` | `uniform:min:max` |
| `zipfian` | `zipfian:skew:min:max` |
| `normal` | `normal:mean:stddev` |
| `sequential` | `sequential:start:step` |
| `constant` | `constant:value` |
| `key_based` | `key_based:min:max` |

#### Querying with Numeric Filters

```bash
# Inclusive range
--numeric-filter "price:[10,100]"

# Exclusive bounds
--numeric-filter "price:(10,100)"

# Unbounded
--numeric-filter "rating:[-inf,4.5]"
--numeric-filter "count:[100,+inf)"

# Multiple filters (AND logic)
--numeric-filter "price:[10,100]" --numeric-filter "rating:[4.0,5.0]"
```

---

## Binary Format Specification

Complete specification of the Valkey dataset binary format.

### Header (128 bytes)

```
Magic:      8 bytes  "VDSET001"
Version:    4 bytes  1 or 2
Dimensions: 4 bytes
Num Vectors: 8 bytes
Num Queries: 8 bytes
Num Neighbors: 4 bytes
Data Type:  1 byte   (0=f32, 1=f16, 2=i8, 3=u8, 4=binary)
Metric:     1 byte   (0=L2, 1=IP, 2=Cosine)
Reserved:   90 bytes
```

### Memory Layout

```
Offset 0:      [Header: 128 bytes]
Offset 128:    [Training vectors: num_vectors x dim x 4 bytes, 64-byte aligned]
Offset X:      [Query vectors: num_queries x dim x 4 bytes, 64-byte aligned]
Offset Y:      [Ground truth: num_queries x num_neighbors x 8 bytes]
```

### Alignment

- Header: 128 bytes
- Vectors: 64 bytes (cache line aligned)
- Ground truth: Natural alignment

### Example Sizes

**SIFT-128 (1M vectors):**
```
Header:          128 bytes
Vectors:         1,000,000 x 128 x 4 = 512 MB
Queries:         10,000 x 128 x 4 = 5 MB
Ground truth:    10,000 x 100 x 8 = 8 MB
Total:           ~525 MB
```

### Version 2 (with Metadata)

Version 2 extends the header with metadata fields for filtered search:

```
has_metadata:      1 byte
vocab_size:        4 bytes
vector_meta_off:   8 bytes
query_meta_off:    8 bytes
vocabulary_off:    8 bytes
```

**Additional sections:**
- Vector Metadata CSR (sparse matrix)
- Query Metadata CSR (predicates)
- Vocabulary (tag names)

### File Verification

```bash
# Verify magic number
hexdump -C dataset.bin | head -1
# Should show: VDSET001

# Use dataset manager
./prep_datasets/dataset.sh verify dataset.bin
```

### Python Creation

```python
import struct
import numpy as np

def write_dataset(output_path, vectors, queries, neighbors, metric=0):
    with open(output_path, 'wb') as f:
        # Header
        f.write(b'VDSET001')  # magic
        f.write(struct.pack('<I', 1))  # version
        f.write(struct.pack('<I', vectors.shape[1]))  # dim
        f.write(struct.pack('<Q', len(vectors)))  # num_vectors
        f.write(struct.pack('<Q', len(queries)))  # num_queries
        f.write(struct.pack('<I', neighbors.shape[1]))  # num_neighbors
        f.write(struct.pack('<B', 0))  # data_type (f32)
        f.write(struct.pack('<B', metric))  # metric
        f.write(b'\x00' * 90)  # reserved

        # Vectors (64-byte aligned)
        vectors.astype(np.float32).tofile(f)
        pad_to_64(f)

        # Queries
        queries.astype(np.float32).tofile(f)
        pad_to_64(f)

        # Ground truth
        neighbors.astype(np.int64).tofile(f)

def pad_to_64(f):
    pos = f.tell()
    pad = (64 - (pos % 64)) % 64
    f.write(b'\x00' * pad)
```

---

## Extending the Benchmark

### Adding Custom Workloads

The benchmark supports several extension points:

1. **Parallel workloads**: Mix traffic with weighted distribution
   ```bash
   --parallel "get:0.8,set:0.2"
   ```

2. **Composite workloads**: Sequential phases
   ```bash
   --composite "vec-load:10000,vec-query:1000"
   ```

3. **Iteration strategies**: Custom key access patterns
   ```bash
   --iteration "zipfian:1.5"
   --iteration "subset:0:10000"
   ```

4. **Address types**: Hash fields and JSON paths
   ```bash
   --address-type "hash:prefix:field1,field2"
   --address-type "json:prefix:$.path1,$.path2"
   ```

### Custom Datasets

1. **Prepare HDF5** with required structure:
   ```python
   import h5py
   with h5py.File('custom.hdf5', 'w') as f:
       f.create_dataset('train', data=train_vectors)
       f.create_dataset('test', data=test_vectors)
       f.create_dataset('neighbors', data=gt_neighbors)
       f.create_dataset('distances', data=gt_distances)
   ```

2. **Convert to binary**:
   ```bash
   python prep_datasets/prepare_binary.py \
     custom.hdf5 custom.bin \
     --metric L2 --max-neighbors 100
   ```

3. **Verify and use**:
   ```bash
   ./prep_datasets/dataset.sh verify custom.bin
   ./target/release/valkey-bench-rs --dataset custom.bin -t vec-query ...
   ```

---

## Next Steps

- **Benchmarking**: See [BENCHMARKING.md](BENCHMARKING.md) for running benchmarks
- **Datasets**: See [DATASETS.md](DATASETS.md) for dataset management
- **Installation**: See [INSTALLATION.md](INSTALLATION.md) for setup
- **Examples**: See [EXAMPLES.md](EXAMPLES.md) for comprehensive examples
