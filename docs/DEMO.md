# PoC: The Demo

Build the smallest thing that proves the thesis: **fresh data, instant reads**.

---

## The Demo Script

```bash
$ cd ~/src/rails

$ isq auth
Found existing gh CLI authentication.
✓ Logged in as camwest (via gh CLI)
✓ Syncing rails/rails in background...

$ isq issue list
#51234  Add support for...           bug, activerecord     2h ago
#51230  Fix regression in...         bug                   5h ago
#51228  Document new API...          docs                  1d ago
... (523 open issues)

$ time isq issue list
real    0m0.034s    # <-- this is the moment

# [Go to GitHub, add label "urgent" to #51234]
# [Wait 30 seconds]

$ isq issue list
#51234  Add support for...           bug, activerecord, urgent     2h ago
                                                        ^^^^^^ appeared

$ isq issue list --json | head -20
[
  {
    "number": 51234,
    "title": "Add support for...",
    "labels": ["bug", "activerecord", "urgent"],
    ...
  }
]
```

**The pitch:** "I changed that label on GitHub 30 seconds ago. Now watch." *runs command in 34ms*

---

## Architecture

```
┌─────────────────┐       ┌─────────────────┐
│    isq CLI      │       │   isq daemon    │
│                 │       │   (background)  │
│  auth ──────────┼──────▶│                 │
│  issue list     │       │  loop:          │
│  issue show     │       │    fetch GitHub │
│  daemon status  │       │    write SQLite │
└────────┬────────┘       │    sleep 30s    │
         │                └────────┬────────┘
         │                         │
         │     (both read/write)   │
         ▼                         ▼
┌─────────────────────────────────────────┐
│           SQLite (WAL mode)             │
│       ~/.cache/isq/cache.db             │
└─────────────────────────────────────────┘
```

**Key insight:** No IPC for reads. CLI and daemon both access SQLite directly. WAL mode allows concurrent read/write.

---

## Milestones

### M1: Scaffolding

**Goal:** `isq --help` works.

- Cargo project with dependencies
- clap CLI with subcommand structure
- Stub implementations that print "not implemented"

**Commands:**
```bash
isq --help
isq auth
isq issue list
isq issue show <id>
isq daemon status
```

---

### M2: GitHub Read

**Goal:** Fetch and display issues from GitHub (no cache yet).

- Detect gh CLI token (`gh auth token` or parse config)
- Detect current repo from git remote
- Fetch open issues via GitHub API (with pagination)
- Print to stdout in simple format

**Commands:**
```bash
$ isq auth
✓ Logged in as camwest (via gh CLI)

$ isq issue list    # fetches live, slow
#51234  Add support for...
...
```

**Notes:**
- This will be slow (~2-5s for large repos)
- That's the point—we'll make it fast in M3

---

### M3: Local Cache

**Goal:** Instant reads from SQLite.

- Initialize SQLite database in cache dir
- Sync writes issues to database
- `issue list` and `issue show` read from database
- Measure and display timing

**Commands:**
```bash
$ isq auth           # syncs to SQLite
$ isq issue list     # reads from SQLite, instant
$ time isq issue list
real    0m0.034s
```

**Schema:**
```sql
CREATE TABLE issues (
    id INTEGER PRIMARY KEY,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    state TEXT NOT NULL,
    author TEXT,
    labels TEXT,  -- JSON array, denormalized for simplicity
    created_at TEXT,
    updated_at TEXT,
    repo TEXT NOT NULL
);

CREATE TABLE sync_state (
    repo TEXT PRIMARY KEY,
    last_sync TEXT,
    issue_count INTEGER
);

CREATE INDEX idx_issues_repo ON issues(repo);
CREATE INDEX idx_issues_number ON issues(repo, number);
```

---

### M4: Background Sync

**Goal:** Data stays fresh without manual sync.

- `isq auth` spawns daemon as background process
- Daemon runs sync loop (fetch → write → sleep 30s)
- PID file for status checking
- `daemon status` shows health

**Commands:**
```bash
$ isq auth
✓ Logged in as camwest (via gh CLI)
✓ Syncing rails/rails in background...

$ isq daemon status
rails/rails: synced 12s ago (523 issues)
Next sync in 18s

$ isq issue list     # always fresh, always instant
```

**Daemon lifecycle:**
- Spawned by `isq auth`
- Writes PID to `~/.cache/isq/daemon.pid`
- Writes logs to `~/.cache/isq/daemon.log`
- Dies on terminal close or reboot (acceptable for PoC)

---

## Technical Decisions

| Area | PoC Approach | Production (Later) |
|------|--------------|-------------------|
| Auth | gh CLI token only | + OAuth flow, PAT input |
| Daemon | Background process | launchd/systemd service |
| Daemon death | Dies on reboot | Survives reboot |
| Sync | Full replace | Incremental (ETags, since) |
| Repos | Single (current dir) | Multi-repo tracking |
| Platform | macOS first | + Linux, Windows |
| Writes | None | create, close, comment, etc. |
| Output | Basic table | Colors, formatting |

---

## Dependencies

```toml
[package]
name = "isq"
version = "0.1.0"
edition = "2021"

[dependencies]
clap = { version = "4", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite = { version = "0.31", features = ["bundled"] }
directories = "5"
anyhow = "1"
```

---

## Risk Areas

### 1. Daemon Spawning
macOS/Linux: `std::process::Command` with `.spawn()` works, but process dies with parent terminal unless properly daemonized.

**Mitigation:** Use `daemonize` crate or manual double-fork. For PoC, accept that closing terminal kills daemon.

### 2. SQLite Concurrent Access
Daemon writes while CLI reads. Without WAL mode, this can block or error.

**Mitigation:** Enable WAL mode on connection:
```rust
conn.pragma_update(None, "journal_mode", "WAL")?;
```

### 3. gh CLI Token Location
Token location varies:
- `~/.config/gh/hosts.yml` (Linux/macOS)
- `%APPDATA%\GitHub CLI\hosts.yml` (Windows)
- Keychain on macOS (sometimes)

**Mitigation:** Shell out to `gh auth token` instead of parsing config. Simpler, always works.

### 4. GitHub Pagination
Large repos have 1000+ issues. Default page size is 30. Need to paginate.

**Mitigation:** Use Link header pagination or `page` param. Fetch all pages sequentially (good enough for PoC).

### 5. Rate Limits
5000 requests/hour authenticated. Full sync of large repo might use 20-50 requests.

**Mitigation:** For PoC, just warn if approaching limit. Production daemon manages budget.

---

## Success Criteria

The demo works when:

1. `isq auth` completes in <5s (initial sync)
2. `isq issue list` returns in <100ms (from cache)
3. Changes on GitHub appear within 60s
4. `--json` output is valid, parseable JSON
5. Works on a real large repo (rails/rails, 500+ issues)

---

## Out of Scope for PoC

- Write operations (create, close, comment)
- Multiple repos
- GitLab/Forgejo
- OAuth flow
- System service installation
- Pretty colors/formatting
- Windows support
- Error recovery in daemon
