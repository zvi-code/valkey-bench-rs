# Tasks Document

> **Task Completion Guidelines:**
> - A task is NOT complete until its **Acceptance Criteria** are met
> - Sub-items (bullets) are work steps, not completion markers
> - Creating a script ≠ running it; writing code ≠ testing it
> - Mark `[x]` only when the deliverable exists AND is verified working

---

## Phase 0: Preparation

### Task 0.1: Simple Baseline (SET/GET on Local Server)

**Goal:** Establish baseline performance metrics using basic SET/GET workloads on localhost. No dataset download required.

**Work Items:**
- [ ] Ensure local Valkey/Redis server is running on localhost:6379
- [ ] Run `bench/scripts/simple_baseline.sh --quick` for fast validation
- [ ] Run `bench/scripts/simple_baseline.sh --full` for comprehensive baseline
- [ ] Review output files in `results/baseline_results/`

**Acceptance Criteria:**
- [ ] Baseline results file exists: `results/baseline_results/set_*.txt` with throughput > 0
- [ ] Baseline results file exists: `results/baseline_results/get_*.txt` with throughput > 0  
- [ ] Baseline results file exists: `results/baseline_results/mixed_*.txt` with throughput > 0
- [ ] Memory usage recorded (initial vs final)

**Artifacts:**
- Script: `bench/scripts/simple_baseline.sh` ✓ created
- Results: `results/baseline_results/*.txt` (generated on run)

---

### Task 0.2: Vector Baseline (Requires Dataset)

**Goal:** Establish baseline for vector workloads (vec-load, vec-query, vec-delete) which require downloading a dataset first.

**Work Items:**
- [ ] Download dataset: `./prep_datasets/dataset.sh get cohere-small-100k`
- [ ] Verify dataset files exist in `datasets/` directory
- [ ] Run `HOST=localhost ./bench/scripts/baseline_benchmark.sh --dataset cohere-small-100k --quick`
- [ ] Review generated markdown report

**Acceptance Criteria:**
- [ ] Dataset files exist: `datasets/cohere-small-100k.yaml` and `datasets/cohere-small-100k.bin`
- [ ] vec-load throughput recorded in `results/baseline_results/baseline_*.md`
- [ ] vec-query latency (p50, p99) recorded
- [ ] vec-delete throughput recorded
- [ ] Memory usage at each phase recorded

**Artifacts:**
- Script: `bench/scripts/baseline_benchmark.sh` ✓ created
- Script: `bench/scripts/regression_test.sh` ✓ created
- Results: `results/baseline_results/baseline_*.md` (generated on run)

---

### Task 0.3: Verify keyspace_tracker Crate API

**Goal:** Confirm the keyspace_tracker crate provides all required APIs before starting migration.

**Work Items:**
- [ ] Clone/locate keyspace_tracker crate
- [ ] Review API for PrefixTracker: `add`, `remove`, `exists`, `count`, `snapshot`
- [ ] Review API for ReferenceSet: `contains`, `count_existing_in`, `missing_in`
- [ ] Review API for TrackerIterBuilder: `set_only`, `unset_only`, `partition`, `distribution`
- [ ] Review BitmapSnapshot diff operations
- [ ] Run `cargo test` in keyspace_tracker directory
- [ ] Run concurrent access stress test (if available)

**Acceptance Criteria:**
- [ ] All listed APIs exist and are public
- [ ] `cargo test` passes with 0 failures
- [ ] Thread-safety verified (concurrent tests pass OR documented as single-threaded)
- [ ] API gaps documented in `docs/keyspace-tracker-api-review.md` (if any)

**Artifacts:**
- Review document: `docs/keyspace-tracker-api-review.md`

---

### Task 0.4: Create Feature Flags

**Goal:** Add Cargo feature flags to enable gradual rollout and easy rollback.

**Work Items:**
- [ ] Add to `Cargo.toml`:
  ```toml
  [features]
  keyspace_tracker = ["dep:keyspace_tracker"]
  legacy_cluster_tags = []
  ```
- [ ] Create abstraction trait/module that can switch implementations
- [ ] Document rollback procedure

