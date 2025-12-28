# Plan: Issue Priority

**Problem:** `isq start <id>` assumes you know what to work on. Users and agents need prioritized data to answer "what should I work on next?"

**Solution:** Enrich `isq issue list` with priority data. Claude Code reasons about priorities; isq provides the data layer.

**Non-goals:** No `isq next` command. No built-in AI features. No goal priority ordering (Claude can reason from target_date + progress). isq is infrastructure.

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

## Phase 2: Filter Issues by Goal

### 2.1 Add `--goal` Flag

```bash
isq issue list --goal "v2.0"           # Issues in v2.0 milestone
isq issue list --goal "v2.0" --json    # For agent consumption
```

Implementation: Filter on `milestone` column (already synced).

### 2.2 Combine with Priority Sort

```bash
isq issue list --goal "v2.0"
# Returns v2.0 issues sorted by priority
```

---

## Why No Goal Priority Ordering?

Goals already have `target_date` and `progress`. Claude can reason:

```json
{
  "name": "v1.0",
  "target_date": "2025-01-15",
  "progress": 0.60,
  "open_count": 8
}
```

"v1.0 is due in 2 weeks and only 60% complete" — no extra field needed.

If user wants to override ("focus on v2.0 instead"), that's a conversation with Claude, not a database field. Linear needs priority ordering for visual roadmaps; isq is infrastructure for agents that can reason.

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
| 9 | Add `--goal` filter to issue list | `db.rs`, `main.rs` |

---

## Example Workflow

```
User: "What should I work on?"

Claude Code:
  $ isq goal list --json
  $ isq issue list --json

Claude Code: "Your closest deadline is 'v1.0 Stability' (60% complete, due Jan 15).
              The highest priority issue in that goal is #54 'isq start panics
              in sandbox mode' (urgent). Should I start it?"

User: "yes"

Claude Code:
  $ isq start 54
```

isq provides the data. Claude provides the reasoning. User stays in control.

---

## Success Criteria

1. `isq issue list` returns issues sorted by priority by default
2. `isq issue list --json` includes `priority`, `priority_label`, and `goal` fields
3. `isq issue list --goal X` filters to a specific milestone
4. Priority extraction works for both GitHub (labels) and Linear (native)
5. Default label→priority mapping works without config; config allows customization

---

## Future Considerations

- **Assigned-to-me boost:** Could add `--mine` flag or scoring weight
- **Staleness signal:** Issues not updated in weeks might need attention
- **Cycle/sprint support:** Time-boxed containers like Linear cycles (separate plan)

These are out of scope for this plan. Focus on priority data first.
