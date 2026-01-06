# Requirements Document

## Introduction

This document specifies the requirements for migrating valkey-bench-rs from its current vector tracking and keyspace management implementation to the new `keyspace_tracker` crate. The migration involves three phases: (1) removal of internally-added cluster slot logic, (2) migration to keyspace_tracker while retaining 100% existing functionality, and (3) introduction of new capabilities.

The `keyspace_tracker` crate provides efficient, thread-safe tracking of key existence and flexible iteration patterns for database benchmarking, replacing the current cluster tag mapping, protected ID tracking, and key generation mechanisms.

## Glossary

- **Keyspace_Tracker**: External crate providing bitmap-based key existence tracking with flexible iteration patterns
- **PrefixTracker**: Core component of keyspace_tracker that tracks existence of IDs under a key prefix
- **ReferenceSet**: Bitmap-based set for efficient membership testing (replaces ProtectedVectorIds)
- **ClusterTagMap**: Current implementation that maps vector IDs to cluster hash tags (to be removed)
- **ProtectedVectorIds**: Current implementation for ground truth protection (to be replaced by ReferenceSet)
- **Slot_Affinity**: The cluster slot (0-16383) a key maps to, determined by CRC16 hash
- **Cluster_Hash_Tag**: Redis/Valkey sharding mechanism using `{tag}` in key names to control slot assignment. CRC16 is computed on the tag string instead of the full key. Example: `vec:{ABC}:000123` routes to slot based on "ABC". **NOT related to search index tags.**
- **Index_Tag_Field**: Valkey Search TAG field type in index schema for categorical filtering. A HASH field containing delimiter-separated strings (e.g., `"electronics,gadgets"`). Used in queries like `@category:{electronics}`. **NOT related to cluster hash tags.**
- **TagDistributionSet**: Code component that generates random index tag field values for benchmarking (e.g., `"tag1,tag2,tag3"`)
- **Ground_Truth**: Vector IDs that must be preserved for recall computation during queries
- **Iteration_Strategy**: Pattern for generating key sequences (sequential, random, zipfian, etc.)
- **Workload_Context**: Runtime state for workload execution including key iteration and dataset access

### Important Terminology Note

This document uses precise terminology to distinguish two unrelated "tag" concepts:

| Term | Context | Example | Code Location |
|------|---------|---------|---------------|
| **Cluster hash tag** | Key routing/sharding | `{ABC}` in `vec:{ABC}:000123` | `cluster_tag_map.rs`, `key_format.rs` |
| **Index tag field** | Search filtering | `category: "electronics,sale"` | `tag_distribution.rs`, `SearchConfig.tag_field` |

Phase 1 removes **cluster hash tag injection**. Index tag fields are unaffected by this migration.

## Requirements

### Requirement 1: Remove Cluster Hash Tag Injection from Keys

**User Story:** As a developer, I want to remove the artificially-injected cluster hash tags from keys, so that slot affinity is determined naturally by the full key hash rather than artificial `{tag}` patterns.

#### Acceptance Criteria

1. WHEN a key is generated in cluster mode, THE Key_Generator SHALL compute slot affinity using CRC16 of the complete key (prefix + padded ID)
2. WHEN a user specifies a cluster hash tag in the key prefix (e.g., `vec:{mytag}:`), THE Key_Generator SHALL preserve the user-specified hash tag for slot calculation
3. WHEN the ClusterTagMap module is removed, THE System SHALL no longer require scanning the cluster to build vector-id to slot mappings
4. WHEN keys are generated without user-specified cluster hash tags, THE Key_Generator SHALL NOT inject any artificial `{ABC}` style tags
5. WHEN the key format changes, THE System SHALL maintain backward compatibility by supporting parsing of both old (with cluster hash tags) and new (without) key formats during transition

**Note:** This requirement affects only cluster hash tags for sharding. Index tag fields (TAG type in search schema) are unaffected.

### Requirement 2: Migrate Key Generation to Keyspace Tracker

**User Story:** As a developer, I want key generation to use keyspace_tracker's iteration capabilities, so that I have flexible, thread-safe key iteration patterns.

#### Acceptance Criteria