**Acceptance Criteria:**
- [ ] `cargo build --features keyspace_tracker` compiles
- [ ] `cargo build --features legacy_cluster_tags` compiles  
- [ ] `cargo build` (no features) compiles with current behavior
- [ ] Rollback procedure documented in `docs/ROLLBACK.md`

**Artifacts:**
- Updated: `Cargo.toml`
- New: `docs/ROLLBACK.md`

---

### Task 0.5: Update Test Infrastructure

**Goal:** Prepare test infrastructure to validate migration doesn't break existing functionality.

**Work Items:**
- [ ] Create equivalence test module comparing old vs new implementations
- [ ] Add integration tests for cluster mode
- [ ] Add stress tests for multi-threaded scenarios

**Acceptance Criteria:**
- [ ] Equivalence tests exist in `tests/` or `src/**/tests.rs`
- [ ] `cargo test` passes with new test infrastructure
- [ ] CI configuration updated (if applicable)

**Artifacts:**
- New tests in `tests/` directory

---

## Phase 1: Remove Cluster Hash Tag Injection

**Scope clarification:** This phase removes the artificial `{ABC}` cluster hash tags injected into keys for slot routing. It does NOT affect index tag fields (TAG type in search schema) used for filtering.

### Task 1.1: Update KeyFormat Module

**Goal:** Modify key formatting to stop injecting cluster hash tags.

**Work Items:**
- [ ] Modify `src/workload/key_format.rs`:
  - Update `format_key()` to not inject cluster hash tags
  - Keep `parse_key()` backward compatible (auto-detect format)
  - Update `total_len()` calculation for keys without hash tags
- [ ] Update unit tests in `key_format.rs`

**Acceptance Criteria:**
- [ ] `format_key()` produces keys without `{ABC}` pattern
- [ ] `parse_key()` correctly parses both old and new key formats
- [ ] All tests in `key_format.rs` pass: `cargo test key_format`
- [ ] Round-trip test: `parse_key(format_key(id)) == id`

---

### Task 1.2: Update CommandTemplate Module

**Goal:** Update command templates to use simplified key format.

**Work Items:**
- [ ] Modify `src/workload/command_template.rs`:
  - Deprecate `arg_prefixed_key_with_cluster_tag()` method
  - Create `arg_prefixed_key_simple()` as replacement
  - Update placeholder handling for simpler key format
- [ ] Update unit tests

**Acceptance Criteria:**
- [ ] New key generation method exists and is used
- [ ] Deprecated method marked with `#[deprecated]` attribute
- [ ] All tests pass: `cargo test command_template`
- [ ] Index tag placeholders (`arg_tag_placeholder()`) unchanged

---

### Task 1.3: Update Template Factory

**Goal:** Update all vector workload templates to use new key format.

**Work Items:**
- [ ] Modify `src/workload/template_factory.rs`:
  - Update `create_vec_load_template()` 
  - Update `create_vec_delete_template()`
  - Update `create_vec_update_template()`
  - Remove cluster hash tag logic from all templates

**Acceptance Criteria:**
- [ ] All `create_vec_*` functions use simple prefixed keys
- [ ] All tests pass: `cargo test template_factory`
- [ ] Index tag field generation in templates unchanged

---

### Task 1.4: Simplify ClusterTagMap

**Goal:** Remove hash tag storage, keep only existence tracking.

**Work Items:**
- [ ] Modify `src/cluster/cluster_tag_map.rs`:
  - Remove `VectorClusterMapping` struct
  - Remove hash tag storage from `ClusterTagMap`
  - Keep existence tracking via bitmap
  - Keep `claim_unmapped_id()` functionality
  - Update `add_mapping()` to just track existence

**Acceptance Criteria:**
- [ ] `ClusterTagMap` no longer stores hash tag strings
- [ ] Memory per entry reduced (no String allocation)
- [ ] All tests pass: `cargo test cluster_tag_map`
- [ ] `parse_vector_key()` handles both old and new formats

---

### Task 1.5: Phase 1 Integration Validation

**Goal:** Verify the complete Phase 1 changes work together.

**Work Items:**
- [ ] Run full test suite: `cargo test`
- [ ] Run vec-load with new key format
- [ ] Verify cluster mode still works
- [ ] Test backward compatibility with existing keys

