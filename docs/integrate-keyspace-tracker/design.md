# Design Document

## Introduction

This document describes the technical design for migrating valkey-bench-rs from its current vector tracking and keyspace management implementation to the `keyspace_tracker` crate. The migration is structured in three phases to ensure stability and validation at each step.

## Current Architecture Analysis

### Terminology Clarification

This document distinguishes two unrelated "tag" concepts:

| Concept | Purpose | Example | Affected by Migration |
|---------|---------|---------|----------------------|
| **Cluster hash tag** | Control slot routing | `{ABC}` in key name | YES - Phase 1 removes injection |
| **Index tag field** | Search filtering | `category: "a,b,c"` | NO - unchanged |

### Components to Replace

#### 1. ClusterTagMap (`src/cluster/cluster_tag_map.rs`)

Current responsibilities:
- Maps vector IDs to cluster hash tags (e.g., `{ABC}`) for routing
- Tracks which vectors exist in the cluster via `vector_exists()`
- Provides atomic claiming of unmapped IDs via `claim_unmapped_id()`
- Supports scan-based initialization via `build_vector_id_mappings()`

Key data structures:
```rust
pub struct ClusterTagMap {
    prefix: String,
    mappings: Vec<VectorClusterMapping>,  // vector_id -> cluster_hash_tag
    count: AtomicU64,
    keys_scanned: AtomicU64,
    is_cluster_mode: bool,
    update_mutex: Mutex<()>,
    unmapped_counter: AtomicU64,
}
```

Issues with current design:
- Artificial cluster hash tags injected into keys (e.g., `vec:{ABC}:000042`)
- Requires scanning cluster to build mappings before operations
- Memory overhead: 6 bytes per vector for hash tag storage
- Tight coupling between existence tracking and cluster routing

#### 2. ProtectedVectorIds (`src/cluster/protected_ids.rs`)

Current responsibilities:
- Stores ground truth vector IDs that must not be deleted
- Provides `is_protected()` check during delete operations
- Atomic claiming of deleteable IDs via `claim_deleteable_id()`

Key data structures:
```rust
pub struct ProtectedVectorIds {
    protected: HashSet<u64>,
    delete_counter: AtomicU64,
    max_id: u64,
    protected_count: u64,
}
```

Issues with current design:
- HashSet has O(1) average but O(n) worst case lookup
- No snapshot/diff capabilities
- No coverage computation against tracker state

#### 3. KeyFormat (`src/workload/key_format.rs`)

Current responsibilities:
- Defines key format with cluster hash tags: `prefix{hash_tag}:vector_id`
- Provides `format_key()` and `parse_key()` for generation/parsing
- Constants: `CLUSTER_TAG_LEN=5`, `DEFAULT_KEY_WIDTH=12`

Key format examples:
- With cluster hash tag: `zvec_:{ABC}:000000000123`
- Without cluster hash tag: `vec:000000000123`

**Note:** This is unrelated to index tag fields used in search schemas.

#### 4. IterationStrategy (`src/workload/iteration.rs`)

Current responsibilities:
- Sequential, Random, Subset, Zipfian iteration patterns
- Thread-safe `IterationState` with atomic counter
- Deterministic pseudo-random via SplitMix64

## Target Architecture

### keyspace_tracker Integration

The `keyspace_tracker` crate provides:

| Component | Replaces | Purpose |
|-----------|----------|---------|
| `PrefixTracker` | `ClusterTagMap` | Bitmap-based existence tracking |
| `ReferenceSet` | `ProtectedVectorIds` | O(1) membership testing for protection |
| `TrackerIterBuilder` | `IterationStrategy` | Flexible iteration with filters |
| `BitmapSnapshot` | N/A (new) | State capture for before/after comparison |
| `AccessDistribution` | Partial `IterationStrategy` | Statistical distributions |

### Key Format Changes

**Phase 1 removes cluster hash tag injection:**

| Before | After |
|--------|-------|
| `vec:{ABC}:000000000042` | `vec:000000000042` |

Slot affinity will be determined by CRC16 of the full key, not artificial `{ABC}` hash tags.

**Note:** Index tag fields (TAG type in search schema, e.g., `category: "electronics,sale"`) are completely unaffected. The `TagDistributionSet` and related code for generating index tag field values remains unchanged.

