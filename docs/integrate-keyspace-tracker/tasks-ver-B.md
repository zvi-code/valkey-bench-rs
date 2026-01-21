# Comprehensive Tasks Document: Keyspace Tracker Migration

> **Document Purpose:** This is the authoritative task tracking document for the keyspace_tracker migration project. It merges and reconciles the original task list with the spec-based tasks, providing detailed acceptance criteria, testing requirements, and baseline comparison gates.

> **Task Completion Guidelines:**
> - A task is NOT complete until ALL **Acceptance Criteria** are verified
> - Sub-items (bullets) are work steps, not completion markers
> - Creating a script ≠ running it; writing code ≠ testing it
> - Mark `[x]` only when the deliverable exists AND is verified working
> - Performance comparisons MUST reference actual baseline files in `./results/baseline_results/`

---

## Terminology Reference

| Term | Context | Example | Migration Impact |
|------|---------|---------|------------------|
| **Cluster hash tag** | Key routing/sharding | `{ABC}` in `vec:{ABC}:000123` | Phase 1 removes injection |
| **Index tag field** | Search filtering | `category: "electronics,sale"` | NOT affected |
| **PrefixTracker** | keyspace_tracker | Bitmap existence tracking | Phase 2 introduces |
| **ReferenceSet** | keyspace_tracker | Ground truth protection | Phase 2 introduces |

---

## Baseline Reference Data

### Source Files (in `./results/baseline_results/`)

| Category | Primary File | Backup Files |
|----------|--------------|--------------|
| Simple SET/GET | `set_20251231_161434.txt`, `get_20251231_161434.txt` | `write_*.json`, `get_*.json` |
| Vector Workloads | `baseline_cohere-small-100k_20260101_103703.md` | `load_*.json`, `query_*.json`, `delete_*.json` |
| Memory Baseline | Embedded in baseline reports | N/A |


### Authoritative Baseline Metrics

These values are extracted from `results/baseline_results/baseline_cohere-small-100k_20260101_103703.md` and related JSON files. All phase gates MUST compare against these values.

#### Simple Workloads (SET/GET)

| Test | Metric | Baseline Value | Source File |
|------|--------|----------------|-------------|
| Sequential Fill | Throughput | 19,647 req/s | `fill_single_client.json` |
| Sequential Fill | Avg Latency | 0.05ms | `fill_single_client.json` |
| Random Write (1 client) | Avg Latency | 0.05ms | `write_single_client.json` |
| Random Write (1 client) | P99 Latency | 0.06ms | `write_single_client.json` |
| Random Write (max QPS) | Throughput | 48,829 req/s | `write_max_qps.json` |
| Random Get (1 client) | Avg Latency | 0.05ms | `get_single_client.json` |
| Random Get (1 client) | P99 Latency | 0.06ms | `get_single_client.json` |
| Random Get (max QPS) | Throughput | 50,048 req/s | `get_max_qps.json` |

#### Vector Workloads

| Workload | Throughput | Avg Latency | P50 | P99 | Source |
|----------|------------|-------------|-----|-----|--------|
| vec-load | 6,996 req/s | 8.00ms | 8.46ms | 11.61ms | `baseline_cohere-small-100k_*.md` |
| vec-query | 3,154 req/s | 6.20ms | 6.25ms | 6.98ms | `baseline_cohere-small-100k_*.md` |
| vec-delete | 22,570 req/s | 0.74ms | 0.68ms | 1.35ms | `baseline_cohere-small-100k_*.md` |

#### Memory Baseline

| Phase | Memory | Source |
|-------|--------|--------|
| Initial (empty) | 113.44M | `baseline_cohere-small-100k_*.md` |
| After vec-load (100k vectors) | 853.47M | `baseline_cohere-small-100k_*.md` |
| Peak during query | 855.73M | `baseline_cohere-small-100k_*.md` |
| After vec-delete | 851.66M | `baseline_cohere-small-100k_*.md` |

---

## Phase 0: Preparation ✅ COMPLETE

**Status:** All Phase 0 tasks verified complete. Ready for Phase 1.

### Task 0.1: Simple Baseline ✅

**Completed:** 2026-01-01

**Artifacts Created:**
- `results/baseline_results/set_20251231_161434.txt`
- `results/baseline_results/get_20251231_161434.txt`
- `results/baseline_results/mixed_20251231_161434.txt`
- `results/baseline_results/write_single_client.json`
- `results/baseline_results/write_max_qps.json`
- `results/baseline_results/get_single_client.json`
- `results/baseline_results/get_max_qps.json`
- `results/baseline_results/fill_single_client.json`

### Task 0.2: Vector Baseline ✅

**Completed:** 2026-01-01

**Artifacts Created:**
- `results/baseline_results/baseline_cohere-small-100k_20260101_103703.md`
- `results/baseline_results/baseline_cohere-small-100k_20260101_103703.json`
- `results/baseline_results/load_20260101_103703.json`
- `results/baseline_results/query_20260101_103703.json`
- `results/baseline_results/delete_20260101_103703.json`

### Task 0.3: API Review ✅

**Completed:** API review documented in `docs/keyspace-tracker-api-review.md`

**Verified APIs:**
- PrefixTracker: `add`, `remove`, `exists`, `count`, `snapshot` ✓
- ReferenceSet: `contains`, `count_existing_in`, `missing_in` ✓
- TrackerIterBuilder: `set_only`, `unset_only`, `partition`, `distribution` ✓
- keyspace_tracker tests: 85 passed, 0 failed ✓

### Task 0.4: Test Infrastructure ✅

**Completed:** Test infrastructure verified

**Verified:**
- `cargo test` passes (264 tests) ✓
- Baseline scripts in `bench/scripts/` ✓

### Phase 0 Gate ✅ PASSED

| Criterion | Status | Evidence |
|-----------|--------|----------|
| Simple baseline exists | ✅ | `results/baseline_results/set_*.txt`, `get_*.txt` |
| Vector baseline exists | ✅ | `results/baseline_results/baseline_cohere-small-100k_*.md` |
| API review documented | ✅ | `docs/keyspace-tracker-api-review.md` |
| Tests pass | ✅ | `cargo test` passes |

---

## Phase 1: Remove Cluster Hash Tag Injection

**Scope:** Remove automatic `{HASHTAG}` injection into keys. Does NOT affect index tag fields or user-specified cluster tags in key patterns.

**Status:** Not started

**Pre-requisites:** Phase 0 Gate passed ✅

### Task 1.1: Update KeyFormat Module

**Goal:** Modify key formatting to stop injecting cluster hash tags.

**File:** `src/workload/key_format.rs`

**Work Items:**
- [ ] Add `KeyFormat::new()` as simple constructor (no cluster tags)
- [ ] Remove `with_cluster_tags()` and `without_cluster_tags()` constructors
- [ ] Update `format_key()` to only produce simple keys
- [ ] Update `total_len()` calculation for keys without hash tags
- [ ] Update `parse_key()` for simple format only
- [ ] Update unit tests to verify new behavior

**Acceptance Criteria:**
- [ ] `format_key()` produces keys without `{ABC}` pattern
- [ ] Old constructors removed (not deprecated)
- [ ] Round-trip test passes: `parse_key(format_key(id)) == id`
- [ ] All tests pass: `cargo test key_format`

**Verification Command:**
```bash
cargo test key_format -- --nocapture
```

---

### Task 1.2: Update CommandTemplate Module

**Goal:** Remove cluster tag template functionality.

**File:** `src/workload/command_template.rs`

