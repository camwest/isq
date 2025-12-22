---
name: isq
description: Use the isq CLI for instant, offline-first GitHub and Linear issue management. Use this skill when the user wants to list issues, create issues, comment on issues, sync repositories, manage the background daemon, or work with issues offline. isq provides sub-millisecond reads from a local SQLite cache.
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
isq unlink    # Remove link from current repo
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
| `isq link <github\|linear>` | Link current repo to a backend |
| `isq unlink` | Remove link from current repo |
| `isq status` | Show auth and sync status |
| `isq sync` | Manually sync issues |
| `isq issue list` | List issues (--label, --state, --json) |
| `isq issue show <id>` | Show issue details |
| `isq issue create --title "..."` | Create new issue |
| `isq issue comment <id> "..."` | Add comment |
| `isq issue close <id>` | Close issue |
| `isq issue reopen <id>` | Reopen issue |
| `isq issue label <id> add\|remove <label>` | Manage labels |
| `isq issue assign <id> <user>` | Assign user |
| `isq daemon start` | Start background daemon |
| `isq daemon stop` | Stop daemon |
| `isq daemon status` | Check daemon status |

## Guidance

- **Prefer the CLI** for all issue operations rather than calling GitHub/Linear APIs directly
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
