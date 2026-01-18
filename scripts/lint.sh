#!/bin/bash
# Rust linting script - used by Claude Code hooks and CI
# Usage:
#   As hook: Reads JSON from stdin, skips non-.rs files
#   As CI:   .claude/hooks/rust-lint.sh --ci

set -euo pipefail

# CI mode skips the stdin JSON parsing
if [ "${1:-}" != "--ci" ]; then
    # Hook mode: check if the edited file is a .rs file
    file_path=$(cat | jq -r '.tool_input.file_path // empty')
    [[ "$file_path" =~ \.rs$ ]] || exit 0
fi

# Change to project directory (set by Claude Code or use current dir)
cd "${CLAUDE_PROJECT_DIR:-.}"

echo "Running cargo clippy..."
if ! cargo clippy --all-targets --all-features -- -D warnings 2>&1; then
    echo
    echo "clippy failed - fix the warnings above"
    exit 2
fi

echo
echo "Running cargo fmt --check..."
if ! cargo fmt --all -- --check 2>&1; then
    echo
    echo "fmt failed - run 'cargo fmt' to fix formatting"
    exit 2
fi

echo
echo "Checking file sizes..."
MAX_LINES=500
FAILED=0
for file in $(find src -name "*.rs" 2>/dev/null || true); do
    lines=$(wc -l < "$file")
    if [ "$lines" -gt "$MAX_LINES" ]; then
        echo "ERROR: $file has $lines lines (max: $MAX_LINES)"
        FAILED=1
    fi
done
if [ "$FAILED" -eq 1 ]; then
    echo
    echo "Files above $MAX_LINES lines must be split into smaller modules."
    exit 2
fi

echo
echo "Rust linting passed"