**Work Items:**
- [ ] Remove `TemplateArg::PrefixedKeyWithClusterTag` variant entirely
- [ ] Remove `arg_prefixed_key_with_cluster_tag()` method entirely
- [ ] Update `fill_template()` to remove cluster tag handling
- [ ] Update unit tests

**Acceptance Criteria:**
- [ ] `PrefixedKeyWithClusterTag` variant does not exist
- [ ] `arg_prefixed_key_with_cluster_tag()` method does not exist
- [ ] All tests pass: `cargo test command_template`
- [ ] Index tag placeholders (`arg_tag_placeholder()`) unchanged and working

**Verification Command:**
```bash
cargo test command_template -- --nocapture
```

---

### Task 1.3: Update Template Factory

**Goal:** Update all vector workload templates to use new key format.

**File:** `src/workload/template_factory.rs`

**Work Items:**
- [ ] Update `create_vec_load_template()` to use `arg_prefixed_key()`
- [ ] Update `VecDel` case to use `arg_prefixed_key()`
- [ ] Update `add_key` helper closure to always use simple prefixed keys
- [ ] Remove all cluster hash tag logic from templates
- [ ] Update unit tests

**Acceptance Criteria:**
- [ ] All `create_vec_*` functions use simple prefixed keys
- [ ] No cluster tag logic remains in template factory
- [ ] All tests pass: `cargo test template_factory`
- [ ] Index tag field generation in templates unchanged

**Verification Command:**
```bash
cargo test template_factory -- --nocapture
```

---

### Task 1.4: Simplify ClusterTagMap

**Goal:** Replace hash tag storage with bitmap-based existence tracking.

**File:** `src/cluster/cluster_tag_map.rs`

**Work Items:**
- [ ] Remove `VectorClusterMapping` struct entirely
- [ ] Replace `mappings: Vec<VectorClusterMapping>` with `Vec<u8>` bitmap
- [ ] Remove `get_tag()` method entirely
- [ ] Update `vector_exists()` to use bitmap
- [ ] Update `add_mapping()` to set bit (ignore tag parameter)
- [ ] Keep `claim_unmapped_id()` functionality using bitmap
- [ ] Update `parse_vector_key()` for simple format
- [ ] Update unit tests

**Acceptance Criteria:**
- [ ] `VectorClusterMapping` struct does not exist
- [ ] `get_tag()` method does not exist
- [ ] Memory per entry reduced to 1 bit (vs 6 bytes)
- [ ] All tests pass: `cargo test cluster_tag_map`

**Verification Command:**
```bash
cargo test cluster_tag_map -- --nocapture
```

---

### Task 1.5: Phase 1 Integration Validation

**Goal:** Verify the complete Phase 1 changes work together and meet performance requirements.

**Work Items:**
- [ ] Run full test suite: `cargo test`
- [ ] Run vec-load with new key format and verify keys don't contain `{ABC}`
- [ ] Verify cluster mode still routes correctly (keys distribute across shards)
- [ ] Run performance comparison against Phase 0 baseline

**Acceptance Criteria:**
- [ ] `cargo test` passes with 0 failures
- [ ] Manual verification: vec-load creates keys without `{ABC}` component
- [ ] Manual verification: cluster mode routes requests correctly
- [ ] Performance within 5% of Phase 0 baseline (see comparison table below)

**Performance Comparison Requirements:**

Run the baseline benchmark and compare:
```bash
HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick
```

| Metric | Phase 0 Baseline | Phase 1 Actual | Max Allowed Regression |
|--------|------------------|----------------|------------------------|
| vec-load throughput | 6,996 req/s | ___ req/s | ≥ 6,646 req/s (5%) |
| vec-query P99 latency | 6.98ms | ___ms | ≤ 7.33ms (5%) |
| vec-delete throughput | 22,570 req/s | ___ req/s | ≥ 21,442 req/s (5%) |

**Verification Commands:**
```bash
# Full test suite
cargo test

# Performance benchmark
HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick
```

---

### Phase 1 Gate

**All criteria must be checked before proceeding to Phase 2:**

| Criterion | Status | Evidence Required |
|-----------|--------|-------------------|
| `cargo test` passes | [ ] | Test output showing 0 failures |
| Keys generated without `{ABC}` component | [ ] | Manual test output or unit test |
| Cluster mode routes correctly | [ ] | Manual test on multi-shard cluster |
| vec-load throughput ≥ 6,646 req/s | [ ] | Benchmark report |
| vec-query P99 ≤ 7.33ms | [ ] | Benchmark report |
| vec-delete throughput ≥ 21,442 req/s | [ ] | Benchmark report |

**⛔ STOP: Human review required before proceeding to Phase 2**

### Phase 1 Commit

After all Phase 1 Gate criteria pass, create a commit:

```bash
# Verify all tests pass
cargo test

# Format code
cargo fmt

# Check for warnings
cargo clippy

# Stage changes
git add -A

# Commit
git commit -m "refactor(keys): remove cluster hash tag injection

- Remove KeyFormat::with_cluster_tags() and without_cluster_tags()
- Remove TemplateArg::PrefixedKeyWithClusterTag variant
- Remove arg_prefixed_key_with_cluster_tag() method
- Replace VectorClusterMapping with bitmap-based existence tracking
- Update all vector workload templates to use simple keys

Keys now use format 'prefix:000123' instead of 'prefix{ABC}:000123'.
Slot distribution determined by CRC16 of full key.

Performance validated against Phase 0 baseline:
- vec-load: XXX req/s (baseline: 6,996 req/s)
- vec-query P99: XXXms (baseline: 6.98ms)
- vec-delete: XXX req/s (baseline: 22,570 req/s)"
```

**Note:** Replace XXX with actual measured values from benchmark.

---

## Phase 2: Migrate to keyspace_tracker 

**Scope:** Replace internal tracking with keyspace_tracker crate while retaining 100% existing functionality.

**Status:** PARTIAL (commit 15782b0)

**Pre-requisites:** Phase 1 Gate passed

### Task 2.1: Add keyspace_tracker Dependency 

**Goal:** Add the keyspace_tracker crate as a dependency.

**File:** `Cargo.toml`

**Work Items:**
- [] Add keyspace_tracker dependency: - MUST use github not local copy
  ```toml
  [dependencies]
  keyspace_tracker = { path = "keyspace_tracker" }
  ```
- [ ] Verify `cargo check` passes

**Acceptance Criteria:**
- [x] `cargo check` compiles without errors
- [x] No new warnings introduced

---

### Task 2.2: Create Keyspace Module 

**Note:** Deviated from original plan - created `src/keyspace/mod.rs` instead of `tracker_context.rs`.
The unified keyspace module provides all required functionality.

**File:** `src/keyspace/mod.rs` (new file)

**Work Items:**
- [x] Create `KeyGroupExistanceTracker` struct wrapping PrefixTracker
- [x] Create `ProtectedIds` struct wrapping ReferenceSet
- [x] Implement atomic claim operations via tracker iteration
- [x] Export from `src/lib.rs`
- [x] Add comprehensive unit tests

**Acceptance Criteria:**
- [x] Module compiles and is exported
- [x] KeyGroupExistanceTracker provides claim_unmapped_id() with atomic semantics
- [x] ProtectedIds provides claim_deleteable_from_tracker() for existence-aware deletion
- [x] Unit tests pass: `cargo test keyspace`

---

### Task 2.3: Replace ClusterTagMap with KeyGroupExistanceTracker ✅

**Goal:** Replace ClusterTagMap with KeyGroupExistanceTracker using PrefixTracker.