### Component Mapping

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Current Architecture                          │
├─────────────────────────────────────────────────────────────────────┤
│  ClusterTagMap          ProtectedVectorIds      IterationStrategy   │
│  - vector_exists()      - is_protected()        - next_key()        │
│  - add_mapping()        - claim_deleteable_id() - sequential/random │
│  - claim_unmapped_id()  - protected_count()     - zipfian/subset    │
│  - get_tag()                                                        │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Target Architecture                           │
├─────────────────────────────────────────────────────────────────────┤
│  PrefixTracker          ReferenceSet            TrackerIterBuilder  │
│  - exists()             - contains()            - sequential()      │
│  - add()                - count_existing_in()   - random()          │
│  - remove()             - missing_in()          - partition()       │
│  - claim()              - from_iter()           - distribution()    │
│  - snapshot()                                   - set_only()        │
│  - count()                                      - unset_only()      │
└─────────────────────────────────────────────────────────────────────┘
```

## Migration Phases

### Phase 1: Remove Cluster Hash Tag Injection

**Goal:** Eliminate automatic cluster hash tag injection from keys. User-specified cluster tags in key patterns remain a supported feature.

**Note:** No backward compatibility is required - this is a pre-GA refactor.

**Changes:**

1. **Dataset schema generation:**
   - Update default key pattern from `vec:{HASHTAG}:%012d` to `vec:%012d`
   - User can still specify custom patterns with cluster tags if needed

2. **KeyFormat modifications:**
   - Remove `arg_prefixed_key_with_cluster_tag()` method entirely
   - Update `format_key()` to not inject `{ABC}` style hash tags
   - Simplify `parse_key()` for new format only

3. **Template factory updates:**
   - `create_vec_load_template()`: Use simple prefixed key
   - `create_vec_delete_template()`: Use simple prefixed key
   - All vector workloads use `arg_prefixed_key()` only

4. **ClusterTagMap simplification:**
   - Remove hash tag storage and mapping logic
   - Keep existence tracking via `vector_exists()`
   - Keep `claim_unmapped_id()` for partial prefill

**What is NOT affected:**
- Index tag fields (TAG type in search schema)
- `TagDistributionSet` for generating index tag values
- `SearchConfig.tag_field`, `tag_filter`, `tag_distributions`
- Any search query filtering by index tags
- User's ability to specify cluster tags in key prefix (feature, not legacy)

**Validation:**
- All existing tests pass
- Keys generated without `{ABC}` component
- Cluster mode still works (slot determined by full key hash)
- Index tag field generation unchanged

### Phase 2: Migrate to keyspace_tracker

**Goal:** Replace internal tracking with keyspace_tracker while retaining 100% existing functionality.

**Changes:**

1. **Add keyspace_tracker dependency:**
   ```toml
   [dependencies]
   keyspace_tracker = { path = "/Volumes/workplace/keyspace_tracker" }
   ```

2. **Replace ClusterTagMap with PrefixTracker:**

   | ClusterTagMap | PrefixTracker |
   |---------------|---------------|
   | `vector_exists(id)` | `exists(id)` |
   | `add_mapping(id, _)` | `add(id)` |
   | `claim_unmapped_id()` | `iter().unset_only().claim()` |
   | `count()` | `count()` |
   | `reset_unmapped_counter()` | N/A (iterator handles this) |

3. **Replace ProtectedVectorIds with ReferenceSet:**

   | ProtectedVectorIds | ReferenceSet |
   |--------------------|--------------|
   | `is_protected(id)` | `contains(id)` |
   | `claim_deleteable_id()` | `tracker.iter().set_only().claim()` + `!reference.contains()` |
   | `protected_count()` | `len()` |
   | `deleteable_count()` | `tracker.count() - reference.count_existing_in(snapshot)` |

4. **Integrate TrackerIterBuilder:**

   | IterationStrategy | TrackerIterBuilder |
   |-------------------|-------------------|
   | `Sequential` | `.sequential()` |
   | `Random { seed }` | `.seed(seed).random()` |
   | `Subset { start, end, .. }` | `.id_range(start, end)` |
   | `Zipfian { skew, seed }` | `.distribution(AccessDistribution::Zipfian { skew }).seed(seed)` |

5. **New WorkloadContext structure:**
   ```rust
   pub struct WorkloadContext {
       pub tracker: Arc<PrefixTracker>,
       pub reference_set: Option<Arc<ReferenceSet>>,
       pub config: TrackerConfig,
   }
   ```

6. **Scan-based initialization:**
   - Implement `init_tracker_from_scan()` using existing SCAN logic
   - Parse keys and call `tracker.add(id)` for each discovered key

**Validation:**
- All existing tests pass
- Benchmark results match pre-migration baseline
- Memory usage reduced (1 bit per ID vs 6 bytes per ID)

### Phase 3: Introduce New Capabilities

**Goal:** Enable advanced features from keyspace_tracker.

**New CLI options:**
```
--keyspace-tracker           Enable keyspace tracking
--tracker-init <MODE>        cold|scan (default: cold)
--reference-set-backfill     Restore missing reference set members before query
--reference-set-check        Verify reference set coverage, report status
--workload-snapshot          Take snapshots before/after workload phases
--protect-reference-set      Skip reference set members during delete operations
--iteration <STRATEGY>       sequential|random|zipfian:SKEW|hotspot:PCT:PROB
```

**New features:**

1. **Reference set coverage checking:**
   ```rust
   let snapshot = tracker.snapshot();
   let existing = reference.count_existing_in(&snapshot);
   let missing = reference.count_missing_in(&snapshot);
   let coverage = existing as f64 / reference.len() as f64;
   ```

2. **Reference set backfill:**
   ```rust
   let missing_ids = reference.missing_in(&tracker.snapshot());
   for id in missing_ids {
       // Reload vector from dataset
       tracker.add(id);
   }
   ```

3. **Workload snapshots:**
   ```rust
   let pre_snapshot = tracker.snapshot();
   // Execute workload
   let added = tracker.added_count_since(&pre_snapshot);
   let removed = tracker.removed_count_since(&pre_snapshot);
   ```

4. **Advanced iteration patterns:**
   ```rust
   // Zipfian distribution
   tracker.iter()
       .set_only()
       .distribution(AccessDistribution::Zipfian { skew: 0.99 })
       .random()
   
   // Mixed existence ratio
   tracker.iter()
       .mixed_ratio(0.8)  // 80% existing, 20% non-existing
       .random()
   ```

**Validation:**
- New tests for advanced features
- Documentation updated
- Example configurations provided

## Detailed Component Design

### WorkloadContext Integration

```rust
/// Workload execution context with keyspace tracking
pub struct WorkloadContext {
    /// Key existence tracker
    pub tracker: Arc<PrefixTracker>,
    
