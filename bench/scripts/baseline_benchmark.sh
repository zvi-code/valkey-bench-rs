#!/bin/bash
#
# SPDX-License-Identifier: BSD-3-Clause
#
# Copyright (c) 2024-present, Zvi Schneider
#
# Baseline Performance Benchmark Script for Keyspace Tracker Migration
#
# This script establishes performance baselines for:
# - vec-load throughput (ops/sec)
# - vec-query latency (p50, p99)
# - vec-delete throughput
# - Memory usage under load
#
# Usage:
#   HOST=<valkey-host> ./baseline_benchmark.sh [options]
#
# Options:
#   --dataset <name>     Dataset to use (default: cohere-small-100k)
#   --output <dir>       Output directory for results (default: ./results/baseline_results)
#   --quick              Run quick benchmark (fewer requests)
#   --full               Run full benchmark (more requests, longer duration)
#   --skip-load          Skip vec-load phase (assume data already loaded)
#   --skip-delete        Skip vec-delete phase
#   --help               Show this help message
#

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================

# Environment variables
BENCHMARK_HOME="${BENCHMARK_HOME:-$(cd "$(dirname "$0")/../.." && pwd)}"
HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-6379}"

# Paths
BINARY_DIR="${BENCHMARK_HOME}/datasets"
BENCHN="${BENCHMARK_HOME}/target/release/valkey-bench-rs"
CLI="$BENCHN --cli"

# Default parameters
DATASET="cohere-small-100k"
OUTPUT_DIR="${BENCHMARK_HOME}/results/baseline_results"
QUICK_MODE=false
FULL_MODE=false
SKIP_LOAD=false
SKIP_DELETE=false
CLEANUP_INDEX="ask"  # ask, yes, no

# Benchmark parameters (will be set based on mode)
LOAD_CLIENTS=100
LOAD_THREADS=16
QUERY_CLIENTS=50
QUERY_THREADS=10
QUERY_REQUESTS=10000
DELETE_CLIENTS=50
DELETE_THREADS=10
DELETE_REQUESTS=1000

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# ============================================================================
# Helper Functions
# ============================================================================

print_header() {
    echo -e "\n${BLUE}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}\n"
}

print_section() {
    echo -e "\n${CYAN}── $1 ──${NC}\n"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

print_info() {
    echo -e "${CYAN}ℹ $1${NC}"
}

show_help() {
    cat << EOF
Baseline Performance Benchmark Script for Keyspace Tracker Migration

Usage:
  HOST=<valkey-host> $0 [options]

Options:
  --dataset <name>     Dataset to use (default: cohere-small-100k)
  --output <dir>       Output directory for results (default: ./results/baseline_results)
  --quick              Run quick benchmark (fewer requests, faster)
  --full               Run full benchmark (more requests, comprehensive)
  --skip-load          Skip vec-load phase (assume data already loaded)
  --skip-delete        Skip vec-delete phase
  --help               Show this help message

Environment Variables:
  HOST                 Valkey server hostname (default: 127.0.0.1)
  PORT                 Valkey server port (default: 6379)
  BENCHMARK_HOME       Path to valkey-bench-rs directory

Available Datasets:
  cohere-small-100k    100K vectors, 768 dimensions (recommended for quick tests)
  cohere-medium-1m     1M vectors, 768 dimensions
  cohere-large-10m     10M vectors, 768 dimensions (requires significant resources)
  sift-128             1M vectors, 128 dimensions
  gist-960             1M vectors, 960 dimensions

Examples:
  # Quick baseline with default dataset
  HOST=localhost ./baseline_benchmark.sh --quick

  # Full baseline with specific dataset
  HOST=myhost.example.com ./baseline_benchmark.sh --full --dataset cohere-medium-1m

  # Skip load phase (data already in database)
  HOST=localhost ./baseline_benchmark.sh --skip-load --dataset cohere-small-100k

EOF
    exit 0
}

# ============================================================================
# Parse Arguments
# ============================================================================

while [[ $# -gt 0 ]]; do
    case $1 in
        --dataset)
            DATASET="$2"
            shift 2
            ;;
        --output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --quick)
            QUICK_MODE=true
            shift
            ;;
        --full)
            FULL_MODE=true
            shift
            ;;
        --skip-load)
            SKIP_LOAD=true
            shift
            ;;
        --skip-delete)
            SKIP_DELETE=true
            shift
            ;;
        --cluster)
            CLUSTER_FLAG="--cluster"
            shift
            ;;
        --cleanup|-y)
            CLEANUP_INDEX="yes"
            shift
            ;;
        --no-cleanup)
            CLEANUP_INDEX="no"
            shift
            ;;
        --help|-h)
            show_help
            ;;
        *)
            print_error "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# ============================================================================
