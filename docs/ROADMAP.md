# Roadmap

6-week release cycles. Refine R2+ after R1 ships.

---

## R1: Production-Ready + Git Integration

**Theme**: Stable foundation + core differentiator

**Production hardening**:
- Bug fixes from real usage
- Error handling edge cases
- Daemon reliability
- Cache coherence under load

**Git integration**:
- Detect current worktree/branch
- Infer current issue from branch name (e.g., `fix/423-auth-bug` → #423)
- `isq` with no args shows current context
- `isq start <id>` / `isq done <id>` lifecycle

**Exit criteria**: You use isq daily without hitting bugs. Git context works.

---

## R2: Personal State & Multi-Repo

- "What am I working on?" persists across sessions
- Multi-repo unified view
- Time-in-context tracking

---

## R3: Forge Expansion

- Forgejo backend
- GitLab backend
- Prove the abstraction scales

---

## R4: Agent Experience

- Rich context command for Claude Code / Cursor
- Error messages optimized for agent recovery
- Agent workflow integration

---

## R5: Distribution & Polish

- One-line installer
- Package managers (brew, cargo, etc.)
- Documentation & onboarding
- TUI (if validated as needed)

---

*R2-R5 are directional. Refine after R1 learnings.*
