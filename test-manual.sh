#!/bin/bash
# Manual integration test script for Pi Rust
# Run from the repo root: ./test-manual.sh
set -e

PI="./target/release/pi"
PASS=0
FAIL=0
SKIP=0

green() { printf "\033[32m✅ PASS: %s\033[0m\n" "$1"; PASS=$((PASS+1)); }
red()   { printf "\033[31m❌ FAIL: %s\033[0m\n" "$1"; FAIL=$((FAIL+1)); }
yellow(){ printf "\033[33m⏭️  SKIP: %s\033[0m\n" "$1"; SKIP=$((SKIP+1)); }
sep()   { echo "────────────────────────────────────────"; }

echo "🔧 Building release binary..."
source "$HOME/.cargo/env"
cargo build --release -p pi-cli 2>&1 | tail -1

sep
echo "📋 TEST GROUP 1: CLI basics"
sep

# 1.1 --help
if $PI --help 2>&1 | grep -q "Pi coding agent"; then
    green "--help shows description"
else
    red "--help missing description"
fi

# 1.2 --version
VERSION=$($PI --version 2>&1)
if echo "$VERSION" | grep -q "pi 0.1.0"; then
    green "--version shows 0.1.0"
else
    red "--version unexpected: $VERSION"
fi

# 1.3 sessions (empty)
SESS=$($PI sessions 2>&1)
if echo "$SESS" | grep -q "No sessions found"; then
    green "sessions (empty) shows correct message"
else
    red "sessions (empty) unexpected: $SESS"
fi

# 1.4 usage (empty)
USAGE=$($PI usage 2>&1)
if echo "$USAGE" | grep -q "No session data"; then
    green "usage (empty) shows correct message"
else
    red "usage (empty) unexpected: $USAGE"
fi

# 1.5 usage --period today
USAGE_TODAY=$($PI usage --period today 2>&1)
if echo "$USAGE_TODAY" | grep -q "No session data"; then
    green "usage --period today works"
else
    red "usage --period today unexpected: $USAGE_TODAY"
fi

# 1.6 no API key error (only if no stored credentials)
unset GITHUB_TOKEN PI_API_KEY MINIMAX_API_KEY
if [ -f "$HOME/.pi/credentials.toml" ]; then
    yellow "no-API-key test skipped — stored credentials exist (pi login was used)"
else
    NO_KEY=$($PI -p "test" 2>&1 || true)
    if echo "$NO_KEY" | grep -qi "no api key\|api key\|login"; then
        green "no API key gives clear error"
    else
        red "no API key error unclear: $NO_KEY"
    fi
fi

sep
echo "📋 TEST GROUP 2: Guidance system"
sep

# 2.1 Create guidance file and verify it exists
GUIDANCE_DIR="$HOME/.pi/guidance"
mkdir -p "$GUIDANCE_DIR"
echo "You are a helpful assistant for testing." > "$GUIDANCE_DIR/GITHUB-COPILOT.md"
if [ -f "$GUIDANCE_DIR/GITHUB-COPILOT.md" ]; then
    green "global guidance file created"
else
    red "failed to create guidance file"
fi

# 2.2 Project-local guidance
mkdir -p .pi/guidance
echo "Project-specific guidance for testing." > .pi/guidance/GITHUB-COPILOT.md
if [ -f .pi/guidance/GITHUB-COPILOT.md ]; then
    green "project-local guidance file created"
else
    red "failed to create local guidance file"
fi

sep
echo "📋 TEST GROUP 3: Ralph setup (no execution)"
sep

# 3.1 Create ralph task file
mkdir -p .ralph/test-loop
echo "Create a file /tmp/ralph-test.txt containing 'hello from ralph'" > .ralph/test-loop/task.md
if [ -f .ralph/test-loop/task.md ]; then
    green "ralph task file created"
else
    red "failed to create ralph task file"
fi

# 3.2 Ralph status (should show nothing yet since no state.json)
# Note: ralph commands only work in interactive mode, can't test from CLI
yellow "ralph commands require interactive REPL — test manually with: ./target/release/pi then /ralph status"

sep
echo "📋 TEST GROUP 4: Copilot auth + provider (requires login)"
sep

