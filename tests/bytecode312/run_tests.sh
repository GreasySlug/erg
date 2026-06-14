#!/usr/bin/env bash
# Run bytecode312 tests: compile each .er targeting a specific Python version, then execute the .pyc
#
# Usage:
#   ./tests/bytecode312/run_tests.sh [python_command]
#
# Examples:
#   ./tests/bytecode312/run_tests.sh                          # auto-detect 3.12
#   ./tests/bytecode312/run_tests.sh python3.12               # use system python3.12
#   ./tests/bytecode312/run_tests.sh ~/.local/share/uv/python/cpython-3.12.11-linux-x86_64-gnu/bin/python3.12

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Known-failing tests (skip with XFAIL status)
KNOWN_FAIL=""

# Resolve Python command
if [[ $# -ge 1 ]]; then
    PY="$1"
else
    # Try uv-managed 3.12 first, then system python3.12
    UV_PY=$(find ~/.local/share/uv/python -maxdepth 1 -name 'cpython-3.12*' -print -quit 2>/dev/null || true)
    if [[ -n "$UV_PY" && -x "$UV_PY/bin/python3.12" ]]; then
        PY="$UV_PY/bin/python3.12"
    elif command -v python3.12 &>/dev/null; then
        PY="python3.12"
    else
        echo "ERROR: python3.12 not found. Specify a Python command as argument." >&2
        exit 1
    fi
fi

echo "Python: $PY ($($PY --version 2>&1))"
echo "Project: $PROJECT_DIR"
echo ""

# Build first
echo "Building erg..."
cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1 | tail -1
ERG="$PROJECT_DIR/target/debug/erg"
echo ""

passed=0
failed=0
skipped=0
errors=""

for er_file in "$SCRIPT_DIR"/*.er; do
    name="$(basename "$er_file" .er)"
    pyc_file="${er_file%.er}.pyc"

    printf "  %-25s" "$name"

    # Skip known-failing tests
    if echo "$KNOWN_FAIL" | grep -qw "$name"; then
        echo "XFAIL (known)"
        skipped=$((skipped + 1))
        continue
    fi

    # Compile
    if ! compile_out=$("$ERG" --py-command "$PY" --mode compile "$er_file" 2>&1); then
        echo "COMPILE_FAIL"
        errors="$errors\n  $name: compile error\n$compile_out\n"
        failed=$((failed + 1))
        continue
    fi

    # Run
    if ! run_out=$("$PY" "$pyc_file" 2>&1); then
        echo "RUNTIME_FAIL"
        errors="$errors\n  $name: runtime error\n$run_out\n"
        failed=$((failed + 1))
        continue
    fi

    echo "OK"
    passed=$((passed + 1))

    # Clean up .pyc
    rm -f "$pyc_file"
done

echo ""
echo "Results: $passed passed, $failed failed, $skipped xfail (total $((passed + failed + skipped)))"

if [[ -n "$errors" ]]; then
    echo ""
    echo "Failures:"
    echo -e "$errors"
    exit 1
fi
