# Valkey-Search Vector Performance Report

**Date:** December 29, 2025  
**Report ID:** 83c133a1f3689a937dd27b6d08269e3a29c1c698

---

## Executive Summary

This report presents comprehensive benchmark results for Valkey-Search vector operations on an AWS EC2 r7g.16xlarge instance. The tests evaluate vector loading performance and query throughput/latency characteristics across varying reader thread configurations using the Cohere-Large-10M dataset.

### Key Findings

| Metric | Best Result |
|--------|-------------|
| **Peak Query Throughput** | 8,309 req/s (56 reader threads, high concurrency) |
| **Lowest Query Latency** | 7.11ms avg (56 reader threads, low concurrency) |
| **Vector Load Throughput** | 8,842 req/s |
| **Recall@100** | 93.4% (consistent across all configurations) |
| **Memory Footprint** | ~78 GB for 10M vectors |

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
| **valkey-search** | main (commit c812cc4c) |
| **valkey-bench-rs** | unstable (commit cb5ddfd5) |

### Dataset Specification

| Property | Value |
|----------|-------|
| **Dataset** | cohere-large-10m |
| **Vector Count** | 10,000,000 |
| **Dimensions** | 768 |
| **Distance Metric** | Cosine |
| **Record Size** | 3,072 bytes |
| **Index Algorithm** | HNSW |

---

## Vector Load Performance

### Results Summary

| Metric | Value |
|--------|-------|
| **Throughput** | 8,842 req/s |
| **Total Requests** | 10,000,000 |
| **Duration** | 1,130.94 seconds (~18.85 minutes) |
| **Average Latency** | 48.85 ms |
| **P50 Latency** | 43.04 ms |
| **P95 Latency** | 48.38 ms |
| **P99 Latency** | 260.74 ms |
| **Max Latency** | 6,397.95 ms |

### Configuration

- **Clients:** 400
- **Threads:** 36
- **Total Connections:** 432

### Resource Utilization

| Metric | Value |
|--------|-------|
| **Memory Used** | 78 GB |
| **Memory Growth Rate** | 69 MB/s |
| **HSET Commands** | 10M @ 8,842/s |

---

## Vector Query Performance

### Reader Thread Scaling Analysis

The following table summarizes query performance across different `search.reader-threads` configurations:

| Reader Threads | Throughput (req/s) | Avg Latency (ms) | P50 (ms) | P99 (ms) | Max (ms) | Recall |
|:--------------:|:------------------:|:----------------:|:--------:|:--------:|:--------:|:------:|
| **56** (max concurrency) | 8,309 | 101.06 | 101.18 | 104.51 | 238.21 | 0.9342 |
| **56** (low concurrency) | 7,768 | 7.11 | 7.06 | 11.15 | 14.53 | 0.9355 |
| **32** | 5,876 | 141.21 | 142.85 | 146.56 | 171.78 | 0.9346 |
| **24** | 4,570 | 181.67 | 183.68 | 187.39 | 204.41 | 0.9346 |
| **16** | 3,140 | 264.65 | 267.52 | 271.87 | 284.93 | 0.9346 |
| **12** | 2,382 | 348.83 | 352.51 | 357.89 | 368.64 | 0.9346 |
| **8** | 1,607 | 517.78 | 522.75 | 529.92 | 538.11 | 0.9346 |
| **4** | 810 | 1,027.69 | 1,037.31 | 1,051.65 | 1,058.82 | 0.9346 |
| **3** | 606 | 342.37 | 343.04 | 354.05 | 359.42 | 0.9337 |
| **2** | 404 | 513.57 | 514.56 | 530.94 | 538.11 | 0.9337 |
| **1** | 205 | 1,004.14 | 1,014.78 | 1,046.02 | 1,058.82 | 0.9340 |

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
56 threads │████████████████████████████████████████████ 8,309
32 threads │██████████████████████████████             5,876
24 threads │████████████████████████                   4,570
16 threads │█████████████████                          3,140
12 threads │█████████████                              2,382
 8 threads │█████████                                  1,607
 4 threads │████                                         810
 3 threads │███                                          606
 2 threads │██                                           404
 1 thread  │█                                            205