# Set Benchmark Parameters Based on Mode
# ============================================================================

if [ "$QUICK_MODE" = true ]; then
    LOAD_CLIENTS=50
    LOAD_THREADS=8
    QUERY_CLIENTS=20
    QUERY_THREADS=4
    QUERY_REQUESTS=1000
    DELETE_CLIENTS=20
    DELETE_THREADS=4
    DELETE_REQUESTS=500
    print_info "Running in QUICK mode (reduced requests for faster results)"
elif [ "$FULL_MODE" = true ]; then
    LOAD_CLIENTS=200
    LOAD_THREADS=32
    QUERY_CLIENTS=100
    QUERY_THREADS=20
    QUERY_REQUESTS=50000
    DELETE_CLIENTS=100
    DELETE_THREADS=20
    DELETE_REQUESTS=10000
    print_info "Running in FULL mode (comprehensive benchmark)"
fi

# ============================================================================
# Validation
# ============================================================================

print_header "Baseline Performance Benchmark"

# Check if benchmark binary exists
if [ ! -x "$BENCHN" ]; then
    print_error "valkey-bench-rs not found or not executable: $BENCHN"
    echo "Run: cargo build --release"
    exit 1
fi

# Check dataset files
SCHEMA_FILE="${BINARY_DIR}/${DATASET}.yaml"
DATA_FILE="${BINARY_DIR}/${DATASET}.bin"
LEGACY_FILE="${BINARY_DIR}/${DATASET}.bin"

# Determine dataset format
USE_SCHEMA=false
if [ -f "$SCHEMA_FILE" ] && [ -f "$DATA_FILE" ]; then
    USE_SCHEMA=true
    print_info "Using schema-driven dataset format"
elif [ -f "$LEGACY_FILE" ]; then
    print_info "Using legacy dataset format"
else
    print_error "Dataset not found: $DATASET"
    echo "Expected files:"
    echo "  Schema format: $SCHEMA_FILE and $DATA_FILE"
    echo "  Legacy format: $LEGACY_FILE"
    echo ""
    echo "Download dataset with: ./prep_datasets/dataset.sh get $DATASET"
    exit 1
fi

# Test connection
print_section "Testing Connection"
echo -n "Connecting to $HOST:$PORT... "
if timeout 5 $CLI -h "$HOST" -p "$PORT" PING > /dev/null 2>&1; then
    print_success "Connected"
else
    print_error "Connection failed"
    exit 1
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_FILE="${OUTPUT_DIR}/baseline_${DATASET}_${TIMESTAMP}.md"
JSON_FILE="${OUTPUT_DIR}/baseline_${DATASET}_${TIMESTAMP}.json"

# ============================================================================
# Display Configuration
# ============================================================================

print_section "Configuration"
echo "Host:              $HOST:$PORT"
echo "Dataset:           $DATASET"
echo "Output Directory:  $OUTPUT_DIR"
echo "Result File:       $RESULT_FILE"
echo ""
echo "Benchmark Parameters:"
echo "  Load:   clients=$LOAD_CLIENTS, threads=$LOAD_THREADS"
echo "  Query:  clients=$QUERY_CLIENTS, threads=$QUERY_THREADS, requests=$QUERY_REQUESTS"
echo "  Delete: clients=$DELETE_CLIENTS, threads=$DELETE_THREADS, requests=$DELETE_REQUESTS"

# ============================================================================
# Initialize Results
# ============================================================================