**Acceptance Criteria:**
- [ ] `cargo test` passes with 0 failures
- [ ] Manual test: vec-load creates keys without `{ABC}` component
- [ ] Manual test: cluster mode routes requests correctly
- [ ] Manual test: can read keys created with old format
- [ ] Performance within 5% of Phase 0 baseline
- [ ] Breaking changes documented in `CHANGELOG.md`

---

## Phase 2: Migrate to keyspace_tracker

### Task 2.1: Add keyspace_tracker Dependency
- [ ] Update `Cargo.toml`:
  ```toml
  [dependencies]
  keyspace_tracker = { path = "/Volumes/workplace/keyspace_tracker" }
  ```
- [ ] Verify crate compiles with new dependency
- [ ] Run `cargo check` to validate integration

### Task 2.2: Create WorkloadContext Module
- [ ] Create `src/workload/context.rs`:
  - Define `WorkloadContext` struct with `PrefixTracker`
  - Implement `new()` from configuration
  - Implement `init_from_scan()` for scan-based initialization
  - Implement `build_reference_set()` from dataset ground truth
- [ ] Export from `src/workload/mod.rs`
- [ ] Add unit tests

### Task 2.3: Replace ClusterTagMap with PrefixTracker
- [ ] Update all usages of `ClusterTagMap`:
  - `vector_exists(id)` → `tracker.exists(id)`
  - `add_mapping(id, _)` → `tracker.add(id)`
  - `claim_unmapped_id()` → `tracker.iter().unset_only().claim()`
  - `count()` → `tracker.count()`
- [ ] Remove `src/cluster/cluster_tag_map.rs`
- [ ] Update `src/cluster/mod.rs` exports
- [ ] Update all import statements

### Task 2.4: Replace ProtectedVectorIds with ReferenceSet
- [ ] Update all usages of `ProtectedVectorIds`:
  - `is_protected(id)` → `reference.contains(id)`
  - `claim_deleteable_id()` → iterator with protection check
  - `protected_count()` → `reference.len()`
- [ ] Remove `src/cluster/protected_ids.rs`
- [ ] Update `src/cluster/mod.rs` exports
- [ ] Update all import statements

### Task 2.5: Integrate TrackerIterBuilder
- [ ] Update `src/workload/iteration.rs`:
  - Map `IterationStrategy::Sequential` → `.sequential()`
  - Map `IterationStrategy::Random` → `.seed(seed).random()`
  - Map `IterationStrategy::Subset` → `.id_range(start, end)`
  - Map `IterationStrategy::Zipfian` → `.distribution(AccessDistribution::Zipfian)`
- [ ] Create adapter function for existing code
- [ ] Update tests

### Task 2.6: Update vec-load Workload
- [ ] Modify vec-load implementation:
  - Call `tracker.add(id)` after successful HSET
  - Use `tracker.iter().unset_only()` for partial prefill
  - Integrate with `WorkloadContext`
- [ ] Update tests

### Task 2.7: Update vec-delete Workload
- [ ] Modify vec-delete implementation:
  - Use `reference.contains(id)` for protection check
  - Call `tracker.remove(id)` after successful DEL
  - Use `tracker.iter().set_only()` for iteration
- [ ] Update tests

### Task 2.8: Update vec-query Workload
- [ ] Modify vec-query implementation:
  - Integrate reference set for ground truth
  - Prepare for coverage checking (Phase 3)
- [ ] Update tests

### Task 2.9: Update Dataset Context
- [ ] Modify `src/dataset/context.rs`:
  - Add method to build `ReferenceSet` from ground truth
  - Integrate with `WorkloadContext`
- [ ] Update tests

### Task 2.10: Phase 2 Validation
- [ ] Run full test suite
- [ ] Benchmark performance comparison
- [ ] Verify memory usage reduction
- [ ] Test all workload types
- [ ] Document migration notes

### Task 2.11: Proof of Capability Demo
- [ ] Implement one new capability to validate architecture:
  - Add basic `mixed_ratio()` support to demonstrate existence-aware iteration
  - Create simple test: iterate with 80% existing / 20% non-existing keys
  - Verify statistical distribution matches configuration
