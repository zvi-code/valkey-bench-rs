#!/bin/bash
# Clean a Valkey/Redis host by flushing all data and dropping all indexes

set -e

HOST="${1:-localhost}"
PORT="${2:-6379}"

BENCH="./target/release/valkey-bench-rs"

echo "Cleaning host: $HOST:$PORT"

# Step 1: FLUSHALL
echo "Running FLUSHALL..."
$BENCH --cli -h "$HOST" -p "$PORT" -- FLUSHALL

# Step 2: Get list of indexes and drop them
echo "Checking for indexes..."
RAW_INDEXES=$($BENCH --cli -h "$HOST" -p "$PORT" -- FT._LIST 2>/dev/null || echo "")

if [ -z "$RAW_INDEXES" ] || echo "$RAW_INDEXES" | grep -q "empty"; then
    echo "No indexes found."
else
    echo "Raw output: $RAW_INDEXES"
    # Parse: "1) idx1\n2) idx2" -> extract just index names
    echo "$RAW_INDEXES" | sed 's/^[0-9]*) //' | while read -r idx; do
        if [ -n "$idx" ]; then
            echo "Dropping index: $idx"
            $BENCH --cli -h "$HOST" -p "$PORT" -- FT.DROPINDEX "$idx"
        fi
    done
fi

# Verify clean state
echo ""
echo "=== Verification ==="
echo -n "DBSIZE: "
$BENCH --cli -h "$HOST" -p "$PORT" -- DBSIZE
echo -n "Indexes: "
$BENCH --cli -h "$HOST" -p "$PORT" -- FT._LIST 2>/dev/null || echo "(FT not available)"

echo ""
echo "Host cleaned successfully!"