# Initialize JSON results
cat > "$JSON_FILE" << EOF
{
  "metadata": {
    "timestamp": "$(date -Iseconds)",
    "host": "$HOST",
    "port": $PORT,
    "dataset": "$DATASET",
    "mode": "$([ "$QUICK_MODE" = true ] && echo "quick" || ([ "$FULL_MODE" = true ] && echo "full" || echo "standard"))"
  },
  "results": {}
}
EOF

# Initialize Markdown report
cat > "$RESULT_FILE" << EOF
# Baseline Performance Report

**Date:** $(date "+%B %d, %Y %H:%M:%S")  
**Dataset:** $DATASET  
**Host:** $HOST:$PORT  
**Mode:** $([ "$QUICK_MODE" = true ] && echo "Quick" || ([ "$FULL_MODE" = true ] && echo "Full" || echo "Standard"))

---

## Test Configuration

| Parameter | Value |
|-----------|-------|
| Load Clients | $LOAD_CLIENTS |
| Load Threads | $LOAD_THREADS |
| Query Clients | $QUERY_CLIENTS |
| Query Threads | $QUERY_THREADS |
| Query Requests | $QUERY_REQUESTS |
| Delete Clients | $DELETE_CLIENTS |
| Delete Threads | $DELETE_THREADS |
| Delete Requests | $DELETE_REQUESTS |

---

EOF

# ============================================================================
# Get Initial Memory Usage
# ============================================================================

print_section "Initial Memory Usage"

get_memory_usage() {
    local info=$($CLI -h "$HOST" -p "$PORT" INFO memory 2>/dev/null || echo "")
    local used_memory=$(echo "$info" | grep "^used_memory:" | cut -d: -f2 | tr -d '\r')
    local used_memory_human=$(echo "$info" | grep "^used_memory_human:" | cut -d: -f2 | tr -d '\r')
    local used_memory_peak=$(echo "$info" | grep "^used_memory_peak:" | cut -d: -f2 | tr -d '\r')
    local used_memory_peak_human=$(echo "$info" | grep "^used_memory_peak_human:" | cut -d: -f2 | tr -d '\r')
    
    echo "used_memory=$used_memory"
    echo "used_memory_human=$used_memory_human"
    echo "used_memory_peak=$used_memory_peak"
    echo "used_memory_peak_human=$used_memory_peak_human"
}

eval $(get_memory_usage)
INITIAL_MEMORY=$used_memory
INITIAL_MEMORY_HUMAN=$used_memory_human

echo "Initial Memory: $INITIAL_MEMORY_HUMAN ($INITIAL_MEMORY bytes)"

# ============================================================================
# Phase 1: vec-load Benchmark
# ============================================================================

INDEX_NAME="baseline_${DATASET}"
KEY_PREFIX="bvec:"

