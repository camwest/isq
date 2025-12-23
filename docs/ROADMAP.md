# Roadmap

6-week release cycles. Each release ships complete. Course is charted; exact steps refined as we climb.

---

## R1: Production-Ready + Git Integration

**Problem**: isq isn't reliable enough for daily use, doesn't understand dev context

**Scope**:
- Bug fixes, error handling, daemon reliability
- Git context: detect worktree/branch → infer current issue
- `isq` with no args shows context
- `isq start <id>` / `isq done <id>` lifecycle
- GitHub releases, install script, docs

**Exit criteria**: You use isq daily without hitting bugs.

---

## R2: Multi-Repo & Personal State

**Problem**: I work across repos, no unified view. I lose track of what I'm working on.

**Scope**:
- Multi-repo unified view
- Persistent "active issues" state across sessions
- "What am I working on?" answered instantly

---

## R3: Universal Forge

**Problem**: Only works with GitHub/Linear. Doesn't fulfill "any backend" promise.

**Scope**:
- Forgejo backend
- GitLab backend
- Prove the abstraction scales

**Impact**: Your workflow survives platform migrations.

---

## R4: Triage at Scale

**Problem**: Open source maintainers drowning in issues. Can't process the queue.

**Scope**:
- High-volume triage workflows
- Bulk operations
- Smart filtering / saved views

---

## R5: Team Visibility

**Problem**: I can see my work, but not my team's.

**Scope**:
- Shared views
- Team coordination features

---

## R6: PR Integration

**Problem**: Issues and PRs are disconnected.

**Scope**:
- Unified view: issue → PR → merge
- Auto-linking, status sync

---

## R7: Workflow Automation

**Problem**: I do the same issue workflows manually.

**Scope**:
- Hooks, triggers
- Automated transitions
- Building block for custom workflows

---

*The mountain is visible. The path becomes clearer as we climb.*
