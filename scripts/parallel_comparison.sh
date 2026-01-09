# Parallel benchmark comparison between H1 and H2 hosts
# Runs both benchmarks simultaneously and combines results
#
# Usage: ./parallel_comparison.sh [INCREMENT_SIZE] [DELETE_PERCENT] [NUM_CYCLES]
#   INCREMENT_SIZE: Number of vectors per increment (default: 1000000)
#   DELETE_PERCENT: Percentage of vectors to delete each cycle (default: 90)
#   NUM_CYCLES: Number of load-query-delete cycles (default: 5)
#   Example: ./parallel_comparison.sh 100000 90 5 H1 H2 # Use 100K increments, 90% delete, 5 cycles, H1 and H2 hosts
#
# Regenerate mode: ./parallel_comparison.sh --regenerate TIMESTAMP
#   Regenerates the comparison table from existing results files
#   Example: ./parallel_comparison.sh --regenerate 20260109_013350

set -e

H1HOST=$4
H2HOST=$5
DATASET="cohere-large-10m"
SCHEMA="datasets/${DATASET}.yaml"
DATA="datasets/${DATASET}.bin"
INDEX="myidx"
PREFIX="vec:"
BENCH="./target/release/valkey-bench-rs"

# Check for regenerate mode
if [[ "$1" == "--regenerate" ]]; then
    REGEN_TIMESTAMP="$2"
    if [[ -z "$REGEN_TIMESTAMP" ]]; then
        echo "Usage: $0 --regenerate TIMESTAMP"
        echo "Example: $0 --regenerate 20260109_013350"
        echo ""
        echo "Available result files in /tmp:"
        ls -1 /tmp/h1_results_*.txt 2>/dev/null | sed 's|.*h1_results_\(.*\)\.txt|\1|' || echo "  (none found)"
        exit 1
    fi
    REGENERATE_MODE=true
    # Set dummy values for config display (not used in regenerate mode)
    INCREMENT="N/A"
    DELETE_PERCENT="N/A"
    NUM_CYCLES="N/A"
else
    REGENERATE_MODE=false
    # Parameterized increment size (default 1M, can be 100K for testing)
    INCREMENT=${1:-1000000}
    DELETE_PERCENT=${2:-90}
    DELETE_COUNT=$((INCREMENT * DELETE_PERCENT / 100))
    # Number of cycles (load + query + delete)
    NUM_CYCLES=${3:-5}
fi

# Output files
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
H1_RESULTS="/tmp/h1_results_${TIMESTAMP}.txt"
H2_RESULTS="/tmp/h2_results_${TIMESTAMP}.txt"
H1_LOG="/tmp/h1_benchmark_${TIMESTAMP}.log"
H2_LOG="/tmp/h2_benchmark_${TIMESTAMP}.log"
TABLE_OUTPUT="benchmark_comparison_parallel_${TIMESTAMP}.md"

# Function to extract metrics from benchmark output
extract_metrics() {
    local output="$1"
    local qps=$(echo "$output" | grep -oP 'Throughput:\s*\K[0-9,]+' | tr -d ',' | head -1)
    local requests=$(echo "$output" | grep -oP 'Requests:\s*\K[0-9,]+' | tr -d ',' | head -1)
    # Extract from "Database: keys=X memory=Y" format
    local keys=$(echo "$output" | grep -oP 'Database: keys=\K[0-9,]+' | tr -d ',' | tail -1)
    local memory=$(echo "$output" | grep -oP 'Database: keys=[0-9,]+ memory=\K[0-9.]+[KMGTB]+' | tail -1)
    local recall=$(echo "$output" | grep -oP 'Recall.*avg=\K[0-9.]+' | head -1)
    local avg_latency=$(echo "$output" | grep -oP 'Latency.*avg=\K[0-9.]+' | head -1)
    
    echo "${qps:-0}|${requests:-0}|${memory:-N/A}|${keys:-0}|${recall:-N/A}|${avg_latency:-0}"
}

