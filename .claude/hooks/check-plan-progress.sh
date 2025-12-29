#!/bin/bash
# Stop hook that uses Codex CLI to evaluate PLAN-PRIORITY.md progress

PLAN_FILE="docs/PLAN-PRIORITY.md"
PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"

# Helper to output JSON response
output_allow() {
  echo '{"ok": true}'
  exit 0
}

output_block() {
  local reason="$1"
  # Escape quotes in reason for JSON
  reason=$(echo "$reason" | sed 's/"/\\"/g')
  echo "{\"ok\": false, \"reason\": \"$reason\"}"
  exit 0
}

# Check if plan file exists
if [ ! -f "$PROJECT_DIR/$PLAN_FILE" ]; then
  output_allow
fi

# Create temp files
PROMPT_FILE=$(mktemp)
OUTPUT_FILE=$(mktemp)
trap "rm -f $PROMPT_FILE $OUTPUT_FILE" EXIT

# Write the prompt to temp file
cat > "$PROMPT_FILE" << 'PROMPT_HEADER'
You are evaluating implementation progress for a Rust CLI project called isq.

## Your Task

Check the current directory for implementation progress on the plan below. For each step in the Implementation Order (1-20), verify if it's implemented by checking the actual source files.

Key files to check:
- src/db.rs - database schema (assignees, priority columns)
- src/forges/github.rs - GitHub API client
- src/forges/linear.rs - Linear API client
- src/forges/mod.rs - common Issue struct
- src/cli/*.rs - CLI commands
- src/config.rs - configuration parsing

## Response

Your FINAL message must be EXACTLY in this format:

DECISION: approve
REASON: All 20 steps complete

OR

DECISION: block
REASON: X/20 complete. Next: [specific task description]

Only say "approve" if ALL steps are verified complete.

## The Plan

PROMPT_HEADER

# Append the plan content
cat "$PROJECT_DIR/$PLAN_FILE" >> "$PROMPT_FILE"

# Read the prompt
PROMPT=$(cat "$PROMPT_FILE")

# Call codex exec and capture output
cd "$PROJECT_DIR"
codex exec \
  -s danger-full-access \
  -c model_reasoning_effort="high" \
  -o "$OUTPUT_FILE" \
  "$PROMPT" >/dev/null 2>&1

if [ $? -ne 0 ]; then
  output_allow
fi

# Read the response
RESPONSE=$(cat "$OUTPUT_FILE")

# Extract decision and reason from response
DECISION=$(echo "$RESPONSE" | grep -i "^DECISION:" | tail -1 | sed 's/^DECISION:[[:space:]]*//' | tr '[:upper:]' '[:lower:]')
REASON=$(echo "$RESPONSE" | grep -i "^REASON:" | tail -1 | sed 's/^REASON:[[:space:]]*//')

if [ "$DECISION" = "block" ]; then
  output_block "${REASON:-Continue working on the plan}"
fi

output_allow
