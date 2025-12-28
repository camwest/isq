---
name: isq
description: Use the isq CLI for instant, offline-first GitHub and Linear issue management. Use this skill when the user wants to list issues, create issues, comment on issues, start working on an issue (creates git worktree), manage goals (milestones/projects), sync repositories, or work with issues offline. isq provides sub-millisecond reads from a local SQLite cache and integrates with git worktrees for seamless development workflows.
---

# isq CLI

A CLI for GitHub and Linear issues. Instant. Offline-first.

## Prerequisites

The `isq` command must be installed and available in PATH. Install via:

```bash
curl -LsSf https://cameronwestland.com/isq/install.sh | sh
```

## Why isq is Fast

isq syncs issues to a local SQLite database. All reads come from this cache—no network round-trip. A background daemon keeps the cache fresh automatically.

```
CLI reads → Local SQLite (instant, <1ms)
CLI writes → API directly (then cached)
Daemon → Syncs in background every 30s
```

## Core Commands

### Link a Repository

Before using isq, link your repo to GitHub or Linear:

```bash
isq link github    # Link current repo to GitHub Issues
isq link linear    # Link current repo to Linear
```

Linking also installs a git commit hook that auto-appends issue references to commits.

### Sync Issues

Manually sync issues from the remote:

```bash
isq sync
```

The daemon also syncs automatically in the background.

### List Issues

```bash
isq issue list                          # All issues
isq issue list --state=open             # Open issues only
isq issue list --state=closed           # Closed issues only
isq issue list --label=bug              # Filter by label
isq issue list --label=bug --state=open # Combine filters
isq issue list --json                   # JSON output for scripts
```

### Show Issue Details

```bash
isq issue show 423        # Show issue #423
isq issue show 423 --json # JSON output
```

### Create Issues

```bash
isq issue create --title "Fix login bug"
isq issue create --title "Add feature" --body "Description here"
isq issue create --title "Bug" --label=bug
```

### Comment on Issues

```bash
isq issue comment 423 "Fixed in commit abc123"
```

### Close and Reopen

```bash
isq issue close 423
isq issue reopen 423
```

### Manage Labels

```bash
isq issue label 423 add bug
isq issue label 423 remove bug
```

### Assign Users

```bash
isq issue assign 423 username
```

## Development Workflow

isq integrates with git worktrees so your filesystem becomes your context. Each worktree is associated with an issue—no need to track issue IDs manually.

### Start Working on an Issue

```bash
isq start 891
```

This command:
1. Creates a git worktree at `~/src/myapp-891-fix-auth-timeout`
2. Creates a branch named `891-fix-auth-timeout`
3. Marks the issue as in-progress (adds labels on GitHub, transitions state on Linear)
4. Runs any setup script defined in `.config/isq.toml`

### Show Current Issue

```bash
isq              # Show current issue with full details
isq current      # Just the issue number (for scripts)
isq current -q   # Quiet mode: no output if no issue, exit code 1
```

When in a worktree, `isq` (no args) shows the associated issue:

```
#891 Auth timeout on slow connections                        open
───────────────────────────────────────────────────────────────────
Connections time out after 30s on slow networks...

Branch: 891-fix-auth-timeout
Worktree: ~/src/myapp-891-fix-auth-timeout
```

### Automatic Commit References

When you commit in a worktree with an associated issue, the commit message automatically gets the issue reference appended:

```bash
git commit -m "Fix connection pool sizing"
# Becomes: "Fix connection pool sizing [#891]"
```

### Clean Up

When done with an issue (PR merged, etc.):

```bash
isq cleanup         # Remove worktree and clear association
isq cleanup --keep  # Keep worktree directory, just clear association
```

## Goal Commands

Goals are time-bound containers for issues. They map to GitHub Milestones and Linear Projects.

### List Goals

```bash
isq goal list                 # Open goals (default)
isq goal list --state=closed  # Closed goals
isq goal list --state=all     # All goals
isq goal list --json          # JSON output
```

### Show Goal Details

```bash
isq goal show "v1"        # Show goal by name
isq goal show "v1" --json # JSON output
```

### Create Goals