# Get baseline info (memory and dbsize)
get_baseline() {
    local host="$1"
    local cluster_flag="$2"
    local dbsize=$($BENCH --cli -h "$host" $cluster_flag -- DBSIZE 2>/dev/null | grep -oP '\d+' || echo "0")
    local memory=$($BENCH --cli -h "$host" $cluster_flag -- INFO memory 2>/dev/null | grep -oP 'used_memory_human:\K[0-9.]+[KMGT]?' || echo "0")
    echo "0|0|${memory}|${dbsize}|N/A|0"
}

# Run benchmark for a single host
run_host_benchmark() {
    local host="$1"
    local results_file="$2"
    local host_label="$3"
    local cluster_flag="$4"
    local log_file="$5"
    
    exec > >(tee -a "$log_file") 2>&1
    
    echo ""
    echo "=========================================="
    echo "Running benchmark on $host_label ($host)"
    echo "Started at: $(date)"
    echo "=========================================="
    
    # Step 1: Cleanup and baseline
    echo "[Step 1] Cleanup and baseline..."
    $BENCH --cli -h "$host" $cluster_flag -- FT.DROPINDEX "$INDEX" 2>/dev/null || true
    $BENCH --cli -h "$host" $cluster_flag -- FLUSHALL 2>/dev/null || true
    sleep 2
    metrics=$(get_baseline "$host" "$cluster_flag")
    echo "1|cleanup|FLUSHALL+DROPINDEX|N/A|$metrics" >> "$results_file"
    echo "  Done: Baseline captured"
    
    # Step 2: Load GT vectors
    echo "[Step 2] Loading GT vectors..."
    output=$($BENCH -h "$host" $cluster_flag \
        --schema "$SCHEMA" --data "$DATA" \
        -t vec-gt-load \
        --search-index "$INDEX" \
        --search-prefix "$PREFIX" \
        -c 100 --threads 8 2>&1)
    echo "$output"
    metrics=$(extract_metrics "$output")
    echo "2|load-gt|vec-gt-load|GT(10K)|$metrics" >> "$results_file"
    echo "  Done: GT loaded"
    
    # Current offset starts at INCREMENT (e.g., 1M or 100K)
    local current_offset=$INCREMENT
    local step=3
    
    for cycle in $(seq 1 $NUM_CYCLES); do
        local range_start=$current_offset
        local range_end=$((current_offset + INCREMENT))
        local range_label="${range_start}-${range_end}"
        
        # Load vectors for this range
        echo "[$host_label] Step $step: Loading vectors ${range_label}..."
        output=$($BENCH -h "$host" $cluster_flag \
            --schema "$SCHEMA" --data "$DATA" \
            -t vec-load -n $INCREMENT \
            --vector-offset $current_offset \
            --search-index "$INDEX" \
            --search-prefix "$PREFIX" \
            -c 200 --threads 16 2>&1)
        echo "$output"
        metrics=$(extract_metrics "$output")
        echo "${step}|load|vec-load|${range_label}|$metrics" >> "$results_file"
        echo "[$host_label] Loaded ${range_label}"
        step=$((step + 1))
        
        # Query
        echo "[$host_label] Step $step: Query 10K with ef-search 350..."
        output=$($BENCH -h "$host" $cluster_flag \
            --schema "$SCHEMA" --data "$DATA" \
            -t vec-query -n 10000 \
            --ef-search 350 \
            --search-index "$INDEX" \
            --search-prefix "$PREFIX" \
            -c 50 --threads 4 2>&1)
        echo "$output"
        metrics=$(extract_metrics "$output")
        echo "${step}|query|vec-query|ef=350|$metrics" >> "$results_file"
        echo "[$host_label] Query done"
        step=$((step + 1))
        
        # Delete 90% of this range (skip on last cycle)
        if [[ $cycle -lt $NUM_CYCLES ]]; then
            echo "[$host_label] Step $step: Deleting ${DELETE_COUNT} vectors from ${range_label}..."
            output=$($BENCH -h "$host" $cluster_flag \
                --schema "$SCHEMA" --data "$DATA" \
                -t vec-del -n $DELETE_COUNT \
                --vector-offset $current_offset \
                --num-vectors $INCREMENT \
                --search-index "$INDEX" \
                --search-prefix "$PREFIX" \
                -c 100 --threads 8 2>&1)
            echo "$output"
            metrics=$(extract_metrics "$output")
            echo "${step}|delete|vec-del|${range_label}(${DELETE_PERCENT}%)|$metrics" >> "$results_file"
            echo "[$host_label] Deleted ${DELETE_COUNT} from ${range_label}"
            step=$((step + 1))
        fi
        
        # Move to next range
        current_offset=$range_end
    done
    
    echo ""
    echo "[$host_label] Benchmark complete!"
    
    echo ""
    echo "=========================================="
    echo "Benchmark complete for $host_label"
    echo "Finished at: $(date)"
    echo "=========================================="
}