    /// Reference set for protection (ground truth)
    pub reference_set: Option<Arc<ReferenceSet>>,
    
    /// Pre-workload snapshot (if --workload-snapshot enabled)
    pub pre_snapshot: Option<BitmapSnapshot>,
    
    /// Tracker configuration
    pub config: TrackerConfig,
}

impl WorkloadContext {
    /// Initialize from configuration
    pub fn new(config: &BenchmarkConfig) -> Self {
        let tracker_config = TrackerConfig::simple(&config.search_prefix)
            .with_max_id(config.num_vectors);
        
        let tracker = Arc::new(PrefixTracker::new(tracker_config));
        
        Self {
            tracker,
            reference_set: None,
            pre_snapshot: None,
            config: tracker_config,
        }
    }
    
    /// Initialize tracker from database scan
    pub fn init_from_scan(&self, client: &mut Connection) {
        let pattern = format!("{}*", self.config.prefix);
        let mut cursor = 0;
        
        loop {
            let (next, keys) = scan(client, cursor, &pattern, 10000);
            for key in keys {
                if let Some(id) = extract_id(&key, &self.config.prefix) {
                    self.tracker.add(id);
                }
            }
            if next == 0 { break; }
            cursor = next;
        }
    }
    
    /// Build reference set from ground truth
    pub fn build_reference_set(&mut self, dataset: &DatasetContext) {
        let mut reference = ReferenceSet::with_capacity(dataset.num_vectors() as u64);
        
        for query_idx in 0..dataset.num_queries() {
            for neighbor_id in dataset.ground_truth(query_idx) {
                reference.insert(neighbor_id as u64);
            }
        }
        
        self.reference_set = Some(Arc::new(reference));
    }
    