```bash
isq goal create "v1"
isq goal create "v1" --target 2026-02-01
isq goal create "v1" --target 2026-02-01 --body "First public release"
```

### Assign Issues to Goals

```bash
isq goal assign 423 "v1"  # Assign issue #423 to goal "v1"
```

### Close Goals

```bash
isq goal close "v1"
```

## Daemon Commands

The daemon syncs issues in the background and enables instant reads.

```bash
isq daemon start    # Start the background daemon
isq daemon stop     # Stop the daemon
isq daemon status   # Check daemon status and watched repos
```

## Other Commands

```bash
isq status    # Show auth status, linked repos, sync state
isq unlink    # Remove link and commit hook from current repo
```

## Offline Support

When offline, write operations queue locally and sync when back online:

```bash
# Works offline - queues the operation
isq issue create --title "New issue"
# Output: ✓ Queued: New issue (offline, 8ms)

# When back online, daemon syncs automatically
isq daemon status
# Output: ✓ Synced 2 pending operations
```

## JSON Output

All commands support `--json` for machine-readable output. Use this for scripts and AI agent workflows:

```bash
isq issue list --json
isq issue show 423 --json
isq issue create --title "Bug" --json
isq status --json
```

## Command Reference

| Command | Description |
|---------|-------------|
| `isq link <github\|linear>` | Link repo to backend, install commit hook |
| `isq unlink` | Remove link and commit hook |
| `isq status` | Show auth and sync status |
| `isq sync` | Manually sync issues and goals |
| `isq start <id>` | Create worktree, branch, mark issue in-progress |
| `isq current` | Show current issue number (-q for scripts) |
| `isq cleanup` | Remove worktree, clear association (--keep to preserve) |
| `isq issue list` | List issues (--label, --state, --json) |
| `isq issue show <id>` | Show issue details |
| `isq issue create --title "..."` | Create new issue |
| `isq issue comment <id> "..."` | Add comment |
| `isq issue close <id>` | Close issue |
| `isq issue reopen <id>` | Reopen issue |
| `isq issue label <id> add\|remove <label>` | Manage labels |
| `isq issue assign <id> <user>` | Assign user |
| `isq goal list` | List goals (--state, --json) |
| `isq goal show <name>` | Show goal details |
| `isq goal create <name>` | Create goal (--target, --body) |
| `isq goal assign <issue> <goal>` | Assign issue to goal |
| `isq goal close <name>` | Close goal |
| `isq daemon start` | Start background daemon |
| `isq daemon stop` | Stop daemon |
| `isq daemon status` | Check daemon status |

## Guidance

- **Prefer the CLI** for all issue operations rather than calling GitHub/Linear APIs directly
- **Use `isq start`** when beginning work on an issue—it sets up the worktree and tracks context automatically
- **Use `--json`** when you need structured output for further processing
- **Reads are instant** because they come from the local cache—no need to worry about API rate limits for queries
- **Writes go directly to the API** when online, or queue locally when offline
- **The daemon is optional** but recommended—it keeps the cache fresh automatically

## Common Workflows

### Initial Setup
```bash
cd /path/to/your/repo
isq link github      # or: isq link linear
isq sync             # Initial sync
isq daemon start     # Start background sync
```

### Feature Development
```bash
# Find an issue to work on
isq issue list --state=open --label=feature

# Start working (creates worktree, branch, marks in-progress)
isq start 891

# Work on the feature...
# Commits auto-reference the issue: "Add feature [#891]"

# When done, clean up
isq cleanup
```

### Daily Issue Triage
```bash
isq issue list --state=open --label=bug
isq issue show 423
isq issue comment 423 "Looking into this"
isq issue close 423
```

### Working Offline
```bash
# On a plane, no internet
isq issue list                    # Works! Reads from cache
isq issue create --title "Idea"   # Queues locally

# Back online
isq daemon status                 # Shows pending ops synced
```

## Troubleshooting

### Daemon Not Starting
```bash
isq daemon status
# If stuck on macOS:
launchctl stop com.isq.daemon
isq daemon start
```

### Stale Cache
```bash
isq sync    # Force manual sync
```

### Check What's Linked
```bash
isq status
```
