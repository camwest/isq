# Roadmap

6-week release cycles. Each release ships with polish and distribution improvements. Refine R2+ after R1.

---

## R1: Production-Ready + Git Integration

**Feature**: Git context awareness
- Detect worktree/branch → infer current issue
- `isq` with no args shows context
- `isq start <id>` / `isq done <id>` lifecycle

**Polish**: Bug fixes, error handling, daemon reliability

**Distribution**: GitHub releases, install script, basic docs

**Exit criteria**: You use isq daily without hitting bugs.

---

## R2: Personal State & Multi-Repo

**Feature**: Track what you're working on across repos
- Persistent "active issues" state
- Multi-repo unified view

---

## R3: Forge Expansion

**Feature**: More backends
- Forgejo
- GitLab

---

## R4: Agent Experience

**Feature**: Optimized for Claude Code / Cursor / Aider
- Rich context command
- Agent-friendly error messages

---

*R2-R4 are directional. Scope refined after each release.*
