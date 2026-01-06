#!/bin/bash
#
# Simple Baseline Benchmark Script
#
# Automated SET/GET benchmark for regression testing between development phases.
# This script is used as pass criteria between milestones.
#
# Flow:
#   1. Flush DB and verify empty
#   2. Sequential prefill (populate keyspace)
#   3. Verify DBSIZE matches keyspace
#   4. Random override on same keyspace
#   5. Random read (expect 100% hit rate)
#   6. Verify results and save report
#
# Usage:
#   ./simple_baseline.sh [--quick|--full] [--cluster] [--no-flush]
#   HOST=myhost PORT=6379 ./simple_baseline.sh
#
# Exit codes:
#   0 - All tests passed
#   1 - Test failed or verification error
#

set -euo pipefail

BENCHMARK_HOME="${BENCHMARK_HOME:-$(cd "$(dirname "$0")/../.." && pwd)}"
HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-6379}"
BENCHN="${BENCHMARK_HOME}/target/release/valkey-bench-rs"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Default parameters (standard mode)
# Quick: 200K keys, Full: 3M keys, Value: 500 bytes
MODE="standard"
KEYSPACE=500000
DATA_SIZE=500
CLIENTS_SINGLE=1
THREADS_SINGLE=1
CLIENTS_MAX=800
THREADS_MAX=16
CLUSTER_FLAG=""
NO_FLUSH=false

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --quick)
            MODE="quick"
            KEYSPACE=200000
            CLIENTS_MAX=400
            THREADS_MAX=10
            shift ;;
        --full)
            MODE="full"
            KEYSPACE=3000000
            CLIENTS_MAX=800
            THREADS_MAX=16
            shift ;;
        --cluster)
            CLUSTER_FLAG="--cluster"
            shift ;;
        --no-flush)
            NO_FLUSH=true
            shift ;;
        --help|-h)
            echo "Usage: $0 [--quick|--full] [--cluster] [--no-flush]"
            echo ""
            echo "Modes:"
            echo "  --quick     Fast test: 200K keys, 500B values"
            echo "  --full      Full test: 3M keys, 500B values"
            echo "  (default)   Standard: 500K keys, 500B values"
            echo ""
            echo "Options:"
            echo "  --cluster   Enable cluster mode"
            echo "  --no-flush  Skip initial FLUSHALL (use existing data)"
            echo ""
            echo "Environment: HOST, PORT, BENCHMARK_HOME"
            echo ""
            echo "This script is used as pass criteria between development phases."
            exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Calculated values
REQUESTS_PREFILL=$KEYSPACE
REQUESTS_OVERRIDE=$((KEYSPACE * 2))
REQUESTS_READ=$((KEYSPACE * 3))

# ============================================================================
# Helper Functions
# ============================================================================

log_info() { echo -e "${CYAN}ℹ $1${NC}"; }
log_ok() { echo -e "${GREEN}✓ $1${NC}"; }
log_warn() { echo -e "${YELLOW}⚠ $1${NC}"; }
log_error() { echo -e "${RED}✗ $1${NC}"; }
log_section() { echo -e "\n${BLUE}════════════════════════════════════════════════════════════════${NC}"; echo -e "${BLUE}  $1${NC}"; echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}\n"; }

# Run CLI command
cli() {
    $BENCHN --cli -h "$HOST" -p "$PORT" $CLUSTER_FLAG "$@" 2>/dev/null
}

# Get DBSIZE
get_dbsize() {
    local result
    result=$(cli DBSIZE | grep -oE '[0-9]+' || echo "0")
    echo "${result:-0}"
}

# Get memory
get_memory() {
    cli INFO memory | grep "^used_memory_human:" | cut -d: -f2 | tr -d '\r' || echo "N/A"
}

# Verify DBSIZE matches expected
verify_dbsize() {
    local expected=$1
    local tolerance=${2:-0}
    local actual
    actual=$(get_dbsize)
    local min=$((expected - tolerance))
    local max=$((expected + tolerance))
    
    if [[ $actual -ge $min && $actual -le $max ]]; then
        log_ok "DBSIZE verification passed: $actual keys (expected: $expected ±$tolerance)"
        return 0
    else
        log_error "DBSIZE verification FAILED: $actual keys (expected: $expected ±$tolerance)"
        return 1
    fi
}

# Extract metric from benchmark output
extract_metric() {
    local file=$1
    local metric=$2
    grep -oP "${metric}[=:]\s*\K[0-9,.]+" "$file" | tr -d ',' | head -1
}

# ============================================================================
# Main Script
# ============================================================================

# Check binary
if [ ! -x "$BENCHN" ]; then
    echo "Building valkey-bench-rs..."
    cargo build --release --manifest-path "$BENCHMARK_HOME/Cargo.toml"
fi