    /// Check reference set coverage
    pub fn check_coverage(&self) -> Option<(u64, u64, f64)> {
        let reference = self.reference_set.as_ref()?;
        let snapshot = self.tracker.snapshot();
        
        let existing = reference.count_existing_in(&snapshot);
        let total = reference.len();
        let coverage = existing as f64 / total as f64;
        
        Some((existing, total, coverage))
    }
    
    /// Take pre-workload snapshot
    pub fn take_snapshot(&mut self) {
        self.pre_snapshot = Some(self.tracker.snapshot());
    }
    
    /// Compute workload diff
    pub fn compute_diff(&self) -> Option<(u64, u64)> {
        let pre = self.pre_snapshot.as_ref()?;
        let added = self.tracker.added_count_since(pre);
        let removed = self.tracker.removed_count_since(pre);
        Some((added, removed))
    }
}
```

**Note on existing code integration:** The existing `VectorLoadContext` has `tag_distributions: Option<TagDistributionSet>` for generating index tag field values. This is completely separate from cluster hash tags and remains unchanged in the migration.

### Iteration Integration

```rust
/// Create iterator for workload based on configuration
pub fn create_workload_iterator<'a>(
    tracker: &'a PrefixTracker,
    config: &WorkloadIterConfig,
    thread_id: usize,
    num_threads: usize,
) -> Box<dyn Iterator<Item = (u64, bool)> + 'a> {
    let mut builder = tracker.iter();
    
    // Apply existence filter
    match config.existence_mode {
        ExistenceMode::SetOnly => builder = builder.set_only(),
        ExistenceMode::UnsetOnly => builder = builder.unset_only(),
        ExistenceMode::Mixed(ratio) => builder = builder.mixed_ratio(ratio),
        ExistenceMode::All => {}
    }
    
    // Apply partitioning
    if num_threads > 1 {
        builder = builder.partition(thread_id, num_threads);
    }
    
    // Apply limit
    if let Some(limit) = config.limit {
        builder = builder.limit(limit);
    }
    
    // Apply seed
    if let Some(seed) = config.seed {
        builder = builder.seed(seed);
    }
    
    // Apply distribution
    if let Some(ref dist) = config.distribution {
        builder = builder.distribution(dist.clone());
    }
    
    // Terminal operation
    match config.iteration_mode {
        IterationMode::Sequential => Box::new(builder.sequential()),
        IterationMode::Random => Box::new(builder.random()),
        IterationMode::Claim => Box::new(builder.claim()),
    }
}
```

### Delete Workload with Protection

```rust
/// Execute vec-delete with reference set protection
pub fn execute_vec_delete(
    ctx: &WorkloadContext,
    client: &mut Connection,
    config: &DeleteConfig,
) -> DeleteResult {
    let mut deleted = 0u64;
    let mut skipped = 0u64;
    
    let reference = ctx.reference_set.as_ref();
    
    for (id, _exists) in ctx.tracker.iter()
        .set_only()
        .partition(config.thread_id, config.num_threads)
        .limit(config.count)
        .random()
    {
        // Skip protected IDs
        if let Some(ref_set) = reference {
            if ref_set.contains(id) {
                skipped += 1;
                continue;
            }
        }
        
        // Execute delete
        let key = format!("{}:{:012}", config.prefix, id);
        client.del(&key)?;
        ctx.tracker.remove(id);
        deleted += 1;
    }
    
    DeleteResult { deleted, skipped }
}
```

## File Changes Summary

### Phase 1 Files

| File | Change | Notes |
|------|--------|-------|
| `src/workload/key_format.rs` | Remove cluster hash tag injection from `format_key()` | Index tag fields unaffected |
| `src/workload/command_template.rs` | Remove `arg_prefixed_key_with_cluster_tag()` | `arg_tag_placeholder()` for index tags unchanged |
| `src/workload/template_factory.rs` | Use `arg_prefixed_key()` for all vector workloads | Index tag field generation unchanged |
| `src/cluster/cluster_tag_map.rs` | Simplify to existence-only tracking | Remove hash tag storage |

### Phase 2 Files

| File | Change |
|------|--------|
| `Cargo.toml` | Add `keyspace_tracker` dependency |
| `src/cluster/mod.rs` | Remove `cluster_tag_map` and `protected_ids` modules |
| `src/workload/context.rs` | New: `WorkloadContext` with tracker integration |
| `src/workload/iteration.rs` | Replace with keyspace_tracker iterators |
| `src/workload/mod.rs` | Export new context module |
| `src/dataset/context.rs` | Integrate `ReferenceSet` for ground truth |
| `src/benchmark/vec_load.rs` | Use `tracker.add()` after successful load |
| `src/benchmark/vec_delete.rs` | Use `tracker.remove()` and reference protection |
| `src/benchmark/vec_query.rs` | Use reference set for coverage checking |

### Phase 3 Files

| File | Change |
|------|--------|
| `src/config/cli.rs` | Add new CLI options |
| `src/config/benchmark_config.rs` | Add tracker configuration fields |
| `src/benchmark/runner.rs` | Integrate snapshot and coverage features |
| `docs/KEYSPACE_TRACKER.md` | New: User documentation |

## Testing Strategy

### Phase 1 Tests

1. **Key format tests:**
   - Keys generated without cluster tags
   - Backward compatible parsing of tagged keys
   - Round-trip format/parse consistency

2. **Cluster mode tests:**
   - Slot distribution without tags
   - No MOVED redirects in cluster mode

### Phase 2 Tests

1. **Tracker equivalence tests:**
   - `PrefixTracker.exists()` matches `ClusterTagMap.vector_exists()`
   - `PrefixTracker.count()` matches `ClusterTagMap.count()`
   - Iteration produces same results

2. **Reference set tests:**
   - `ReferenceSet.contains()` matches `ProtectedVectorIds.is_protected()`
   - Coverage computation accuracy

3. **Integration tests:**
   - vec-load populates tracker correctly
   - vec-delete respects protection
   - vec-query coverage check works

### Phase 3 Tests

1. **New feature tests:**
   - Snapshot diff accuracy
   - Backfill restores missing IDs
   - Distribution patterns match expected

2. **CLI tests:**
   - New options parse correctly
   - Feature flags enable correct behavior

## Hierarchical Address Space Design

### Problem Statement

Current implementation only supports flat key spaces. Real applications often use HASH keys with multiple fields, requiring iteration patterns like:
- All fields of keys 0-1000
- Specific fields across all keys
- Random (key, field) pairs

### Two-Dimensional Address Model

```
Keys:    [0, 1, 2, ..., N-1]
Fields:  [f1, f2, f3, ..., M]

