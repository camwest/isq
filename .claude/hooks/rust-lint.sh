#!/bin/bash
# Claude Code hook: Run Rust linting after file edits

set -euo pipefail

file_path=$(cat | jq -r '.tool_input.file_path // empty')

[[ "$file_path" =~ \.rs$ ]] || exit 0

cd "$CLAUDE_PROJECT_DIR"

echo "Running cargo clippy..."
if ! cargo clippy --all-targets -- -D warnings 2>&1; then
    echo
    echo "clippy failed - fix the warnings above"
    exit 2
fi

echo
echo "Running cargo fmt --check..."
if ! cargo fmt --check 2>&1; then
    echo
    echo "fmt failed - run 'cargo fmt' to fix formatting"
    exit 2
fi

echo
echo "Rust linting passed"