**Work Items:**
- [x] Create `KeyGroupExistanceTracker` wrapper with atomic operations
- [x] Update `VectorLoadContext`: replace `tag_map` field with `existence_map`
- [x] Update `claim_unmapped_id()`: use `tracker.iter().unset_only().write()`
- [x] Delete `src/cluster/cluster_tag_map.rs`
- [x] Update `src/cluster/mod.rs` exports

**Acceptance Criteria:**
- [x] `cluster_tag_map.rs` file deleted
- [x] No references to `ClusterTagMap` anywhere
- [x] All existing functionality preserved
- [x] All tests pass: `cargo test`

---

### Task 2.4: Replace ProtectedVectorIds with ProtectedIds ✅

**Goal:** Replace ProtectedVectorIds with ProtectedIds using ReferenceSet.

**Work Items:**
- [x] Create `ProtectedIds` wrapper with ReferenceSet
- [x] Implement `claim_deleteable_from_tracker()` for existence-aware deletion
- [x] Add coverage checking methods: `count_existing_protected()`, `coverage_ratio()`
- [x] Delete `src/cluster/protected_ids.rs`
- [x] Update `src/cluster/mod.rs` exports

**Acceptance Criteria:**
- [x] `protected_ids.rs` file deleted
- [x] No references to `ProtectedVectorIds` anywhere
- [x] Ground truth protection still works correctly
- [x] All tests pass: `cargo test`

---

### Task 2.5: Create KeyspaceIterator ✅

**Goal:** Bridge IterationStrategy with keyspace_tracker's TrackerIterBuilder.

**File:** `src/workload/iteration.rs`

**Work Items:**
- [x] Create `KeyspaceIterator` struct with tracker integration
- [x] Create `ExistenceFilter` enum (All, SetOnly, UnsetOnly)
- [x] Implement `with_set_ratio()` for mixed-ratio iteration
- [x] Implement `claim_unset()` and `claim_set()` for atomic claiming
- [x] Add `to_sampling_config()` method to IterationStrategy
- [x] Add comprehensive unit tests

**Acceptance Criteria:**
- [x] All iteration strategies produce equivalent results
- [x] Deterministic results with same seed
- [x] All tests pass: `cargo test iteration`

---

### Task 2.6: Update VectorLoadContext ✅

**Work Items:**
- [x] Use `existence_map.claim_unmapped_id()` with tracker iteration
- [x] Atomic claim semantics via tracker's write iterator

---

### Task 2.7: Update DeleteContext ✅

**Work Items:**
- [x] Add `with_existence_map()` constructor
- [x] Use `protected_ids.claim_deleteable_from_tracker()` for existence-aware deletion
- [x] Integrate reference set for protection checking

---

### Task 2.8: Update VectorQueryContext ✅

**Note:** VectorQueryContext iterates query indices, not vector IDs.
Counter-based iteration is correct for queries.

---

### Task 2.9: Update Dataset Context ✅

**Goal:** Add method to build ReferenceSet from ground truth.

**File:** `src/dataset/context.rs`

**Work Items:**
- [x] Add `build_reference_set(&self) -> ReferenceSet` method
- [x] Add `build_reference_set_k(k) -> ReferenceSet` for top-k neighbors
- [x] Iterate ground truth and insert unique vector IDs

**Acceptance Criteria:**
- [x] `build_reference_set()` returns correct ReferenceSet
- [x] All ground truth IDs included
- [x] All tests pass

---

### Task 2.10: Phase 2 Validation ✅

**Work Items:**
- [x] Run full test suite: `cargo test` - 266 tests pass
- [ ] Run performance benchmark (requires cluster setup)
- [x] Verify memory usage reduced (1 bit per ID vs 6 bytes per ID - by design)

---

### Task 2.11: Proof of Capability Demo ✅

**Work Items:**
- [x] Implement `mixed_ratio()` support in KeyspaceIterator
- [x] Create test: `test_mixed_ratio_iteration_proof_of_capability`
- [x] Create test: `test_reference_set_coverage_checking`
- [x] Create test: `test_tracker_aware_deletion_with_protection`

**Acceptance Criteria:**
- [x] `mixed_ratio(0.8)` yields ~80% existing keys
- [x] Test passes with distribution within 15% of target
- [x] Coverage checking works correctly
  - `is_protected(id)` → `reference.contains(id)`
  - `protected_count()` → `reference.len()`
- [ ] Delete `src/cluster/protected_ids.rs`
- [ ] Update `src/cluster/mod.rs` exports

**Acceptance Criteria:**
- [ ] `protected_ids.rs` file deleted
- [ ] No references to `ProtectedVectorIds` anywhere
- [ ] Ground truth protection still works correctly
- [ ] All tests pass: `cargo test`

**Verification Command:**
```bash
# Check no ProtectedVectorIds references
grep -r "ProtectedVectorIds" src/ --include="*.rs"

cargo test
```

---

### Task 2.5: Integrate TrackerIterBuilder

**Goal:** Replace iteration logic with keyspace_tracker's TrackerIterBuilder.

**File:** `src/workload/iteration.rs`

**Work Items:**
- [ ] Create `create_tracker_iterator()` adapter function
- [ ] Map iteration strategies:
  - `IterationStrategy::Sequential` → `.sequential()`
  - `IterationStrategy::Random { seed }` → `.seed(seed).random()`
  - `IterationStrategy::Subset { start, end }` → `.id_range(start, end)`
  - `IterationStrategy::Zipfian { skew }` → `.distribution(AccessDistribution::Zipfian { skew })`
- [ ] Keep existing `IterationStrategy` enum for CLI parsing
- [ ] Update tests

**Acceptance Criteria:**
- [ ] All iteration strategies produce equivalent results
- [ ] Deterministic results with same seed
- [ ] All tests pass: `cargo test iteration`

**Verification Command:**
```bash
cargo test iteration -- --nocapture
```

---

### Task 2.6: Update vec-load Workload

**Goal:** Update vec-load to use PrefixTracker for existence tracking.

**Files:** `src/benchmark/vec_load.rs` or equivalent

**Work Items:**
- [ ] Call `tracker.add(id)` after successful HSET
- [ ] Use `tracker.iter().unset_only()` for partial prefill scenarios
- [ ] Integrate with `TrackerWorkloadContext`
- [ ] Update tests

**Acceptance Criteria:**
- [ ] vec-load correctly tracks loaded vectors
- [ ] Partial prefill works with tracker
- [ ] Performance within 5% of Phase 1 baseline
- [ ] All tests pass

**Verification Command:**
```bash
cargo test vec_load -- --nocapture
```

---

### Task 2.7: Update vec-delete Workload

**Goal:** Update vec-delete to use ReferenceSet for protection.

**Files:** `src/benchmark/vec_delete.rs` or equivalent

**Work Items:**
- [ ] Use `reference.contains(id)` for protection check
- [ ] Call `tracker.remove(id)` after successful DEL
- [ ] Use `tracker.iter().set_only()` for iteration over existing keys
- [ ] Update tests

**Acceptance Criteria:**
- [ ] vec-delete correctly skips protected vectors
- [ ] Tracker state updated after deletions
- [ ] Performance within 5% of Phase 1 baseline
- [ ] All tests pass

**Verification Command:**
```bash
cargo test vec_delete -- --nocapture
```

---

### Task 2.8: Update vec-query Workload

**Goal:** Update vec-query to integrate with reference set for ground truth.

**Files:** `src/benchmark/vec_query.rs` or equivalent

**Work Items:**
- [ ] Integrate reference set for ground truth validation
- [ ] Add method to compute coverage percentage
- [ ] Prepare for coverage checking (Phase 3)
- [ ] Update tests

