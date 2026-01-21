# Valkey Search Benchmark Examples

This document provides comprehensive examples for all benchmark features, organized by category. For basic usage and API reference, see the [README.md](../README.md).

## Prerequisites

Set these environment variables before running the examples:

```bash
# Required: Cluster endpoint
export HOST="your-cluster.example.com"

# Required for vector search examples: Dataset path
export DATASET="/path/to/your/vectors.bin"
```

## Table of Contents

- [Quick Start](#quick-start)
- [Schema-Driven Datasets](#schema-driven-datasets)
- [Creating Custom Datasets](#creating-custom-datasets)
- [Parallel Workloads (Mixed Traffic)](#parallel-workloads-mixed-traffic)
- [Composite Workloads (Sequential Phases)](#composite-workloads-sequential-phases)
- [Iteration Strategies](#iteration-strategies)
- [Addressable Spaces (Hash Fields, JSON Paths)](#addressable-spaces-hash-fields-json-paths)
- [Numeric Field Configurations](#numeric-field-configurations)
- [Tag-Based Filtered Search](#tag-based-filtered-search)
- [Query-Side Numeric Filters](#query-side-numeric-filters)
- [Vector Search Workloads](#vector-search-workloads)
- [Real-World Dataset: Cohere-Medium-1M](#real-world-dataset-cohere-medium-1m)
- [Parameter Optimization](#parameter-optimization)
- [Performance Tuning Recipes](#performance-tuning-recipes)

---

## Quick Start

```bash
# Build the benchmark tool
cd valkey-bench-rs
cargo build --release

# Basic connectivity test
./target/release/valkey-bench-rs -h $HOST -t ping -n 10000

# Cluster mode with auto-discovery
./target/release/valkey-bench-rs -h $HOST --cluster -t ping

# Interactive CLI mode
./target/release/valkey-bench-rs --cli -h $HOST
```

---

## Schema-Driven Datasets

The schema-driven format uses separate YAML schema and binary data files for flexible dataset configuration.

### Basic Usage

```bash
# Set paths for schema-driven datasets
export SCHEMA="datasets/mnist.yaml"
export DATA="datasets/mnist.bin"

# Load vectors using schema + data files
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-load -n 60000 -c 100 --threads 16

# Query with recall measurement
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 10000 -c 50 --search-index mnist_idx
```

### Vector Dataset with Ground Truth

```bash
# Load MNIST vectors (60,000 training vectors)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/mnist.yaml \
  --data datasets/mnist.bin \
  -t vec-load -n 60000 -c 100 --threads 16 \
  --search-index mnist_idx --search-prefix "vec:"

# Query with automatic recall computation (uses 10,000 query vectors)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/mnist.yaml \
  --data datasets/mnist.bin \
  -t vec-query -n 10000 -c 50 \
  --search-index mnist_idx --ef-search 100 -k 10
```

### Key-Value Dataset Replay

```bash
# Replay recorded SET commands (3M keys with 500-byte values)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/kv_3m.yaml \
  --data datasets/kv_3m.bin \
  -t set -n 3000000 -c 200 --threads 16

# Run GET benchmark on same keyspace
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 3000000 -r 3000000 -c 500 --threads 52
```

### E-commerce Product Catalog

```bash
# Load products with embeddings, categories, and prices
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/product_catalog.yaml \
  --data datasets/product_catalog.bin \
  -t vec-load -n 100000 -c 100 \
  --search-index products --search-prefix "product:"

# Query with category filter
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/product_catalog.yaml \
  --data datasets/product_catalog.bin \
  -t vec-query -n 10000 -c 50 \
  --search-index products \
  --tag-field category --tag-filter "electronics"
```

---

## Creating Custom Datasets

Use the CommandRecorder API to create custom datasets for your specific benchmarking needs.

### Using create_kv_dataset.py Example

```bash
cd prep_datasets

# Generate 3M keys with 500-byte values
python create_kv_dataset.py -o ../datasets/kv_3m -n 3000000 -d 500

# Generate smaller test dataset
python create_kv_dataset.py -o ../datasets/kv_test -n 10000 -d 100

# Custom prefix
python create_kv_dataset.py -o ../datasets/users -n 100000 -d 256 --prefix "user:"
```

### CommandRecorder API: Key-Value Dataset

```python
from command_recorder import CommandRecorder, Blob
import numpy as np

# Create recorder
rec = CommandRecorder(name="my_kv_dataset")

# Declare schema upfront (recommended for large datasets)
rec.declare_field("_arg0", "blob", max_bytes=500)

# Record SET commands
for i in range(3000000):
    value = np.random.bytes(500)
    rec.record("SET", f"key:{i:012d}", Blob(value))

# Generate schema YAML + binary data
rec.generate("datasets/my_kv")  # Creates my_kv.yaml + my_kv.bin
```

### CommandRecorder API: Vector Search Dataset

```python
from command_recorder import CommandRecorder, Vector, Tag, Numeric
import numpy as np

rec = CommandRecorder(name="product_embeddings")

# Declare fields upfront
rec.declare_field("embedding", "vector", dim=768, dtype="float32")
rec.declare_field("category", "tag", max_bytes=32)
rec.declare_field("price", "numeric", dtype="float64")

# Record HSET commands
np.random.seed(42)
categories = ["electronics", "clothing", "home", "sports", "books"]

for i in range(100000):
    vec = np.random.randn(768).astype(np.float32)
    category = np.random.choice(categories)
    price = np.random.uniform(9.99, 999.99)

    rec.record("HSET", f"product:{i:06d}",
               "embedding", Vector(vec),
               "category", Tag(category),
               "price", Numeric(price))

rec.generate("datasets/products")
```

### CommandRecorder API: With Query Vectors

```python
from command_recorder import CommandRecorder, Vector
import numpy as np

rec = CommandRecorder(name="vectors_with_queries")
rec.declare_field("embedding", "vector", dim=128, dtype="float32")

# Training vectors
np.random.seed(42)
for i in range(10000):
    vec = np.random.randn(128).astype(np.float32)
    rec.record("HSET", f"vec:{i:06d}", "embedding", Vector(vec))

# Add query vectors (optional)
rec.add_queries([np.random.randn(128).astype(np.float32) for _ in range(100)])

# Add ground truth neighbors (optional, for recall computation)
# Each entry is a list of neighbor IDs for the corresponding query
rec.add_ground_truth([[i*10, i*10+1, i*10+2] for i in range(100)])

rec.generate("datasets/vectors")
```

### Using Generated Datasets

```bash
# Use the generated dataset
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/products.yaml \
  --data datasets/products.bin \
  -t vec-load -n 100000 -c 100 --threads 16 \
  --search-index products --search-prefix "product:"

# Query with filters
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema datasets/products.yaml \
  --data datasets/products.bin \
  -t vec-query -n 10000 -c 50 \
  --search-index products \
  --tag-field category --tag-filter "electronics|clothing" \
  --numeric-filter "price:[10,100]"
```

---

## Parallel Workloads (Mixed Traffic)

The `--parallel` flag enables running multiple workload types simultaneously with weighted traffic distribution. This is useful for simulating real-world mixed read/write patterns.

### Basic Syntax

```
--parallel "workload1:weight1,workload2:weight2,..."
```

Weights are normalized automatically - they don't need to sum to 1.0.

### Examples

#### Mixed GET/SET Traffic

```bash
# 80% reads, 20% writes
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:80,set:20" \
  -n 1000000 -r 1000000 -c 100 --threads 8

# Equal read/write split
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:50,set:50" \
  -n 500000 -r 100000 -c 50

# Read-heavy with occasional writes (95/5)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:95,set:5" \
  -n 2000000 -r 500000 -c 200 --threads 16
```

#### Multiple Workload Types

```bash
# GET, SET, and INCR mixed traffic
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:60,set:25,incr:15" \
  -n 1000000 -r 100000 -c 100

# List operations mix
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "lpush:40,rpush:30,lpop:15,rpop:15" \
  -n 500000 -r 10000 -c 50

# Hash and sorted set mix
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "hset:50,zadd:30,sadd:20" \
  -n 500000 -r 50000 -c 100 -d 64
```

#### Extreme Ratios

```bash
# Write-heavy (for replication testing)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "set:90,get:10" \
  -n 1000000 -r 1000000 -d 512 -c 100

# Read-only with ping (connection testing)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:80,ping:20" \
  -n 1000000 -r 100000 -c 50
```

#### Rate-Limited Mixed Traffic

```bash
# Mixed traffic at 50,000 requests per second
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:70,set:30" \
  --rps 50000 -n 500000 -r 100000 -c 100
```

---

## Composite Workloads (Sequential Phases)

The `--composite` flag runs workloads in sequence, useful for setup-then-test patterns like loading data before querying.

### Basic Syntax

```
--composite "workload1:count1,workload2:count2,..."
```

Each phase runs to completion before the next begins.

### Examples

#### Vector Load Then Query

```bash
# Load 10,000 vectors, then run 1,000 queries
./target/release/valkey-bench-rs -h $HOST --cluster \
  --composite "vec-load:10000,vec-query:1000" \
  --dataset $DATASET \
  --search-index mnist_idx \
  --search-prefix "vec:" \
  -c 50 --threads 4
```

#### Warmup Then Benchmark

```bash
# SET 100K keys as warmup, then measure GET performance
./target/release/valkey-bench-rs -h $HOST --cluster \
  --composite "set:100000,get:1000000" \
  -r 100000 -d 100 -c 100 --threads 8
```

#### Multi-Phase Data Pipeline

```bash
# Load -> Query -> Update -> Query (measure impact of updates)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --composite "vec-load:50000,vec-query:5000,vec-update:10000,vec-query:5000" \
  --dataset $DATASET \
  --search-index sift_idx \
  --search-prefix "sift:" \
  -c 100 --threads 8
```

#### CRUD Lifecycle Test

```bash
# Create keys, read them
./target/release/valkey-bench-rs -h $HOST --cluster \
  --composite "set:50000,get:100000" \
  -r 50000 -d 256 -c 50
```

---

## Iteration Strategies

The `--iteration` flag controls how keys are generated and accessed across the keyspace.

### Basic Syntax

```
--iteration "strategy[:params]"
```

### Available Strategies

| Strategy | Syntax | Description |
|----------|--------|-------------|
| Sequential | `sequential` or `seq` | Keys 0, 1, 2, ... N-1 in order |
| Random | `random` or `random:SEED` | Uniform random with optional seed |
| Subset | `subset:START:END` | Only use keys in range [START, END) |
| Zipfian | `zipfian:SKEW` or `zipfian:SKEW:SEED` | Power-law distribution |

### Examples

#### Sequential Access

```bash
# Sequential keys for cache warming (100% unique keys)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t set -n 1000000 -r 1000000 \
  --iteration "sequential" \
  -c 100 --threads 8

# Shorthand
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 1000000 \
  --iteration "seq"
```

#### Random Access with Seed

```bash
# Random keys with default seed (12345)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 1000000 \
  --iteration "random"

# Random with specific seed for reproducibility
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 1000000 \
  --iteration "random:42"
```

#### Subset Range

```bash
# Only access keys 1000-5999 (partial keyspace)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 100000 -r 10000 \
  --iteration "subset:1000:6000" \
  -c 50

# Access first 10% of keyspace
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 500000 -r 1000000 \
  --iteration "subset:0:100000" \
  -c 100

# Access last 20% of keyspace
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 200000 -r 1000000 \
  --iteration "subset:800000:1000000"
```

#### Zipfian (Hot-Spot) Distribution

```bash
# Light skew (skew=0.5) - some keys accessed more than others
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 100000 \
  --iteration "zipfian:0.5" \
  -c 100

# Medium skew (skew=1.0) - moderate hot-spot pattern
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 100000 \
  --iteration "zipfian:1.0" \
  -c 100

# Heavy skew (skew=1.5) - few keys get most traffic
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 100000 \
  --iteration "zipfian:1.5" \
  -c 100

# Zipfian with specific seed
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 100000 \
  --iteration "zipfian:1.0:99999"
```

#### Combining with Other Options

```bash
# Zipfian access pattern with rate limiting
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 500000 -r 50000 \
  --iteration "zipfian:1.2" \
  --rps 100000 -c 100

# Sequential SET then random GET
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t set -n 100000 -r 100000 \
  --iteration "sequential" -c 100

./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 500000 -r 100000 \
  --iteration "random" -c 100
```

---

## Addressable Spaces (Hash Fields, JSON Paths)

The `--address-type` flag enables benchmarking operations across hash fields or JSON paths, not just top-level keys.

### Basic Syntax

```
--address-type "type:prefix:fields_or_paths"
```

### Available Types

| Type | Syntax | Description |
|------|--------|-------------|
| Key | `key` or `key:prefix` | Simple key-value (default) |
| Hash Field | `hash:prefix:field1,field2,...` | Iterate over hash fields |
| JSON Path | `json:prefix:$.path1,$.path2,...` | Iterate over JSON paths |

### Examples

#### Hash Field Iteration

```bash
# HSET across 3 fields per key
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t hset \
  --address-type "hash:user:name,email,age" \
  -n 30000 -r 10000 -d 64 -c 50

# Result: 10,000 keys, each with 3 fields (30,000 total operations)
# Verify: HLEN user:000000000001 = 3

# Hash with more fields for wide objects
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t hset \
  --address-type "hash:product:id,name,price,category,stock,rating,created,updated" \
  -n 80000 -r 10000 -d 32 -c 100

# Result: 10,000 keys with 8 fields each

# Hash field iteration with custom prefix
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t hset \
  --address-type "hash:session:user_id,token,expires,data" \
  -n 40000 -r 10000 -d 128 -c 50
```

#### JSON Path Iteration

```bash
# JSON.SET across multiple paths
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t hset \
  --address-type "json:doc:$.name,$.email,$.profile.bio" \
  -n 30000 -r 10000 -d 100 -c 50

# Nested JSON paths
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t hset \
  --address-type "json:event:$.type,$.data.user_id,$.data.timestamp,$.metadata.source" \
  -n 40000 -r 10000 -d 64 -c 50
```

#### Combining Address Types with Iteration

```bash
# Sequential hash field population
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t hset \
  --address-type "hash:obj:f1,f2,f3,f4,f5" \
  --iteration "sequential" \
  -n 50000 -r 10000 -d 64 -c 50

# Zipfian access to specific fields (hot fields)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t hset \
  --address-type "hash:cache:hits,misses,size,ttl" \
  --iteration "zipfian:1.0" \
  -n 40000 -r 10000 -d 16 -c 50
```

---

## Numeric Field Configurations

The `--numeric-field-config` flag enables adding numeric fields with configurable value types and distributions during vector loading.

### Basic Syntax

```
--numeric-field-config "name:type:distribution:params..."
```

### Value Types

| Type | Description | Example Output |
|------|-------------|----------------|
| `int` | Integer values | `42`, `1000` |
| `float` or `float:N` | Float with N decimals (default 6) | `123.45` |
| `unix_timestamp` | Unix seconds since epoch | `1703001234` |
| `iso_datetime` | ISO 8601 datetime | `2024-12-19T15:30:45Z` |
| `date_only` | Date only (YYYY-MM-DD) | `2024-12-19` |

### Distributions

| Distribution | Syntax | Description |
|--------------|--------|-------------|
| Uniform | `uniform:min:max` | Even spread between min and max |
| Zipfian | `zipfian:skew:min:max` | Power-law (skew 0.5-2.0) |
| Normal | `normal:mean:stddev` | Gaussian/bell curve |
| Sequential | `sequential:start:step` | Incrementing values |
| Constant | `constant:value` | Fixed value for all |
| Key-based | `key_based:min:max` | Derived from key number |

### Examples

#### E-Commerce Product Catalog

```bash
# Products with price, quantity, rating
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "product:" \
  --search-index product_idx \
  --numeric-field-config "price:float:uniform:9.99:999.99:2" \
  --numeric-field-config "quantity:int:zipfian:1.5:0:1000" \
  --numeric-field-config "rating:float:normal:4.0:0.8:1" \
  -n 100000 -c 50
```

#### Time-Series Data

```bash
# Events with timestamps
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "event:" \
  --search-index event_idx \
  --numeric-field-config "created_at:unix_timestamp:uniform:1672531200:1735689600" \
  --numeric-field-config "event_type:int:zipfian:2.0:1:10" \
  --numeric-field-config "priority:int:uniform:1:5" \
  -n 50000 -c 100

# With ISO datetime
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "log:" \
  --search-index log_idx \
  --numeric-field-config "timestamp:iso_datetime:uniform:1672531200:1735689600" \
  --numeric-field-config "severity:int:zipfian:1.5:1:5" \
  -n 100000 -c 50
```

#### Sequential IDs

```bash
# Documents with auto-incrementing IDs
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "doc:" \
  --search-index doc_idx \
  --numeric-field-config "seq_id:int:sequential:1:1" \
  --numeric-field-config "version:int:constant:1" \
  -n 100000 -c 50
```

#### Financial Data

```bash
# Transactions with amounts and timestamps
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "txn:" \
  --search-index txn_idx \
  --numeric-field-config "amount:float:zipfian:1.2:0.01:10000.00:2" \
  --numeric-field-config "timestamp:unix_timestamp:sequential:1704067200:1" \
  --numeric-field-config "account_id:int:uniform:1000:9999" \
  -n 500000 -c 100 --threads 8
```

#### Sensor Data

```bash
# IoT sensor readings
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "sensor:" \
  --search-index sensor_idx \
  --numeric-field-config "temperature:float:normal:22.0:5.0:1" \
  --numeric-field-config "humidity:float:uniform:30.0:90.0:1" \
  --numeric-field-config "pressure:float:normal:1013.25:10.0:2" \
  --numeric-field-config "battery:int:uniform:0:100" \
  -n 100000 -c 50
```

---

## Tag-Based Filtered Search

Use `--tag-field`, `--search-tags`, and `--tag-filter` for tag-based vector filtering.

### Examples

#### Single Category

```bash
# Load vectors with category tags
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "prod:" \
  --search-index prod_idx \
  --tag-field category \
  --search-tags "electronics:100" \
  -n 50000 -c 50

# Query only electronics
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index prod_idx \
  --tag-field category \
  --tag-filter "electronics" \
  -k 10 -n 10000 -c 50
```

#### Multiple Categories with Probabilities

```bash
# Mixed categories
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "prod:" \
  --search-index prod_idx \
  --tag-field category \
  --search-tags "electronics:30,clothing:25,home:20,sports:15,other:10" \
  -n 100000 -c 100

# Query multiple categories (OR)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index prod_idx \
  --tag-field category \
  --tag-filter "electronics|clothing|home" \
  -k 10 -n 10000 -c 50
```

#### Combined Tags and Numeric Fields

```bash
# Full e-commerce setup
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "prod:" \
  --search-index prod_idx \
  --tag-field category \
  --search-tags "electronics:40,accessories:35,cables:25" \
  --numeric-field-config "price:float:uniform:4.99:299.99:2" \
  --numeric-field-config "rating:float:normal:4.2:0.5:1" \
  -n 100000 -c 100 --threads 8
```

---

## Query-Side Numeric Filters

Use `--numeric-filter` to add numeric range constraints to FT.SEARCH queries. This filters results by numeric field values at query time.

### Basic Syntax

```
--numeric-filter "field:[min,max]"    # Inclusive bounds
--numeric-filter "field:(min,max)"    # Exclusive bounds
--numeric-filter "field:[min,max)"    # Mixed bounds
--numeric-filter "field:[-inf,max]"   # Unbounded lower
--numeric-filter "field:[min,+inf)"   # Unbounded upper
```

Multiple filters can be combined - all must match (AND logic).

### Examples

#### Single Numeric Filter

```bash
# Create index with numeric field
./target/release/valkey-bench-rs --cli -h $HOST --cluster \
  "FT.CREATE filtered_idx ON HASH PREFIX 1 vec: SCHEMA embedding VECTOR HNSW 6 TYPE FLOAT32 DIM 784 DISTANCE_METRIC L2 score NUMERIC"

# Load vectors with numeric field
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "vec:" \
  --search-index filtered_idx \
  --numeric-field-config "score:int:uniform:0:100" \
  -n 50000 -c 50

# Query with score filter [50, 100]
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index filtered_idx \
  --numeric-filter "score:[50,100]" \
  -k 10 -n 10000 -c 50
```

#### Exclusive Bounds

```bash
# Strict greater-than / less-than (excludes boundaries)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index filtered_idx \
  --numeric-filter "price:(10,100)" \
  -k 10 -n 10000 -c 50

# Half-open interval: > 0 and <= 100
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index filtered_idx \
  --numeric-filter "score:(0,100]" \
  -k 10 -n 10000 -c 50
```

#### Unbounded Ranges

```bash
# All values <= 4.5 (no minimum)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index filtered_idx \
  --numeric-filter "rating:[-inf,4.5]" \
  -k 10 -n 10000 -c 50

# All values >= 100 (no maximum)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index filtered_idx \
  --numeric-filter "count:[100,+inf)" \
  -k 10 -n 10000 -c 50
```

#### Multiple Numeric Filters

```bash
# Filter by both price AND rating (AND logic)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index filtered_idx \
  --numeric-filter "price:[10,100]" \
  --numeric-filter "rating:[4.0,5.0]" \
  -k 10 -n 10000 -c 50
```

#### Combined with Tag Filters

```bash
# Create index with tag and numeric fields
./target/release/valkey-bench-rs --cli -h $HOST --cluster \
  "FT.CREATE ecomm_idx ON HASH PREFIX 1 prod: SCHEMA embedding VECTOR HNSW 6 TYPE FLOAT32 DIM 784 DISTANCE_METRIC L2 category TAG price NUMERIC"

# Load with both tag and numeric
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "prod:" \
  --search-index ecomm_idx \
  --tag-field category \
  --search-tags "electronics:40,clothing:30,home:30" \
  --numeric-field-config "price:float:uniform:9.99:199.99:2" \
  -n 100000 -c 100

# Query: electronics category AND price range
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index ecomm_idx \
  --tag-field category \
  --tag-filter "electronics" \
  --numeric-filter "price:[20,100]" \
  -k 10 -n 10000 -c 50
```

---

## Vector Search Workloads

### Complete Vector Search Pipeline

```bash
# Step 1: Create index via CLI
./target/release/valkey-bench-rs --cli -h $HOST --cluster \
  "FT.CREATE test_idx ON HASH PREFIX 1 vec: SCHEMA embedding VECTOR HNSW 6 TYPE FLOAT32 DIM 784 DISTANCE_METRIC L2"

# Step 2: Load vectors
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "vec:" \
  --search-index test_idx \
  -n 60000 -c 100 --threads 8

# Step 3: Run queries with recall measurement
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index test_idx \
  --ef-search 100 \
  -k 10 -n 10000 -c 50 --threads 4

# Step 4: Cleanup (optional)
./target/release/valkey-bench-rs --cli -h $HOST --cluster \
  "FT.DROPINDEX test_idx"
```

### Different HNSW Parameters

```bash
# High recall configuration
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index idx \
  --ef-search 500 \
  --hnsw-m 32 \
  -k 100 -n 5000 -c 20

# Fast search (lower recall)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index idx \
  --ef-search 10 \
  -k 10 -n 50000 -c 100
```

### Load with Delete Protection

```bash
# Load vectors with ground truth protection for recall testing
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-load \
  --dataset $DATASET \
  --search-prefix "vec:" \
  --search-index test_idx \
  -n 60000 -c 100

# Delete some vectors (ground truth neighbors are protected)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-delete \
  --dataset $DATASET \
  --search-prefix "vec:" \
  -n 10000 -c 50

# Query and verify recall is still valid
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index test_idx \
  -k 10 -n 5000 -c 50
```

---

## Real-World Dataset: Cohere-Medium-1M

This section provides production-ready examples using the **cohere-medium-1m** dataset (768-dimensional vectors with ground truth for recall validation).

### Setup

```bash
# Set environment variables
export HOST="your-cluster.example.com"
export SCHEMA="datasets/cohere-medium-1m.yaml"
export DATA="datasets/cohere-medium-1m.bin"

# Build the tool
cargo build --release
```

### Basic Vector Load and Query

```bash
# Load 1M vectors
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-load -n 1000000 \
  --search-index cohere_1m \
  --search-prefix "vec:" \
  -c 200 --threads 16

# Query with recall measurement
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 1000 \
  --search-index cohere_1m \
  --search-prefix "vec:" \
  -k 10 --ef-search 200 \
  -c 50 --threads 4
```

### Load with Tags (E-commerce Categories)

The index is created automatically with TAG field when `--tag-field` is specified.

```bash
# Load with category distribution:
# - all: 100% (for recall validation)
# - electronics: 30%
# - clothing: 25%
# - home: 20%
# - sports: 15%
# - books: 10%
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-load -n 1000000 \
  --search-index cohere_ecom \
  --search-prefix "vec:" \
  --tag-field category \
  --search-tags "all:100,electronics:30,clothing:25,home:20,sports:15,books:10" \
  -c 200 --threads 16
```

### Filtered Query Examples

#### Single Tag Filter

```bash
# Query only "electronics" category (~30% of vectors)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 1000 \
  --search-index cohere_ecom \
  --search-prefix "vec:" \
  --tag-field category \
  --tag-filter "electronics" \
  -k 10 --ef-search 200 \
  -c 50 --threads 4
```

#### Recall Validation with "all" Tag

Use the "all" tag (100% match) to validate that filtered recall matches baseline.

```bash
# This should produce the same recall as unfiltered query
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 1000 \
  --search-index cohere_ecom \
  --search-prefix "vec:" \
  --tag-field category \
  --tag-filter "all" \
  -k 10 --ef-search 200 \
  -c 50 --threads 4
```

### Tags with Numeric Fields (Full E-commerce)

```bash
# Load with tags + numeric fields (index created automatically)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-load -n 1000000 \
  --search-index cohere_ecom_full \
  --search-prefix "prod:" \
  --tag-field category \
  --search-tags "electronics:30,clothing:25,home:20,sports:15,books:10" \
  --numeric-field-config "price:float:uniform:9.99:999.99:2" \
  --numeric-field-config "rating:float:normal:4.0:0.8:1" \
  -c 200 --threads 16

# Query: electronics + price range $20-$100
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 1000 \
  --search-index cohere_ecom_full \
  --search-prefix "prod:" \
  --tag-field category \
  --tag-filter "electronics" \
  --numeric-filter "price:[20,100]" \
  -k 10 --ef-search 200 \
  -c 50 --threads 4
```

### Country-Based Tags (Geographic Filtering)

```bash
# Load with country distribution
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-load -n 1000000 \
  --search-index cohere_geo \
  --search-prefix "doc:" \
  --tag-field country \
  --search-tags "US:40,UK:20,DE:15,FR:10,JP:10,Other:5" \
  -c 200 --threads 16

# Query US-only documents
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 1000 \
  --search-index cohere_geo \
  --search-prefix "doc:" \
  --tag-field country \
  --tag-filter "US" \
  -k 10 --ef-search 200 \
  -c 50 --threads 4
```

### Performance Comparison: Baseline vs Filtered

```bash
echo "=== Baseline (no filter) ==="
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 10000 \
  --search-index cohere_ecom \
  --search-prefix "vec:" \
  -k 10 --ef-search 200 \
  -c 100 --threads 8

echo "=== Filtered (electronics only, ~30%) ==="
./target/release/valkey-bench-rs -h $HOST --cluster \
  --schema $SCHEMA --data $DATA \
  -t vec-query -n 10000 \
  --search-index cohere_ecom \
  --search-prefix "vec:" \
  --tag-field category \
  --tag-filter "electronics" \
  -k 10 --ef-search 200 \
  -c 100 --threads 8
```

### Expected Results

| Test | Filter | Expected Recall | Notes |
|------|--------|-----------------|-------|
| Baseline | None | ~0.97 | Full dataset |
| Tag: `all` | 100% match | ~0.97 | Same as baseline |
| Tag: `electronics` | ~30% match | ~0.30 | Proportional to coverage |
| Combined | tag + numeric | <0.10 | Very selective filtering |

---

## Parameter Optimization

### Optimize GET Throughput

```bash
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 100000 -r 1000000 \
  --optimize \
  --objective "maximize:qps" \
  --tune "clients:10:300:20" \
  --tune "threads:1:32:2"
```

### Multi-Objective: QPS with Latency Bound

```bash
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 100000 -r 1000000 \
  --optimize \
  --objective "maximize:qps,minimize:p99_ms" \
  --constraint "p99_ms:lt:1.0" \
  --tune "clients:10:200:10" \
  --tune "pipeline:1:20:2"
```

### Vector Search: Recall vs Throughput

```bash
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t vec-query \
  --dataset $DATASET \
  --search-index idx \
  -n 10000 \
  --optimize \
  --objective "maximize:qps" \
  --constraint "recall:gt:0.95" \
  --tune "ef_search:10:500:20" \
  --tune "clients:10:100:10"
```

---

## Performance Tuning Recipes

### Maximum GET Throughput

```bash
# Prefill 3M keys with 500B values
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t set -n 3000000 -r 3000000 -d 500 \
  --iteration "sequential" \
  -c 200 --threads 16 -P 100

# Maximum GET (target: ~1M req/s on 16xlarge node cluster)
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 3000000 -r 3000000 \
  -c 275 --threads 24 -P 100
```

### Sustained Mixed Load

```bash
# Duration-based mixed traffic (run for 5 minutes)
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:80,set:20" \
  --duration 300 \
  -r 1000000 -d 128 \
  -c 200 --threads 16
```

### Hot-Spot Simulation

```bash
# Zipfian GET with rate limit
./target/release/valkey-bench-rs -h $HOST --cluster \
  -t get -n 1000000 -r 100000 \
  --iteration "zipfian:1.5" \
  --rps 50000 \
  -c 100 --threads 8
```

### Mixed Traffic with Subset Keys

```bash
# Parallel traffic on specific key range
./target/release/valkey-bench-rs -h $HOST --cluster \
  --parallel "get:70,set:30" \
  --iteration "subset:10000:50000" \
  -n 500000 -c 100
```

---

## See Also

**Documentation:**
- [README.md](../README.md) - Main documentation and API reference
- [DATASETS.md](DATASETS.md) - Dataset format and schema reference
- [BENCHMARKING.md](BENCHMARKING.md) - Complete benchmarking guide
- [ADVANCED.md](ADVANCED.md) - Optimizer, metadata filtering, advanced features
- [DEMO.md](DEMO.md) - Interactive demo walkthrough

**Design Documents:**
- [valkey-bench-rs-HLD.md](valkey-bench-rs-HLD.md) - High-Level Design
- [valkey-bench-rs-LLD.md](valkey-bench-rs-LLD.md) - Low-Level Design