# Generate comparison table from results
generate_comparison_table() {
    local h1_file="$1"
    local h2_file="$2"
    local output_file="$3"
    
    echo "# Benchmark Comparison: H1 vs H2" > "$output_file"
    echo "" >> "$output_file"
    echo "**Generated:** $(date)" >> "$output_file"
    echo "" >> "$output_file"
    echo "## Configuration" >> "$output_file"
    echo "" >> "$output_file"
    echo "- **H1 Host:** \`$H1HOST\` " >> "$output_file"
    echo "- **H2 Host:** \`$H2HOST\` " >> "$output_file"
    echo "- **Dataset:** \`$DATASET\`" >> "$output_file"
    echo "- **Index:** \`$INDEX\`" >> "$output_file"
    echo "- **Increment Size:** $INCREMENT vectors" >> "$output_file"
    echo "- **Delete Percent:** ${DELETE_PERCENT}%" >> "$output_file"
    echo "- **Cycles:** $NUM_CYCLES" >> "$output_file"
    echo "" >> "$output_file"
    
    # Helper function to convert memory string (e.g., "725.00M", "4.06G") to bytes
    mem_to_bytes() {
        local mem_str="$1"
        local val=$(echo "$mem_str" | grep -oP '^[0-9.]+')
        local unit=$(echo "$mem_str" | grep -oP '[KMGTB]+$' | head -c1)
        awk -v val="$val" -v unit="$unit" 'BEGIN {
            if (unit == "K") printf "%.0f", val * 1024
            else if (unit == "M") printf "%.0f", val * 1024 * 1024
            else if (unit == "G") printf "%.0f", val * 1024 * 1024 * 1024
            else if (unit == "T") printf "%.0f", val * 1024 * 1024 * 1024 * 1024
            else printf "%.0f", val
        }'
    }
    
    # Helper function to format bytes to human-readable with appropriate unit
    bytes_to_human() {
        local bytes="$1"
        local is_delta="${2:-false}"
        awk -v bytes="$bytes" -v is_delta="$is_delta" 'BEGIN {
            if (bytes < 0) { sign = "-"; bytes = -bytes } else { sign = (is_delta == "true" && bytes > 0) ? "+" : "" }
            if (bytes >= 1024*1024*1024*1024) printf "%s%.2fT", sign, bytes/(1024*1024*1024*1024)
            else if (bytes >= 1024*1024*1024) printf "%s%.2fG", sign, bytes/(1024*1024*1024)
            else if (bytes >= 1024*1024) printf "%s%.2fM", sign, bytes/(1024*1024)
            else if (bytes >= 1024) printf "%s%.2fK", sign, bytes/1024
            else printf "%s%.0fB", sign, bytes
        }'
    }
    
    # First pass: extract baseline values from GT load (step 2)
    local h1_baseline_keys=0
    local h1_baseline_mem_bytes=0
    local h2_baseline_keys=0
    local h2_baseline_mem_bytes=0
    
    while IFS='|' read -r step name cmd range qps requests memory keys recall latency; do
        if [[ "$step" == "2" ]]; then
            h1_baseline_keys=$keys
            h1_baseline_mem_bytes=$(mem_to_bytes "$memory")
            break
        fi
    done < "$h1_file"
    
    while IFS='|' read -r step name cmd range qps requests memory keys recall latency; do
        if [[ "$step" == "2" ]]; then
            h2_baseline_keys=$keys
            h2_baseline_mem_bytes=$(mem_to_bytes "$memory")
            break
        fi
    done < "$h2_file"
    
    # Format baseline for display
    local h1_baseline_mem_human=$(bytes_to_human "$h1_baseline_mem_bytes")
    local h2_baseline_mem_human=$(bytes_to_human "$h2_baseline_mem_bytes")
    
    echo "## Results Comparison (Keys & Memory relative to baseline)" >> "$output_file"
    echo "" >> "$output_file"
    echo "**Baseline (after GT load):** H1 = ${h1_baseline_keys} keys / ${h1_baseline_mem_human} | H2 = ${h2_baseline_keys} keys / ${h2_baseline_mem_human}" >> "$output_file"
    echo "" >> "$output_file"
    echo "| Step | Operation | Range | H1 QPS | H2 QPS | H1 Keys | H2 Keys | H1 Memory | H2 Memory | H1 Mem/Key | H2 Mem/Key | H1 Recall | H2 Recall |" >> "$output_file"
    echo "|-----:|-----------|-------|-------:|--------:|--------:|---------:|----------:|-----------:|-----------:|------------:|:---------:|:----------:|" >> "$output_file"
    
    # Second pass: output rows with baseline subtracted
    # Format: step|name|cmd|range|qps|requests|memory|keys|recall|latency
    while IFS='|' read -r step name cmd range qps requests memory keys recall latency; do
        # Find matching line in H2 results
        h2_line=$(grep "^${step}|" "$h2_file" 2>/dev/null || echo "")
        if [[ -n "$h2_line" ]]; then
            IFS='|' read -r h2_step h2_name h2_cmd h2_range h2_qps h2_requests h2_memory h2_keys h2_recall h2_latency <<< "$h2_line"
            
            # Format QPS with commas
            qps_fmt=$(printf "%'d" "$qps" 2>/dev/null || echo "$qps")
            h2_qps_fmt=$(printf "%'d" "$h2_qps" 2>/dev/null || echo "$h2_qps")
            
            # Calculate relative keys and memory
            if [[ "$step" == "1" || "$step" == "2" ]]; then
                # Baseline rows show 0
                h1_keys_rel="0"
                h2_keys_rel="0"
                h1_mem_rel="0"
                h2_mem_rel="0"
                h1_mem_per_key="-"
                h2_mem_per_key="-"
            else
                # Subtract baseline
                h1_keys_delta=$((keys - h1_baseline_keys))
                h2_keys_delta=$((h2_keys - h2_baseline_keys))
                
                # Format with + sign
                if [[ $h1_keys_delta -ge 0 ]]; then
                    h1_keys_rel="+$(printf "%'d" $h1_keys_delta)"
                else
                    h1_keys_rel="$(printf "%'d" $h1_keys_delta)"
                fi
                if [[ $h2_keys_delta -ge 0 ]]; then
                    h2_keys_rel="+$(printf "%'d" $h2_keys_delta)"
                else
                    h2_keys_rel="$(printf "%'d" $h2_keys_delta)"
                fi
                
                # Calculate memory delta using bytes (handles different units correctly)
                h1_mem_bytes=$(mem_to_bytes "$memory")
                h2_mem_bytes=$(mem_to_bytes "$h2_memory")
                
                h1_mem_delta_bytes=$(awk "BEGIN {printf \"%.0f\", $h1_mem_bytes - $h1_baseline_mem_bytes}")
                h2_mem_delta_bytes=$(awk "BEGIN {printf \"%.0f\", $h2_mem_bytes - $h2_baseline_mem_bytes}")
                
                # Format memory delta with human-readable units
                h1_mem_rel=$(bytes_to_human "$h1_mem_delta_bytes" "true")
                h2_mem_rel=$(bytes_to_human "$h2_mem_delta_bytes" "true")
                
                # Calculate memory per key (in KB)
                if [[ $h1_keys_delta -gt 0 ]]; then
                    h1_mem_per_key=$(awk -v mem="$h1_mem_delta_bytes" -v keys="$h1_keys_delta" 'BEGIN {
                        if (mem < 0) mem = -mem
                        if (keys < 0) keys = -keys
                        kb = mem / 1024 / keys
                        printf "%.1fKB", kb
                    }')
                else
                    h1_mem_per_key="-"
                fi
                
                if [[ $h2_keys_delta -gt 0 ]]; then
                    h2_mem_per_key=$(awk -v mem="$h2_mem_delta_bytes" -v keys="$h2_keys_delta" 'BEGIN {
                        if (mem < 0) mem = -mem
                        if (keys < 0) keys = -keys
                        kb = mem / 1024 / keys
                        printf "%.1fKB", kb
                    }')
                else
                    h2_mem_per_key="-"
                fi
            fi
            
            echo "| $step | $name | $range | $qps_fmt | $h2_qps_fmt | $h1_keys_rel | $h2_keys_rel | $h1_mem_rel | $h2_mem_rel | $h1_mem_per_key | $h2_mem_per_key | $recall | $h2_recall |" >> "$output_file"
        else
            qps_fmt=$(printf "%'d" "$qps" 2>/dev/null || echo "$qps")
            echo "| $step | $name | $range | $qps_fmt | - | - | - | - | - | $recall | - |" >> "$output_file"
        fi
    done < "$h1_file"
    
    echo "" >> "$output_file"
    echo "## Detailed Logs" >> "$output_file"
    echo "- H1 Log: $H1_LOG" >> "$output_file"
    echo "- H2 Log: $H2_LOG" >> "$output_file"
    echo "" >> "$output_file"
    
    # Add QPS comparison summary
    echo "## QPS Summary" >> "$output_file"
    echo "" >> "$output_file"
    echo "| Operation Type | H1 Avg QPS | H2 Avg QPS | Ratio (H1/H2) |" >> "$output_file"
    echo "|----------------|------------|-------------|----------------|" >> "$output_file"
    
    # Calculate averages for each operation type
    for op_type in "load" "query" "delete"; do
        h1_sum=0
        h1_count=0
        h2_sum=0
        h2_count=0
        
        while IFS='|' read -r step name cmd range qps requests memory keys recall latency; do
            if [[ "$name" == *"$op_type"* ]] || [[ "$cmd" == *"$op_type"* ]]; then
                if [[ "$qps" =~ ^[0-9]+$ ]] && [[ "$qps" -gt 0 ]]; then
                    h1_sum=$((h1_sum + qps))
                    h1_count=$((h1_count + 1))
                fi
            fi
        done < "$h1_file"
        
        while IFS='|' read -r step name cmd range qps requests memory keys recall latency; do
            if [[ "$name" == *"$op_type"* ]] || [[ "$cmd" == *"$op_type"* ]]; then
                if [[ "$qps" =~ ^[0-9]+$ ]] && [[ "$qps" -gt 0 ]]; then
                    h2_sum=$((h2_sum + qps))
                    h2_count=$((h2_count + 1))
                fi
            fi
        done < "$h2_file"
        
        if [[ $h1_count -gt 0 ]] && [[ $h2_count -gt 0 ]]; then
            h1_avg=$((h1_sum / h1_count))
            h2_avg=$((h2_sum / h2_count))
            if [[ $h2_avg -gt 0 ]]; then
                ratio=$(awk "BEGIN {printf \"%.2fx\", $h1_avg / $h2_avg}")
            else
                ratio="N/A"
            fi
            # Format numbers with commas
            h1_avg_fmt=$(printf "%'d" $h1_avg)
            h2_avg_fmt=$(printf "%'d" $h2_avg)
            echo "| $op_type | $h1_avg_fmt | $h2_avg_fmt | **$ratio** |" >> "$output_file"
        fi
    done
    
    echo "" >> "$output_file"
}