1. WHEN a workload starts, THE System SHALL initialize a PrefixTracker with the configured key prefix and maximum ID
2. WHEN generating keys sequentially (--sequential flag), THE System SHALL use keyspace_tracker's sequential iteration mode
3. WHEN generating keys randomly (-r flag), THE System SHALL use keyspace_tracker's random iteration mode with the configured seed
4. WHEN a keyspace size is specified (-n or -r flags), THE System SHALL configure the tracker's max_id accordingly
5. WHEN multiple threads execute a workload, THE System SHALL partition the keyspace using tracker's partition(thread_id, num_threads) method
6. WHEN the --iteration flag specifies a strategy, THE System SHALL map it to the corresponding keyspace_tracker distribution (sequential, random, zipfian, subset)

### Requirement 3: Migrate Ground Truth Protection to ReferenceSet

**User Story:** As a developer, I want ground truth vector protection to use keyspace_tracker's ReferenceSet, so that I have efficient O(1) membership testing and set operations.

#### Acceptance Criteria

1. WHEN a dataset with ground truth is loaded, THE System SHALL build a ReferenceSet containing all unique ground truth vector IDs
2. WHEN vec-delete workload executes, THE System SHALL skip deletion of IDs that exist in the ground truth ReferenceSet
3. WHEN checking if a vector ID is protected, THE ReferenceSet SHALL provide O(1) lookup time
4. WHEN computing ground truth coverage before queries, THE System SHALL use ReferenceSet's count_existing_in() with a tracker snapshot
5. WHEN the ProtectedVectorIds module is removed, THE System SHALL provide equivalent functionality through ReferenceSet

### Requirement 4: Migrate Vector Existence Tracking

**User Story:** As a developer, I want vector existence tracking to use keyspace_tracker's PrefixTracker, so that I have efficient bitmap-based existence tracking with snapshot capabilities.

#### Acceptance Criteria

1. WHEN vec-load completes loading a vector, THE System SHALL call tracker.add(vector_id) to mark it as existing
2. WHEN vec-delete removes a vector, THE System SHALL call tracker.remove(vector_id) to mark it as deleted
3. WHEN a workload needs to know existing keys, THE System SHALL use tracker.iter().set_only() for iteration
4. WHEN a workload needs to know non-existing keys, THE System SHALL use tracker.iter().unset_only() for iteration
5. WHEN the ClusterTagMap's vector_exists() is removed, THE System SHALL provide equivalent functionality through PrefixTracker.exists()

### Requirement 5: Support Scan-Based Tracker Initialization

**User Story:** As a developer, I want to initialize the tracker from database scans, so that the tracker state matches actual database state for warm-start scenarios.

#### Acceptance Criteria

1. WHEN --tracker-init scan is specified, THE System SHALL run SCAN commands to discover existing keys
2. WHEN scanning discovers a key matching the prefix, THE System SHALL parse the ID and call tracker.add(id)
3. WHEN scan completes, THE Tracker state SHALL accurately reflect the database state
4. WHEN --tracker-init cold is specified (default), THE System SHALL start with an empty tracker
5. WHEN scanning in cluster mode, THE System SHALL scan all primary nodes to discover keys across all slots

### Requirement 6: Support Multi-Threaded Workload Partitioning

**User Story:** As a developer, I want thread-safe keyspace partitioning, so that multiple worker threads can operate on disjoint key ranges without coordination overhead.

#### Acceptance Criteria

1. WHEN multiple threads execute vec-load, THE System SHALL assign disjoint ID ranges to each thread using partition()
2. WHEN multiple threads execute vec-delete, THE System SHALL use atomic claim() operations to prevent duplicate deletions
3. WHEN threads need overlapping access (contention testing), THE System SHALL use overlapping() mode
4. WHEN a thread completes its partition, THE System SHALL not block other threads
5. WHEN workload uses random iteration, THE System SHALL ensure deterministic results with the same seed across runs

### Requirement 7: Support Workload Stop Conditions

**User Story:** As a developer, I want flexible stop conditions for workloads, so that I can control when workloads terminate based on count, time, or coverage.

#### Acceptance Criteria