if [ "$SKIP_LOAD" = false ]; then
    print_header "Phase 1: vec-load Benchmark"
    
    # Drop existing index if present
    print_info "Cleaning up existing index..."
    $CLI -h "$HOST" -p "$PORT" FT.DROPINDEX "$INDEX_NAME" 2>/dev/null || true
    
    # Build command based on dataset format
    LOAD_CMD="$BENCHN -h $HOST -p $PORT"
    if [ "$USE_SCHEMA" = true ]; then
        LOAD_CMD="$LOAD_CMD --schema $SCHEMA_FILE --data $DATA_FILE"
    else
        LOAD_CMD="$LOAD_CMD --dataset $LEGACY_FILE"
    fi
    LOAD_CMD="$LOAD_CMD -t vec-load --search-index $INDEX_NAME --search-prefix $KEY_PREFIX"
    LOAD_CMD="$LOAD_CMD -c $LOAD_CLIENTS --threads $LOAD_THREADS"
    LOAD_CMD="$LOAD_CMD -o ${OUTPUT_DIR}/load_${TIMESTAMP}.json --output-format json"
    
    print_info "Running vec-load benchmark..."
    echo "Command: $LOAD_CMD"
    echo ""
    
    LOAD_OUTPUT=$(mktemp)
    if $LOAD_CMD 2>&1 | tee "$LOAD_OUTPUT"; then
        print_success "vec-load completed"
    else
        print_error "vec-load failed"
        cat "$LOAD_OUTPUT"
        rm -f "$LOAD_OUTPUT"
        exit 1
    fi
    
    # Extract metrics from JSON output (more reliable than text parsing)
    JSON_OUTPUT="${OUTPUT_DIR}/load_${TIMESTAMP}.json"
    if [ -f "$JSON_OUTPUT" ]; then
        LOAD_THROUGHPUT=$(python3 -c "import json; d=json.load(open('$JSON_OUTPUT')); print(int(d['tests'][0]['summary']['throughput']))" 2>/dev/null || echo "N/A")
        LOAD_AVG_LATENCY=$(python3 -c "import json; d=json.load(open('$JSON_OUTPUT')); print(f\"{d['tests'][0]['summary']['latency']['mean_ms']:.2f}\")" 2>/dev/null || echo "N/A")
        LOAD_P50=$(python3 -c "import json; d=json.load(open('$JSON_OUTPUT')); print(f\"{d['tests'][0]['summary']['latency']['p50_ms']:.2f}\")" 2>/dev/null || echo "N/A")
        LOAD_P99=$(python3 -c "import json; d=json.load(open('$JSON_OUTPUT')); print(f\"{d['tests'][0]['summary']['latency']['p99_ms']:.2f}\")" 2>/dev/null || echo "N/A")
        LOAD_DURATION=$(python3 -c "import json; d=json.load(open('$JSON_OUTPUT')); print(f\"{d['tests'][0]['summary']['duration_secs']:.2f}\")" 2>/dev/null || echo "N/A")
        LOAD_REQUESTS=$(python3 -c "import json; d=json.load(open('$JSON_OUTPUT')); print(d['tests'][0]['summary']['total_ops'])" 2>/dev/null || echo "N/A")
    else
        # Fallback to text parsing
        LOAD_THROUGHPUT=$(grep -oP 'Throughput: \K[0-9,]+' "$LOAD_OUTPUT" | tr -d ',' | head -1 || echo "N/A")
        LOAD_AVG_LATENCY=$(grep -oP 'avg=\K[0-9.]+' "$LOAD_OUTPUT" | head -1 || echo "N/A")
        LOAD_P50=$(grep -oP 'p50=\K[0-9.]+' "$LOAD_OUTPUT" | head -1 || echo "N/A")
        LOAD_P99=$(grep -oP 'p99=\K[0-9.]+' "$LOAD_OUTPUT" | head -1 || echo "N/A")
        LOAD_DURATION=$(grep -oP 'Duration: \K[0-9.]+' "$LOAD_OUTPUT" | head -1 || echo "N/A")
        LOAD_REQUESTS=$(grep -oP 'Requests: \K[0-9,]+' "$LOAD_OUTPUT" | tr -d ',' | head -1 || echo "N/A")
    fi
    
    rm -f "$LOAD_OUTPUT"
    
    # Get memory after load
    sleep 2  # Allow memory stats to stabilize
    eval $(get_memory_usage)
    POST_LOAD_MEMORY=$used_memory
    POST_LOAD_MEMORY_HUMAN=$used_memory_human
    MEMORY_DELTA=$((POST_LOAD_MEMORY - INITIAL_MEMORY))
    MEMORY_DELTA_MB=$((MEMORY_DELTA / 1024 / 1024))
    
    print_section "vec-load Results"
    echo "Throughput:     $LOAD_THROUGHPUT req/s"
    echo "Avg Latency:    ${LOAD_AVG_LATENCY}ms"
    echo "P50 Latency:    ${LOAD_P50}ms"
    echo "P99 Latency:    ${LOAD_P99}ms"
    echo "Duration:       ${LOAD_DURATION}s"
    echo "Requests:       $LOAD_REQUESTS"
    echo "Memory Used:    $POST_LOAD_MEMORY_HUMAN (delta: ${MEMORY_DELTA_MB}MB)"
    
    # Append to report
    cat >> "$RESULT_FILE" << EOF

## vec-load Results