Total address space size = N × M
Linear index mapping: idx → (key_idx, field_idx)
  key_idx   = idx / M
  field_idx = idx % M
```

### Iteration Orders

| Order | Pattern | Use Case |
|-------|---------|----------|
| **Key-major** | (k0,f1), (k0,f2), (k1,f1), (k1,f2), ... | Batch operations per key |
| **Field-major** | (k0,f1), (k1,f1), (k0,f2), (k1,f2), ... | Column-oriented access |
| **Random** | Random (key, field) pairs | Realistic mixed workloads |

### Integration with AddressableSpace

```rust
/// Hierarchical address space for HASH-type workloads
pub struct HierarchicalSpace {
    /// Key existence tracker
    tracker: Arc<PrefixTracker>,
    /// Field names for this address space
    fields: Vec<String>,
    /// Iteration order
    order: HierarchicalOrder,
    /// Total keys in space
    num_keys: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum HierarchicalOrder {
    KeyMajor,    // All fields of key 0, then key 1, ...
    FieldMajor,  // Field 0 of all keys, then field 1, ...
    Random,      // Random (key, field) pairs
}

impl AddressableSpace for HierarchicalSpace {
    fn len(&self) -> u64 {
        self.num_keys * self.fields.len() as u64
    }

    fn address_type(&self) -> AddressType {
        AddressType::HashField
    }