# Handle regenerate mode
if [[ "$REGENERATE_MODE" == "true" ]]; then
    H1_RESULTS="/tmp/h1_results_${REGEN_TIMESTAMP}.txt"
    H2_RESULTS="/tmp/h2_results_${REGEN_TIMESTAMP}.txt"
    H1_LOG="/tmp/h1_benchmark_${REGEN_TIMESTAMP}.log"
    H2_LOG="/tmp/h2_benchmark_${REGEN_TIMESTAMP}.log"
    TABLE_OUTPUT="benchmark_comparison_parallel_${REGEN_TIMESTAMP}_regen.md"
    
    # Verify files exist
    if [[ ! -f "$H1_RESULTS" ]]; then
        echo "ERROR: H1 results file not found: $H1_RESULTS"
        exit 1
    fi
    if [[ ! -f "$H2_RESULTS" ]]; then
        echo "ERROR: H2 results file not found: $H2_RESULTS"
        exit 1
    fi
    
    echo "=========================================="
    echo "Regenerating Comparison Table"
    echo "=========================================="
    echo "Timestamp: $REGEN_TIMESTAMP"
    echo "H1 results: $H1_RESULTS"
    echo "H2 results: $H2_RESULTS"
    echo "Output: $TABLE_OUTPUT"
    echo "=========================================="
    echo ""
    
    generate_comparison_table "$H1_RESULTS" "$H2_RESULTS" "$TABLE_OUTPUT"
    
    echo "Regeneration complete!"
    echo ""
    echo "--- Comparison Table ---"
    cat "$TABLE_OUTPUT"
    exit 0