| Metric | Value |
|--------|-------|
| **Throughput** | $LOAD_THROUGHPUT req/s |
| **Avg Latency** | ${LOAD_AVG_LATENCY}ms |
| **P50 Latency** | ${LOAD_P50}ms |
| **P99 Latency** | ${LOAD_P99}ms |
| **Duration** | ${LOAD_DURATION}s |
| **Total Requests** | $LOAD_REQUESTS |
| **Memory After Load** | $POST_LOAD_MEMORY_HUMAN |
| **Memory Delta** | ${MEMORY_DELTA_MB}MB |

EOF

else
    print_info "Skipping vec-load phase (--skip-load specified)"
    LOAD_THROUGHPUT="N/A (skipped)"
    LOAD_AVG_LATENCY="N/A"
    LOAD_P50="N/A"
    LOAD_P99="N/A"
    
    # Get current memory
    eval $(get_memory_usage)
    POST_LOAD_MEMORY=$used_memory
    POST_LOAD_MEMORY_HUMAN=$used_memory_human
fi

# ============================================================================
# Phase 2: vec-query Benchmark
# ============================================================================

print_header "Phase 2: vec-query Benchmark"

# Build command based on dataset format
QUERY_CMD="$BENCHN -h $HOST -p $PORT"
if [ "$USE_SCHEMA" = true ]; then
    QUERY_CMD="$QUERY_CMD --schema $SCHEMA_FILE --data $DATA_FILE"
else
    QUERY_CMD="$QUERY_CMD --dataset $LEGACY_FILE"
fi
QUERY_CMD="$QUERY_CMD -t vec-query --search-index $INDEX_NAME --search-prefix $KEY_PREFIX"
QUERY_CMD="$QUERY_CMD -c $QUERY_CLIENTS --threads $QUERY_THREADS -n $QUERY_REQUESTS"
QUERY_CMD="$QUERY_CMD --ef-search 100 -k 10"
QUERY_CMD="$QUERY_CMD -o ${OUTPUT_DIR}/query_${TIMESTAMP}.json --output-format json"

print_info "Running vec-query benchmark..."
echo "Command: $QUERY_CMD"
echo ""

QUERY_OUTPUT=$(mktemp)
if $QUERY_CMD 2>&1 | tee "$QUERY_OUTPUT"; then
    print_success "vec-query completed"
else
    print_warning "vec-query may have encountered issues"
fi

# Extract metrics from JSON output (more reliable than text parsing)
QUERY_JSON="${OUTPUT_DIR}/query_${TIMESTAMP}.json"
if [ -f "$QUERY_JSON" ]; then
    QUERY_THROUGHPUT=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); print(int(d['tests'][0]['summary']['throughput']))" 2>/dev/null || echo "N/A")
    QUERY_AVG_LATENCY=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); print(f\"{d['tests'][0]['summary']['latency']['mean_ms']:.2f}\")" 2>/dev/null || echo "N/A")
    QUERY_P50=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); print(f\"{d['tests'][0]['summary']['latency']['p50_ms']:.2f}\")" 2>/dev/null || echo "N/A")
    QUERY_P95=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); print(f\"{d['tests'][0]['summary']['latency']['p95_ms']:.2f}\")" 2>/dev/null || echo "N/A")
    QUERY_P99=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); print(f\"{d['tests'][0]['summary']['latency']['p99_ms']:.2f}\")" 2>/dev/null || echo "N/A")
    QUERY_P999=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); print(f\"{d['tests'][0]['summary']['latency']['p999_ms']:.2f}\")" 2>/dev/null || echo "N/A")
    QUERY_MAX=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); print(f\"{d['tests'][0]['summary']['latency']['max_ms']:.2f}\")" 2>/dev/null || echo "N/A")
    QUERY_RECALL=$(python3 -c "import json; d=json.load(open('$QUERY_JSON')); r=d['tests'][0]['summary'].get('recall',{}).get('mean',0); print(f'{r*100:.2f}' if r else 'N/A')" 2>/dev/null || echo "N/A")
