# isq

Offline-first CLI for GitHub/Linear issues. Rust.

## Build & Test

```
cargo build --release
cargo test
```

## Structure

- `src/forges/{github,linear}.rs` - Forge API clients
- `src/daemon.rs` - Background sync
- `src/db.rs` - SQLite cache
- `src/cli/` - Commands

## Docs

See `docs/` for context: STRATEGY.md (vision), ROADMAP.md (focus), DESIGN.md (architecture).

## Principles

**Local-first**: Sync everything, filter locally. SQLite is source of truth. Never filter at API level.

**Forge abstraction**: Forge-specific code stays in `src/forges/{github,linear}.rs`. Common types in `mod.rs`. Check GitHub impl for consistency.
