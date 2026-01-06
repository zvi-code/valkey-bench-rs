//! Test: GT-Aware Recall with Cohere Dataset
//!
//! This test demonstrates both Protected and Adjusted recall modes
//! using the real cohere-medium-1m dataset.

use std::sync::Arc;

use keyspace_tracker::{PrefixTracker, TrackerConfig};

use valkey_bench_rs::dataset::DatasetContext;
use valkey_bench_rs::keyspace::GroundTruthMode;

fn create_tracker(num_vectors: u64) -> PrefixTracker {
    PrefixTracker::new(
        TrackerConfig::simple("vec:")
            .with_max_id(num_vectors)
            .with_initial_capacity(num_vectors as usize),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== GT-Aware Recall Test with Cohere-Medium-1M ===\n");

    // Load the Cohere dataset
    let schema_path = "datasets/cohere-medium-1m.yaml";
    let data_path = "datasets/cohere-medium-1m.bin";

    println!("Loading dataset...");
    let dataset = DatasetContext::open(schema_path, data_path)?;
    println!("{}\n", dataset.summary());

    let num_vectors = dataset.num_vectors();
    let num_queries = dataset.num_queries();
    let k = 10; // Use k=10 for recall computation

    // Build GT-aware recall calculators for both modes
    println!("Building ground truth reference sets...");
    let gt_recall_protected = Arc::new(dataset.build_gt_aware_recall(GroundTruthMode::Protected, k));
    let gt_recall_adjusted = Arc::new(dataset.build_gt_aware_recall(GroundTruthMode::Adjusted, k));

    println!(
        "  Total unique GT vectors (k={}): {}",
        k,
        gt_recall_protected.gt_count()
    );

    // Create and populate tracker (simulates loaded vectors)
    println!("\nSimulating vector load (marking all {} vectors as existing)...", num_vectors);
    let tracker = create_tracker(num_vectors);
    for id in 0..num_vectors {
        tracker.add(id);
    }
    println!("  Loaded: {} vectors", tracker.count());

    // Check initial GT coverage
    let (existing, total, coverage) = gt_recall_protected.compute_coverage(&tracker);
    println!("  Initial GT coverage: {}/{} ({:.2}%)\n", existing, total, coverage * 100.0);

    // =========================================================================
    // Test different deletion ratios
    // =========================================================================
    let deletion_ratios = [0.0, 0.1, 0.25, 0.5, 0.75];

    println!("=== Testing Different Deletion Ratios ===\n");
    println!("{:>8} {:>12} {:>12} {:>12} {:>12}",
             "Del%", "Remaining", "GT Existing", "GT Coverage", "Mode");
    println!("{}", "-".repeat(60));

    for &del_ratio in &deletion_ratios {
        // Create fresh tracker
        let tracker = create_tracker(num_vectors);
        for id in 0..num_vectors {
            tracker.add(id);
        }

        // Delete vectors (using deterministic selection based on ID)
        for id in 0..num_vectors {
            // Deterministic "random" selection using hash
            let hash = (id.wrapping_mul(2654435761) >> 16) % 1000;
            let should_delete = hash < (del_ratio * 1000.0) as u64;

            if should_delete {
                tracker.remove(id);
            }
        }

        // Compute coverage after deletion
        let (gt_existing, _gt_total, gt_coverage) = gt_recall_adjusted.compute_coverage(&tracker);

        println!(
            "{:>7.0}% {:>12} {:>12} {:>11.1}% Adjusted",
            del_ratio * 100.0,
            tracker.count(),
            gt_existing,
            gt_coverage * 100.0
        );

        // Sample query recall computation
        if del_ratio > 0.0 {
            let sample_queries = [0u64, 100, 500, 999];
            let mut sum_recall = 0.0;
            let mut sum_coverage = 0.0;

            for &q in &sample_queries {
                // Simulate perfect search results (would return surviving GT)
                let gt_ids = gt_recall_adjusted.get_query_gt(q);
                let snapshot = tracker.snapshot();

                // Surviving GT for this query
                let surviving: Vec<u64> = gt_ids
                    .iter()
                    .filter(|&&id| snapshot.test(id as usize))
                    .copied()
                    .collect();

                // Simulate search returning the surviving GT (best case)
                let (recall, stats) = gt_recall_adjusted.compute_recall_with_stats(
                    q,
                    &surviving,
                    k,
                    &tracker,
                );
                sum_recall += recall;
                sum_coverage += stats.gt_coverage;
            }

            let avg_recall = sum_recall / sample_queries.len() as f64;
            let avg_coverage = sum_coverage / sample_queries.len() as f64;
            println!(
                "         Sample queries: avg_recall={:.3}, avg_gt_coverage={:.1}%",
                avg_recall,
                avg_coverage * 100.0
            );
        }
    }

    // =========================================================================
    // Detailed analysis at 50% deletion
    // =========================================================================
    println!("\n=== Detailed Analysis at 50% Deletion ===\n");

    // Create tracker and delete 50%
    let tracker = create_tracker(num_vectors);
    for id in 0..num_vectors {
        tracker.add(id);
    }

    let del_ratio = 0.5;
    for id in 0..num_vectors {
        let hash = (id.wrapping_mul(2654435761) >> 16) % 1000;
        if hash < (del_ratio * 1000.0) as u64 {
            tracker.remove(id);
        }
    }

    println!("Vectors remaining: {}/{}", tracker.count(), num_vectors);

    // Analyze per-query GT coverage distribution
    let mut coverage_buckets = [0u64; 11]; // 0-10%, 10-20%, ..., 90-100%

    for q in 0..num_queries {
        let (_, stats) = gt_recall_adjusted.compute_recall_with_stats(
            q,
            &[], // Empty results - just want stats
            k,
            &tracker,
        );
        let bucket = (stats.gt_coverage * 10.0).min(10.0) as usize;
        coverage_buckets[bucket] += 1;
    }

    println!("\nPer-query GT coverage distribution:");
    for (i, count) in coverage_buckets.iter().enumerate() {
        let pct_low = i * 10;
        let pct_high = (i + 1) * 10;
        let bar_len = (*count as usize * 50) / num_queries as usize;
        println!(
            "  {:>3}-{:<3}%: {:>4} queries {}",
            pct_low,
            pct_high,
            count,
            "#".repeat(bar_len)
        );
    }

    // Show specific query examples
    println!("\nSample query analysis:");
    for q in [0u64, 42, 500, 999] {
        let gt_ids = gt_recall_adjusted.get_query_gt(q);
        let snapshot = tracker.snapshot();

        let surviving: Vec<u64> = gt_ids
            .iter()
            .filter(|&&id| snapshot.test(id as usize))
            .copied()
            .collect();

        let (recall, stats) = gt_recall_adjusted.compute_recall_with_stats(
            q,
            &surviving, // Best case: search returns all surviving GT
            k,
            &tracker,
        );

        println!(
            "  Query {:>4}: GT surviving {}/{}, coverage={:.0}%, best_recall={:.3}",
            q,
            stats.gt_existing,
            stats.gt_total,
            stats.gt_coverage * 100.0,
            recall
        );
    }

    // =========================================================================
    // Compare Protected vs Adjusted at 30% deletion
    // =========================================================================
    println!("\n=== Protected vs Adjusted Mode Comparison (30% deletion) ===\n");

    let del_ratio = 0.3;
    let mut protected_deletions = 0u64;
    let mut adjusted_deletions = 0u64;
    let mut skipped_gt = 0u64;

    for id in 0..num_vectors {
        let hash = (id.wrapping_mul(2654435761) >> 16) % 1000;
        let should_delete = hash < (del_ratio * 1000.0) as u64;

        if should_delete {
            // Protected mode would skip GT
            if gt_recall_protected.is_ground_truth(id) {
                skipped_gt += 1;
            } else {
                protected_deletions += 1;
            }
            // Adjusted mode deletes everything
            adjusted_deletions += 1;
        }
    }

    println!("Adjusted mode: would delete {} vectors", adjusted_deletions);
    println!(
        "Protected mode: would delete {} vectors ({} GT skipped)",
        protected_deletions, skipped_gt
    );

    // Simulate Protected mode (GT not deleted)
    let tracker_protected = create_tracker(num_vectors);
    for id in 0..num_vectors {
        tracker_protected.add(id);
    }
    for id in 0..num_vectors {
        let hash = (id.wrapping_mul(2654435761) >> 16) % 1000;
        let should_delete = hash < (del_ratio * 1000.0) as u64;
        if should_delete && !gt_recall_protected.is_ground_truth(id) {
            tracker_protected.remove(id);
        }
    }

    let (_, _, cov_prot) = gt_recall_protected.compute_coverage(&tracker_protected);
    println!(
        "\nProtected mode result: {} vectors, GT coverage={:.1}%",
        tracker_protected.count(),
        cov_prot * 100.0
    );

    // Simulate Adjusted mode (GT can be deleted)
    let tracker_adjusted = create_tracker(num_vectors);
    for id in 0..num_vectors {
        tracker_adjusted.add(id);
    }
    for id in 0..num_vectors {
        let hash = (id.wrapping_mul(2654435761) >> 16) % 1000;
        if hash < (del_ratio * 1000.0) as u64 {
            tracker_adjusted.remove(id);
        }
    }

    let (_, _, cov_adj) = gt_recall_adjusted.compute_coverage(&tracker_adjusted);
    println!(
        "Adjusted mode result:  {} vectors, GT coverage={:.1}%",
        tracker_adjusted.count(),
        cov_adj * 100.0
    );

    println!("\n=== Test Complete ===");

    Ok(())
}
