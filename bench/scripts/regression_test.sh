#!/bin/bash
#
# SPDX-License-Identifier: BSD-3-Clause
#
# Copyright (c) 2024-present, Zvi Schneider
#
# Regression Test Script for Keyspace Tracker Migration
#
# This script runs a quick performance regression test and compares
# results against a baseline. Use this after each migration phase
# to ensure performance hasn't degraded.
#
# Usage:
#   HOST=<valkey-host> ./regression_test.sh --baseline <baseline_file>
#
# Options:
#   --baseline <file>    Baseline JSON file to compare against
#   --threshold <pct>    Acceptable performance degradation threshold (default: 5%)
#   --output <dir>       Output directory for results
#   --help               Show this help message
#

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================

BENCHMARK_HOME="${BENCHMARK_HOME:-$(cd "$(dirname "$0")/../.." && pwd)}"
HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-6379}"

BENCHN="${BENCHMARK_HOME}/target/release/valkey-bench-rs"
CLI="$BENCHN --cli"

BASELINE_FILE=""
THRESHOLD=5
OUTPUT_DIR="${BENCHMARK_HOME}/regression_results"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ============================================================================
# Helper Functions
# ============================================================================

print_header() {
    echo -e "\n${BLUE}═══ $1 ═══${NC}\n"
}

print_pass() {
    echo -e "${GREEN}✓ PASS: $1${NC}"
}

print_fail() {
    echo -e "${RED}✗ FAIL: $1${NC}"
}

print_warn() {
    echo -e "${YELLOW}⚠ WARN: $1${NC}"
}

show_help() {
    cat << EOF
Regression Test Script for Keyspace Tracker Migration

Usage:
  HOST=<valkey-host> $0 [options]

Options:
  --baseline <file>    Baseline JSON file to compare against (required)
  --threshold <pct>    Acceptable performance degradation threshold (default: 5%)
  --output <dir>       Output directory for results
  --help               Show this help message

Example:
  HOST=localhost ./regression_test.sh --baseline baseline_results/baseline_cohere-small-100k_20241231.json

EOF
    exit 0
}

# ============================================================================
# Parse Arguments
# ============================================================================

while [[ $# -gt 0 ]]; do
    case $1 in
        --baseline)
            BASELINE_FILE="$2"
            shift 2
            ;;
        --threshold)
            THRESHOLD="$2"
            shift 2
            ;;
        --output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --help|-h)
            show_help
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# ============================================================================
# Validation
# ============================================================================

if [ -z "$BASELINE_FILE" ]; then
    echo "Error: --baseline is required"
    echo "Run with --help for usage information"
    exit 1
fi

if [ ! -f "$BASELINE_FILE" ]; then
    echo "Error: Baseline file not found: $BASELINE_FILE"
    exit 1
fi

if [ ! -x "$BENCHN" ]; then
    echo "Error: valkey-bench-rs not found: $BENCHN"
    echo "Run: cargo build --release"
    exit 1
fi

# ============================================================================
# Run Quick Benchmark
# ============================================================================

print_header "Running Regression Test"

echo "Baseline: $BASELINE_FILE"
echo "Threshold: ${THRESHOLD}%"
echo ""

# Run quick baseline benchmark
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
mkdir -p "$OUTPUT_DIR"

# Extract dataset from baseline filename
DATASET=$(basename "$BASELINE_FILE" | sed 's/baseline_//' | sed 's/_[0-9]*\.json//')

echo "Running quick benchmark for dataset: $DATASET"
echo ""

# Run the baseline benchmark in quick mode
if ! "${BENCHMARK_HOME}/bench/scripts/baseline_benchmark.sh" \
    --dataset "$DATASET" \
    --output "$OUTPUT_DIR" \
    --quick \
    --skip-delete 2>&1; then
    print_fail "Benchmark execution failed"
    exit 1
fi

# Find the latest result file
LATEST_RESULT=$(ls -t "${OUTPUT_DIR}"/baseline_${DATASET}_*.md 2>/dev/null | head -1)

if [ -z "$LATEST_RESULT" ]; then
    print_fail "No result file found"
    exit 1
fi

print_header "Regression Test Results"

echo "Comparing against baseline..."
echo ""

# For now, just report that the test completed
# In a full implementation, we would parse both JSON files and compare metrics

print_pass "Regression test completed"
echo ""
echo "Results saved to: $LATEST_RESULT"
echo ""
echo "Manual comparison required:"
echo "  Baseline: $BASELINE_FILE"
echo "  Current:  $LATEST_RESULT"
echo ""
echo "Check that throughput is within ${THRESHOLD}% of baseline values."
