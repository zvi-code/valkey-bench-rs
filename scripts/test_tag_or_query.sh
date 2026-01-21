#!/bin/bash
# Test script to reproduce TAG OR query issue
#
# Issue: Single tag queries work, but OR queries with multiple tags return 0 results
# - Query "@asin:{B0CK1BXTN9}" returns 1 result
# - Query "@asin:{B07CT1XJFH}" returns 1 result  
# - Query "@asin:{B07CT1XJFH | B0CK1BXTN9}" returns 0 results (BUG)
# - Query "@asin:{B07CT1XJFH|B0CK1BXTN9}" returns 0 results (BUG)

set -e

# Configuration - modify these for your environment
HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-6379}"
CLUSTER_FLAG="${CLUSTER_FLAG:---cluster}"  # Use "" for standalone

# Path to the benchmark tool
BENCH="./target/release/valkey-bench-rs"

# Test parameters
INDEX_NAME="test_tag_or"
PREFIX="test_or:"
ASIN1="B0CK1BXTN9"
ASIN2="B07CT1XJFH"

# Use MNIST dataset for testing (784 dims)
SCHEMA="datasets/mnist.yaml"
DATA="datasets/mnist.bin"
MNIST_DIM=784

echo "========================================"
echo "TAG OR Query Bug Reproduction Test"
echo "========================================"
echo "Host: $HOST:$PORT"
echo "Index: $INDEX_NAME"
echo ""

# Helper function to run CLI commands
cli() {
    $BENCH --cli -h "$HOST" -p "$PORT" $CLUSTER_FLAG -- "$@"
}

echo "Step 1: Clean up existing test data"
echo "------------------------------------"
cli FT.DROPINDEX "$INDEX_NAME" DD 2>/dev/null || echo "(index didn't exist)"
cli DEL "${PREFIX}1" "${PREFIX}2" 2>/dev/null || true
echo ""

echo "Step 2: Create index with TAG field via CLI"
echo "--------------------------------------------"
echo "FT.CREATE $INDEX_NAME ON HASH PREFIX 1 $PREFIX SCHEMA embedding VECTOR HNSW 6 TYPE FLOAT32 DIM $MNIST_DIM DISTANCE_METRIC L2 asin TAG"
cli FT.CREATE "$INDEX_NAME" ON HASH PREFIX 1 "$PREFIX" SCHEMA \
    embedding VECTOR HNSW 6 TYPE FLOAT32 DIM "$MNIST_DIM" DISTANCE_METRIC L2 \
    asin TAG
echo ""

echo "Step 3: Create test vectors directly via HSET"
echo "----------------------------------------------"
# Generate 784-dim vectors as binary hex
VEC1=$(python3 -c "import struct; print(''.join(f'{b:02x}' for b in struct.pack('<' + 'f'*$MNIST_DIM, *([0.1]*$MNIST_DIM))))")
VEC2=$(python3 -c "import struct; print(''.join(f'{b:02x}' for b in struct.pack('<' + 'f'*$MNIST_DIM, *([0.2]*$MNIST_DIM))))")

echo "Creating ${PREFIX}1 with asin=$ASIN1..."
cli HSET "${PREFIX}1" asin "$ASIN1" embedding "$VEC1"

echo "Creating ${PREFIX}2 with asin=$ASIN2..."
cli HSET "${PREFIX}2" asin "$ASIN2" embedding "$VEC2"
echo ""

echo "Step 4: Verify data loaded"
echo "--------------------------"
echo "Record 1:"
cli HGET "${PREFIX}1" asin
echo "Record 2:"
cli HGET "${PREFIX}2" asin
echo ""

echo "Step 5: Wait for indexing"
echo "-------------------------"
sleep 2
cli FT.INFO "$INDEX_NAME" | head -20 || true
echo ""

echo "========================================"
echo "Running Query Tests"
echo "========================================"