else
    # Fallback to text parsing
    QUERY_THROUGHPUT=$(grep -oP 'Throughput: \K[0-9,]+' "$QUERY_OUTPUT" | tr -d ',' | head -1 || echo "N/A")
    QUERY_AVG_LATENCY=$(grep -oP 'avg=\K[0-9.]+' "$QUERY_OUTPUT" | head -1 || echo "N/A")
    QUERY_P50=$(grep -oP 'p50=\K[0-9.]+' "$QUERY_OUTPUT" | head -1 || echo "N/A")
    QUERY_P95=$(grep -oP 'p95=\K[0-9.]+' "$QUERY_OUTPUT" | head -1 || echo "N/A")
    QUERY_P99=$(grep -oP 'p99=\K[0-9.]+' "$QUERY_OUTPUT" | head -1 || echo "N/A")
    QUERY_P999=$(grep -oP 'p99\.9=\K[0-9.]+' "$QUERY_OUTPUT" | head -1 || echo "N/A")
    QUERY_MAX=$(grep -oP 'max=\K[0-9.]+' "$QUERY_OUTPUT" | head -1 || echo "N/A")
    QUERY_RECALL=$(grep -oP 'recall=\K[0-9.]+' "$QUERY_OUTPUT" | head -1 || echo "N/A")
fi

rm -f "$QUERY_OUTPUT"

# Get memory during query
eval $(get_memory_usage)
QUERY_MEMORY=$used_memory
QUERY_MEMORY_HUMAN=$used_memory_human
QUERY_PEAK_MEMORY=$used_memory_peak
QUERY_PEAK_MEMORY_HUMAN=$used_memory_peak_human

print_section "vec-query Results"
echo "Throughput:     $QUERY_THROUGHPUT req/s"
echo "Avg Latency:    ${QUERY_AVG_LATENCY}ms"
echo "P50 Latency:    ${QUERY_P50}ms"
echo "P95 Latency:    ${QUERY_P95}ms"
echo "P99 Latency:    ${QUERY_P99}ms"
echo "P99.9 Latency:  ${QUERY_P999}ms"
echo "Max Latency:    ${QUERY_MAX}ms"
echo "Recall:         ${QUERY_RECALL}%"
echo "Memory:         $QUERY_MEMORY_HUMAN (peak: $QUERY_PEAK_MEMORY_HUMAN)"

# Append to report
cat >> "$RESULT_FILE" << EOF

## vec-query Results

| Metric | Value |
|--------|-------|
| **Throughput** | $QUERY_THROUGHPUT req/s |
| **Avg Latency** | ${QUERY_AVG_LATENCY}ms |
| **P50 Latency** | ${QUERY_P50}ms |
| **P95 Latency** | ${QUERY_P95}ms |
| **P99 Latency** | ${QUERY_P99}ms |
| **P99.9 Latency** | ${QUERY_P999}ms |
| **Max Latency** | ${QUERY_MAX}ms |
| **Recall** | ${QUERY_RECALL}% |
| **Memory During Query** | $QUERY_MEMORY_HUMAN |
| **Peak Memory** | $QUERY_PEAK_MEMORY_HUMAN |

EOF

# ============================================================================
# Phase 3: vec-delete Benchmark
# ============================================================================

