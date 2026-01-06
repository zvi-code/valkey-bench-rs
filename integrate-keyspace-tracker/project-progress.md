# Keyspace Tracker Integration - Project Progress

> **Purpose:** Track progress of keyspace_tracker integration into valkey-bench-rs.  
> **Reference Plan:** [tasks-ver-B.md](./tasks-ver-B.md) (authoritative task document)  
> **Started:** 2026-01-01

---

## Quick Links

| Document | Description |
|----------|-------------|
| [tasks-ver-B.md](./tasks-ver-B.md) | Detailed tasks with acceptance criteria |
| [design.md](./design.md) | Architecture and migration strategy |
| [requirements.md](./requirements.md) | Original requirements |
| [keyspace-tracker-api-review.md](./keyspace-tracker-api-review.md) | keyspace_tracker crate API |

---

## Project Overview

### Goal
Migrate valkey-bench-rs from custom `ClusterTagMap` and `ProtectedVectorIds` implementations to the `keyspace_tracker` crate, reducing memory overhead and adding new iteration capabilities.

### Components to Replace

| Current Component | Location | Replacement | Phase |
|-------------------|----------|-------------|-------|
| ClusterTagMap | `src/cluster/cluster_tag_map.rs` | PrefixTracker | Phase 2 |
| ProtectedVectorIds | `src/cluster/protected_ids.rs` | ReferenceSet | Phase 2 |
| KeyFormat (cluster tags) | `src/workload/key_format.rs` | Simplified format | Phase 1 |
| IterationStrategy | `src/workload/iteration.rs` | TrackerIterBuilder | Phase 2 |

### Migration Phases

| Phase | Description | Status |
|-------|-------------|--------|
| Phase 0 | Baseline establishment & preparation | ✅ Complete |
| Phase 1 | Remove cluster hash tag injection | ✅ Complete |
| Phase 2 | Migrate to keyspace_tracker | 🔲 Not Started |
| Phase 3 | New capabilities (hit rate control, etc.) | 🔲 Not Started |

---

## Phase 0: Preparation ✅ COMPLETE

**Completed:** 2026-01-01

### Summary
Established performance baselines and verified keyspace_tracker API compatibility.

### Key Decisions
1. **Task Plan:** Using `tasks-ver-B.md` as authoritative (has performance gates)
2. **Dependency:** Will use GitHub repo `github.com/zvi-code/keyspace_tracker`
3. **Test Environment:** ElastiCache Valkey 8.2.0 cluster (zvi-vss-16xl)

### Baseline Results (2026-01-01)

#### Simple KV Baseline (200K keys, 500B values)

| Workload | Throughput | P50 | P99 | Max |
|----------|------------|-----|-----|-----|
| Sequential SET (prefill) | 412,178 req/s | 0.93ms | 1.85ms | 5.28ms |
| Random SET (override) | 498,782 req/s | 0.77ms | 1.29ms | 4.30ms |
| Random GET | 920,796 req/s | 0.41ms | 0.65ms | 3.76ms |
| Single-client GET | 9,179 req/s | 0.10ms | 0.13ms | 2.79ms |
| Single-client SET | 8,972 req/s | 0.11ms | 0.13ms | 2.94ms |

#### Vector Baseline: MNIST (60K vectors, 784 dims)

| Workload | Throughput | P50 | P99 | Recall |
|----------|------------|-----|-----|--------|
| vec-load | 31,580 req/s | 1.63ms | 5.36ms | - |
| vec-query | 19,112 req/s | 0.88ms | 1.61ms | 99.91% |
| vec-delete | 46,959 req/s | 0.27ms | 0.46ms | - |

#### Vector Baseline: Cohere-1M (1M vectors, 768 dims)

| Workload | Throughput | P50 | P99 | Recall |
|----------|------------|-----|-----|--------|
| vec-load | 14,247 req/s | 3.74ms | 10.38ms | - |
| vec-query | 12,040 req/s | 1.50ms | 4.30ms | 96.00% |
| vec-delete | 46,412 req/s | 0.28ms | 0.46ms | - |

### Artifacts Created
- `baseline_results/baseline_report_20260101_*.txt` - Simple KV results
- `baseline_results/baseline_mnist_20260101_*.md` - MNIST vector results  
- `baseline_results/baseline_cohere-medium-1m_20260101_*.md` - Cohere 1M results
- `bench/scripts/simple_baseline.sh` - Automated KV benchmark (updated)
- `bench/scripts/baseline_benchmark.sh` - Automated vector benchmark (updated with `--cleanup` flag)