# Generate query vector (784 dims for MNIST)
QUERY_VEC=$(python3 -c "import struct; print(''.join(f'{b:02x}' for b in struct.pack('<' + 'f'*$MNIST_DIM, *([0.15]*$MNIST_DIM))))")

echo ""
echo "Query 1: Single tag filter - @asin:{$ASIN1}"
echo "-------------------------------------------"
echo "Expected: 1 result"
cli FT.SEARCH "$INDEX_NAME" "@asin:{$ASIN1}=>[KNN 5 @embedding \$vec AS score]" \
    PARAMS 2 vec "$QUERY_VEC" RETURN 2 asin score DIALECT 2
echo ""

echo "Query 2: Single tag filter - @asin:{$ASIN2}"
echo "-------------------------------------------"
echo "Expected: 1 result"
cli FT.SEARCH "$INDEX_NAME" "@asin:{$ASIN2}=>[KNN 5 @embedding \$vec AS score]" \
    PARAMS 2 vec "$QUERY_VEC" RETURN 2 asin score DIALECT 2
echo ""

echo "Query 3: OR with spaces - @asin:{$ASIN2 | $ASIN1}"
echo "--------------------------------------------------"
echo "Expected: 2 results (BUG: returns 0)"
cli FT.SEARCH "$INDEX_NAME" "@asin:{$ASIN2 | $ASIN1}=>[KNN 5 @embedding \$vec AS score]" \
    PARAMS 2 vec "$QUERY_VEC" RETURN 2 asin score DIALECT 2
echo ""

echo "Query 4: OR without spaces - @asin:{$ASIN2|$ASIN1}"
echo "---------------------------------------------------"
echo "Expected: 2 results (BUG: returns 0)"
cli FT.SEARCH "$INDEX_NAME" "@asin:{$ASIN2|$ASIN1}=>[KNN 5 @embedding \$vec AS score]" \
    PARAMS 2 vec "$QUERY_VEC" RETURN 2 asin score DIALECT 2
echo ""

echo "========================================"
echo "Additional Diagnostic Queries"
echo "========================================"

echo ""
echo "Query 5: Pure tag query (no KNN) - @asin:{$ASIN1}"
echo "--------------------------------------------------"
cli FT.SEARCH "$INDEX_NAME" "@asin:{$ASIN1}" RETURN 1 asin DIALECT 2
echo ""

echo "Query 6: Pure tag OR query (no KNN) - @asin:{$ASIN2|$ASIN1}"
echo "------------------------------------------------------------"
cli FT.SEARCH "$INDEX_NAME" "@asin:{$ASIN2|$ASIN1}" RETURN 1 asin DIALECT 2
echo ""

echo "Query 7: Alternative OR syntax with parentheses"
echo "------------------------------------------------"
cli FT.SEARCH "$INDEX_NAME" "(@asin:{$ASIN1} | @asin:{$ASIN2})=>[KNN 5 @embedding \$vec AS score]" \
    PARAMS 2 vec "$QUERY_VEC" RETURN 2 asin score DIALECT 2
echo ""

echo "Query 8: Wildcard query (should return all)"
echo "--------------------------------------------"
cli FT.SEARCH "$INDEX_NAME" "*=>[KNN 5 @embedding \$vec AS score]" \
    PARAMS 2 vec "$QUERY_VEC" RETURN 2 asin score DIALECT 2
echo ""

echo "========================================"
echo "Test Complete"
echo "========================================"
echo ""
echo "If Queries 3 and 4 return 0 results while Queries 1 and 2 return 1 result each,"
echo "this confirms the TAG OR query bug with KNN pre-filtering."
echo ""
echo "Workaround options to test:"
echo "  - Use separate tag fields per ASIN"
echo "  - Use multiple OR'd tag filter expressions: (@asin:{A} | @asin:{B})"
echo "  - Avoid combining TAG OR with KNN pre-filter"