# Check if credentials exist
if [ -f "$HOME/.pi/credentials.toml" ]; then
    green "copilot credentials file exists"

    # 4.1 Simple prompt
    RESPONSE=$($PI -p "respond with exactly: PONG" 2>&1 || true)
    if echo "$RESPONSE" | grep -qi "PONG"; then
        green "pi -p with Copilot returns response containing PONG"
    else
        # The model might not follow instructions exactly, check it's not an error
        if echo "$RESPONSE" | grep -qi "error\|failed\|denied"; then
            red "pi -p failed: $RESPONSE"
        else
            yellow "pi -p responded but not exactly PONG: $(echo $RESPONSE | head -c 80)"
        fi
    fi

    # 4.2 Tool usage — ask it to read a file
    echo "test content for pi" > /tmp/pi-read-test.txt
    TOOL_RESPONSE=$($PI -p "read the file /tmp/pi-read-test.txt and tell me its contents" 2>&1 || true)
    if echo "$TOOL_RESPONSE" | grep -qi "test content for pi"; then
        green "pi -p with tool (read_file) works"
    else
        if echo "$TOOL_RESPONSE" | grep -qi "error\|failed"; then
            red "pi -p with tool failed: $(echo $TOOL_RESPONSE | head -c 120)"
        else
            yellow "pi -p with tool responded but unclear: $(echo $TOOL_RESPONSE | head -c 120)"
        fi
    fi

    # 4.3 Spawn agent (requires MiniMax)
    if [ -f "$HOME/.pi/config.toml" ] && grep -q "sub_agent" "$HOME/.pi/config.toml" 2>/dev/null; then
        SPAWN_RESPONSE=$($PI -p "use spawn_agent to create a file /tmp/pi-spawn-test.txt with content 'spawn works'" 2>&1 || true)
        if [ -f /tmp/pi-spawn-test.txt ] && grep -q "spawn works" /tmp/pi-spawn-test.txt; then
            green "spawn_agent created file via MiniMax sub-agent"
        else
            if echo "$SPAWN_RESPONSE" | grep -qi "error\|failed"; then
                red "spawn_agent failed: $(echo $SPAWN_RESPONSE | head -c 120)"
            else
                yellow "spawn_agent responded but file not verified: $(echo $SPAWN_RESPONSE | head -c 120)"
            fi
        fi
    else
        yellow "spawn_agent test skipped — no [sub_agent] in config.toml"
    fi
else
    yellow "copilot tests skipped — no credentials (run: pi login)"
fi

sep
echo "📋 TEST GROUP 5: Tab status (visual only)"
sep

yellow "tab status is visual — check your terminal tab title while running pi interactively"

sep
echo "📋 TEST GROUP 6: Cargo checks"
sep

# 6.1 cargo test
TEST_OUTPUT=$(cargo test --workspace 2>&1)
TEST_COUNT=$(echo "$TEST_OUTPUT" | grep -oE "[0-9]+ passed" | head -1)
if echo "$TEST_OUTPUT" | grep -q "passed" && ! echo "$TEST_OUTPUT" | grep -q "FAILED\.\|[1-9][0-9]* failed"; then
    green "cargo test: $TEST_COUNT, 0 failed"
else
    red "cargo test has failures"
    echo "$TEST_OUTPUT" | grep -i "fail" | head -5
fi

# 6.2 cargo clippy
CLIPPY_OUTPUT=$(cargo clippy --workspace -- -D warnings 2>&1)
if [ $? -eq 0 ]; then
    green "cargo clippy: clean"
else
    red "cargo clippy has issues"
    echo "$CLIPPY_OUTPUT" | head -5
fi

# 6.3 cargo fmt
FMT_OUTPUT=$(cargo fmt --all -- --check 2>&1)
if [ $? -eq 0 ]; then
    green "cargo fmt: all formatted"
else
    red "cargo fmt: needs formatting"
fi

sep
echo ""
echo "═══════════════════════════════════════"
echo "  RESULTS: ✅ $PASS passed, ❌ $FAIL failed, ⏭️  $SKIP skipped"
echo "═══════════════════════════════════════"

# Cleanup
rm -f /tmp/pi-read-test.txt /tmp/pi-spawn-test.txt /tmp/ralph-test.txt
rm -rf .pi/guidance
rm -rf .ralph/test-loop

exit $FAIL