- [ ] This validates the tracker integration before Phase 3 expansion
- [ ] Document any API adjustments needed for Phase 3

---

## Phase 3: Introduce New Capabilities

### Task 3.1: Add New CLI Options
- [ ] Update `src/config/cli.rs`:
  - Add `--keyspace-tracker` flag
  - Add `--tracker-init <MODE>` option (cold|scan)
  - Add `--reference-set-backfill` flag
  - Add `--reference-set-check` flag
  - Add `--workload-snapshot` flag
  - Add `--protect-reference-set` flag
  - Add `--iteration <STRATEGY>` option
  - Add `--hit-rate <RATIO>` option (0.0-1.0)
  - Add `--fields <FIELD_LIST>` option for hierarchical iteration
  - Add `--field-order <ORDER>` option (key-major|field-major|random)
  - Add `--partition-mode <MODE>` option (disjoint|overlapping|partial:RATIO)
  - Add `--overlap-ratio <RATIO>` option for rewrite phases
- [ ] Update help text and documentation

### Task 3.2: Add Tracker Configuration
- [ ] Update `src/config/benchmark_config.rs`:
  - Add `TrackerConfig` fields
  - Add `tracker_init_mode` field
  - Add `reference_set_options` field
  - Add `snapshot_enabled` field
  - Add `hit_rate` field
  - Add `hierarchical_config` field (fields, order)
  - Add `partition_mode` field
- [ ] Update configuration parsing

### Task 3.3: Implement Hit Rate Control (HIGH PRIORITY)
- [ ] Implement `--hit-rate` option:
  - Map to `tracker.iter().mixed_ratio(ratio)`
  - Validate ratio is between 0.0 and 1.0
  - Apply to all iteration-based workloads
- [ ] Add tests for various hit rates
- [ ] Verify statistical distribution matches configured ratio

### Task 3.4: Implement Hierarchical Key-Field Iteration (HIGH PRIORITY)
- [ ] Create `src/workload/hierarchical.rs`:
  - Define `HierarchicalSpace` struct
  - Implement `AddressableSpace` trait
  - Support key-major, field-major, random orders
- [ ] Integrate with existing `AddressableContext`
- [ ] Update HSET/HGET templates to use field from iterator
- [ ] Add CLI parsing for `--fields` and `--field-order`
- [ ] Add tests for (key, field) iteration patterns
- [ ] Export from `src/workload/mod.rs`

### Task 3.5: Implement Advanced Iteration Patterns (HIGH PRIORITY)
- [ ] Implement `--iteration` options:
  - `sequential` - sequential iteration
  - `random` or `random:SEED` - random with optional seed
  - `zipfian:SKEW` - Zipfian distribution
  - `hotspot:PCT:PROB` - hotspot distribution
  - `subset:START:END` - range iteration
  - `latest:PCT:PROB` - recent keys prioritized
- [ ] Map to `TrackerIterBuilder.distribution()`
- [ ] Update iteration parsing
- [ ] Add tests for each pattern

### Task 3.6: Implement Parallel Partition Modes (HIGH PRIORITY)
- [ ] Implement `--partition-mode` options:
  - `disjoint` - non-overlapping ranges (default)
  - `overlapping` - full keyspace access for all threads
  - `partial:RATIO` - configurable overlap between threads
- [ ] Create partition calculation functions
- [ ] Integrate with `TrackerIterBuilder.partition()`
- [ ] Add tests for overlap calculations
- [ ] Verify thread-safety under contention

### Task 3.7: Implement Scan-Based Initialization
- [ ] Implement `--tracker-init scan`:
  - Run SCAN commands on all primary nodes
  - Parse keys and populate tracker
  - Report initialization statistics
- [ ] Add progress reporting
- [ ] Handle cluster mode scanning

### Task 3.8: Implement Workload Snapshots
- [ ] Implement `--workload-snapshot`:
  - Take snapshot before workload execution
  - Take snapshot after workload completion
  - Compute and report diff (added/removed)
- [ ] Add snapshot statistics to results
- [ ] Support multi-phase snapshot chains

