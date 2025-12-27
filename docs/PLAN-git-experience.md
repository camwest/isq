# Git Experience Plan

The defining feature that makes isq distinct. Your filesystem is your context.

---

## The Insight

jj (Jujutsu) made the working copy mean something—it's a commit, not limbo. isq does the same for issues: **your worktree is your issue**.

| jj | isq |
|-----|------|
| Enhanced local Git interface | Enhanced local issue forge client |
| Working copy is a commit | Current worktree is an issue |
| Conflicts are first-class citizens | Sync conflicts handled gracefully |
| Operation log for safety | Full offline history |

No issue IDs after `isq start`. Your location is your intent.

---

## The Experience

```bash
$ isq start 891
Created worktree ~/src/myapp-891-fix-auth-timeout
Running setup script...
✓ npm install
✓ .env linked
Issue #891: "Auth timeout on slow connections"
Status: Open → In Progress

$ # ... hack hack hack ...

$ isq comment "Root cause: connection pool exhaustion"
Comment added to #891

$ git commit -m "Fix connection pool sizing"
[891-fix-auth-timeout abc123] Fix connection pool sizing [#891]

$ isq pr
Creating PR for #891...
Title: Fix auth timeout on slow connections
✓ PR #456 created, linked to issue #891

$ # PR merged with "Fixes #891" → issue auto-closes

$ isq cleanup
Removing worktree ~/src/myapp-891-fix-auth-timeout...
Back to ~/src/myapp
```

---

## Architecture

### Worktree Identity

Git tracks worktrees in `.git/worktrees/<name>/`. Each has a stable identity via:

```bash
$ git rev-parse --git-dir
/path/to/main/.git/worktrees/my-feature
```

This path is stable even if the worktree directory moves. Use it as the key.

### Storage

Schema designed to support multiple issues per worktree (for future jj-style workflows):

```sql
CREATE TABLE worktree_issues (
    git_dir TEXT NOT NULL,             -- /path/.git or /path/.git/worktrees/foo
    repo TEXT NOT NULL,                -- owner/repo
    issue_number INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (git_dir, issue_number)
);
```

For v1, enforce one issue per worktree in code. The schema allows future expansion.

### Forge Actions

GitHub and Linear have fundamentally different models:

- **GitHub**: No workflow states. Issues are Open or Closed. "In progress" is a label convention.
- **Linear**: First-class workflow states (Backlog → Todo → In Progress → Done → Canceled).

The `on_start` action means "mark this issue as being worked on"—but the mechanism differs:

```toml
# ~/.config/isq/config.toml

[forge.github]
# GitHub has no states, so we use labels + assignment
on_start = { add_labels = ["in progress"], assign_self = true }

[forge.linear]
# Linear has real workflow transitions
on_start = { transition = "In Progress" }
```

Both achieve the same semantic goal through forge-appropriate mechanisms.

---

## Repo Configuration