### Verification
- [x] keyspace_tracker unit tests pass (85/85)
- [x] Simple KV baseline captured
- [x] Vector baselines captured (MNIST, Cohere-1M)
- [x] Baseline scripts automated (no manual intervention required)

---

## Phase 1: Remove Cluster Hash Tag Injection ✅ COMPLETE

**Status:** Complete
**Completed:** 2026-01-01  
**Reference:** [tasks-ver-B.md](./tasks-ver-B.md) - Tasks 1.1-1.5

### Objective
Remove automatic `{HASHTAG}` injection into keys. This is a standalone refactor that doesn't require keyspace_tracker.

### Tasks

| Task | Description | Status |
|------|-------------|--------|
| 1.1 | Update KeyFormat module | ✅ |
| 1.2 | Update CommandTemplate module | ✅ |
| 1.3 | Update Template Factory | ✅ |
| 1.4 | Simplify ClusterTagMap to bitmap | ✅ |
| 1.5 | Integration Testing | ✅ |

### Code Changes

**`src/workload/key_format.rs`:**
- Removed `with_cluster_tags()` and `without_cluster_tags()` constructors
- Updated `KeyFormat::new()` to always use simple key format
- Simplified `format_key()` to ignore cluster tag parameter
- Updated `total_len()` for simple format only
- Parser still supports legacy keys for backward compatibility

**`src/workload/command_template.rs`:**
- Removed `TemplateArg::PrefixedKeyWithClusterTag` variant entirely
- Removed `arg_prefixed_key_with_cluster_tag()` method entirely
- Removed unused imports for cluster tag constants

**`src/cluster/cluster_tag_map.rs`:**
- Removed `VectorClusterMapping` struct entirely
- Replaced `Vec<VectorClusterMapping>` (6 bytes/entry) with `Vec<AtomicU64>` bitmap (1 bit/entry)
- Memory reduced by ~98% (1 bit vs 48 bits per entry)
- Added `memory_usage_bytes()` for tracking
- Atomic bitmap operations for thread-safe concurrent updates

### Key Format Change

| Before (Phase 0) | After (Phase 1) |
|------------------|-----------------|
| `vec:{ABC}:000000000123` | `vec:000000000123` |
| Keys tied to specific cluster slots | Keys distributed naturally |

### Performance Comparison (vs Phase 0 Baseline)

#### Simple KV Workloads

| Workload | Phase 0 | Phase 1 | Change |
|----------|---------|---------|--------|
| Sequential Prefill | 412,178 req/s | 420,318 req/s | **+2.0%** ✅ |
| Random Override | 498,782 req/s | 501,755 req/s | **+0.6%** ✅ |
| Random GET | 920,796 req/s | 907,236 req/s | **-1.5%** ✅ |
| Single-client GET P99 | 0.13ms | 0.13ms | **0%** ✅ |
| Single-client SET P99 | 0.13ms | 0.12ms | **-8%** ✅ |

#### Vector Workloads (MNIST, 60K vectors)

| Workload | Phase 0 | Phase 1 | Change |
|----------|---------|---------|--------|
| vec-load | 31,580 req/s | 30,775 req/s | **-2.5%** ✅ |
| vec-query | 19,112 req/s | 18,328 req/s | **-4.1%** ✅ |
| vec-delete | 46,959 req/s | 45,500 req/s | **-3.1%** ✅ |
| vec-query recall | 99.91% | 99.95% | **+0.04%** ✅ |

**Verdict:** All metrics within 5% threshold. Phase 1 gate **PASSED**.

### Memory Improvement

| Structure | Phase 0 | Phase 1 | Improvement |
|-----------|---------|---------|-------------|
| ClusterTagMap (1M vectors) | ~6MB | ~125KB | **98% reduction** |
| ClusterTagMap (10M vectors) | ~60MB | ~1.25MB | **98% reduction** |

### Test Results
- All 264 tests pass
- New bitmap-specific tests added
- Legacy key parsing still works for backward compatibility

### Progress Log

**2026-01-01 (Initial Attempt):**
- Started Phase 1 with deprecation annotations
- Review identified incomplete implementation