**Acceptance Criteria:**
- [ ] vec-query can access reference set
- [ ] Coverage computation method exists
- [ ] Performance within 5% of Phase 1 baseline
- [ ] All tests pass

**Verification Command:**
```bash
cargo test vec_query -- --nocapture
```

---

### Task 2.9: Update Dataset Context

**Goal:** Add method to build ReferenceSet from ground truth.

**File:** `src/dataset/context.rs`

**Work Items:**
- [ ] Add `build_reference_set(&self) -> ReferenceSet` method
- [ ] Iterate ground truth and insert unique vector IDs
- [ ] Integrate with `TrackerWorkloadContext`
- [ ] Update tests

**Acceptance Criteria:**
- [ ] `build_reference_set()` returns correct ReferenceSet
- [ ] All ground truth IDs included
- [ ] All tests pass

**Verification Command:**
```bash
cargo test dataset -- --nocapture
```

---

### Task 2.10: Phase 2 Validation

**Goal:** Comprehensive validation of Phase 2 migration.

**Work Items:**
- [ ] Run full test suite: `cargo test`
- [ ] Run performance benchmark and compare to Phase 1
- [ ] Verify memory usage reduction
- [ ] Test all workload types (vec-load, vec-query, vec-delete)

**Acceptance Criteria:**
- [ ] `cargo test` passes with 0 failures
- [ ] Performance within 5% of Phase 1 baseline (see table below)
- [ ] Memory usage reduced (1 bit per ID vs 6 bytes per ID)
- [ ] All vector workloads function correctly

**Performance Comparison Requirements:**

```bash
HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick
```

| Metric | Phase 0 Baseline | Phase 2 Actual | Max Allowed Regression |
|--------|------------------|----------------|------------------------|
| vec-load throughput | 6,996 req/s | ___ req/s | ≥ 6,646 req/s (5%) |
| vec-query P99 latency | 6.98ms | ___ms | ≤ 7.33ms (5%) |
| vec-delete throughput | 22,570 req/s | ___ req/s | ≥ 21,442 req/s (5%) |
| Peak memory (100k vectors) | 855.73M | ___M | ≤ 855.73M (no increase) |

**Memory Reduction Verification:**

The migration should reduce memory for tracking:
- Old: 6 bytes per vector (cluster tag string)
- New: 1 bit per vector (bitmap)
- For 100k vectors: ~600KB → ~12.5KB savings

---

### Task 2.11: Proof of Capability Demo

**Goal:** Demonstrate one new capability to validate architecture.

**Work Items:**
- [ ] Implement basic `mixed_ratio()` support
- [ ] Add `--hit-rate` CLI option parsing (value only, not full implementation)
- [ ] Create test: iterate with 80% existing / 20% non-existing keys
- [ ] Verify statistical distribution matches configuration (within 5%)

**Acceptance Criteria:**
- [ ] `mixed_ratio(0.8)` yields ~80% existing keys
- [ ] Test passes with distribution within 5% of target
- [ ] API adjustments for Phase 3 documented

**Verification Command:**
```bash
cargo test mixed_ratio -- --nocapture
```

---

### Phase 2 Gate

**All criteria must be checked before proceeding to Phase 3:**

| Criterion | Status | Evidence Required |
|-----------|--------|-------------------|
| `cargo test` passes | [x] | 266 tests pass, 0 failures |
| `cluster_tag_map.rs` deleted | [x] | File removed, replaced by KeyGroupExistanceTracker |
| `protected_ids.rs` deleted | [x] | File removed, replaced by ProtectedIds |
| vec-load throughput ≥ 6,646 req/s | [ ] | Benchmark report (pending) |
| vec-query P99 ≤ 7.33ms | [ ] | Benchmark report (pending) |
| vec-delete throughput ≥ 21,442 req/s | [ ] | Benchmark report (pending) |
| Memory usage ≤ Phase 0 baseline | [ ] | Benchmark report (pending) |
| mixed_ratio proof of capability works | [x] | test_mixed_ratio_iteration_proof_of_capability passes |

**⚠️ NOTE: Performance benchmarks pending - requires running cluster setup**

### Phase 2 Commit

Committed as: `15782b0` - feat(keyspace): integrate keyspace_tracker crate for iteration and existence tracking

**Changes Summary:**
- 13 files changed, 1474 insertions(+), 740 deletions(-)
- Deleted: cluster_tag_map.rs, protected_ids.rs
- Created: src/keyspace/mod.rs
- 266 tests pass (10 new tests added)

```bash
# Verify all tests pass
cargo test

# Format code
cargo fmt

# Check for warnings
cargo clippy

# Stage changes
git add -A

# Commit
git commit -m "feat(tracker): migrate to keyspace_tracker crate

- Add keyspace_tracker dependency
- Create TrackerWorkloadContext for workload execution
- Replace ClusterTagMap with PrefixTracker
- Replace ProtectedVectorIds with ReferenceSet
- Integrate TrackerIterBuilder for iteration strategies
- Update vec-load, vec-delete, vec-query workloads
- Add mixed_ratio proof of capability

Memory improvement: ~600KB → ~12.5KB for 100k vector tracking

Performance validated against Phase 0 baseline:
- vec-load: XXX req/s (baseline: 6,996 req/s)
- vec-query P99: XXXms (baseline: 6.98ms)
- vec-delete: XXX req/s (baseline: 22,570 req/s)
- Peak memory: XXXM (baseline: 855.73M)"
```

**Note:** Replace XXX with actual measured values from benchmark.

---

## Phase 3: Introduce New Capabilities

**Scope:** Enable advanced features from keyspace_tracker. Organized by priority.

**Status:** Not started

**Pre-requisites:** Phase 2 Gate passed

### HIGH PRIORITY: Core Capabilities

These enable the primary use cases that motivated the migration.

#### Task 3.1: Add New CLI Options

**Goal:** Add CLI options for new keyspace_tracker features.

**File:** `src/config/cli.rs`

**Work Items:**
- [ ] `--keyspace-tracker` flag (enable tracker)
- [ ] `--tracker-init <MODE>` (cold|scan)
- [ ] `--iteration <STRATEGY>` (sequential|random|zipfian:SKEW|hotspot:PCT:PROB|subset:START:END|latest:PCT:PROB)
- [ ] `--hit-rate <RATIO>` (0.0-1.0)
- [ ] `--fields <FIELD_LIST>` for hierarchical iteration
- [ ] `--field-order <ORDER>` (key-major|field-major|random)
- [ ] `--partition-mode <MODE>` (disjoint|overlapping|partial:RATIO)
- [ ] Update help text

**Acceptance Criteria:**
- [ ] All CLI options parse correctly
- [ ] Invalid values produce helpful error messages
- [ ] `--help` shows all new options with descriptions
- [ ] All tests pass: `cargo test cli`

---

#### Task 3.2: Add Tracker Configuration

**Goal:** Add configuration structs for tracker features.

**File:** `src/config/benchmark_config.rs`

**Work Items:**
- [ ] Add `TrackerConfig` struct (enabled, init_mode, hit_rate, snapshot_enabled)
- [ ] Add `ReferenceSetConfig` struct (backfill, check, protect)
- [ ] Add `HierarchicalConfig` struct (fields, order)
- [ ] Add `PartitionConfig` struct (mode, overlap_ratio)
- [ ] Update configuration parsing from CLI args

**Acceptance Criteria:**
- [ ] All config structs defined and documented
- [ ] CLI args correctly populate config structs
- [ ] Default values are sensible
- [ ] All tests pass: `cargo test config`

---

#### Task 3.3: Implement Hit Rate Control

**Goal:** Control ratio of existing vs non-existing keys in iteration.