Per-project settings live in `.config/isq.toml` (following the [.config directory convention](https://github.com/pi0/config-dir)):

```
myapp/
├── .config/
│   └── isq.toml      # project-specific isq config
├── src/
└── ...
```

### Setup Script

Runs automatically when `isq start` creates a new worktree:

```toml
# .config/isq.toml

[worktree]
setup = """
npm install
ln -s "$ISQ_MAIN_WORKTREE/.env" .env
"""
```

**Environment variables available:**
- `$ISQ_MAIN_WORKTREE` - Path to the main worktree (where `.git` lives)
- `$ISQ_ISSUE_NUMBER` - The issue number being started
- `$ISQ_WORKTREE_PATH` - Path to the new worktree

**Why explicit scripts, not magic:**
- Auto-copying `.env` is dangerous (might contain prod credentials)
- Projects have wildly different setup needs
- Transparency: user sees exactly what runs

---

## Commands

### `isq current`

Returns current issue for this worktree. Used by hooks and scripts.

```bash
$ isq current
891

$ isq current --quiet  # for scripts, no output if none
```

### `isq` (no args)

Shows current issue context:

```bash
$ isq
#891: Auth timeout on slow connections
Status: In Progress
Assignee: @camwest
Labels: bug, backend

Last comment (2 hours ago):
  @reviewer: Have you checked the connection pool settings?

Branch: 891-fix-auth-timeout
Worktree: ~/src/myapp-891-fix-auth-timeout
```

### `isq start <id>`

Creates worktree, branch, association. Triggers `on_start` forge actions. Runs setup script.

```bash
$ isq start 891
Created worktree ~/src/myapp-891-fix-auth-timeout
Checked out branch 891-fix-auth-timeout
Running setup script...
✓ npm install (3.2s)
✓ .env linked
Issue #891: "Auth timeout on slow connections"
Status: Open → In Progress
```

**Worktree location:** Sibling to main repo
- Main: `~/src/myapp`
- Worktree: `~/src/myapp-891-fix-auth-timeout`

**Branch naming:** Follows Linear's convention: `{issue-number}-{slugified-title}`
- Example: `891-fix-auth-timeout`

### `isq cleanup`

Removes worktree and clears association. Does NOT close the issue.

```bash
$ isq cleanup
Removing worktree ~/src/myapp-891-fix-auth-timeout...
Back to ~/src/myapp
```

Issue closure happens through normal PR workflows ("Fixes #891" in PR description). The `isq cleanup` command just handles local state.

Options:
- `isq cleanup --keep` - Clear association but keep the worktree directory

---

## Hooks

### Installation

Per-repo, explicit, reversible:

```bash
$ isq hooks install
Installing prepare-commit-msg hook...
✓ Hook installed for camwest/isq

$ isq hooks uninstall
✓ Hook removed
```

### Behavior

The hook appends the issue reference to commit messages:

```bash
$ git commit -m "Fix connection pool sizing"
# becomes: "Fix connection pool sizing [#891]"
```

Skips if:
- No current issue
- Reference already present
- User runs `git commit --no-verify`

### Configuration

```toml
# ~/.config/isq/config.toml
[hooks]
commit_format = "[#%i]"   # or "Fixes #%i" or "%i: "
```

### Implementation

```bash
#!/bin/sh
# .git/hooks/prepare-commit-msg
# Installed by isq - remove with: isq hooks uninstall

ISSUE=$(isq current --quiet 2>/dev/null)

if [ -n "$ISSUE" ] && ! grep -q "#$ISSUE" "$1"; then
    sed -i '' "1s/$/ [#$ISSUE]/" "$1"
fi
```

---

## Implementation Sequence

1. **Worktree identity detection** - `git rev-parse --git-dir` wrapper
2. **Schema migration** - Add `worktree_issues` table
3. **`isq current`** - Query worktree association
4. **`isq` (no args)** - Show current issue context
5. **`isq start <id>`** - Create worktree, branch, association, forge actions, setup script
6. **`isq cleanup`** - Remove worktree, clear association
7. **`isq hooks install/uninstall`** - Commit message integration

---

## Related Issues

- #15 - Document jj-inspired design patterns
- #19 - Infer linked repository from git remotes (foundation, exists)
- #20 - Remember current issue per git worktree (this plan)
- #24 - Optional git integration for commits/PRs (hooks section)

---

## Decisions Made

1. **Worktree naming**: `{issue-number}-{slugified-title}` (Linear's convention)
2. **Config location**: `.config/isq.toml` in repo root (not cluttering root)
3. **Setup scripts**: Explicit user-defined scripts, not magic auto-detection
4. **Issue closure**: Via PR workflow ("Fixes #X"), not `isq done`
5. **Cleanup command**: `isq cleanup` handles local state only
6. **Multi-issue**: Defer to v2, but schema supports it

---

## Future

Not in scope for v1, but the foundation enables:

- `isq pr` - Create PR linked to current issue
- `isq switch <id>` - Switch between worktrees
- Multiple issues per worktree (jj-style simultaneous edits)
- PR-issue bidirectional linking
- Worktree status in `isq list` output