1. WHEN -n requests is specified, THE System SHALL stop after processing the specified number of requests
2. WHEN --duration is specified, THE System SHALL stop after the specified time duration
3. WHEN iterating with limit(), THE System SHALL stop after yielding the specified number of items
4. WHEN all IDs in a partition are exhausted, THE Iterator SHALL return None to signal completion
5. WHEN using sample() with a probability, THE System SHALL probabilistically include items based on the configured ratio

### Requirement 8: Retain Existing CLI Compatibility

**User Story:** As a user, I want existing CLI options to continue working, so that my existing benchmark scripts remain functional.

#### Acceptance Criteria

1. WHEN --sequential is specified, THE System SHALL use sequential key iteration (equivalent to current behavior)
2. WHEN -r keyspace_len is specified, THE System SHALL use random iteration within the specified keyspace
3. WHEN -n requests is specified, THE System SHALL process the specified number of requests
4. WHEN --search-prefix is specified, THE System SHALL use it as the tracker's key prefix
5. WHEN --num-vectors and --vector-offset are specified, THE System SHALL configure effective limits on the tracker

### Requirement 9: Support Reference Set Backfill

**User Story:** As a developer, I want to restore missing ground truth vectors before query phase, so that recall measurements remain valid after delete workloads.

#### Acceptance Criteria

1. WHEN --reference-set-backfill is specified, THE System SHALL identify ground truth IDs missing from the tracker
2. WHEN missing IDs are identified, THE System SHALL reload those vectors from the dataset
3. WHEN backfill completes, THE System SHALL verify 100% ground truth coverage
4. IF backfill cannot restore all missing vectors, THEN THE System SHALL report an error with the coverage percentage

### Requirement 10: Support Reference Set Coverage Checking

**User Story:** As a developer, I want to verify ground truth coverage before running queries, so that I can detect when recall measurements would be invalid.

#### Acceptance Criteria

1. WHEN --reference-set-check is specified, THE System SHALL compute ground truth coverage percentage
2. WHEN coverage is computed, THE System SHALL report existing count, missing count, and percentage
3. IF coverage is below a threshold (e.g., 95%), THEN THE System SHALL warn the user
4. WHEN coverage check fails, THE System SHALL provide option to abort or continue

### Requirement 11: Support Workload Snapshots

**User Story:** As a developer, I want to capture tracker state before and after workloads, so that I can measure the impact of workload execution.

#### Acceptance Criteria

1. WHEN --workload-snapshot is specified, THE System SHALL take a snapshot before workload execution
2. WHEN workload completes, THE System SHALL take a post-workload snapshot
3. WHEN snapshots are available, THE System SHALL compute added_count_since() and removed_count_since()
4. WHEN reporting results, THE System SHALL include the delta statistics (keys added, keys removed)

### Requirement 12: Clean Up Deprecated Code

**User Story:** As a developer, I want deprecated cluster tag code removed, so that the codebase is simpler and easier to maintain.

#### Acceptance Criteria

1. WHEN Phase 1 completes, THE System SHALL have removed the ClusterTagMap struct and related functions
2. WHEN Phase 2 completes, THE System SHALL have removed the ProtectedVectorIds struct
3. WHEN migration completes, THE System SHALL have no references to the old cluster tag key format in production code
4. WHEN tests are updated, THE System SHALL verify both old and new key formats can be parsed (for backward compatibility)

### Requirement 13: Introduce New Iteration Capabilities

**User Story:** As a developer, I want access to advanced iteration patterns, so that I can simulate realistic workload distributions.

#### Acceptance Criteria

1. WHEN --iteration zipfian:SKEW is specified, THE System SHALL use Zipfian distribution for key access
2. WHEN --iteration hotspot:PCT:PROB is specified, THE System SHALL concentrate traffic on hot keys
3. WHEN --iteration subset:START:END is specified, THE System SHALL iterate only within the specified range
4. WHEN distribution is configured, THE System SHALL apply it consistently across all worker threads

### Requirement 14: Support Mixed Existence Ratio Iteration

**User Story:** As a developer, I want to iterate over a mix of existing and non-existing keys, so that I can simulate cache hit/miss ratios.

#### Acceptance Criteria