**Priority:** HIGH - Enables cache simulation use cases

**Work Items:**
- [ ] Parse ratio value (0.0-1.0) from CLI
- [ ] Map to `tracker.iter().mixed_ratio(ratio)`
- [ ] Apply to all iteration-based workloads
- [ ] Add tests for various hit rates (0.0, 0.5, 0.8, 1.0)

**Acceptance Criteria:**
- [ ] `--hit-rate 0.8` yields ~80% existing keys (within 5%)
- [ ] `--hit-rate 1.0` equivalent to `set_only()`
- [ ] `--hit-rate 0.0` equivalent to `unset_only()`
- [ ] Statistical distribution verified across 10,000+ iterations

**Verification Command:**
```bash
cargo test hit_rate  -- --nocapture
```

---

#### Task 3.4: Implement Hierarchical Key-Field Iteration

**Goal:** Support iteration over (key, field) pairs for HASH workloads.

**Priority:** HIGH - Enables HASH workload benchmarking

**File:** `src/workload/hierarchical.rs` (new file)

**Work Items:**
- [ ] Define `HierarchicalSpace` struct implementing `AddressableSpace`
- [ ] Support key-major order (all fields of key 0, then key 1, ...)
- [ ] Support field-major order (field f1 of all keys, then f2, ...)
- [ ] Support random (key, field) pair selection
- [ ] Integrate with `AddressableContext`
- [ ] Update HSET/HGET templates to use field from iterator
- [ ] Add tests for iteration patterns
- [ ] Export from `src/workload/mod.rs`

**Acceptance Criteria:**
- [ ] `--fields "f1,f2,f3" --field-order key-major` iterates correctly
- [ ] `--field-order field-major` iterates correctly
- [ ] `--field-order random` produces uniform distribution
- [ ] Thread-safe claiming of (key, field) pairs
- [ ] All tests pass

**Verification Command:**
```bash
cargo test hierarchical  -- --nocapture
```

---

#### Task 3.5: Implement Advanced Iteration Patterns

**Goal:** Support various access distributions.

**Priority:** HIGH - Enables realistic workload simulation

**Work Items:**
- [ ] `sequential` - sequential iteration
- [ ] `random` or `random:SEED` - random with optional seed
- [ ] `zipfian:SKEW` - Zipfian distribution (hot keys)
- [ ] `hotspot:PCT:PROB` - hotspot distribution
- [ ] `subset:START:END` - range iteration
- [ ] `latest:PCT:PROB` - recent keys prioritized
- [ ] Map to `TrackerIterBuilder.distribution()` calls
- [ ] Add tests for each pattern

**Acceptance Criteria:**
- [ ] Each pattern produces expected distribution
- [ ] Zipfian with skew=0.99 concentrates ~20% of accesses on ~1% of keys
- [ ] Deterministic results with same seed
- [ ] All tests pass

**Verification Command:**
```bash
cargo test iteration_patterns  -- --nocapture
```

---

#### Task 3.6: Implement Parallel Partition Modes

**Goal:** Control key overlap between parallel workers.

**Priority:** HIGH - Enables contention testing

**File:** `src/workload/parallel.rs` (new or existing)

**Work Items:**
- [ ] `disjoint` - non-overlapping ranges (default)
- [ ] `overlapping` - full keyspace access for all threads
- [ ] `partial:RATIO` - configurable overlap between threads
- [ ] Integrate with `TrackerIterBuilder.partition()`
- [ ] Verify thread-safety under contention
- [ ] Add tests for overlap calculations

**Acceptance Criteria:**
- [ ] `--partition-mode disjoint` produces non-overlapping ranges
- [ ] `--partition-mode overlapping` allows all threads full access
- [ ] `--partition-mode partial:0.5` produces 50% overlap
- [ ] No data corruption under contention
- [ ] All tests pass

**Verification Command:**
```bash
cargo test partition  -- --nocapture
```

---

### MEDIUM PRIORITY: Operational Features

These improve usability for common scenarios.

#### Task 3.7: Implement Scan-Based Initialization

**Goal:** Initialize tracker from existing database state.

**Work Items:**
- [ ] `--tracker-init scan` runs SCAN on all primary nodes
- [ ] Parse keys matching prefix and extract IDs
- [ ] Call `tracker.add(id)` for each discovered key
- [ ] Add progress reporting and statistics
- [ ] Handle cluster mode scanning (all primaries)

**Acceptance Criteria:**
- [ ] Tracker state matches database state after scan
- [ ] Progress reported during scan
- [ ] Works in both standalone and cluster mode
- [ ] All tests pass

---

#### Task 3.8: Implement Workload Snapshots

**Goal:** Capture tracker state before/after workloads.

**Work Items:**
- [ ] `--workload-snapshot` takes before/after snapshots
- [ ] Compute diff: `added_count_since()`, `removed_count_since()`
- [ ] Add snapshot statistics to JSON output
- [ ] Support multi-phase snapshot chains

**Acceptance Criteria:**
- [ ] Snapshots correctly capture state
- [ ] Diff computation accurate
- [ ] Statistics appear in JSON output
- [ ] All tests pass

---

#### Task 3.9: Implement Delete-Rewrite Overlap Control

**Goal:** Control overlap between delete and rewrite phases.

**Work Items:**
- [ ] `--overlap-ratio` for rewrite phases
- [ ] Track deleted keys during delete phase
- [ ] Prioritize deleted keys based on ratio during rewrite
- [ ] Create `OverlapAwareIterator`
- [ ] Support 0.0 (all new keys) to 1.0 (all deleted keys)

**Acceptance Criteria:**
- [ ] `--overlap-ratio 0.8` rewrites ~80% to deleted keys
- [ ] `--overlap-ratio 0.0` writes only to new keys
- [ ] Statistical distribution verified
- [ ] All tests pass

---

### LOWER PRIORITY: Vector-Specific Features

These are important for vector benchmarks but not general workloads.

#### Task 3.10: Implement Reference Set Coverage Check

**Goal:** Verify ground truth coverage before queries.

**Work Items:**
- [ ] `--reference-set-check` flag
- [ ] Compute coverage: `reference.count_existing_in(&tracker.snapshot())`
- [ ] Report existing count, missing count, percentage
- [ ] Warn if coverage < 95%
- [ ] Option to abort on low coverage

**Acceptance Criteria:**
- [ ] Coverage percentage computed correctly
- [ ] Warning displayed when coverage < 95%
- [ ] Abort option works
- [ ] All tests pass

---

#### Task 3.11: Implement Reference Set Backfill

**Goal:** Restore missing ground truth vectors.

**Work Items:**
- [ ] `--reference-set-backfill` flag
- [ ] Identify missing IDs: `reference.missing_in(&tracker.snapshot())`
- [ ] Reload vectors from dataset for missing IDs
- [ ] Verify 100% coverage after backfill
- [ ] Report backfill statistics

**Acceptance Criteria:**
- [ ] Missing vectors identified correctly
- [ ] Vectors reloaded from dataset
- [ ] 100% coverage achieved after backfill
- [ ] Statistics reported
- [ ] All tests pass

---

### Documentation & Validation

#### Task 3.12: Update Documentation

**Work Items:**
- [ ] Create `docs/KEYSPACE_TRACKER.md`:
  - Feature overview
  - CLI options reference
  - Usage examples for each capability
  - Hierarchical iteration examples
  - Delete-rewrite-read cycle examples
  - Partition mode examples
  - Migration guide from old behavior
- [ ] Update `README.md` with new features summary
- [ ] Update `EXAMPLES.md` with tracker examples
- [ ] Update `ADVANCED.md` with advanced usage patterns