**2026-01-01 (Completion):**
- Updated tasks-ver-B.md to require complete removal (not deprecation)
- Removed all deprecated code from key_format.rs
- Removed PrefixedKeyWithClusterTag from command_template.rs
- Implemented bitmap-based ClusterTagMap (1 bit per entry)
- All 264 tests pass
- Performance verified within 5% of baseline

---

## Phase 2: Migrate to keyspace_tracker

**Status:** 🔲 Not Started  
**Reference:** [tasks-ver-B.md](./tasks-ver-B.md) - Tasks 2.1-2.11

### Objective
Replace custom implementations with keyspace_tracker crate types.

### Key Migrations
- `ClusterTagMap` → `PrefixTracker`
- `ProtectedVectorIds` → `ReferenceSet`
- Iteration patterns → `TrackerIterBuilder`

### Tasks

| Task | Description | Status |
|------|-------------|--------|
| 2.1 | Add keyspace_tracker dependency | 🔲 |
| 2.2 | Create WorkloadContext abstraction | 🔲 |
| 2.3 | Migrate ClusterTagMap to PrefixTracker | 🔲 |
| 2.4 | Migrate ProtectedVectorIds to ReferenceSet | 🔲 |
| 2.5 | Update iteration strategies | 🔲 |
| 2.6-2.11 | Additional integration tasks | 🔲 |

### Performance Gate
After Phase 2, verify:
- **Throughput:** Within 5% of Phase 1 results
- **Memory:** Reduced (1-bit vs 6-byte per entry)
- **All tests pass:** `cargo test`

### Progress Log

_No progress yet._

---

## Phase 3: New Capabilities

**Status:** 🔲 Not Started  
**Reference:** [tasks-ver-B.md](./tasks-ver-B.md) - Phase 3 tasks

### Planned Features (Priority Order)
1. **Hit Rate Control** (HIGH) - Simulate cache misses
2. **Scan-based Initialization** - Warm-start from existing data
3. **Reference Set Backfill** - Enhanced recall validation
4. **Bitmap Snapshots** - Before/after state comparison

### Progress Log

_No progress yet._

---

## Discussion Log

### 2026-01-01: Project Kickoff

**Topics Discussed:**
1. Reviewed codebase structure and current implementations
2. Compared task plans (tasks.md vs tasks-ver-B.md)
3. Selected tasks-ver-B.md as authoritative (has performance gates)
4. Established baselines on ElastiCache cluster

**Key Decisions:**
- Use GitHub dependency for keyspace_tracker (not local path)
- Phase 1 can proceed independently (no external dependencies)
- Automated baseline scripts required for CI/CD gates
- Scripts must work unchanged between phases

**Script Updates Made:**
- `simple_baseline.sh`: Fixed hit-rate parsing, added proper flow (prefill→verify→override→verify→read)
- `baseline_benchmark.sh`: Added `--cleanup`/`--no-cleanup` flags, added `--cluster` flag

**Environment:**
- Host: `zvi-vss-16xl.ajfdds.clustercfg.euw1devo.cache.amazonaws.com`
- Engine: Valkey 8.2.0 (ElastiCache)
- Topology: 1 primary, 3 nodes total

---

## Commit History

| Date | Commit | Description |
|------|--------|-------------|
| 2026-01-01 | `f5830a5` | Phase 0 complete: baselines established |
| 2026-01-01 | _pending_ | Phase 1 complete: cluster hash tag injection removed |

---

## Notes

### Running Baselines

```bash
# Simple KV baseline (quick mode, ~2 min)
HOST=<host> bash bench/scripts/simple_baseline.sh --quick --cluster

# Vector baseline with auto-cleanup (~5 min for MNIST)
HOST=<host> bash bench/scripts/baseline_benchmark.sh --quick --dataset mnist --cleanup

# Vector baseline for Cohere-1M (~2 min load)
HOST=<host> bash bench/scripts/baseline_benchmark.sh --quick --dataset cohere-medium-1m --cleanup
```

### Performance Gate Thresholds
- **Throughput regression:** Max 5% decrease allowed
- **P99 latency regression:** Max 10% increase allowed
- **Memory:** Should decrease or stay constant

### Rollback Strategy
If performance regresses:
1. Revert to previous commit
2. Re-run baselines to verify recovery
3. Investigate root cause before re-attempting