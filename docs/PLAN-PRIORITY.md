# Plan: Issue Priority & Goal Ordering

**Problem:** `isq start <id>` assumes you know what to work on. Users and agents need prioritized data to answer "what should I work on next?"

**Solution:** Enrich `isq issue list` and `isq goal list` with priority data. Claude Code reasons about priorities; isq provides the data layer.

**Non-goals:** No `isq next` command. No built-in AI features. isq is infrastructure.

---

## Phase 1: Priority Data for Issues

### 1.1 Schema Change

Add priority column to issues table:

```sql
ALTER TABLE issues ADD COLUMN priority INTEGER DEFAULT 4;
-- 0=urgent, 1=high, 2=medium, 3=low, 4=none
```

### 1.2 Priority Extraction During Sync

**GitHub:** Extract from labels using configurable mapping.

**Linear:** Use native priority field (already 0-4).

Update `forges/github.rs` and `forges/linear.rs` to populate priority during sync.

### 1.3 Config for Label Mapping

In `isq.toml` (repo-level) or `~/.config/isq/config.toml` (global):

```toml
[priority.labels]
urgent = ["priority:urgent", "P0", "critical", "security"]
high = ["priority:high", "P1", "bug"]
medium = ["priority:medium", "P2"]
low = ["priority:low", "P3", "backlog"]
```

Default mapping if no config exists:
- `urgent` / `critical` / `P0` → 0
- `high` / `bug` / `P1` → 1
- `medium` / `P2` → 2
- `low` / `P3` / `backlog` → 3
- No match → 4

### 1.4 Change Default Sort Order

Current: `ORDER BY number DESC`

New: `ORDER BY priority ASC, updated_at DESC`

Add `--sort` flag for explicit control:
```bash
isq issue list                    # Priority first (new default)
isq issue list --sort number      # Old behavior
isq issue list --sort updated     # Most recently updated
isq issue list --sort created     # Oldest first
```

### 1.5 Enrich JSON Output

Current:
```json
{"number": 54, "title": "...", "labels": [...]}
```

New:
```json
{
  "number": 54,
  "title": "...",
  "priority": 0,
  "priority_label": "urgent",
  "goal": "v1.0",
  "labels": [...]
}
```

---

## Phase 2: Goal Priority Ordering

### 2.1 Schema Change

Add priority_order to goals table:

```sql
ALTER TABLE goals ADD COLUMN priority_order INTEGER DEFAULT 0;
```

### 2.2 New Command: `isq goal prioritize`

```bash
isq goal prioritize "v2.0" --position 1     # Make top priority
isq goal prioritize "backlog" --last        # Push to bottom
isq goal prioritize "v1.5" --after "v1.0"   # Relative positioning
```

This is local-only (not synced to forge). Goal priority is a personal/team workflow choice.

### 2.3 Update Goal List Output

Sort by: `priority_order ASC, target_date ASC NULLS LAST`

JSON output includes:
```json
{
  "name": "v2.0",
  "priority_order": 1,
  "progress": 0.45,
  "open_count": 12,
  "target_date": "2025-02-01"
}
```

---

## Phase 3: Filter Issues by Goal

### 3.1 Add `--goal` Flag

```bash
isq issue list --goal "v2.0"           # Issues in v2.0 milestone
isq issue list --goal "v2.0" --json    # For agent consumption
```

Implementation: Filter on `milestone` column (already synced).

### 3.2 Combine with Priority Sort

```bash
isq issue list --goal "v2.0"
# Returns v2.0 issues sorted by priority
```

---

## Implementation Order

| Step | Description | Files |
|------|-------------|-------|
| 1 | Add `priority` column + migration | `db.rs` |
| 2 | Add default label→priority mapping | `config.rs` |
| 3 | Extract priority during GitHub sync | `forges/github.rs` |
| 4 | Extract priority during Linear sync | `forges/linear.rs` |
| 5 | Change default sort to priority-first | `db.rs`, `main.rs` |
| 6 | Add `--sort` flag to issue list | `main.rs` |
| 7 | Add `priority` and `priority_label` to JSON | `forges/mod.rs`, `main.rs` |
| 8 | Add `goal` field to issue JSON (from milestone) | `main.rs` |
| 9 | Add `priority_order` column to goals | `db.rs` |
| 10 | Add `isq goal prioritize` command | `main.rs` |
| 11 | Add `--goal` filter to issue list | `db.rs`, `main.rs` |

---

## Example Workflow

```
User: "What should I work on?"

Claude Code:
  $ isq goal list --json
  $ isq issue list --json

Claude Code: "Your top priority goal is 'v1.0 Stability' (60% complete, due Jan 15).
              The highest priority issue is #54 'isq start panics in sandbox mode' (urgent).
              Should I start it?"

User: "yes"

Claude Code:
  $ isq start 54
```

isq provides the data. Claude provides the reasoning. User stays in control.

---

## Success Criteria

1. `isq issue list` returns issues sorted by priority by default
2. `isq issue list --json` includes `priority`, `priority_label`, and `goal` fields
3. `isq goal list --json` includes `priority_order` field
4. `isq goal prioritize` allows reordering goals locally
5. `isq issue list --goal X` filters to a specific milestone
6. Priority extraction works for both GitHub (labels) and Linear (native)

---

## Future Considerations

- **Assigned-to-me boost:** Could add scoring weight for issues assigned to current user
- **Staleness signal:** Issues not updated in weeks might need attention or closing
- **Cycle/sprint support:** Time-boxed containers like Linear cycles (separate plan)

These are out of scope for this plan. Focus on priority data first.