if [ "$SKIP_DELETE" = false ]; then
    print_header "Phase 3: vec-delete Benchmark"
    
    # Build command based on dataset format
    DELETE_CMD="$BENCHN -h $HOST -p $PORT"
    if [ "$USE_SCHEMA" = true ]; then
        DELETE_CMD="$DELETE_CMD --schema $SCHEMA_FILE --data $DATA_FILE"
    else
        DELETE_CMD="$DELETE_CMD --dataset $LEGACY_FILE"
    fi
    DELETE_CMD="$DELETE_CMD -t vec-delete --search-index $INDEX_NAME --search-prefix $KEY_PREFIX"
    DELETE_CMD="$DELETE_CMD -c $DELETE_CLIENTS --threads $DELETE_THREADS -n $DELETE_REQUESTS"
    DELETE_CMD="$DELETE_CMD -o ${OUTPUT_DIR}/delete_${TIMESTAMP}.json --output-format json"
    
    print_info "Running vec-delete benchmark..."
    echo "Command: $DELETE_CMD"
    echo ""
    
    DELETE_OUTPUT=$(mktemp)
    if $DELETE_CMD 2>&1 | tee "$DELETE_OUTPUT"; then
        print_success "vec-delete completed"
    else
        print_warning "vec-delete may have encountered issues"
    fi
    
    # Extract metrics from JSON output (more reliable than text parsing)
    DELETE_JSON="${OUTPUT_DIR}/delete_${TIMESTAMP}.json"
    if [ -f "$DELETE_JSON" ]; then
        DELETE_THROUGHPUT=$(python3 -c "import json; d=json.load(open('$DELETE_JSON')); print(int(d['tests'][0]['summary']['throughput']))" 2>/dev/null || echo "N/A")
        DELETE_AVG_LATENCY=$(python3 -c "import json; d=json.load(open('$DELETE_JSON')); print(f\"{d['tests'][0]['summary']['latency']['mean_ms']:.2f}\")" 2>/dev/null || echo "N/A")
        DELETE_P50=$(python3 -c "import json; d=json.load(open('$DELETE_JSON')); print(f\"{d['tests'][0]['summary']['latency']['p50_ms']:.2f}\")" 2>/dev/null || echo "N/A")
        DELETE_P99=$(python3 -c "import json; d=json.load(open('$DELETE_JSON')); print(f\"{d['tests'][0]['summary']['latency']['p99_ms']:.2f}\")" 2>/dev/null || echo "N/A")
        DELETE_REQUESTS_ACTUAL=$(python3 -c "import json; d=json.load(open('$DELETE_JSON')); print(d['tests'][0]['summary']['total_ops'])" 2>/dev/null || echo "N/A")
    else
        # Fallback to text parsing
        DELETE_THROUGHPUT=$(grep -oP 'Throughput: \K[0-9,]+' "$DELETE_OUTPUT" | tr -d ',' | head -1 || echo "N/A")
        DELETE_AVG_LATENCY=$(grep -oP 'avg=\K[0-9.]+' "$DELETE_OUTPUT" | head -1 || echo "N/A")
        DELETE_P50=$(grep -oP 'p50=\K[0-9.]+' "$DELETE_OUTPUT" | head -1 || echo "N/A")
        DELETE_P99=$(grep -oP 'p99=\K[0-9.]+' "$DELETE_OUTPUT" | head -1 || echo "N/A")
        DELETE_REQUESTS_ACTUAL=$(grep -oP 'Requests: \K[0-9,]+' "$DELETE_OUTPUT" | tr -d ',' | head -1 || echo "N/A")
    fi
    
    rm -f "$DELETE_OUTPUT"
    
    # Get memory after delete
    sleep 2
    eval $(get_memory_usage)
    POST_DELETE_MEMORY=$used_memory
    POST_DELETE_MEMORY_HUMAN=$used_memory_human
    
    print_section "vec-delete Results"
    echo "Throughput:     $DELETE_THROUGHPUT req/s"
    echo "Avg Latency:    ${DELETE_AVG_LATENCY}ms"
    echo "P50 Latency:    ${DELETE_P50}ms"
    echo "P99 Latency:    ${DELETE_P99}ms"
    echo "Requests:       $DELETE_REQUESTS_ACTUAL"
    echo "Memory After:   $POST_DELETE_MEMORY_HUMAN"
    
    # Append to report
    cat >> "$RESULT_FILE" << EOF

## vec-delete Results

| Metric | Value |
|--------|-------|
| **Throughput** | $DELETE_THROUGHPUT req/s |
| **Avg Latency** | ${DELETE_AVG_LATENCY}ms |
| **P50 Latency** | ${DELETE_P50}ms |
| **P99 Latency** | ${DELETE_P99}ms |
| **Total Requests** | $DELETE_REQUESTS_ACTUAL |
| **Memory After Delete** | $POST_DELETE_MEMORY_HUMAN |

EOF

else
    print_info "Skipping vec-delete phase (--skip-delete specified)"
    DELETE_THROUGHPUT="N/A (skipped)"
    DELETE_AVG_LATENCY="N/A"
    DELETE_P50="N/A"
    DELETE_P99="N/A"
fi

# ============================================================================
# Memory Summary
# ============================================================================

print_header "Memory Usage Summary"