1. WHEN mixed_ratio(0.8) is configured, THE Iterator SHALL yield approximately 80% existing keys and 20% non-existing keys
2. WHEN iterating with mixed ratio, THE System SHALL use the tracker's existence bitmap to classify keys
3. WHEN hit ratio is specified via CLI, THE System SHALL configure the appropriate mixed_ratio

### Requirement 15: Validate Phase Completion

**User Story:** As a developer, I want validation gates between migration phases, so that I can ensure each phase is complete before proceeding.

#### Acceptance Criteria

1. WHEN Phase 1 completes, THE System SHALL pass all existing tests without cluster tag injection
2. WHEN Phase 2 completes, THE System SHALL pass all existing tests using keyspace_tracker
3. WHEN Phase 3 completes, THE System SHALL pass new tests for advanced iteration capabilities
4. IF validation fails, THEN THE System SHALL block progression to the next phase

### Requirement 16: Support Hierarchical Address Spaces

**User Story:** As a developer, I want to iterate over fields within keys, so that I can benchmark HASH-type workloads with realistic access patterns across multi-field data structures.

#### Acceptance Criteria

1. WHEN a workload targets HASH keys with multiple fields, THE System SHALL support iteration over (key, field) pairs as a two-dimensional address space
2. WHEN iterating hierarchically, THE System SHALL support key-major order (all fields of key 0, then all fields of key 1, etc.)
3. WHEN iterating hierarchically, THE System SHALL support field-major order (field f1 of all keys, then field f2 of all keys, etc.)
4. WHEN iterating hierarchically, THE System SHALL support random (key, field) pair selection
5. WHEN partitioning hierarchical iteration across threads, THE System SHALL ensure thread-safe claiming of (key, field) pairs without duplicates
6. WHEN --fields is specified via CLI, THE System SHALL use the provided field list for hierarchical iteration
7. WHEN hierarchical iteration is combined with existence filtering, THE System SHALL filter based on key existence (field existence is assumed if key exists)

### Requirement 17: Support Controlled Hit Rate Iteration

**User Story:** As a developer, I want to control the ratio of existing vs non-existing keys in iteration, so that I can simulate realistic cache hit/miss scenarios.

#### Acceptance Criteria

1. WHEN --hit-rate RATIO is specified, THE System SHALL yield approximately RATIO proportion of existing keys
2. WHEN hit rate is configured, THE System SHALL use the tracker's existence bitmap to classify keys
3. WHEN hit rate is 1.0, THE System SHALL behave equivalently to set_only() filter
4. WHEN hit rate is 0.0, THE System SHALL behave equivalently to unset_only() filter
5. WHEN hit rate is between 0.0 and 1.0, THE System SHALL probabilistically select from existing or non-existing keys

### Requirement 18: Support Delete-Rewrite-Read Cycles

**User Story:** As a developer, I want to simulate application lifecycles with delete/rewrite/read phases, so that I can benchmark realistic data churn scenarios.

#### Acceptance Criteria

1. WHEN a delete phase executes, THE System SHALL track which keys were removed via tracker.remove()
2. WHEN a rewrite phase follows deletion, THE System SHALL support configurable overlap with deleted keys (e.g., 80% rewrite to same keys, 20% to new keys)
3. WHEN a read phase follows rewrite, THE System SHALL use the updated tracker state for existence-aware iteration
4. WHEN --overlap-ratio is specified for rewrite, THE System SHALL prioritize recently-deleted keys according to the ratio
5. WHEN snapshots are enabled, THE System SHALL capture state before each phase for diff computation

### Requirement 19: Support Parallel Execution with Controlled Overlap

**User Story:** As a developer, I want to control the degree of key overlap between parallel workers, so that I can benchmark both contention-free and high-contention scenarios.

#### Acceptance Criteria

1. WHEN --partition-mode disjoint is specified, THE System SHALL assign non-overlapping key ranges to each thread
2. WHEN --partition-mode overlapping is specified, THE System SHALL allow all threads to access the full keyspace
3. WHEN --partition-mode partial:RATIO is specified, THE System SHALL create RATIO overlap between adjacent thread partitions
4. WHEN using overlapping mode, THE System SHALL use atomic claim() operations to track which keys have been processed
5. WHEN contention occurs, THE System SHALL handle it gracefully without data corruption