    fn address_at(&self, idx: u64) -> Address {
        let (key_idx, field_idx) = match self.order {
            HierarchicalOrder::KeyMajor => {
                let k = idx / self.fields.len() as u64;
                let f = idx % self.fields.len() as u64;
                (k, f as usize)
            }
            HierarchicalOrder::FieldMajor => {
                let f = idx / self.num_keys;
                let k = idx % self.num_keys;
                (k, f as usize)
            }
            HierarchicalOrder::Random => {
                // Use deterministic hash for reproducibility
                let hash = splitmix64(idx);
                let k = hash % self.num_keys;
                let f = (hash / self.num_keys) % self.fields.len() as u64;
                (k, f as usize)
            }
        };

        Address {
            key: key_idx,
            field: Some(self.fields[field_idx].clone()),
            path: None,
        }
    }
}
```

### CLI Integration

```bash
# HSET with multiple fields, key-major iteration
valkey-bench-rs -t hset --fields "name,email,score,data" \
    --field-order key-major -n 1000000

# HGET with field-major iteration (column access pattern)
valkey-bench-rs -t hget --fields "score" \
    --field-order field-major -n 1000000

# Mixed HASH operations with random field access
valkey-bench-rs -t hset --fields "f1,f2,f3,f4,f5" \
    --field-order random --hit-rate 0.8 -n 1000000
```

## Delete-Rewrite-Read Cycle Design

### Lifecycle Phases

```
Phase 1: LOAD
├── Initialize tracker (cold or scan)
├── Load N keys: tracker.add(id) for each
└── Snapshot: pre_delete = tracker.snapshot()

Phase 2: DELETE
├── Delete D keys (e.g., 30% of N)
├── For each: tracker.remove(id)
├── Track deleted IDs for rewrite phase
└── Snapshot: post_delete = tracker.snapshot()

Phase 3: REWRITE
├── Rewrite R keys with overlap control
│   ├── overlap_ratio × R: rewrite to deleted keys
│   └── (1 - overlap_ratio) × R: write to new keys
├── For each: tracker.add(id)
└── Snapshot: post_rewrite = tracker.snapshot()

Phase 4: READ
├── Read with hit_rate control
│   ├── hit_rate: read existing keys
│   └── (1 - hit_rate): read non-existing keys
└── Compute diffs from snapshots
```

### Overlap Control for Rewrite

```rust
/// Iterator that prioritizes recently-deleted keys
pub struct OverlapAwareIterator<'a> {
    tracker: &'a PrefixTracker,
    deleted_snapshot: &'a BitmapSnapshot,
    overlap_ratio: f64,
    rng: SplitMix64,
}

impl<'a> Iterator for OverlapAwareIterator<'a> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        let use_deleted = self.rng.next_f64() < self.overlap_ratio;
        
        if use_deleted {
            // Find a key that was deleted (in snapshot but not in current)
            self.find_deleted_key()
        } else {
            // Find a key that never existed
            self.find_new_key()
        }
    }
}
```

## Parallel Execution with Controlled Overlap

### Partition Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| **Disjoint** | Non-overlapping ranges | Maximum throughput, no contention |
| **Overlapping** | Full keyspace access | Contention testing, realistic simulation |
| **Partial(ratio)** | Configurable overlap | Controlled contention levels |

### Partial Overlap Implementation

```
Thread 0: [0 -------- 25% -------- 50%]
Thread 1:           [25% -------- 50% -------- 75%]
Thread 2:                       [50% -------- 75% -------- 100%]
Thread 3: [0 -- 12.5%]                              [87.5% -- 100%]

With 50% overlap, each thread's range overlaps 25% with neighbors.
```

```rust
pub fn partition_with_overlap(
    total_keys: u64,
    thread_id: usize,
    num_threads: usize,
    overlap_ratio: f64,
) -> (u64, u64) {
    let base_size = total_keys / num_threads as u64;
    let overlap_size = (base_size as f64 * overlap_ratio) as u64;
    
    let start = (thread_id as u64 * base_size).saturating_sub(overlap_size / 2);
    let end = ((thread_id + 1) as u64 * base_size + overlap_size / 2).min(total_keys);
    
    (start, end)
}
```

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Performance regression | Benchmark before/after each phase |
| Memory usage increase | keyspace_tracker uses 1 bit/ID vs 6 bytes/ID |
| Breaking existing scripts | CLI compatibility maintained |
| Cluster mode issues | Extensive cluster testing |
| Ground truth corruption | Reference set protection validated |
| Hierarchical complexity | Start with key-major, add others incrementally |
| Overlap calculation bugs | Extensive unit tests for partition logic |

## Dependencies

- `keyspace_tracker` crate at `/Volumes/workplace/keyspace_tracker`
- No external crate.io dependencies added
- Rust 2021 edition compatibility

## Success Criteria

1. **Phase 1:** All tests pass, keys generated without cluster tags
2. **Phase 2:** All tests pass, memory usage reduced, performance maintained
3. **Phase 3:** New features work, documentation complete