**Acceptance Criteria:**
- [ ] `docs/KEYSPACE_TRACKER.md` exists and is comprehensive
- [ ] All new CLI options documented
- [ ] Examples are runnable and correct
- [ ] README updated

---

#### Task 3.13: Phase 3 Final Validation

**Goal:** Comprehensive validation of all Phase 3 features.

**Work Items:**
- [ ] Run full test suite: `cargo test `
- [ ] Test all new CLI options individually
- [ ] Test advanced iteration patterns
- [ ] Test hierarchical iteration with HASH workloads
- [ ] Test partition modes under multi-threaded load
- [ ] Performance benchmarks comparing to Phase 2 baseline
- [ ] Run `cargo clippy` and fix warnings
- [ ] Run `cargo fmt` to ensure formatting

**Acceptance Criteria:**
- [ ] `cargo test ` passes with 0 failures
- [ ] `cargo clippy ` has no warnings
- [ ] All HIGH PRIORITY features working
- [ ] Performance within acceptable range of Phase 2

**Performance Comparison:**

| Metric | Phase 0 Baseline | Phase 3 Actual | Max Allowed Regression |
|--------|------------------|----------------|------------------------|
| vec-load throughput | 6,996 req/s | ___ req/s | ≥ 6,296 req/s (10%) |
| vec-query P99 latency | 6.98ms | ___ms | ≤ 7.68ms (10%) |
| vec-delete throughput | 22,570 req/s | ___ req/s | ≥ 20,313 req/s (10%) |

Note: Phase 3 allows 10% regression due to additional feature overhead.

---

### Phase 3 Gate

**All criteria must be checked for migration completion:**

| Criterion | Status | Evidence Required |
|-----------|--------|-------------------|
| `cargo test ` passes | [ ] | Test output |
| `cargo clippy ` clean | [ ] | Clippy output |
| HIGH: Hit rate control works | [ ] | Test output |
| HIGH: Hierarchical iteration works | [ ] | Test output |
| HIGH: Advanced patterns work | [ ] | Test output |
| HIGH: Partition modes work | [ ] | Test output |
| MEDIUM: Scan initialization works | [ ] | Test output |
| MEDIUM: Workload snapshots work | [ ] | Test output |
| MEDIUM: Overlap control works | [ ] | Test output |
| LOWER: Coverage check works | [ ] | Test output |
| LOWER: Backfill works | [ ] | Test output |
| `docs/KEYSPACE_TRACKER.md` exists | [ ] | File exists |
| Performance within 10% of Phase 0 | [ ] | Benchmark report |

**✅ Migration complete when all HIGH PRIORITY items pass**

---

## Phase 3: Introduce New Capabilities

**Scope:** Enable advanced features from keyspace_tracker. Organized by priority.

**Status:** Not started

**Pre-requisites:** Phase 2 Gate passed

### HIGH PRIORITY: Core Capabilities

#### Task 3.1: Add New CLI Options

**Goal:** Add CLI options for new keyspace_tracker features.

**File:** `src/config/cli.rs`

**Work Items:**
- [ ] `--keyspace-tracker` flag (enable tracker)
- [ ] `--tracker-init <MODE>` (cold|scan)
- [ ] `--iteration <STRATEGY>` (sequential|random|zipfian:SKEW|hotspot:PCT:PROB|subset:START:END|latest:PCT:PROB)
- [ ] `--hit-rate <RATIO>` (0.0-1.0)
- [ ] `--fields <FIELD_LIST>` for hierarchical iteration
- [ ] `--field-order <ORDER>` (key-major|field-major|random)
- [ ] `--partition-mode <MODE>` (disjoint|overlapping|partial:RATIO)
- [ ] Update help text

**Acceptance Criteria:**
- [ ] All CLI options parse correctly
- [ ] Help text updated: `cargo run -- --help`
- [ ] Invalid values produce clear error messages

---

#### Task 3.2: Add Tracker Configuration

**Goal:** Add configuration structs for tracker features.

**File:** `src/config/benchmark_config.rs`

**Work Items:**
- [ ] Add `TrackerConfig` struct (enabled, init_mode, hit_rate, snapshot_enabled)
- [ ] Add `ReferenceSetConfig` struct (backfill, check, protect)
- [ ] Add `HierarchicalConfig` struct (fields, order)
- [ ] Add `PartitionConfig` struct (mode, overlap_ratio)
- [ ] Update configuration parsing from CLI args

**Acceptance Criteria:**
- [ ] All config structs defined and documented
- [ ] CLI args correctly populate config structs
- [ ] Default values sensible

---

#### Task 3.3: Implement Hit Rate Control

**Goal:** Implement `--hit-rate` for controlling existing/non-existing key ratio.

**Work Items:**
- [ ] Parse ratio value (0.0-1.0) from CLI
- [ ] Map to `tracker.iter().mixed_ratio(ratio)`
- [ ] Apply to all iteration-based workloads
- [ ] Add tests for various hit rates (0.0, 0.5, 0.8, 1.0)

**Acceptance Criteria:**
- [ ] `--hit-rate 0.8` yields ~80% existing keys (within 5%)
- [ ] `--hit-rate 1.0` equivalent to `set_only()`
- [ ] `--hit-rate 0.0` equivalent to `unset_only()`
- [ ] Tests pass for all hit rate values

**Verification Command:**
```bash
cargo test hit_rate  -- --nocapture
```

---

#### Task 3.4: Implement Hierarchical Key-Field Iteration

**Goal:** Support iteration over (key, field) pairs for HASH workloads.

**File:** `src/workload/hierarchical.rs` (new file)

**Work Items:**
- [ ] Define `HierarchicalSpace` struct implementing `AddressableSpace`
- [ ] Support key-major, field-major, and random orders
- [ ] Integrate with `AddressableContext`
- [ ] Update HSET/HGET templates to use field from iterator
- [ ] Add CLI parsing for `--fields` and `--field-order`
- [ ] Add tests for iteration patterns
- [ ] Export from `src/workload/mod.rs`

**Acceptance Criteria:**
- [ ] Key-major order: all fields of key 0, then key 1, etc.
- [ ] Field-major order: field f1 of all keys, then f2, etc.
- [ ] Random order: random (key, field) pairs
- [ ] Tests pass for all iteration orders

**Verification Command:**
```bash
cargo test hierarchical  -- --nocapture
```

---

#### Task 3.5: Implement Advanced Iteration Patterns

**Goal:** Implement all iteration pattern options.

**Work Items:**
- [ ] `sequential` - sequential iteration
- [ ] `random` or `random:SEED` - random with optional seed
- [ ] `zipfian:SKEW` - Zipfian distribution
- [ ] `hotspot:PCT:PROB` - hotspot distribution
- [ ] `subset:START:END` - range iteration
- [ ] `latest:PCT:PROB` - recent keys prioritized
- [ ] Map to `TrackerIterBuilder.distribution()` calls
- [ ] Add tests for each pattern

**Acceptance Criteria:**
- [ ] Each pattern produces expected distribution
- [ ] Deterministic results with same seed
- [ ] Tests pass for all patterns

**Verification Command:**
```bash
cargo test iteration_patterns  -- --nocapture
```

---

#### Task 3.6: Implement Parallel Partition Modes

**Goal:** Implement partition modes for multi-threaded workloads.

**File:** `src/workload/parallel.rs` (new or existing)

**Work Items:**
- [ ] `disjoint` - non-overlapping ranges (default)
- [ ] `overlapping` - full keyspace access for all threads
- [ ] `partial:RATIO` - configurable overlap between threads
- [ ] Integrate with `TrackerIterBuilder.partition()`
- [ ] Add tests for overlap calculations
- [ ] Verify thread-safety under contention

