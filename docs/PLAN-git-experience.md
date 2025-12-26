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
Created worktree ~/src/myapp-891-fix-auth
Checked out branch 891-fix-auth
Issue #891: "Auth timeout on slow connections"
Status: Open → In Progress

$ # ... hack hack hack ...

$ isq comment "Root cause: connection pool exhaustion"
Comment added to #891

$ git commit -m "Fix connection pool sizing"
[891-fix-auth abc123] Fix connection pool sizing [#891]

$ isq pr
Creating PR for #891...
Title: Fix auth timeout on slow connections
✓ PR #456 created, linked to issue #891

$ isq done
Issue #891: In Progress → Closed
PR #456 merged
Worktree cleaned up
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

Add to existing SQLite schema:

```sql
CREATE TABLE worktree_issues (
    git_dir TEXT PRIMARY KEY,      -- /path/.git or /path/.git/worktrees/foo
    repo TEXT NOT NULL,            -- owner/repo
    issue_number INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

The key is the git-dir path, not the worktree path. Worktrees can move; their git identity is stable.

### Forge Actions

Configurable with sensible defaults:

```toml
# ~/.config/isq/config.toml
[forge.github]
on_start = { add_labels = ["in progress"], assign_self = true }
on_done = { remove_labels = ["in progress"], close = true }

[forge.linear]
on_start = { transition = "In Progress" }
on_done = { transition = "Done" }
```

---

## Commands

### `isq current`

Returns current issue for this worktree. Used by hooks and other commands.

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

Branch: 891-fix-auth
Worktree: ~/src/myapp-891-fix-auth
```

### `isq start <id>`

Creates worktree, branch, association. Triggers `on_start` forge actions.

```bash
$ isq start 891
Created worktree ~/src/myapp-891-fix-auth
Checked out branch 891-fix-auth
Issue #891: "Auth timeout on slow connections"
Status: Open → In Progress
```

The worktree is created as a sibling to the current repo:
- If in `~/src/myapp` → creates `~/src/myapp-891-fix-auth`
- Branch name: `891-fix-auth` (issue number + slugified title)

### `isq done`

Triggers `on_done` forge actions. Cleans up worktree.

```bash
$ isq done
Issue #891: In Progress → Closed
Removing worktree ~/src/myapp-891-fix-auth...
Back to ~/src/myapp
```

Options:
- `isq done --keep` - Don't delete the worktree
- `isq done --no-close` - Don't close the issue

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
5. **`isq start <id>`** - Create worktree, branch, association, forge actions
6. **`isq done`** - Forge actions, cleanup
7. **`isq hooks install/uninstall`** - Commit message integration

---

## Related Issues

- #15 - Document jj-inspired design patterns
- #19 - Infer linked repository from git remotes (foundation, exists)
- #20 - Remember current issue per git worktree (this plan)
- #24 - Optional git integration for commits/PRs (hooks section)

---

## Open Questions

1. **Worktree naming**: `myapp-891-fix-auth` or `myapp-worktrees/891-fix-auth`?
2. **Setup hooks**: Should `isq start` support running setup commands (npm install, etc.)?
3. **Multiple issues per worktree**: Error? Warning? Override?

---

## Future

Not in scope for v1, but the foundation enables:

- `isq pr` - Create PR linked to current issue
- `isq switch <id>` - Switch between worktrees, stash current work
- PR-issue bidirectional linking
- Worktree status in `isq list` output
