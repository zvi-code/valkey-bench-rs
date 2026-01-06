# Keyspace Tracker API Review

This document provides a comprehensive review of the `keyspace_tracker` crate API for integration into valkey-bench-rs.

## Overview

The keyspace_tracker crate provides efficient bitmap-based tracking of key IDs with support for:
- Thread-safe concurrent operations
- Hierarchical key patterns (e.g., `prefix:{id}:{sub_id}`)
- Snapshot-based change tracking
- Flexible iteration with partitioning and distribution patterns

## Test Results

- **Unit Tests**: 85 passed, 0 failed
- **Doc Tests**: Some failures due to incomplete code examples in documentation (not affecting functionality)

---

## Core Types

### PrefixTracker

The main tracker type for managing a set of key IDs under a common prefix.

#### Construction

```rust
// Via TrackerConfig builder
let config = TrackerConfig::simple("user")
    .with_max_id(1_000_000);
let tracker = PrefixTracker::new(config);

// Hierarchical pattern: "vec:{id}:{sub_id}"
let config = TrackerConfig::hierarchical("vec")
    .with_max_id(100_000)
    .with_max_sub_id(768);
let tracker = PrefixTracker::new(config);
```

#### Core Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `add` | `fn add(&self, id: u64) -> bool` | Add ID, returns true if newly added |
| `remove` | `fn remove(&self, id: u64) -> bool` | Remove ID, returns true if was present |
| `exists` | `fn exists(&self, id: u64) -> bool` | O(1) existence check |
| `claim` | `fn claim(&self, id: u64) -> bool` | Atomic test-and-set (thread-safe) |
| `count` | `fn count(&self) -> u64` | Population count |
| `density` | `fn density(&self) -> f64` | Ratio: count / max_id |

#### Bulk Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `add_range` | `fn add_range(&self, start: u64, end: u64) -> u64` | Add range of IDs, returns count added |
| `remove_range` | `fn remove_range(&self, start: u64, end: u64) -> u64` | Remove range, returns count removed |
| `clear` | `fn clear(&self)` | Remove all IDs |

#### Snapshot Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `snapshot` | `fn snapshot(&self) -> BitmapSnapshot` | Capture immutable state |
| `added_since` | `fn added_since(&BitmapSnapshot) -> Vec<u64>` | IDs added since snapshot |
| `removed_since` | `fn removed_since(&BitmapSnapshot) -> Vec<u64>` | IDs removed since snapshot |
| `added_count_since` | `fn added_count_since(&BitmapSnapshot) -> u64` | Count of IDs added |
| `removed_count_since` | `fn removed_count_since(&BitmapSnapshot) -> u64` | Count of IDs removed |

#### Reference Set Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `restore_from_reference` | `fn restore_from_reference(&ReferenceSet) -> u64` | Restore missing IDs from reference |
| `iter_intersection` | `fn iter_intersection(&ReferenceSet) -> impl Iterator` | Iterate IDs in both tracker and reference |
| `iter_missing_from_reference` | `fn iter_missing_from_reference(&ReferenceSet) -> impl Iterator` | Iterate IDs in reference but not tracker |

#### Iterator Access

```rust
// Get iterator builder
let iter = tracker.iter();
```

---

### ReferenceSet

An immutable set of IDs used for comparison operations (e.g., ground truth, protected keys).

#### Construction

```rust
// Empty with capacity
let set = ReferenceSet::with_capacity(1_000_000);

// From iterator
let set = ReferenceSet::from_iter(0..1000);

// From slice
let set = ReferenceSet::from_slice(&[1, 2, 3, 4, 5]);
```

#### Mutation

| Method | Signature | Description |
|--------|-----------|-------------|
| `insert` | `fn insert(&mut self, id: u64) -> bool` | Insert ID, returns true if new |
| `remove` | `fn remove(&mut self, id: u64) -> bool` | Remove ID, returns true if was present |

#### Query Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `contains` | `fn contains(&self, id: u64) -> bool` | O(1) membership test |
| `len` | `fn len(&self) -> u64` | Count of IDs |
| `is_empty` | `fn is_empty(&self) -> bool` | Check if empty |
| `iter` | `fn iter(&self) -> impl Iterator<Item = u64>` | Iterate all IDs |

#### Snapshot Comparison

| Method | Signature | Description |
|--------|-----------|-------------|
| `count_existing_in` | `fn count_existing_in(&BitmapSnapshot) -> u64` | Count IDs present in snapshot |
| `count_missing_in` | `fn count_missing_in(&BitmapSnapshot) -> u64` | Count IDs absent from snapshot |
| `missing_in` | `fn missing_in(&BitmapSnapshot) -> Vec<u64>` | Get IDs absent from snapshot |
| `existing_in` | `fn existing_in(&BitmapSnapshot) -> Vec<u64>` | Get IDs present in snapshot |

#### Set Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `intersection` | `fn intersection(&ReferenceSet) -> ReferenceSet` | IDs in both sets |
| `union` | `fn union(&ReferenceSet) -> ReferenceSet` | IDs in either set |
| `difference` | `fn difference(&ReferenceSet) -> ReferenceSet` | IDs in self but not other |

---

### BitmapSnapshot

An immutable point-in-time capture of tracker state.

#### Query Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `capacity` | `fn capacity(&self) -> usize` | Maximum ID capacity |
| `count` | `fn count(&self) -> u64` | Population count |
| `test` | `fn test(&self, index: usize) -> bool` | Check if ID exists |
| `iter_set` | `fn iter_set(&self) -> impl Iterator<Item = u64>` | Iterate all set IDs |

