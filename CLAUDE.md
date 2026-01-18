# isq

Offline-first CLI for GitHub/Linear/JIRA issues. Rust.

## Build & Test

```
cargo build --release
cargo test
```

## Structure

- `src/forges/{github,linear,jira}.rs` - Forge API clients
- `src/daemon/` - Background sync
- `src/db.rs` - SQLite cache
- `src/cli/` - Commands

## Database

SQLite cache: `~/Library/Caches/isq/cache.db`

## Docs

See `docs/` for context: STRATEGY.md (vision), ROADMAP.md (focus), DESIGN.md (architecture), CONTRIBUTING.md (issue creation).

## Linting (Agent Constraints)

Lint rules enforce code quality - they fail builds, unlike prose instructions. Run before committing:

```
scripts/lint.sh --ci
```

This runs clippy, fmt check, and enforces the 500-line file size limit. When splitting files, use screaming architecture: name modules after domain concepts (`comments.rs`, `queue.rs`) not abstractions (`lifecycle.rs`, `helpers.rs`).

Key constraints in `clippy.toml`:
- **cognitive-complexity-threshold = 15** - Break up complex functions
- **too-many-lines-threshold = 100** - Keep functions focused
- **too-many-arguments-threshold = 7** - Use structs for config

## Principles

**Pre-1.0, zero users**: No backward compatibility concerns. Break anything freely.

**Local-first**: Sync everything, filter locally. SQLite is source of truth. Never filter at API level.

**Forge abstraction**: Forge-specific code stays in `src/forges/{github,linear,jira}.rs`. Common types in `mod.rs`. Check GitHub impl for consistency.