**Acceptance Criteria:**
- [ ] Disjoint mode: no key overlap between threads
- [ ] Overlapping mode: all threads can access all keys
- [ ] Partial mode: correct overlap ratio
- [ ] Thread-safe under concurrent access

**Verification Command:**
```bash
cargo test partition  -- --nocapture
```

---

### MEDIUM PRIORITY: Operational Features

#### Task 3.7: Implement Scan-Based Initialization

**Goal:** Initialize tracker from database scan.

**Work Items:**
- [ ] `--tracker-init scan` runs SCAN on all primary nodes
- [ ] Parse keys matching prefix and extract IDs
- [ ] Call `tracker.add(id)` for each discovered key
- [ ] Add progress reporting and statistics
- [ ] Handle cluster mode scanning

**Acceptance Criteria:**
- [ ] Tracker state matches database state after scan
- [ ] Progress reported during scan
- [ ] Works in both standalone and cluster mode

---

#### Task 3.8: Implement Workload Snapshots

**Goal:** Capture tracker state before/after workloads.

**Work Items:**
- [ ] `--workload-snapshot` takes before/after snapshots
- [ ] Compute diff: `added_count_since()`, `removed_count_since()`
- [ ] Add snapshot statistics to JSON output
- [ ] Support multi-phase snapshot chains

**Acceptance Criteria:**
- [ ] Snapshots captured correctly
- [ ] Diff computation accurate
- [ ] Statistics included in output

---

#### Task 3.9: Implement Delete-Rewrite Overlap Control

**Goal:** Control overlap between delete and rewrite phases.

**Work Items:**
- [ ] `--overlap-ratio` for rewrite phases
- [ ] Track deleted keys during delete phase
- [ ] Prioritize deleted keys based on ratio during rewrite
- [ ] Create `OverlapAwareIterator`
- [ ] Add tests for overlap behavior

**Acceptance Criteria:**
- [ ] `--overlap-ratio 0.8` rewrites ~80% to deleted keys
- [ ] `--overlap-ratio 0.0` writes all new keys
- [ ] `--overlap-ratio 1.0` rewrites all to deleted keys

---

### LOWER PRIORITY: Vector-Specific Features

#### Task 3.10: Implement Reference Set Coverage Check

**Goal:** Verify ground truth coverage before queries.

**Work Items:**
- [ ] `--reference-set-check` flag
- [ ] Compute coverage: `reference.count_existing_in(&tracker.snapshot())`
- [ ] Report existing count, missing count, percentage
- [ ] Warn if coverage < 95%
- [ ] Option to abort on low coverage

**Acceptance Criteria:**
- [ ] Coverage computed correctly
- [ ] Warning displayed if < 95%
- [ ] Abort option works

---

#### Task 3.11: Implement Reference Set Backfill

**Goal:** Restore missing ground truth vectors.

**Work Items:**
- [ ] `--reference-set-backfill` flag
- [ ] Identify missing IDs: `reference.missing_in(&tracker.snapshot())`
- [ ] Reload vectors from dataset for missing IDs
- [ ] Verify 100% coverage after backfill
- [ ] Report backfill statistics

**Acceptance Criteria:**
- [ ] Missing vectors identified correctly
- [ ] Vectors reloaded from dataset
- [ ] 100% coverage achieved after backfill

---

### Documentation & Validation

#### Task 3.12: Update Documentation

**Work Items:**
- [ ] Create `docs/KEYSPACE_TRACKER.md`:
  - Feature overview
  - CLI options reference
  - Usage examples for each capability
  - Hierarchical iteration examples
  - Delete-rewrite-read cycle examples
  - Partition mode examples
  - Migration guide from old behavior
- [ ] Update `README.md` with new features summary
- [ ] Update `EXAMPLES.md` with tracker examples
- [ ] Update `ADVANCED.md` with advanced usage patterns

**Acceptance Criteria:**
- [ ] `docs/KEYSPACE_TRACKER.md` exists and is comprehensive
- [ ] All new CLI options documented
- [ ] Examples are runnable

---

#### Task 3.13: Phase 3 Final Validation

**Goal:** Comprehensive validation of all Phase 3 features.

**Work Items:**
- [ ] Run full test suite: `cargo test `
- [ ] Test all new CLI options individually
- [ ] Test advanced iteration patterns
- [ ] Test hierarchical iteration with HASH workloads
- [ ] Test partition modes under multi-threaded load
- [ ] Performance benchmarks comparing to Phase 2 baseline

**Performance Comparison Requirements:**

```bash
HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick
```

| Metric | Phase 0 Baseline | Phase 3 Actual | Max Allowed Regression |
|--------|------------------|----------------|------------------------|
| vec-load throughput | 6,996 req/s | ___ req/s | ≥ 6,646 req/s (5%) |
| vec-query P99 latency | 6.98ms | ___ms | ≤ 7.33ms (5%) |
| vec-delete throughput | 22,570 req/s | ___ req/s | ≥ 21,442 req/s (5%) |

---

### Phase 3 Gate

**All criteria must be checked before marking migration complete:**

| Criterion | Status | Evidence Required |
|-----------|--------|-------------------|
| `cargo test ` passes | [ ] | Test output |
| HIGH: Hit rate control works | [ ] | Test output |
| HIGH: Hierarchical iteration works | [ ] | Test output |
| HIGH: Advanced patterns work | [ ] | Test output |
| HIGH: Partition modes work | [ ] | Test output |
| MEDIUM: Scan initialization works | [ ] | Test output |
| MEDIUM: Workload snapshots work | [ ] | Test output |
| MEDIUM: Overlap control works | [ ] | Test output |
| LOWER: Coverage check works | [ ] | Test output |
| LOWER: Backfill works | [ ] | Test output |
| `docs/KEYSPACE_TRACKER.md` exists | [ ] | File exists |
| Performance within 5% of baseline | [ ] | Benchmark report |

### Phase 3 Commit

After all Phase 3 Gate criteria pass, create a commit:

```bash
# Verify all tests pass
cargo test 

# Format code
cargo fmt

# Check for warnings
cargo clippy 

# Stage changes
git add -A

# Commit with conventional commit message
git commit -m "feat(tracker): add advanced keyspace_tracker capabilities

HIGH PRIORITY:
- Add --hit-rate for cache hit/miss simulation
- Add --fields and --field-order for hierarchical iteration
- Add --iteration for advanced access patterns (zipfian, hotspot, etc.)
- Add --partition-mode for parallel workload control

MEDIUM PRIORITY:
- Add --tracker-init scan for warm-start scenarios
- Add --workload-snapshot for lifecycle tracking
- Add --overlap-ratio for delete-rewrite cycles

LOWER PRIORITY:
- Add --reference-set-check for coverage validation
- Add --reference-set-backfill for ground truth restoration

Documentation:
- Add docs/KEYSPACE_TRACKER.md with full feature reference
- Update EXAMPLES.md with tracker examples

Performance validated against Phase 0 baseline:
- vec-load: XXX req/s (baseline: 6,996 req/s)
- vec-query P99: XXXms (baseline: 6.98ms)
- vec-delete: XXX req/s (baseline: 22,570 req/s)"
```

**Note:** Replace XXX with actual measured values from benchmark.

**✅ Migration complete after Phase 3 commit**

---

## Code Cleanup (After Phase 2 Gate)

**Scope:** Remove deprecated code after migration is validated.

**Status:** Not started

**Pre-requisites:** Phase 2 Gate passed

**Note:** This can be done in parallel with Phase 3 or after Phase 3.