#### Comparison Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `intersection` | `fn intersection(&BitmapSnapshot) -> Vec<u64>` | IDs in both snapshots |
| `difference` | `fn difference(&BitmapSnapshot) -> Vec<u64>` | IDs in self but not other |
| `intersection_count` | `fn intersection_count(&BitmapSnapshot) -> u64` | Count of intersection |
| `difference_count` | `fn difference_count(&BitmapSnapshot) -> u64` | Count of difference |

---

### TrackerIterBuilder

Fluent builder for creating iterators with filtering, partitioning, and distribution options.

#### Filtering

| Method | Signature | Description |
|--------|-----------|-------------|
| `set_only` | `fn set_only(self) -> Self` | Only existing IDs |
| `unset_only` | `fn unset_only(self) -> Self` | Only non-existing IDs |
| `mixed_ratio` | `fn mixed_ratio(self, ratio: f64) -> Self` | X% existing, (1-X)% non-existing |

#### Partitioning (for multi-threaded workloads)

| Method | Signature | Description |
|--------|-----------|-------------|
| `partition` | `fn partition(self, index: usize, total: usize) -> Self` | Disjoint partition for thread |
| `overlapping` | `fn overlapping(self) -> Self` | Allow thread overlap |

#### Sampling

| Method | Signature | Description |
|--------|-----------|-------------|
| `sample` | `fn sample(self, probability: f64) -> Self` | Probabilistic sampling |
| `limit` | `fn limit(self, max_items: u64) -> Self` | Limit items returned |
| `seed` | `fn seed(self, seed: u64) -> Self` | Reproducible randomness |

#### Distribution

| Method | Signature | Description |
|--------|-----------|-------------|
| `distribution` | `fn distribution(self, dist: AccessDistribution) -> Self` | Set access pattern |

#### Range

| Method | Signature | Description |
|--------|-----------|-------------|
| `id_range` | `fn id_range(self, start: u64, end: u64) -> Self` | Limit to ID range |

#### Terminal Operations

| Method | Signature | Description |
|--------|-----------|-------------|
| `sequential` | `fn sequential(self) -> SequentialIter` | Sequential iteration |
| `random` | `fn random(self) -> RandomIter` | Random access iteration |
| `claim` | `fn claim(self) -> ClaimIter` | Atomic claim iteration |

---

### AccessDistribution

Enum for access pattern distributions.

```rust
pub enum AccessDistribution {
    Uniform,                              // Equal probability
    Zipfian { skew: f64 },               // Power-law (web cache)
    Exponential { lambda: f64 },          // Recency bias (session store)
    Hotspot { hot_pct: f64, hot_prob: f64 }, // Hot/cold split
    Latest { window: u64 },               // Recent IDs only
}
```

#### Preset Constructors

| Method | Distribution |
|--------|-------------|
| `AccessDistribution::web_cache()` | Zipfian { skew: 0.99 } |
| `AccessDistribution::session_store()` | Exponential { lambda: 0.1 } |
| `AccessDistribution::hotspot()` | Hotspot { hot_pct: 0.2, hot_prob: 0.8 } |

---

### TrackerConfig

Builder for configuring PrefixTracker instances.

```rust
// Simple pattern: "prefix:{id}"
TrackerConfig::simple("user")
    .with_max_id(1_000_000)

// Hierarchical pattern: "prefix:{id}:{sub_id}"
TrackerConfig::hierarchical("vec")
    .with_max_id(100_000)
    .with_max_sub_id(768)
    .with_slot_range(0, 8192)  // Optional: Valkey cluster slot filtering
```

---

## Usage Patterns

### Basic Tracking

```rust
let tracker = PrefixTracker::new(TrackerConfig::simple("key").with_max_id(10000));

// Track operations
tracker.add(42);
assert!(tracker.exists(42));
tracker.remove(42);
assert!(!tracker.exists(42));
```

### Concurrent Workload

```rust
// Thread-safe claim for exclusive access
if tracker.claim(id) {
    // This thread owns this ID
    perform_operation(id);
}
```

### Partitioned Multi-threaded Access

```rust
// Thread 0 of 4
let iter = tracker.iter()
    .set_only()
    .partition(0, 4)
    .random();

for (id, key) in iter {
    // Process only this thread's partition
}
```

### Change Tracking

```rust
let before = tracker.snapshot();
// ... perform operations ...
let added = tracker.added_count_since(&before);
let removed = tracker.removed_count_since(&before);
```

### Ground Truth Verification

```rust
let expected = ReferenceSet::from_iter(0..1000);
let snapshot = tracker.snapshot();
let missing = expected.missing_in(&snapshot);
// Restore missing keys
for id in missing {
    load_key(id);
    tracker.add(id);
}
```

---

## Thread Safety

- `PrefixTracker`: All operations are thread-safe via atomic operations
- `ReferenceSet`: Immutable after construction, safe to share
- `BitmapSnapshot`: Immutable, safe to share
- `TrackerIterBuilder`: Not thread-safe, create per-thread

---

## Performance Characteristics

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `add`, `remove`, `exists` | O(1) | Atomic bitmap operations |
| `claim` | O(1) | Compare-and-swap |
| `count` | O(1) | Cached, updated atomically |
| `snapshot` | O(n) | Copies bitmap data |
| `iter().sequential()` | O(n) | Scans bitmap |
| `iter().random()` | O(1) per item | Random sampling |

---

## Integration Notes for valkey-bench-rs

1. **Replace HashSet tracking**: Use `PrefixTracker` instead of `HashSet<u64>` for key tracking
2. **Snapshot for metrics**: Use `snapshot()` before/after workload phases for accurate change tracking
3. **Partitioned iteration**: Use `partition(thread_id, num_threads)` for multi-threaded benchmarks
4. **Distribution patterns**: Use `AccessDistribution` for realistic workload simulation
5. **Ground truth**: Use `ReferenceSet` for dataset verification and recall calculation
