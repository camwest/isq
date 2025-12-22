# Tech Stack

CLI-first. Rust. Speed obsession.

## Why Rust

uv (Python package manager) is I/O bound like isq. They chose Rust and achieved 10-100x speedup over pip. Not just "Rust is fast"—Rust gives you tools to obsess: custom allocators, zero-copy, parallel everything.

Go is "fast enough." Rust lets you obsess.

## Core Stack

| Component | Library | Rationale |
|-----------|---------|-----------|
| CLI | clap | Best-in-class arg parsing, auto-generated help |
| Async | tokio | Industry standard, excellent for background sync |
| HTTP | reqwest | Built on tokio, handles GitHub/GitLab/Forgejo APIs |
| JSON | serde | Zero-copy deserialization, trivial `--json` output |
| Local DB | rusqlite | SQLite bindings, single-file cache |
| Config | toml + directories | XDG-compliant config paths |

## Architecture

```
isq <command>
       │
       ▼
┌─────────────────┐
│   CLI (clap)    │
└────────┬────────┘
         │
         ▼
┌─────────────────┐     ┌─────────────────┐
│   Local Cache   │◄────│   Sync Engine   │
│    (SQLite)     │     │    (tokio)      │
└────────┬────────┘     └────────┬────────┘
         │                       │
         ▼                       ▼
   instant reads          background sync
   for CLI/agents         with forge APIs
```

## Why This Matters

1. **CLI reads from local cache** - instant, no network
2. **Sync runs in background** - cache stays fresh
3. **AI agents get instant responses** - `isq list --json` hits disk, not network
4. **Offline works** - queue writes, sync when online

## Future: TUI Layer

If TUI is added later:
- ratatui (60+ FPS, low memory)
- tui-textarea (inline editing)

Same sync engine, different interface.

## References

- [uv architecture](https://astral.sh/blog/uv) - inspiration for speed obsession
- [Linear sync engine](https://linear.app) - inspiration for local-first
- [clap](https://github.com/clap-rs/clap)
- [tokio](https://tokio.rs/)
- [ratatui](https://github.com/ratatui/ratatui) (future)