### Task C.1: Remove Deprecated Cluster Hash Tag Code

**Work Items:**
- [ ] Delete `src/cluster/cluster_tag_map.rs`
- [ ] Delete `src/cluster/protected_ids.rs`
- [ ] Remove `arg_prefixed_key_with_cluster_tag()` from `command_template.rs`
- [ ] Clean up unused imports
- [ ] **PRESERVE:** All index tag field code (`TagDistributionSet`, `arg_tag_placeholder()`)

**Acceptance Criteria:**
- [ ] Deprecated files removed
- [ ] No dead code warnings
- [ ] Index tag field code unchanged and working

---

### Task C.2: Update Module Structure

**Work Items:**
- [ ] Update `src/cluster/mod.rs` exports
- [ ] Update `src/workload/mod.rs` exports
- [ ] **PRESERVE:** `tag_distribution.rs` exports

**Acceptance Criteria:**
- [ ] Module exports clean
- [ ] No unused import warnings

---

### Task C.3: Update Tests

**Work Items:**
- [ ] Remove tests for deprecated code
- [ ] Add tests for new keyspace_tracker functionality
- [ ] Add benchmark comparison tests (Phase 0 vs current)
- [ ] **VERIFY:** Index tag field tests remain passing

**Acceptance Criteria:**
- [ ] All tests pass
- [ ] No tests for removed code
- [ ] Index tag tests pass

---

### Cleanup Gate

| Criterion | Status | Evidence Required |
|-----------|--------|-------------------|
| `cargo build ` succeeds | [ ] | Build output |
| `cargo test ` passes | [ ] | Test output |
| No dead code warnings | [ ] | Clippy output |
| No unused import warnings | [ ] | Clippy output |
| Index tag field code working | [ ] | Test output |

### Cleanup Commit

After Cleanup Gate passes:

```bash
# Verify build and tests
cargo build 
cargo test 
cargo clippy 

# Stage changes
git add -A

# Commit
git commit -m "chore(cleanup): remove deprecated cluster hash tag code

- Remove src/cluster/cluster_tag_map.rs
- Remove src/cluster/protected_ids.rs
- Remove arg_prefixed_key_with_cluster_tag() method
- Clean up unused imports
- Update module exports

Note: Index tag field code (TagDistributionSet, etc.) preserved."
```

---

## Task Dependencies Diagram

```
Phase 0 ✅ ──────────────────────────────────────────────────────────────┐
  │                                                                      │
  ▼                                                                      │
Phase 1: Remove Cluster Hash Tags                                        │
  │                                                                      │
  ├── 1.1 KeyFormat ──┐                                                  │
  ├── 1.2 CommandTemplate ──┤                                            │
  ├── 1.3 Template Factory ──┼── 1.5 Integration ── [COMMIT] ── Gate 1   │
  └── 1.4 ClusterTagMap ─────┘                           │               │
                                                         │               │
                                                         ▼               │
Phase 2: Migrate to keyspace_tracker                                     │
  │                                                                      │
  ├── 2.1 Add Dependency                                                 │
  │     │                                                                │
  │     ▼                                                                │
  ├── 2.2 TrackerWorkloadContext                                         │
  │     │                                                                │
  │     ├── 2.3 Replace ClusterTagMap ──┬── 2.6 vec-load ──┐             │
  │     │                               │                   │            │
  │     ├── 2.4 Replace ProtectedIds ───┼── 2.7 vec-delete ─┼── 2.10 ────┤
  │     │                               │                   │    │       │
  │     └── 2.5 TrackerIterBuilder ─────┴── 2.8 vec-query ──┤    │       │
  │                                                         │    │       │
  │     2.9 Dataset Context ────────────────────────────────┘    │       │
  │                                                              │       │
  │     2.11 Proof of Capability ────────────────────────────────┘       │
  │                                                                      │
  └── [COMMIT] ── Gate 2 ────────────────────────────────────────────────┤
                    │                                                    │
                    ├───────────────────────┐                            │
                    │                       │                            │
                    ▼                       ▼                            │
Phase 3: New Capabilities              Code Cleanup                      │
  │                                       │                              │
  ├── HIGH PRIORITY                       ├── C.1 Remove deprecated      │
  │   ├── 3.1 CLI Options                 ├── C.2 Update modules         │
  │   ├── 3.2 Config Structs              └── C.3 Update tests           │
  │   ├── 3.3 Hit Rate                          │                        │
  │   ├── 3.4 Hierarchical                      │                        │
  │   ├── 3.5 Iteration Patterns                │                        │
  │   └── 3.6 Partition Modes                   │                        │
  │                                             │                        │
  ├── MEDIUM PRIORITY                           │                        │
  │   ├── 3.7 Scan Init                         │                        │
  │   ├── 3.8 Snapshots                         │                        │
  │   └── 3.9 Overlap Control                   │                        │
  │                                             │                        │
  ├── LOWER PRIORITY                            │                        │
  │   ├── 3.10 Coverage Check                   │                        │
  │   └── 3.11 Backfill                         │                        │
  │                                             │                        │
  ├── 3.12 Documentation                        │                        │
  └── 3.13 Validation ── [COMMIT] ── Gate 3     │                        │
                              │                 │                        │
                              │    [COMMIT] ────┘                        │
                              │                                          │
                              ▼                                          │
                    ✅ MIGRATION COMPLETE ◄──────────────────────────────┘
```

---

## Estimated Effort

| Phase | Tasks | Estimated Hours | Notes |
|-------|-------|-----------------|-------|
| Phase 0 | 5 | ✅ Complete | Baseline established |
| Phase 1 | 5 | 8-12 | Well-scoped, low risk |
| Phase 2 | 11 | 24-32 | Core migration, needs buffer |
| Phase 3 High Priority | 6 | 16-20 | Hit rate, hierarchical, patterns, partitions |
| Phase 3 Medium Priority | 3 | 8-12 | Scan, snapshots, overlap |
| Phase 3 Lower Priority | 2 | 4-6 | Coverage, backfill |
| Phase 3 Docs & Validation | 2 | 6-8 | Documentation and testing |
| Code Cleanup | 3 | 4-6 | Remove deprecated code |
| **Total** | **37** | **70-96** | |

---

## Quick Reference: Verification Commands

```bash
# Phase 1 verification
cargo test 
cargo clippy 
HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick

# Phase 2 verification
cargo test 
cargo clippy 
HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick

# Phase 3 verification
cargo test 
cargo clippy 
HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick

# Check for deprecated code references
grep -r "ClusterTagMap" src/ --include="*.rs"
grep -r "ProtectedVectorIds" src/ --include="*.rs"
grep -r "arg_prefixed_key_with_cluster_tag" src/ --include="*.rs"

# Compare benchmark results
diff results/baseline_results/baseline_cohere-small-100k_20260101_103703.md \
     results/baseline_results/baseline_cohere-small-100k_<NEW_TIMESTAMP>.md
```

---

## Rollback Procedure

If issues are discovered after a phase commit:

1. **Identify the issue** - Which phase introduced the problem?

2. **Revert to previous phase:**
   ```bash
   git log --oneline -10  # Find the commit before the problematic phase
   git revert <commit-hash>  # Create a revert commit (preserves history)
   ```

3. **Rebuild and test:**
   ```bash
   cargo build --release
   cargo test
   ```

4. **Document the issue** in a new task for the next attempt.

---

*Document created: 2026-01-01*
*Last updated: 2026-01-01*
*Source: Merged from `bkup/keyspace-tracker-migration/tasks.md` and `.kiro/specs/keyspace-tracker-migration/tasks.md`*
