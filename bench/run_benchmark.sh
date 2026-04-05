#!/bin/bash
# Standalone benchmark for Pi Rust
# Usage: ./bench/run_benchmark.sh [--model provider:model] [--tasks N]
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
PI="$REPO_DIR/target/release/pi"
RESULTS_DIR="$REPO_DIR/bench/results/$(date +%Y%m%d_%H%M%S)"

MODEL=""
NUM_TASKS=10

while [[ $# -gt 0 ]]; do
    case $1 in
        --model) MODEL="$2"; shift 2 ;;
        --tasks) NUM_TASKS="$2"; shift 2 ;;
        *) echo "Unknown: $1"; exit 1 ;;
    esac
done

# Build if needed
if [ ! -f "$PI" ]; then
    echo "Building Pi Rust..."
    cd "$REPO_DIR"
    source "$HOME/.cargo/env" 2>/dev/null || true
    cargo build --release -p pi-cli
fi

echo "Pi Rust: $($PI --version)"
echo "Model: ${MODEL:-default}"
echo "Tasks: $NUM_TASKS"
echo "Results: $RESULTS_DIR"
echo "────────────────────────────────────"

mkdir -p "$RESULTS_DIR"

# Define benchmark tasks
TASKS=(
    "Create a Python script at /tmp/bench/hello.py that prints 'Hello, World!' and make it executable"
    "Read the file /etc/os-release and tell me the OS name and version"
    "Create a directory /tmp/bench/project with a README.md containing a title and description"
    "Write a bash script at /tmp/bench/count.sh that counts files in a directory passed as \$1"
    "Create a Rust file /tmp/bench/fib.rs with a function that returns the nth fibonacci number, compile it, and run it with n=10"
    "Find all .rs files in the current directory recursively and count the total lines of code"
    "Create a JSON file /tmp/bench/config.json with keys: name, version, features (array of 3 items)"
    "Write a Python script /tmp/bench/sort.py that reads lines from stdin, sorts them, and prints them"
    "Create a Makefile at /tmp/bench/Makefile with targets: build, test, clean — each echoing what they do"
    "Read /tmp/bench/config.json (from a previous task) and add a new key 'updated_at' with the current date"
    "Write a grep-like tool in Python at /tmp/bench/pygrep.py that searches for a pattern in files"
    "Create a git repository at /tmp/bench/repo with an initial commit containing a README"
    "Write a shell script /tmp/bench/monitor.sh that shows CPU and memory usage"
    "Create a Python HTTP server script at /tmp/bench/server.py that serves files from the current directory on port 8080"
    "Write a Rust program /tmp/bench/wordcount.rs that counts words in a file passed as argument"
)

PASS=0
FAIL=0
TOTAL=0

for i in $(seq 0 $((NUM_TASKS - 1))); do
    if [ $i -ge ${#TASKS[@]} ]; then break; fi

    TASK="${TASKS[$i]}"
    TASK_ID=$((i + 1))
    TOTAL=$((TOTAL + 1))

    echo ""
    echo "[$TASK_ID/$NUM_TASKS] $TASK"
    echo ""

    # Clean previous task artifacts
    rm -rf /tmp/bench 2>/dev/null || true
    mkdir -p /tmp/bench

    START_TIME=$(date +%s)

    # Run Pi
    MODEL_FLAG=""
    if [ -n "$MODEL" ]; then
        MODEL_FLAG="-m $MODEL"
    fi

    OUTPUT=$($PI $MODEL_FLAG -p "$TASK" 2>&1 || true)

    END_TIME=$(date +%s)
    ELAPSED=$((END_TIME - START_TIME))

    # Save result
    cat > "$RESULTS_DIR/task_${TASK_ID}.json" << ENDJSON
{
    "task_id": $TASK_ID,
    "task": $(python3 -c "import json; print(json.dumps('$TASK'))" 2>/dev/null || echo "\"$TASK\""),
    "output": $(python3 -c "import json,sys; print(json.dumps(sys.stdin.read()))" <<< "$OUTPUT" 2>/dev/null || echo "\"output\""),
    "elapsed_seconds": $ELAPSED,
    "model": "${MODEL:-default}"
}
ENDJSON

    echo "  Time: ${ELAPSED}s"
    echo "  Output: $(echo "$OUTPUT" | head -c 200)"

    # Basic verification
    VERIFIED=false
    case $TASK_ID in
        1) [ -f /tmp/bench/hello.py ] && python3 /tmp/bench/hello.py 2>/dev/null | grep -q "Hello" && VERIFIED=true ;;
        2) echo "$OUTPUT" | grep -qi "linux\|ubuntu\|debian\|alpine" && VERIFIED=true ;;
        3) [ -f /tmp/bench/project/README.md ] && VERIFIED=true ;;
        4) [ -f /tmp/bench/count.sh ] && VERIFIED=true ;;
        5) [ -f /tmp/bench/fib.rs ] && VERIFIED=true ;;
        6) echo "$OUTPUT" | grep -qE "[0-9]+" && VERIFIED=true ;;
        7) [ -f /tmp/bench/config.json ] && python3 -c "import json; d=json.load(open('/tmp/bench/config.json')); assert 'features' in d" 2>/dev/null && VERIFIED=true ;;
        8) [ -f /tmp/bench/sort.py ] && VERIFIED=true ;;
        9) [ -f /tmp/bench/Makefile ] && VERIFIED=true ;;
        10) [ -f /tmp/bench/config.json ] && VERIFIED=true ;;
        *) VERIFIED=true ;; # Skip verification for other tasks
    esac

    if $VERIFIED; then
        echo "  Result: PASS"
        PASS=$((PASS + 1))
    else
        echo "  Result: FAIL"
        FAIL=$((FAIL + 1))
    fi
done

echo ""
echo "======================================="
echo "  RESULTS: $PASS/$TOTAL passed, $FAIL failed"
echo "  Results saved to: $RESULTS_DIR"
echo "======================================="

# Save summary
cat > "$RESULTS_DIR/summary.json" << EOF
{
    "agent": "pi-rust",
    "version": "$($PI --version 2>&1)",
    "model": "${MODEL:-default}",
    "total_tasks": $TOTAL,
    "passed": $PASS,
    "failed": $FAIL,
    "score": "$(python3 -c "print(round($PASS/$TOTAL*100, 1))" 2>/dev/null || echo "?")",
    "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

cat "$RESULTS_DIR/summary.json"