### Task 3.9: Implement Delete-Rewrite Overlap Control
- [ ] Implement `--overlap-ratio` for rewrite phases:
  - Track deleted keys during delete phase
  - Prioritize deleted keys during rewrite based on ratio
  - Support 0.0 (all new keys) to 1.0 (all deleted keys)
- [ ] Create `OverlapAwareIterator`
- [ ] Add tests for overlap behavior

### Task 3.10: Implement Reference Set Coverage Check
- [ ] Implement `--reference-set-check`:
  - Compute coverage percentage
  - Report existing/missing counts
  - Warn if coverage below threshold
  - Option to abort if coverage too low
- [ ] Add coverage reporting to results

### Task 3.11: Implement Reference Set Backfill
- [ ] Implement `--reference-set-backfill`:
  - Identify missing reference set members
  - Reload vectors from dataset
  - Verify 100% coverage after backfill
  - Report backfill statistics
- [ ] Add error handling for partial backfill

### Task 3.12: Update Documentation
- [ ] Create `docs/KEYSPACE_TRACKER.md`:
  - Feature overview
  - CLI options reference
  - Usage examples for each capability
  - Hierarchical iteration examples
  - Delete-rewrite-read cycle examples
  - Partition mode examples
  - Migration guide from old behavior
- [ ] Update `README.md` with new features
- [ ] Update `EXAMPLES.md` with tracker examples

### Task 3.13: Phase 3 Validation
- [ ] Run full test suite
- [ ] Test all new CLI options
- [ ] Test advanced iteration patterns
- [ ] Test hierarchical iteration
- [ ] Test partition modes under load
- [ ] Test delete-rewrite cycles
- [ ] Performance benchmarks
- [ ] Documentation review

---

## Code Cleanup Tasks

### Task C.1: Remove Deprecated Cluster Hash Tag Code
- [ ] Remove `src/cluster/cluster_tag_map.rs` (after Phase 2)
- [ ] Remove `src/cluster/protected_ids.rs` (after Phase 2)
- [ ] Remove deprecated methods from `command_template.rs`:
  - `arg_prefixed_key_with_cluster_tag()` (cluster hash tag injection)
- [ ] Clean up unused imports
- [ ] **Preserve:** All index tag field code (`TagDistributionSet`, `arg_tag_placeholder()`, etc.)

### Task C.2: Update Module Structure
- [ ] Update `src/cluster/mod.rs`:
  - Remove deprecated module exports (`cluster_tag_map`, `protected_ids`)
  - Keep `topology.rs` for cluster topology
- [ ] Update `src/workload/mod.rs`:
  - Export new `context` module
  - Update iteration exports
  - **Preserve:** `tag_distribution.rs` exports (index tag fields)

### Task C.3: Update Tests
- [ ] Remove tests for deprecated cluster hash tag code
- [ ] Add tests for new functionality
- [ ] Update integration tests
- [ ] Add benchmark comparison tests
- [ ] **Verify:** Index tag field tests remain passing

---

## Validation Gates

### Gate 0: Preparation Complete
**All acceptance criteria from Phase 0 tasks must be checked off.**

- [ ] Task 0.1 complete: Simple baseline results exist in `results/baseline_results/`
- [ ] Task 0.2 complete: Vector baseline results exist (or explicitly skipped with justification)
- [ ] Task 0.3 complete: keyspace_tracker API review document exists
- [ ] Task 0.4 complete: Feature flags compile successfully
- [ ] Task 0.5 complete: Test infrastructure in place

**⛔ STOP: Human review required before proceeding to Phase 1**

### Gate 1: Phase 1 Complete
**All acceptance criteria from Phase 1 tasks must be checked off.**

- [ ] Task 1.1 complete: KeyFormat tests pass, keys have no `{ABC}`
- [ ] Task 1.2 complete: CommandTemplate tests pass, deprecated method marked
- [ ] Task 1.3 complete: Template factory tests pass
- [ ] Task 1.4 complete: ClusterTagMap simplified, tests pass
- [ ] Task 1.5 complete: Full integration validated, performance verified

**⛔ STOP: Human review required before proceeding to Phase 2**

