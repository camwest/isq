# Plan: Issue Priority & Assignment

**Problem:** `isq start <id>` assumes you know what to work on. Users and agents need data to answer "what should I work on next?"

**Solution:** Enrich `isq issue list` with assignee and priority data, add filters for common workflows. Claude Code reasons about the data; isq provides the infrastructure.

**Non-goals:** No `isq next` command. No built-in AI features. isq is infrastructure.

---

## How Teams Find Work

Two dominant patterns:

**Pre-assigned:** Issues assigned during sprint planning. Developer asks "what's mine?" and picks highest priority from their list.

**Pull model:** Sprint backlog exists with unassigned issues. Developer picks highest priority they can do, assigns to self, starts.

Both need:
- Assignee data (who owns what, what's available)
- Priority data (what's most important)
- Filters (my issues, sprint backlog, unassigned)

---

## Phase 1: Assignee Data

### 1.1 Schema Change

```sql
ALTER TABLE issues ADD COLUMN assignees TEXT DEFAULT '[]';
-- JSON array of usernames: ["camwest", "alice"]
```

### 1.2 Sync Assignees

**GitHub:** Already in API response, just not deserialized.

```rust
// forges/github.rs
struct GitHubIssue {
    // ... existing fields
    assignees: Vec<GitHubUser>,  // ADD THIS
}
```

**Linear:** Map from `assignee` field (Linear issues have single assignee).

### 1.3 Expose in JSON Output

```json
{
  "number": 54,
  "title": "Fix auth bug",
  "assignees": ["camwest"],
  "state": "open",
  ...
}
```

### 1.4 Add `--mine` Filter

```bash
isq issue list --mine              # Issues assigned to me
isq issue list --mine --json       # For agent consumption
```

Implementation: Filter where `assignees` contains current username (from repo_links.username).

### 1.5 Add `--unassigned` Filter

```bash
isq issue list --unassigned        # Issues with no assignee
```

Implementation: Filter where `assignees = '[]'`.

---

## Phase 2: Priority Data

### 2.1 Schema Change

```sql
ALTER TABLE issues ADD COLUMN priority INTEGER DEFAULT 4;
-- 0=urgent, 1=high, 2=medium, 3=low, 4=none
```

### 2.2 Priority Extraction During Sync

**GitHub:** Extract from labels using configurable mapping.

**Linear:** Use native priority field (already 0-4).

### 2.3 Config for Label Mapping

In `isq.toml` (repo-level) or `~/.config/isq/config.toml` (global):

```toml
[priority.labels]
urgent = ["priority:urgent", "P0", "critical", "security"]
high = ["priority:high", "P1", "bug"]
medium = ["priority:medium", "P2"]
low = ["priority:low", "P3", "backlog"]
```

Default mapping if no config:
- `urgent` / `critical` / `P0` → 0
- `high` / `bug` / `P1` → 1
- `medium` / `P2` → 2
- `low` / `P3` / `backlog` → 3
- No match → 4

### 2.4 Change Default Sort Order

Current: `ORDER BY number DESC`

New: `ORDER BY priority ASC, updated_at DESC`

### 2.5 Add `--sort` Flag

```bash
isq issue list                    # Priority first (new default)
isq issue list --sort number      # Old behavior
isq issue list --sort updated     # Most recently updated
isq issue list --sort created     # Oldest first
```

### 2.6 Expose Priority in JSON

```json
{
  "number": 54,
  "priority": 0,
  "priority_label": "urgent",
  ...
}
```

---

## Phase 3: Goal Filter

### 3.1 Add `--goal` Flag

```bash
isq issue list --goal "Sprint 5"           # Issues in milestone
isq issue list --goal "v1.0" --json        # For agent consumption
```

Implementation: Filter on `milestone` column (already synced).

### 3.2 Combine Filters

All filters composable:

```bash
isq issue list --goal "Sprint 5" --unassigned    # Sprint backlog, available
isq issue list --goal "Sprint 5" --mine          # My sprint work
isq issue list --mine --label bug                # My bugs
```

---

## Example Workflows

### Pre-assigned: "What's my top priority?"

```
User: "What should I work on?"

Claude:
  $ isq issue list --mine --json

Claude: "You have 4 assigned issues. Top priority is #54 'Fix auth bug' (urgent).
         Should I start it?"

User: "yes"

Claude:
  $ isq start 54
```

### Pull model: "What can I pick up?"

```
User: "What's available in the current sprint?"

Claude:
  $ isq issue list --goal "Sprint 5" --unassigned --json

Claude: "3 unassigned issues in Sprint 5. Highest priority is #42 'Add rate limiting' (high).
         Want me to assign it to you and start?"

User: "yes"

Claude:
  $ isq issue assign 42 camwest
  $ isq start 42
```

### Planning: "What's left in this milestone?"

```
User: "How's v1.0 looking?"

Claude:
  $ isq goal show v1.0 --json
  $ isq issue list --goal "v1.0" --json

Claude: "v1.0 is 60% complete (8 open, 12 closed). 3 urgent issues remain.
         Top blocker is #54 'Fix auth bug' assigned to you."
```

---

## Phase 4: Update Claude Code Skill

The skill at `skills/isq/SKILL.md` teaches Claude how to use isq. It needs to document the "what should I work on?" workflows so Claude knows how to help.

### 4.1 Add "Finding Work" Section

Document the two patterns and when to use each:

```markdown
## Finding What to Work On

When users ask "what should I work on?" or similar, use these patterns:

### Pre-assigned Teams
If issues are assigned during sprint planning:
\`\`\`bash
isq issue list --mine --json
\`\`\`
Recommend the highest priority assigned issue.

### Pull-from-Backlog Teams
If the team pulls from a shared backlog:
\`\`\`bash
isq issue list --goal "Sprint 5" --unassigned --json
\`\`\`
Recommend the highest priority unassigned issue, then assign and start.

### Not Sure Which Model?
Ask: "Does your team pre-assign issues, or do you pull from a shared backlog?"
```

### 4.2 Add Priority Interpretation Guidance

```markdown
## Understanding Priority

Issues have priority levels (shown in JSON output):
- `0` / `urgent` — Drop everything, fix now
- `1` / `high` — Important, do soon
- `2` / `medium` — Normal priority
- `3` / `low` — Nice to have
- `4` / `none` — No priority set

When recommending work, always suggest highest priority first.
Issues are sorted by priority by default.
```

### 4.3 Add Workflow Examples

Add these to the "Common Workflows" section:

```markdown
### What Should I Work On? (Pre-assigned)
\`\`\`bash
isq issue list --mine --json
# Look at top result (highest priority)
# Recommend it to user
isq start <id>
\`\`\`

### What Should I Work On? (Pull Model)
\`\`\`bash
isq issue list --goal "Current Sprint" --unassigned --json
# Look at top result
# Recommend it, then:
isq issue assign <id> <username>
isq start <id>
\`\`\`

### What's Blocking the Milestone?
\`\`\`bash
isq goal show "v1.0" --json
isq issue list --goal "v1.0" --json
# Report on progress, highlight urgent/high priority blockers
\`\`\`
```

### 4.4 Update Command Reference

Add new flags to the command reference table:

| Command | Description |
|---------|-------------|
| `isq issue list --mine` | Show only issues assigned to me |
| `isq issue list --unassigned` | Show only unassigned issues |
| `isq issue list --goal "X"` | Filter to issues in goal/milestone X |
| `isq issue list --sort priority` | Sort by priority (default) |
| `isq issue list --sort number` | Sort by issue number |

---

## Implementation Order (Updated)

| Step | Description | Files |
|------|-------------|-------|
| 1 | Add `assignees` column + migration | `db.rs` |
| 2 | Deserialize assignees in GitHub sync | `forges/github.rs` |
| 3 | Deserialize assignee in Linear sync | `forges/linear.rs` |
| 4 | Add `assignees` to Issue struct | `forges/mod.rs` |
| 5 | Add `--mine` filter | `db.rs`, `main.rs` |
| 6 | Add `--unassigned` filter | `db.rs`, `main.rs` |
| 7 | Add `priority` column + migration | `db.rs` |
| 8 | Extract priority from labels (GitHub) | `forges/github.rs` |
| 9 | Map priority field (Linear) | `forges/linear.rs` |
| 10 | Add priority config parsing | `config.rs` |
| 11 | Change default sort to priority-first | `db.rs` |
| 12 | Add `--sort` flag | `main.rs` |
| 13 | Add `priority` + `priority_label` to JSON | `forges/mod.rs`, `main.rs` |
| 14 | Add `--goal` filter | `db.rs`, `main.rs` |
| 15 | Update skill: add "Finding Work" section | `skills/isq/SKILL.md` |
| 16 | Update skill: add priority guidance | `skills/isq/SKILL.md` |
| 17 | Update skill: add workflow examples | `skills/isq/SKILL.md` |
| 18 | Update skill: update command reference | `skills/isq/SKILL.md` |

---

## Success Criteria (Updated)

1. `isq issue list --json` includes `assignees` array
2. `isq issue list --mine` filters to current user's issues
3. `isq issue list --unassigned` filters to issues with no assignee
4. `isq issue list` returns issues sorted by priority by default
5. `isq issue list --json` includes `priority` and `priority_label`
6. `isq issue list --goal X` filters to a specific milestone
7. Filters are composable (`--mine --goal X`, etc.)
8. **Skill documents "Finding Work" workflows**
9. **Skill explains priority levels and interpretation**
10. **Skill shows both pre-assigned and pull-model patterns**

---

## Future Considerations

- **Cycle/sprint as first-class concept:** Time-boxed containers beyond milestones
- **Workload balancing:** Show who has capacity
- **Skills matching:** Labels indicating required expertise

Out of scope for this plan.