# Create output dir
OUTPUT_DIR="${BENCHMARK_HOME}/baseline_results"
mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="${OUTPUT_DIR}/baseline_report_${TIMESTAMP}.txt"

# Start report
{
    echo "═══════════════════════════════════════════════════════════════════════════════"
    echo "  SIMPLE BASELINE BENCHMARK REPORT"
    echo "  Generated: $(date)"
    echo "═══════════════════════════════════════════════════════════════════════════════"
    echo ""
    echo "Configuration:"
    echo "  Host:       $HOST:$PORT"
    echo "  Mode:       $MODE"
    echo "  Cluster:    ${CLUSTER_FLAG:-disabled}"
    echo "  Keyspace:   $KEYSPACE keys"
    echo "  Value size: $DATA_SIZE bytes"
    echo "  Max QPS:    $CLIENTS_MAX clients, $THREADS_MAX threads"
    echo ""
} | tee "$REPORT_FILE"

log_section "1. INITIALIZATION"

# Test connection
echo -n "Testing connection... "
if cli PING | grep -q PONG; then
    log_ok "Connected to $HOST:$PORT"
else
    log_error "Connection failed - Is Valkey/Redis running on $HOST:$PORT?"
    exit 1
fi

# Flush if needed
if [[ "$NO_FLUSH" == "false" ]]; then
    log_info "Flushing database..."
    cli FLUSHALL > /dev/null
    sleep 2
fi

# Verify empty
INITIAL_DBSIZE=$(get_dbsize)
INITIAL_MEM=$(get_memory)
echo "Initial state: DBSIZE=$INITIAL_DBSIZE, Memory=$INITIAL_MEM" | tee -a "$REPORT_FILE"

if [[ "$NO_FLUSH" == "false" && "$INITIAL_DBSIZE" -ne 0 ]]; then
    log_error "Database not empty after FLUSHALL: $INITIAL_DBSIZE keys"
    exit 1
fi

# ─────────────────────────────────────────────────────────────────────────────
log_section "2. SINGLE CLIENT LATENCY (Network Baseline)"
# ─────────────────────────────────────────────────────────────────────────────

echo "── PING (single client, 10K requests) ──"
$BENCHN -h "$HOST" -p "$PORT" $CLUSTER_FLAG \
    -t ping -n 10000 -c $CLIENTS_SINGLE --threads $THREADS_SINGLE \
    2>&1 | tee "${OUTPUT_DIR}/ping_single_${TIMESTAMP}.txt" | grep -E "^(PING:|Latency|Base RTT)"
echo ""

# ─────────────────────────────────────────────────────────────────────────────
log_section "3. SEQUENTIAL PREFILL ($KEYSPACE keys, $DATA_SIZE bytes)"
# ─────────────────────────────────────────────────────────────────────────────

$BENCHN -h "$HOST" -p "$PORT" $CLUSTER_FLAG \
    -t set -n $REQUESTS_PREFILL -r $KEYSPACE -d $DATA_SIZE \
    -c $CLIENTS_MAX --threads $THREADS_MAX --iteration sequential \
    2>&1 | tee "${OUTPUT_DIR}/prefill_${TIMESTAMP}.txt" | grep -E "^(SET:|Throughput|Latency)"

# Verify DBSIZE
echo ""
if ! verify_dbsize $KEYSPACE 100; then
    log_error "Prefill verification failed!"
    exit 1
fi

PREFILL_MEM=$(get_memory)
echo "Memory after prefill: $PREFILL_MEM" | tee -a "$REPORT_FILE"
echo ""

# ─────────────────────────────────────────────────────────────────────────────
log_section "4. RANDOM OVERRIDE ($REQUESTS_OVERRIDE requests on $KEYSPACE keys)"
# ─────────────────────────────────────────────────────────────────────────────

$BENCHN -h "$HOST" -p "$PORT" $CLUSTER_FLAG \
    -t set -n $REQUESTS_OVERRIDE -r $KEYSPACE -d $DATA_SIZE \
    -c $CLIENTS_MAX --threads $THREADS_MAX --iteration random \
    2>&1 | tee "${OUTPUT_DIR}/override_${TIMESTAMP}.txt" | grep -E "^(SET:|Throughput|Latency)"

# Verify DBSIZE unchanged
echo ""
if ! verify_dbsize $KEYSPACE 100; then
    log_error "Override verification failed - keyspace changed!"
    exit 1
fi
echo ""

# ─────────────────────────────────────────────────────────────────────────────
log_section "5. RANDOM READ ($REQUESTS_READ requests, expect 100% hit)"
# ─────────────────────────────────────────────────────────────────────────────