fi

# Main execution
echo "=========================================="
echo "Parallel Benchmark Comparison: H1 vs H2"
echo "=========================================="
echo "H1 Host: $H1HOST (cluster mode)"
echo "H2 Host: $H2HOST (standalone)"
echo "Dataset: $DATASET"
echo "Index: $INDEX"
echo "=========================================="
echo ""

# Build if needed
echo "Building valkey-bench-rs..."
cargo build --release 2>/dev/null || cargo build --release

# Verify binary exists
if [[ ! -f "$BENCH" ]]; then
    echo "ERROR: Binary not found at $BENCH"
    exit 1
fi

# Verify dataset files exist
if [[ ! -f "$SCHEMA" ]]; then
    echo "ERROR: Schema file not found at $SCHEMA"
    exit 1
fi

if [[ ! -f "$DATA" ]]; then
    echo "ERROR: Data file not found at $DATA"
    exit 1
fi

# Initialize result files
> "$H1_RESULTS"
> "$H2_RESULTS"
> "$H1_LOG"
> "$H2_LOG"

echo ""
echo "Starting parallel benchmarks..."
echo "  H1 log: $H1_LOG"
echo "  H2 log: $H2_LOG"
echo ""

# Run both benchmarks in parallel
echo "[$(date +%H:%M:%S)] Starting H1 benchmark in background..."
(run_host_benchmark "$H1HOST" "$H1_RESULTS" "H1" "--cluster" "$H1_LOG") &
H1_PID=$!

