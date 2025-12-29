# Valkey-Search Vector Performance Report (SIMD Optimizations)

**Date:** December 29, 2025  
**Report ID:** 6d80ea0560a4853c2d36d213238ee96641b361ee

---

## Executive Summary

This report presents benchmark results for Valkey-Search with **SIMD enhancements and performance optimizations**. The tests evaluate vector loading performance and query throughput/latency characteristics on an AWS EC2 r7g.16xlarge instance using the Cohere-Large-10M dataset.

### Key Findings

| Metric | Result | Improvement vs Baseline |
|--------|--------|-------------------------|
| **Peak Query Throughput** | 9,158 req/s | +10.2% |
| **Lowest Query Latency** | 6.67ms avg | -6.2% (faster) |
| **Vector Load Throughput** | 11,510 req/s | +30.2% |
| **Recall@100** | ~93.6% | Consistent |

---

## Test Environment

### Hardware Configuration

| Component | Specification |
|-----------|---------------|
| **Instance Type** | AWS EC2 r7g.16xlarge |
| **CPU** | 64 vCPUs (ARM-based Graviton3) |
| **Memory** | 512 GB |
| **Operating System** | Ubuntu 22.04 LTS |

### Software Versions

| Component | Version |
|-----------|---------|
| **Valkey Server** | unstable (commit 122070cf) |
| **valkey-search** | main (commit 6d80ea05) [SIMD Optimized] |
| **valkey-bench-rs** | unstable (commit cb5ddfd5) |

### Dataset Specification

| Property | Value |
|----------|-------|
| **Dataset** | cohere-large-10m |
| **Vector Count** | 10,000,000 |
| **Dimensions** | 768 |
| **Distance Metric** | Cosine |
| **Index Algorithm** | HNSW |

---

## Vector Load Performance

Significant improvements in indexing speed were observed with the new optimizations.

### Results Summary

| Metric | Value |
|--------|-------|
| **Throughput** | 11,510 req/s |
| **Total Requests** | 10,000,000 |
| **Duration** | 868.74 seconds (~14.5 minutes) |
| **Average Latency** | 37.53 ms |
| **P50 Latency** | 31.82 ms |
| **P99 Latency** | 245.76 ms |

### Configuration

- **Clients:** 400
- **Threads:** 36

---

## Vector Query Performance

### Reader Thread Scaling Analysis

The following table summarizes query performance across different `search.reader-threads` configurations:

| Reader Threads | Throughput (req/s) | Avg Latency (ms) | P50 (ms) | P99 (ms) | Max (ms) | Recall |
|:--------------:|:------------------:|:----------------:|:--------:|:--------:|:--------:|:------:|
| **56** (high concurrency) | 9,158 | 86.23 | 82.37 | 166.14 | 493.57 | 0.9348 |
| **56** (low concurrency) | 8,360 | 6.67 | 7.15 | 10.22 | 14.83 | 0.9352 |
| **32** | 8,158 | 6.76 | 6.70 | 11.27 | 13.38 | 0.9361 |
| **24** | 6,789 | 8.15 | 8.17 | 11.54 | 15.02 | 0.9361 |
| **16** | 4,704 | 11.80 | 11.86 | 14.22 | 16.77 | 0.9360 |
| **12** | 3,643 | 15.26 | 15.32 | 17.52 | 19.79 | 0.9361 |
| **8** | 2,596 | 21.44 | 21.52 | 23.68 | 25.61 | 0.9360 |
| **4** | 1,349 | 41.31 | 41.47 | 44.19 | 45.95 | 0.9361 |
| **2** | 687 | 81.21 | 81.53 | 85.69 | 88.58 | 0.9361 |

### Query Parameters

- **Search Algorithm:** HNSW
- **ef_search:** 280
- **k (neighbors):** 100

---

## Performance Analysis

### Throughput Scaling

```
Reader Threads vs Throughput (req/s)
────────────────────────────────────────────────────────────
56 threads │████████████████████████████████████████████ 9,158
32 threads │████████████████████████████████████████     8,158
24 threads │█████████████████████████████████              6,789
16 threads │███████████████████████                        4,704
12 threads │██████████████████                             3,643
 8 threads │█████████████                                  2,596
 4 threads │███████                                        1,349
 2 threads │███                                              687
```

### Scaling Efficiency

| Comparison | Throughput Multiplier | Notes |
|------------|:---------------------:|-------|
| 2 → 4 threads | 1.96x | Near-linear scaling |
| 4 → 8 threads | 1.92x | Near-linear scaling |
| 8 → 16 threads | 1.81x | Strong scaling |
| 16 → 32 threads | 1.73x | Good scaling |
| 32 → 56 threads | 1.12x | Diminishing returns (saturation) |

### Latency vs Throughput Trade-off

The SIMD optimizations have shifted the performance curve:

1. **High Throughput Mode** (800 clients)
   - Throughput: **9,158 req/s** (vs 8,309 baseline)
   - Latency: ~86 ms average (improved from ~101 ms)

2. **Low Latency Mode** (56 clients)
   - Throughput: **8,360 req/s** (vs 7,768 baseline)
   - Latency: **6.67 ms** average (improved from 7.11 ms)

---

## Recall Analysis

| Configuration | Recall@100 | Perfect Matches | Zero Matches |
|---------------|:----------:|:---------------:|:------------:|
| All tests | 93.48-93.61% | ~26.5% of queries | 0 |

**Observations:**
- Recall has slightly improved or remained stable compared to baseline.
- The optimizations do not compromise accuracy.

---

## Conclusions

1. **Significant Load Performance Boost:** Vector loading is **30% faster**, reaching 11.5k vectors/sec. This suggests the SIMD optimizations heavily benefit the ingestion/indexing path.

2. **Improved Query Efficiency:** Query throughput has increased by **~10%** at peak, and latency has dropped by **~6%** at low concurrency.

3. **Better Scaling:** The system maintains near-linear scaling up to higher thread counts compared to the baseline, showing better CPU utilization efficiency.

4. **Production Readiness:** The build is stable and provides a substantial free performance upgrade over the previous version.

---

*Report generated from benchmark results: res-6d80ea0560a4853c2d36d213238ee96641b361ee.md*