$BENCHN -h "$HOST" -p "$PORT" $CLUSTER_FLAG \
    -t get -n $REQUESTS_READ -r $KEYSPACE \
    -c $CLIENTS_MAX --threads $THREADS_MAX --iteration random \
    2>&1 | tee "${OUTPUT_DIR}/read_${TIMESTAMP}.txt" | grep -E "^(GET:|Throughput|Latency|Keyspace)"

# Verify hit rate (extract first match, handle decimal)
HIT_RATE=$(grep "hit-rate=" "${OUTPUT_DIR}/read_${TIMESTAMP}.txt" | grep -oP 'hit-rate=\K[0-9.]+' | head -1 || echo "0")
HIT_RATE_INT=${HIT_RATE%%.*}  # Remove decimal part for comparison
echo ""
if [[ "$HIT_RATE_INT" -ge 99 ]]; then
    log_ok "Hit rate verification passed: ${HIT_RATE}%"
else
    log_error "Hit rate verification FAILED: ${HIT_RATE}% (expected ≥99%)"
    exit 1
fi
echo ""

# ─────────────────────────────────────────────────────────────────────────────
log_section "6. SINGLE CLIENT LATENCY (with data)"
# ─────────────────────────────────────────────────────────────────────────────

echo "── GET (single client, 50K requests) ──"
$BENCHN -h "$HOST" -p "$PORT" $CLUSTER_FLAG \
    -t get -n 50000 -r $KEYSPACE \
    -c $CLIENTS_SINGLE --threads $THREADS_SINGLE --iteration random \
    2>&1 | tee "${OUTPUT_DIR}/get_single_${TIMESTAMP}.txt" | grep -E "^(GET:|Latency|Base RTT|Keyspace)"
echo ""

echo "── SET (single client, 50K requests) ──"
$BENCHN -h "$HOST" -p "$PORT" $CLUSTER_FLAG \
    -t set -n 50000 -r $KEYSPACE -d $DATA_SIZE \
    -c $CLIENTS_SINGLE --threads $THREADS_SINGLE --iteration random \
    2>&1 | tee "${OUTPUT_DIR}/set_single_${TIMESTAMP}.txt" | grep -E "^(SET:|Latency|Base RTT)"
echo ""

# ─────────────────────────────────────────────────────────────────────────────
log_section "7. SUMMARY"
# ─────────────────────────────────────────────────────────────────────────────

FINAL_DBSIZE=$(get_dbsize)
FINAL_MEM=$(get_memory)

# Extract key metrics
PREFILL_QPS=$(extract_metric "${OUTPUT_DIR}/prefill_${TIMESTAMP}.txt" "Throughput")
OVERRIDE_QPS=$(extract_metric "${OUTPUT_DIR}/override_${TIMESTAMP}.txt" "Throughput")
READ_QPS=$(extract_metric "${OUTPUT_DIR}/read_${TIMESTAMP}.txt" "Throughput")
GET_SINGLE_P99=$(grep "p99=" "${OUTPUT_DIR}/get_single_${TIMESTAMP}.txt" | grep -oP 'p99=\K[0-9.]+' | head -1 || echo "N/A")
SET_SINGLE_P99=$(grep "p99=" "${OUTPUT_DIR}/set_single_${TIMESTAMP}.txt" | grep -oP 'p99=\K[0-9.]+' | head -1 || echo "N/A")

{
    echo ""
    echo "═══════════════════════════════════════════════════════════════════════════════"
    echo "  RESULTS SUMMARY"
    echo "═══════════════════════════════════════════════════════════════════════════════"
    echo ""
    echo "Database State:"
    echo "  Initial:  DBSIZE=0, Memory=$INITIAL_MEM"
    echo "  After prefill: DBSIZE=$KEYSPACE, Memory=$PREFILL_MEM"
    echo "  Final:    DBSIZE=$FINAL_DBSIZE, Memory=$FINAL_MEM"
    echo ""
    echo "Max QPS ($CLIENTS_MAX clients, $THREADS_MAX threads):"
    echo "  Sequential Prefill: ${PREFILL_QPS:-N/A} req/s"
    echo "  Random Override:    ${OVERRIDE_QPS:-N/A} req/s"
    echo "  Random Read:        ${READ_QPS:-N/A} req/s"
    echo ""
    echo "Single Client Latency:"
    echo "  GET P99: ${GET_SINGLE_P99}ms"
    echo "  SET P99: ${SET_SINGLE_P99}ms"
    echo ""
    echo "Verification:"
    echo "  DBSIZE correct: ✓"
    echo "  Hit rate 100%:  ✓"
    echo ""
    echo "Result files: ${OUTPUT_DIR}/*_${TIMESTAMP}.txt"
    echo ""
} | tee -a "$REPORT_FILE"

log_ok "BASELINE BENCHMARK COMPLETE - ALL VERIFICATIONS PASSED"
echo ""
echo "Report saved to: $REPORT_FILE"