```

### Scaling Efficiency

| Comparison | Throughput Multiplier | Notes |
|------------|:---------------------:|-------|
| 1 → 2 threads | 1.97x | Near-linear scaling |
| 1 → 4 threads | 3.95x | Near-linear scaling |
| 1 → 8 threads | 7.84x | Near-linear scaling |
| 1 → 16 threads | 15.32x | Near-linear scaling |
| 1 → 32 threads | 28.66x | Sub-linear (89.6% efficiency) |
| 1 → 56 threads | 40.53x | Sub-linear (72.4% efficiency) |

### Latency vs Throughput Trade-off

The benchmark reveals two distinct operating modes:

1. **High Throughput Mode** (800 clients, 60 threads)
   - Throughput: 8,309 req/s
   - Latency: ~101 ms average
   - Best for: Batch processing, background workloads

2. **Low Latency Mode** (56 clients, 56 threads)
   - Throughput: 7,768 req/s
   - Latency: ~7.11 ms average
   - Best for: Real-time applications, user-facing queries

---

## Recall Analysis

| Configuration | Recall@100 | Perfect Matches | Zero Matches |
|---------------|:----------:|:---------------:|:------------:|
| All tests | 93.37-93.55% | ~25.3% of queries | 0 |

**Observations:**
- Recall remains highly consistent (~93.4%) regardless of thread count
- Approximately 25% of queries achieve perfect recall (100 correct neighbors)
- Zero queries returned no results, indicating robust index coverage
- Minimum recall observed: 41% (edge cases with difficult query vectors)

---

## Resource Efficiency

### Memory Usage

| Phase | Memory Consumption |
|-------|:------------------:|
| **Index + Vectors** | 78 GB |
| **Per Vector** | ~7.8 KB |
| **Query Overhead** | ~25-29 MB |

### CPU Utilization Patterns

- **Vector Load:** ~100% utilization across writer threads
- **Query (56 threads):** ~833M µsec/s aggregate CPU time
- **Query (1 thread):** ~206M µsec/s aggregate CPU time

---

## Recommendations

### For Maximum Throughput
- Use **56 reader threads** (matching vCPU count)
- Configure **800+ concurrent clients**
- Expected: **~8,300 req/s** at ~100ms latency

### For Minimum Latency
- Use **56 reader threads**
- Keep concurrent connections **≤ reader thread count**
- Expected: **~7ms average latency** at ~7,700 req/s

### Optimal Balance
- Use **32-56 reader threads**
- Tune client count based on SLA requirements
- Monitor P99 latency for tail latency optimization

---

## Conclusions

1. **Linear Scaling:** Valkey-Search demonstrates near-linear throughput scaling up to 16 reader threads, with diminishing returns beyond 32 threads.

2. **Consistent Quality:** Recall@100 remains stable at ~93.4% regardless of concurrency or thread configuration.

3. **Low Latency Capability:** With proper tuning (56 threads, matched concurrency), sub-10ms average latency is achievable at 7,700+ req/s.

4. **Production Ready:** The system successfully indexed and queried 10 million 768-dimensional vectors with predictable performance characteristics.

---

## Appendix: Test Commands

### Vector Load
```bash
./target/release/valkey-bench-rs \
  -h $HOST -p $PORT --cluster \
  --schema datasets/cohere-large-10m.yaml \
  --data datasets/cohere-large-10m.bin \
  -t vec-load \
  --search-index cohere-large-10m \
  --search-prefix "vec:" \
  -c 400 --threads 36
```

### Vector Query (High Throughput)
```bash
./target/release/valkey-bench-rs \
  -h $HOST -p $PORT --cluster \
  --schema datasets/cohere-large-10m.yaml \
  --data datasets/cohere-large-10m.bin \
  -t vec-query \
  --search-index cohere-large-10m \
  --ef-search 280 -k 100 \
  -n 5000000 -c 800 --threads 60
```

### Vector Query (Low Latency)
```bash
./target/release/valkey-bench-rs \
  -h $HOST -p $PORT --cluster \
  --schema datasets/cohere-large-10m.yaml \
  --data datasets/cohere-large-10m.bin \
  -t vec-query \
  --search-index cohere-large-10m \
  --ef-search 280 -k 100 \
  -n 10000 -c 56 --threads 56
```

---

*Report generated from benchmark results: res-83c133a1f3689a937dd27b6d08269e3a29c1c698.md*