### Gate 2: Phase 2 Complete
**All acceptance criteria from Phase 2 tasks must be checked off.**

- [ ] Task 2.1-2.9 complete: All module migrations done
- [ ] Task 2.10 complete: Full validation passed
- [ ] Task 2.11 complete: Proof of capability demo working
- [ ] `cargo test` passes with 0 failures
- [ ] Performance within 5% of Phase 0 baseline
- [ ] Memory usage reduced (verify bitmap vs String storage)

**⛔ STOP: Human review required before proceeding to Phase 3**

### Gate 3: Phase 3 Complete
**All acceptance criteria from Phase 3 tasks must be checked off.**

- [ ] High priority features working and tested:
  - [ ] Hit rate control (Task 3.3)
  - [ ] Hierarchical iteration (Task 3.4)
  - [ ] Advanced patterns (Task 3.5)
  - [ ] Partition modes (Task 3.6)
- [ ] Documentation updated (Task 3.12)
- [ ] All tests pass: `cargo test`
- [ ] Examples in `EXAMPLES.md` updated

**✅ Migration complete**

---

## Task Dependencies

```
Phase 1:
  1.1 → 1.2 → 1.3 → 1.4 → 1.5 (Gate 1)

Phase 2:
  2.1 → 2.2 → 2.3 ─┬→ 2.6 ─┐
                   ├→ 2.7 ─┼→ 2.10 (Gate 2)
        2.4 ───────┤       │
        2.5 ───────┼→ 2.8 ─┤
        2.9 ───────┘       │
                           │
Phase 3 (reordered by priority):
  3.1 → 3.2 ───────────────┘
        │
        ├─── HIGH PRIORITY (core capabilities) ───┐
        │                                         │
        ├→ 3.3 (hit rate) ────────────────────────┤
        ├→ 3.4 (hierarchical) ────────────────────┤
        ├→ 3.5 (iteration patterns) ──────────────┤
        ├→ 3.6 (partition modes) ─────────────────┤
        │                                         │
        ├─── MEDIUM PRIORITY (operational) ───────┤
        │                                         │
        ├→ 3.7 (scan init) ───────────────────────┤
        ├→ 3.8 (snapshots) ───────────────────────┤
        ├→ 3.9 (overlap control) ─────────────────┤
        │                                         │
        ├─── LOWER PRIORITY (nice-to-have) ───────┤
        │                                         │
        ├→ 3.10 (coverage check) ─────────────────┤
        ├→ 3.11 (backfill) ───────────────────────┤
        │                                         │
        └→ 3.12 (docs) → 3.13 (Gate 3) ───────────┘

Cleanup:
  C.1, C.2, C.3 (after Gate 2)
```

## Estimated Effort

| Phase | Tasks | Estimated Hours | Notes |
|-------|-------|-----------------|-------|
| Phase 1 | 5 | 8-12 | Well-scoped, low risk |
| Phase 2 | 10 | 24-32 | Core migration, needs buffer for integration |
| Phase 3 High Priority | 4 | 16-20 | Hit rate, hierarchical, patterns, partitions |
| Phase 3 Medium Priority | 3 | 8-12 | Scan, snapshots, overlap |
| Phase 3 Lower Priority | 2 | 4-6 | Coverage, backfill |
| Phase 3 Docs & Validation | 2 | 6-8 | Documentation and testing |
| Cleanup | 3 | 4-6 | Remove deprecated code |
| **Total** | **29** | **70-96** | More realistic with new scope |

## Priority Rationale

**HIGH PRIORITY (Phase 3.3-3.6):**
These enable the core use cases that motivated the migration:
- Hit rate control → cache simulation
- Hierarchical iteration → HASH workloads
- Advanced patterns → realistic distributions
- Partition modes → contention testing

**MEDIUM PRIORITY (Phase 3.7-3.9):**
Operational features that improve usability:
- Scan init → warm start scenarios
- Snapshots → lifecycle tracking
- Overlap control → delete-rewrite cycles

**LOWER PRIORITY (Phase 3.10-3.11):**
Vector-specific features (ground truth protection):
- Coverage check → recall validation
- Backfill → restore after deletes

These are important for vector benchmarks but not for general workloads.
