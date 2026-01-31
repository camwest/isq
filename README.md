# isq

A CLI for GitHub, Linear & JIRA issues. Instant. Offline-first.

## Why

Issue trackers shouldn't own your workflow.

GitHub is great until it isn't—until CI gets unreliable, until pricing changes break your setup, until you want to try Linear or self-host. Switching shouldn't mean relearning everything.

isq keeps your workflow separate from the tracker. Issues live locally. Same commands—GitHub, Linear, or whatever comes next.

## Highlights

- Sub-millisecond reads from local cache
- Works offline, syncs when online
- GitHub + Linear + JIRA Cloud (Forgejo planned)
- Git worktree integration—your directory is your issue
- `--json` on all commands

## Install

```bash
curl -LsSf https://cameronwestland.com/isq/install.sh | sh
```

Or download directly from [GitHub Releases](https://github.com/camwest/isq/releases).

> **Note:** macOS and Linux only. Windows is not supported.

<details>
<summary>Troubleshooting</summary>

**"command not found" after install**

The installer places `isq` in `~/.local/bin` if `/usr/local/bin` isn't writable.
Add it to your PATH:

```bash
# bash
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc

# zsh
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.zshrc
```

**Checksum verification failed**

Re-run the installer. If it persists, download manually from GitHub Releases and verify the checksum.

</details>

<details>
<summary>Verifying releases</summary>

All release artifacts include SHA256 checksums and [GitHub Artifact Attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations) for supply chain security.

**Verify checksum:**
```bash
# After downloading isq-<target>.tar.gz and checksums.txt
grep "isq-<target>.tar.gz" checksums.txt | sha256sum -c
```

**Verify attestation (requires [GitHub CLI](https://cli.github.com/)):**
```bash
gh attestation verify isq-<target>.tar.gz --repo camwest/isq
```

This cryptographically proves the binary was built by our CI from the expected source code.

</details>

## Updating

```bash
isq update check    # Check if update is available
isq update install  # Download and install latest version
```

## Uninstalling

```bash
isq uninstall              # Guided uninstall (stops daemon, removes data)
isq uninstall --keep-config  # Preserve your views and settings
isq uninstall --dry-run      # Preview what would be removed
```

## Agent Skill

Teach your AI coding agent to use isq for issue management:

```bash
npx add-skill camwest/isq
```

Works with Claude Code, Cursor, Codex, Windsurf, and [other agents](https://github.com/vercel-labs/add-skill#available-agents).

## Quick Start

```bash
# Link your repo to GitHub, Linear, or JIRA
isq link github
isq link linear
isq link jira                    # JIRA Cloud (OAuth or API token)
isq link jira -o list-projects   # List available JIRA projects
isq link jira -o project=MYPROJ  # Link to specific project

# List issues (instant, from cache)
isq issue list
isq issue list --label=bug --state=open
isq issue list --mine                    # Assigned to me
isq issue list --id 7,12,45              # Specific issues by ID
isq issue list --sort priority           # Sort by priority (default)

# Hierarchy (when sub-issues exist, shows root issues by default)
isq issue list                           # Root issues only (hides sub-issues)
isq issue list --tree                    # Tree view with indentation
isq issue list --flat                    # Flat list including all sub-issues
isq issue list --children-of 42          # Children of issue #42

# Create, comment, close
isq issue create --title "Fix login bug"
isq issue comment 423 "Fixed in abc123"
isq issue close 423

# Pipe content from files or clipboard
cat description.md | isq issue create --title "Feature request"
pbpaste | isq issue comment 423
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

# Clean up when done (removes "in progress" label)
$ isq cleanup
Cleaned up issue state
Removed worktree ~/src/myapp-891-fix-auth-timeout
Cleared issue #891 association
```

## Commands

| Command | Description |
|---------|-------------|
| `isq link <github\|linear\|jira>` | Link repo to a backend (installs commit hook) |
| `isq unlink` | Remove link (removes commit hook) |
| `isq logout <forge>` | Remove stored credentials |
| `isq status` | Show auth, sync health, and daemon status |
| `isq doctor` | Diagnose common issues (`--check=auth\|repo\|service\|sync\|database\|network\|install`) |
| `isq sync` | Manually sync issues and goals |
| `isq update check` | Check if a newer version is available |
| `isq update install` | Download and install the latest version |
| `isq uninstall` | Remove isq (stops daemon, removes config/cache) |
| `isq start <id>` | Start working: create worktree, branch, mark in progress |
| `isq current` | Show current issue number (for scripts) |
| `isq cleanup` | Remove worktree and clear issue association |
| `isq issue list` | List issues (`--label`, `--state`, `--mine`, `--tree`, `--flat`, `--children-of`) |
| `isq issue show <id>` | Show issue details |
| `isq issue create --title "..."` | Create new issue |
| `isq issue comment <id> "..."` | Add comment |
| `isq issue close <id>` | Close issue |
| `isq issue reopen <id>` | Reopen issue |
| `isq issue label <id> add\|remove <label>` | Manage labels |
| `isq issue assign <id> <user>` | Assign user |
| `isq label list` | List repository labels |
| `isq label create <name>` | Create label (`--color`, `--description`) |
| `isq goal list` | List goals (GitHub milestones / Linear projects) |
| `isq goal show <name>` | Show goal details |
| `isq goal create <name>` | Create new goal |
| `isq goal assign <issue> <goal>` | Assign issue to goal |
| `isq goal close <name>` | Close goal |
| `isq view create <name>` | Create a saved view (filter combination) |
| `isq view list` | List all views |
| `isq view delete <name>` | Delete a view |
| `isq issue list @<view>` | Use a view in issue list |

Add `--json` to any command for machine-readable output.

## Views (Saved Filters)

Create named filter combinations to avoid typing the same flags repeatedly:

```bash
# Create views
isq view create my-bugs --label=bug --state=open --mine
isq view create stale --unassigned --updated-before="30 days"
isq view create epics --root-only                  # Only parent issues
isq view create subtasks --children-of=42          # Children of issue #42
isq view create hierarchy --tree                   # Tree display mode

# Use views with @ prefix
isq issue list @my-bugs
isq issue list @epics                              # Shows only root issues
isq issue list @stale --json   # CLI flags can override view settings
```

Views are stored in `~/.config/isq/config.toml` and work across all repositories.

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
                                    ┌───────────────────────┐
                                    │ GitHub / Linear / JIRA│
                                    └───────────────────────┘
```

1. **Daemon** syncs issues from GitHub/Linear/JIRA to local SQLite cache
2. **CLI** reads from cache (instant) and writes directly to API
3. **Offline writes** queue locally, sync when back online

## Configuration

isq auto-detects your repo from git remotes. Cache lives at:
- macOS: `~/Library/Caches/isq/`
- Linux: `~/.cache/isq/`

### Per-repo settings (`.config/isq.toml`):

```toml
[worktree]
setup = """
npm install
ln -s "$ISQ_MAIN_WORKTREE/.env" .env
"""

[on_start]
add_labels = ["in progress"]  # GitHub
assign_self = true
# transition = "started"        # Linear: use workflow state
# transition = "In Progress"    # JIRA: use workflow transition

[on_cleanup]
remove_labels = ["in progress"]  # GitHub: remove labels on cleanup
# transition = "backlog"         # Linear/JIRA: transition back to state

# Priority mapping (GitHub only - Linear and JIRA have native priority)
[priority]
P0 = 0  # urgent
P1 = 1  # high
bug = 1 # treat bugs as high priority
P2 = 2  # medium
P3 = 3  # low
```

### User settings (`~/.config/isq/config.toml`):

```toml
[defaults]
json = true   # All commands output JSON by default

[views.my-bugs]
label = "bug"
state = "open"
mine = true

[views.stale]
unassigned = true
updated_before = "30 days"

[views.epics]
root_only = true              # Only show parent issues (no sub-issues)
state = "open"

[views.all-flat]
flat = true                   # Show all issues including sub-issues
```

## License

MIT
