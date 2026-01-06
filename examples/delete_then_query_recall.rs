//! Example: Vector Search Recall Under Delete Conditions
//!
//! This example demonstrates how to test vector search recall when vectors
//! are randomly deleted. It showcases two modes:
//!
//! 1. **Protected Mode**: Ground truth vectors are never deleted, so recall
//!    should remain constant (useful for baseline measurements).
//!
//! 2. **Adjusted Mode**: Ground truth vectors can be deleted, and recall is
//!    computed against only the surviving ground truth vectors.
//!
//! ## Usage Flow
//!
//! ```text
//! ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
//! │  1. Load Data   │───▶│  2. Delete %    │───▶│  3. Query+Recall│
//! │  (vec-load)     │    │  (vec-delete)   │    │  (vec-query)    │
//! └─────────────────┘    └─────────────────┘    └─────────────────┘
//!        │                       │                      │
//!        ▼                       ▼                      ▼
//!   VectorExistenceMap   GroundTruthAwareRecall  AdjustedRecallStats
//!   tracks all vectors    decides what to skip    shows GT coverage
//! ```

use std::sync::Arc;

use keyspace_tracker::{PrefixTracker, ReferenceSet, TrackerConfig};

// These would be imported from valkey_bench_rs in actual usage
// use valkey_bench_rs::keyspace::{
//     GroundTruthAwareRecall, GroundTruthMode, ProtectedIds, VectorExistenceMap,
// };
// use valkey_bench_rs::dataset::DatasetContext;

/// Simulates a ground truth dataset for demonstration
struct MockDataset {
    num_vectors: u64,
    num_queries: u64,
    neighbors_per_query: usize,
    /// Query -> [neighbor_ids]
    ground_truth: Vec<Vec<u64>>,
}

impl MockDataset {
    fn new(num_vectors: u64, num_queries: u64, neighbors_per_query: usize) -> Self {
        // Generate deterministic ground truth
        let mut ground_truth = Vec::new();
        for q in 0..num_queries {
            let neighbors: Vec<u64> = (0..neighbors_per_query as u64)
                .map(|n| (q * 10 + n) % num_vectors)
                .collect();
            ground_truth.push(neighbors);
        }
        Self {
            num_vectors,
            num_queries,
            neighbors_per_query,
            ground_truth,
        }
    }

    fn get_all_gt_ids(&self) -> Vec<u64> {
        self.ground_truth
            .iter()
            .flat_map(|v| v.iter().copied())
            .collect()
    }

    fn get_query_gt(&self, query_idx: u64) -> &[u64] {
        &self.ground_truth[query_idx as usize]
    }
}

