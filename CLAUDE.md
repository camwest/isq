# isq Development Guidelines

## Docs

Reference `docs/` before architectural decisions:
- `STRATEGY.md` - Winning aspiration, where to play, how to win
- `ROADMAP.md` - Current focus and milestones
- `DESIGN.md` - Core decisions (sync model, conflict resolution, auth)
- `TECH_STACK.md` - Technology choices

## Core Principle: Local-First

**Sync everything, filter locally.**

- The local SQLite cache is the source of truth for reads
- CLI filters (`--state`, `--label`, `--assignee`) query the local cache
- Forges sync ALL data - never filter at the API level
- If a CLI flag exists, the underlying data MUST be synced to support it

Example: `isq issue list --state=closed` only works if we sync closed issues.

## Forge Abstraction

All forge-specific code (GitHub, Linear, etc.) must stay in its respective module (`github.rs`, `linear.rs`).

- Common types like `Issue` belong in `forge.rs`
- Never import forge-specific types (e.g., `crate::github::Issue`) outside the forge modules
- Forge modules should convert their API responses to the common `forge::Issue` type internally
- When implementing a forge method, check how GitHub does it first for consistency

## Issue Creation

Feature issues use problem-framing titles (what's wrong, not the solution).

Body format (keep it brief):
```
**Problem**: [1-2 sentences]
**Goal**: [1 sentence]
**Success criteria**: [bullet list]
```