echo "[$(date +%H:%M:%S)] Starting H2 benchmark in background..."
(run_host_benchmark "$H2HOST" "$H2_RESULTS" "H2" "" "$H2_LOG") &
H2_PID=$!

echo ""
echo "Both benchmarks running in parallel..."
echo "  H1 PID: $H1_PID"
echo "  H2 PID: $H2_PID"
echo ""
echo "You can monitor progress with:"
echo "  tail -f $H1_LOG"
echo "  tail -f $H2_LOG"
echo ""

# Wait for both to complete
echo "Waiting for benchmarks to complete..."

H1_STATUS=0
H2_STATUS=0

wait $H1_PID || H1_STATUS=$?
echo "[$(date +%H:%M:%S)] H1 benchmark finished (exit code: $H1_STATUS)"

wait $H2_PID || H2_STATUS=$?
echo "[$(date +%H:%M:%S)] H2 benchmark finished (exit code: $H2_STATUS)"

echo ""

# Check if either failed
if [[ $H1_STATUS -ne 0 ]]; then
    echo "WARNING: H1 benchmark failed with exit code $H1_STATUS"
    echo "Check log: $H1_LOG"
fi

if [[ $H2_STATUS -ne 0 ]]; then
    echo "WARNING: H2 benchmark failed with exit code $H2_STATUS"
    echo "Check log: $H2_LOG"
fi

# Generate comparison table
echo ""
echo "Generating comparison table..."
generate_comparison_table "$H1_RESULTS" "$H2_RESULTS" "$TABLE_OUTPUT"

echo ""
echo "=========================================="
echo "Benchmark Complete!"
echo "=========================================="
echo ""
echo "Results:"
echo "  Comparison table: $TABLE_OUTPUT"
echo "  H1 results: $H1_RESULTS"
echo "  H2 results: $H2_RESULTS"
echo "  H1 log: $H1_LOG"
echo "  H2 log: $H2_LOG"
echo ""

# Display the table
echo "--- Comparison Table ---"
cat "$TABLE_OUTPUT"
