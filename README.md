# isq

A CLI for GitHub & Linear issues. Instant. Offline-first.

## Why

Issue trackers shouldn't own your workflow.

GitHub is great until it isn't—until CI gets unreliable, until pricing changes break your setup, until you want to try Linear or self-host. Switching shouldn't mean relearning everything.

isq keeps your workflow separate from the tracker. Issues live locally. Same commands—GitHub, Linear, or whatever comes next.

## Highlights

- Sub-millisecond reads from local cache
- Works offline, syncs when online
- GitHub + Linear (Forgejo planned)
- Git worktree integration—your directory is your issue
- `--json` on all commands

## Install

```bash
curl -LsSf https://cameronwestland.com/isq/install.sh | sh
```

Or download directly from [GitHub Releases](https://github.com/camwest/isq/releases).

## Quick Start

```bash
# Link your repo to GitHub or Linear
isq link github
isq link linear

# List issues (instant, from cache)
isq issue list
isq issue list --label=bug --state=open

# Create, comment, close
isq issue create --title "Fix login bug"
isq issue comment 423 "Fixed in abc123"
isq issue close 423
```

## Development Workflow

isq integrates with git worktrees so your filesystem becomes your context. No more juggling issue IDs.

```bash
# Start working on an issue (creates worktree + branch)
$ isq start 891
Created worktree ~/src/myapp-891-fix-auth-timeout
Branch: 891-fix-auth-timeout
Running setup... done (2.1s)
Marked in progress
Issue #891: "Auth timeout on slow connections"

# Your current directory knows the issue
$ isq
#891 Auth timeout on slow connections                        open
───────────────────────────────────────────────────────────────────
Connections time out after 30s on slow networks...

Branch: 891-fix-auth-timeout
Worktree: ~/src/myapp-891-fix-auth-timeout

# Commits auto-reference the issue
$ git commit -m "Fix connection pool sizing"
[891-fix-auth-timeout abc123] Fix connection pool sizing [#891]

# Clean up when done
$ isq cleanup
Removed worktree ~/src/myapp-891-fix-auth-timeout
Cleared issue #891 association
```

## Commands

| Command | Description |
|---------|-------------|
| `isq link <github\|linear>` | Link repo to a backend (installs commit hook) |
| `isq unlink` | Remove link (removes commit hook) |
| `isq status` | Show auth and sync status |
| `isq sync` | Manually sync issues and goals |
| `isq start <id>` | Start working: create worktree, branch, mark in progress |
| `isq current` | Show current issue number (for scripts) |
| `isq cleanup` | Remove worktree and clear issue association |
| `isq issue list` | List issues (filters: `--label`, `--state`) |
| `isq issue show <id>` | Show issue details |
| `isq issue create --title "..."` | Create new issue |
| `isq issue comment <id> "..."` | Add comment |
| `isq issue close <id>` | Close issue |
| `isq issue reopen <id>` | Reopen issue |
| `isq issue label <id> add\|remove <label>` | Manage labels |
| `isq issue assign <id> <user>` | Assign user |
| `isq goal list` | List goals (GitHub milestones / Linear projects) |
| `isq goal show <name>` | Show goal details |
| `isq goal create <name>` | Create new goal |
| `isq goal assign <issue> <goal>` | Assign issue to goal |
| `isq goal close <name>` | Close goal |

Add `--json` to any command for machine-readable output.

## How It Works

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   isq CLI   │────▶│ SQLite Cache│◀────│   Daemon    │
└─────────────┘     └─────────────┘     └─────────────┘
                           ▲                   │
                           │                   ▼
                    instant reads      background sync
                                              │
                                              ▼
                                    ┌─────────────────┐
                                    │ GitHub / Linear │
                                    └─────────────────┘
```

1. **Daemon** syncs issues from GitHub/Linear to local SQLite cache
2. **CLI** reads from cache (instant) and writes directly to API
3. **Offline writes** queue locally, sync when back online

## Configuration

isq auto-detects your repo from git remotes. Cache lives at:
- macOS: `~/Library/Caches/isq/`
- Linux: `~/.cache/isq/`

Per-repo settings live in `.config/isq.toml`:

```toml
[worktree]
setup = """
npm install
ln -s "$ISQ_MAIN_WORKTREE/.env" .env
"""

[on_start]
add_labels = ["in progress"]  # GitHub
assign_self = true
# transition = "started"      # Linear: use workflow state instead
```

## License

MIT
