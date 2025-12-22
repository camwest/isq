# MVP: DEMO → v0.1

Bridges the completed PoC (DEMO.md) to full MVP scope (README.md).

---

## Scope Decisions

| Area | MVP Decision |
|------|--------------|
| Backends | GitHub only (Forge trait for future) |
| Repos | Single repo (current directory) |
| Daemon | System service (survives reboot) |

---

## Gap: DEMO → MVP

| Feature | DEMO | MVP |
|---------|------|-----|
| Read operations | ✓ | ✓ |
| Filter flags (--label, --state) | ✗ | ✓ |
| Write operations | ✗ | ✓ |
| Offline queue | ✗ | ✓ |
| System service daemon | ✗ | ✓ |

---

## Milestones

### M5: Filter Flags

Add `--label` and `--state` filters to `issue list`.

```bash
isq issue list --label=bug
isq issue list --state=closed
isq issue list --label=bug --state=open
```

**Files:** `src/main.rs`, `src/db.rs`

---

### M6: Write Operations

Add create, comment, close, reopen, label, assign commands.

```bash
isq issue create --title="Bug" [--body="..."] [--label=bug]
isq issue comment <id> "message"
isq issue close <id>
isq issue reopen <id>
isq issue label <id> add <label>
isq issue label <id> remove <label>
isq issue assign <id> <user>
```

**Architecture:** Introduce `Forge` trait per DESIGN.md. CLI calls trait methods, `GitHubForge` implements. Enables GitLab/Forgejo later without rewrite.

```rust
trait Forge {
    fn list_issues(&self, filters: Filters) -> Result<Vec<Issue>>;
    fn create_issue(&self, req: CreateIssue) -> Result<Issue>;
    fn create_comment(&self, issue: &str, body: &str) -> Result<Comment>;
    fn close_issue(&self, id: &str) -> Result<()>;
    fn reopen_issue(&self, id: &str) -> Result<()>;
    fn add_label(&self, issue: &str, label: &str) -> Result<()>;
    fn remove_label(&self, issue: &str, label: &str) -> Result<()>;
    fn assign(&self, issue: &str, user: &str) -> Result<()>;
}
```

**Files:** `src/main.rs`, `src/forge.rs` (trait), `src/github.rs` (impl)

---

### M7: Write Model (Sync-First, Offline-Aware)

Writes go directly to GitHub by default. Queue automatically when offline.

**Rationale:**
- Reads can be instant because data exists locally (cache)
- Writes must hit the server eventually - async just hides latency
- 150ms for a single write is fine; user gets immediate confirmation
- Async creates complexity: no issue numbers, uncertain state, silent failures

**Behavior:**

| Scenario | Behavior |
|----------|----------|
| Online (default) | Direct API call, immediate confirmation with issue number |
| Offline | Auto-queue locally, sync when back online |
| Bulk (future) | Queue for optimal throughput |

**UX:**
```bash
# Online - direct, get the number immediately
$ isq issue create --title "Fix login"
✓ Created #426 Fix login (150ms)

# Offline - auto-queues transparently
$ isq issue create --title "Fix login"
✓ Queued: Fix login (offline, 8ms)

# When back online, daemon syncs and reports
$ isq daemon status
✓ Synced 2 pending operations
```

**Schema (for offline queue):**
```sql
CREATE TABLE pending_ops (
    id INTEGER PRIMARY KEY,
    repo TEXT NOT NULL,
    op_type TEXT NOT NULL,  -- create, comment, close, etc.
    payload TEXT NOT NULL,  -- JSON
    created_at TEXT NOT NULL
);
```

**Conflict resolution** per DESIGN.md: Server wins, user informed on sync.

**Files:** `src/db.rs`, `src/github.rs`, `src/daemon.rs`

---

### M8: System Service Daemon

Install as launchd (macOS) / systemd (Linux) user service.

**Paths:**
- macOS: `~/Library/LaunchAgents/com.isq.daemon.plist`
- Linux: `~/.config/systemd/user/isq.service`

**Commands:**
```bash
isq auth           # installs + starts service
isq auth --logout  # stops + removes service
isq daemon status  # show service state
isq daemon restart # restart if misbehaving
```

**Files:** `src/daemon.rs`, `src/service.rs`

---

## File Structure After MVP

```
src/
├── main.rs       # CLI with all commands
├── auth.rs       # gh CLI token detection
├── repo.rs       # git remote detection
├── forge.rs      # Forge trait definition
├── github.rs     # GitHubForge implementation
├── db.rs         # SQLite cache + pending_ops
├── daemon.rs     # sync loop
└── service.rs    # launchd/systemd integration
```

---

## Success Criteria

1. All commands from README MVP scope work
2. Offline queue syncs correctly with conflict reporting
3. Daemon survives reboot (launchd/systemd)
4. `--json` output on all commands for AI agents

---

## Deferred to v0.2+

Per DESIGN.md, these are explicitly out of scope for MVP:

- OAuth flow (auth tier 2)
- PAT input (auth tier 3)
- Keychain/secret-service token storage
- Multi-repo (-R flag, aliases)
- Priority-based sync intervals
- GitLab/Forgejo backends
- Distribution (install scripts, releases)
- Auto-updates
- **SQLite migration system** - Currently using `CREATE TABLE IF NOT EXISTS` which works for adding new tables but breaks for schema modifications. Acceptable for MVP since all data is rebuildable from GitHub. For v0.2+, consider `refinery` crate or manual version table + migration functions.
