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

## Commands

| Command | Description |
|---------|-------------|
| `isq link <github\|linear>` | Link current repo to a backend |
| `isq unlink` | Remove link from current repo |
| `isq status` | Show auth and sync status |
| `isq sync` | Manually sync issues |
| `isq issue list` | List issues (filters: `--label`, `--state`) |
| `isq issue show <id>` | Show issue details |
| `isq issue create --title "..."` | Create new issue |
| `isq issue comment <id> "..."` | Add comment |
| `isq issue close <id>` | Close issue |
| `isq issue reopen <id>` | Reopen issue |
| `isq issue label <id> add\|remove <label>` | Manage labels |
| `isq issue assign <id> <user>` | Assign user |

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

## License

MIT