fn main() {
    println!("=== Vector Search Recall Under Delete Conditions ===\n");

    // Setup mock dataset
    let dataset = MockDataset::new(1000, 100, 10);
    let k = 10; // Top-k for recall

    // Build ground truth ReferenceSet
    let gt_ids = dataset.get_all_gt_ids();
    let gt_reference = ReferenceSet::from_iter(gt_ids.iter().copied());
    println!(
        "Dataset: {} vectors, {} queries, {} GT neighbors per query",
        dataset.num_vectors, dataset.num_queries, dataset.neighbors_per_query
    );
    println!("Total unique GT vectors: {}\n", gt_reference.len());

    // =========================================================================
    // MODE 1: Protected Ground Truth
    // =========================================================================
    println!("--- MODE 1: Protected Ground Truth ---");
    println!("In this mode, GT vectors are NEVER deleted.\n");

    // Create tracker to track existing vectors
    let tracker = PrefixTracker::new(
        TrackerConfig::simple("vec:")
            .with_max_id(dataset.num_vectors)
            .with_initial_capacity(dataset.num_vectors as usize),
    );

    // Simulate loading all vectors
    for id in 0..dataset.num_vectors {
        tracker.add(id);
    }
    println!("Loaded {} vectors", tracker.count());

    // Delete 50% of vectors, but protect GT
    let delete_ratio = 0.5;
    let mut deleted_count = 0u64;
    let mut skipped_gt = 0u64;

    for id in 0..dataset.num_vectors {
        // Simulate random selection based on hash
        let should_delete = (id * 17 + 31) % 100 < (delete_ratio * 100.0) as u64;
        if should_delete {
            if gt_reference.contains(id) {
                skipped_gt += 1;
                continue; // Protected mode: skip GT
            }
            tracker.remove(id);
            deleted_count += 1;
        }
    }

    println!(
        "After deletion: {} remaining ({} deleted, {} GT protected)",
        tracker.count(),
        deleted_count,
        skipped_gt
    );

    // Check GT coverage (should be 100% in protected mode)
    let snapshot = tracker.snapshot();
    let gt_existing = gt_reference.count_existing_in(&snapshot);
    let gt_coverage = gt_existing as f64 / gt_reference.len() as f64;
    println!(
        "GT coverage: {}/{} ({:.1}%) - should be 100%\n",
        gt_existing,
        gt_reference.len(),
        gt_coverage * 100.0
    );

    // Simulate query and compute recall
    let query_idx = 0u64;
    let gt_neighbors = dataset.get_query_gt(query_idx);

    // Simulate search results (perfect results for this example)
    let search_results: Vec<u64> = gt_neighbors.iter().take(k).copied().collect();

    // Compute recall (standard - all GT exist)
    let matches = search_results
        .iter()
        .filter(|&id| gt_neighbors.contains(id))
        .count();
    let recall = matches as f64 / k as f64;
    println!(
        "Query {}: recall@{} = {:.3} ({}/{} matches)",
        query_idx, k, recall, matches, k
    );
    println!("→ Recall unchanged because all GT vectors were protected\n");

    // =========================================================================
    // MODE 2: Adjusted Recall (GT can be deleted)
    // =========================================================================
    println!("--- MODE 2: Adjusted Recall ---");
    println!("In this mode, GT vectors CAN be deleted.\n");

    // Create fresh tracker
    let tracker2 = PrefixTracker::new(
        TrackerConfig::simple("vec:")
            .with_max_id(dataset.num_vectors)
            .with_initial_capacity(dataset.num_vectors as usize),
    );

    // Load all vectors
    for id in 0..dataset.num_vectors {
        tracker2.add(id);
    }

    // Delete 50% of vectors INCLUDING GT
    let mut deleted_count2 = 0u64;
    let mut deleted_gt = 0u64;

    for id in 0..dataset.num_vectors {
        let should_delete = (id * 17 + 31) % 100 < (delete_ratio * 100.0) as u64;
        if should_delete {
            if gt_reference.contains(id) {
                deleted_gt += 1;
            }
            tracker2.remove(id);
            deleted_count2 += 1;
        }
    }

    println!(
        "After deletion: {} remaining ({} deleted, {} were GT)",
        tracker2.count(),
        deleted_count2,
        deleted_gt
    );

    // Check GT coverage (will be < 100%)
    let snapshot2 = tracker2.snapshot();
    let gt_existing2 = gt_reference.count_existing_in(&snapshot2);
    let gt_coverage2 = gt_existing2 as f64 / gt_reference.len() as f64;
    println!(
        "GT coverage: {}/{} ({:.1}%)\n",
        gt_existing2,
        gt_reference.len(),
        gt_coverage2 * 100.0
    );

    // Compute ADJUSTED recall for query 0
    let query_idx = 0u64;
    let gt_neighbors = dataset.get_query_gt(query_idx);

    // Find surviving GT for this query
    let surviving_gt: Vec<u64> = gt_neighbors
        .iter()
        .filter(|&&id| snapshot2.test(id as usize))
        .copied()
        .collect();

    println!("Query {} GT analysis:", query_idx);
    println!("  Original GT: {:?}", gt_neighbors);
    println!("  Surviving GT: {:?}", &surviving_gt);
    println!(
        "  Query GT coverage: {}/{} ({:.1}%)",
        surviving_gt.len(),
        gt_neighbors.len(),
        (surviving_gt.len() as f64 / gt_neighbors.len() as f64) * 100.0
    );

    // Simulate search results - in real scenario, search would return different results
    // because some vectors are deleted
    let search_results: Vec<u64> = surviving_gt.iter().take(k).copied().collect();

    // Compute ADJUSTED recall (against surviving GT only)
    let effective_k = k.min(surviving_gt.len());
    let matches = if effective_k > 0 {
        search_results
            .iter()
            .take(effective_k)
            .filter(|&id| surviving_gt.contains(id))
            .count()
    } else {
        0
    };

    let adjusted_recall = if effective_k > 0 {
        matches as f64 / effective_k as f64
    } else {
        0.0 // All GT deleted
    };

    println!(
        "\nAdjusted recall@{} = {:.3} ({}/{} matches against surviving GT)",
        k, adjusted_recall, matches, effective_k
    );
    println!("→ Recall computed only against the {} surviving GT vectors", surviving_gt.len());

    // =========================================================================
    // Summary
    // =========================================================================
    println!("\n=== Summary ===");
    println!(
        "Mode 1 (Protected): GT coverage=100%, recall unchanged by deletions"
    );
    println!(
        "Mode 2 (Adjusted):  GT coverage={:.1}%, recall computed against surviving GT",
        gt_coverage2 * 100.0
    );
    println!("\nUse Protected mode when you need baseline recall measurements.");
    println!("Use Adjusted mode to understand how deletion affects search quality.");
}