eval $(get_memory_usage)
FINAL_MEMORY=$used_memory
FINAL_MEMORY_HUMAN=$used_memory_human
PEAK_MEMORY=$used_memory_peak
PEAK_MEMORY_HUMAN=$used_memory_peak_human

echo "Initial Memory:     $INITIAL_MEMORY_HUMAN"
echo "After Load:         $POST_LOAD_MEMORY_HUMAN"
echo "Peak Memory:        $PEAK_MEMORY_HUMAN"
echo "Final Memory:       $FINAL_MEMORY_HUMAN"

cat >> "$RESULT_FILE" << EOF

## Memory Usage Summary

| Phase | Memory |
|-------|--------|
| Initial | $INITIAL_MEMORY_HUMAN |
| After Load | $POST_LOAD_MEMORY_HUMAN |
| Peak | $PEAK_MEMORY_HUMAN |
| Final | $FINAL_MEMORY_HUMAN |

EOF

# ============================================================================
# Summary
# ============================================================================

print_header "Baseline Summary"

cat >> "$RESULT_FILE" << EOF

---

## Summary

| Workload | Throughput (req/s) | P50 Latency (ms) | P99 Latency (ms) |
|----------|-------------------|------------------|------------------|
| vec-load | $LOAD_THROUGHPUT | $LOAD_P50 | $LOAD_P99 |
| vec-query | $QUERY_THROUGHPUT | $QUERY_P50 | $QUERY_P99 |
| vec-delete | $DELETE_THROUGHPUT | $DELETE_P50 | $DELETE_P99 |

### Key Metrics for Migration Comparison

These metrics should be compared after each phase of the keyspace tracker migration:

- **vec-load throughput:** $LOAD_THROUGHPUT req/s
- **vec-query P50 latency:** ${QUERY_P50}ms
- **vec-query P99 latency:** ${QUERY_P99}ms
- **vec-delete throughput:** $DELETE_THROUGHPUT req/s
- **Peak memory usage:** $PEAK_MEMORY_HUMAN

---

*Report generated by baseline_benchmark.sh*
*Timestamp: $(date -Iseconds)*

EOF

echo ""
echo "╔════════════════════════════════════════════════════════════════╗"
echo "║                    BASELINE SUMMARY                            ║"
echo "╠════════════════════════════════════════════════════════════════╣"
printf "║ %-20s │ %12s │ %8s │ %8s ║\n" "Workload" "Throughput" "P50 (ms)" "P99 (ms)"
echo "╠════════════════════════════════════════════════════════════════╣"
printf "║ %-20s │ %12s │ %8s │ %8s ║\n" "vec-load" "$LOAD_THROUGHPUT" "$LOAD_P50" "$LOAD_P99"
printf "║ %-20s │ %12s │ %8s │ %8s ║\n" "vec-query" "$QUERY_THROUGHPUT" "$QUERY_P50" "$QUERY_P99"
printf "║ %-20s │ %12s │ %8s │ %8s ║\n" "vec-delete" "$DELETE_THROUGHPUT" "$DELETE_P50" "$DELETE_P99"
echo "╠════════════════════════════════════════════════════════════════╣"
printf "║ %-20s │ %37s ║\n" "Peak Memory" "$PEAK_MEMORY_HUMAN"
echo "╚════════════════════════════════════════════════════════════════╝"
echo ""

print_success "Results saved to: $RESULT_FILE"
print_success "JSON output: $JSON_FILE"

# ============================================================================
# Cleanup (optional)
# ============================================================================

echo ""
if [[ "$CLEANUP_INDEX" == "yes" ]]; then
    $CLI -h "$HOST" -p "$PORT" FT.DROPINDEX "$INDEX_NAME" 2>/dev/null || true
    print_success "Index cleaned up"
elif [[ "$CLEANUP_INDEX" == "no" ]]; then
    print_info "Keeping index '$INDEX_NAME' (use --cleanup to auto-remove)"
else
    read -p "Clean up test index ($INDEX_NAME)? [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        $CLI -h "$HOST" -p "$PORT" FT.DROPINDEX "$INDEX_NAME" 2>/dev/null || true
        print_success "Index cleaned up"
    fi
fi

print_success "Baseline benchmark complete!"
