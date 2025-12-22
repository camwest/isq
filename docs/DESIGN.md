# Design Decisions

Open questions to resolve before building.

---

## 1. Auth Strategy

**Decision:** Tiered approach, best UX wins.

```
$ isq auth

# Path 1: gh CLI exists and authed (zero friction)
Found existing gh CLI authentication.
✓ Logged in as camwest (via gh CLI)

# Path 2: No gh CLI, do browser OAuth
Opening browser to authenticate with GitHub...
✓ Logged in as camwest

# Path 3: Headless/CI fallback
$ isq auth --token
Paste your GitHub personal access token:
✓ Logged in as camwest
```

**Priority order:**
1. Reuse `gh` CLI token (instant for most devs)
2. Browser OAuth (one click, no token copying)
3. PAT fallback (CI, headless, scripts)

**Token storage:**
- macOS: Keychain
- Linux: secret-service (fallback: encrypted file)
- Windows: Credential Manager
- Never plaintext in config

**Tradeoff:** OAuth requires registering GitHub OAuth app + local server dance. Worth it—auth is first impression.

**Status:** ✅ Decided

---

## 2. Sync Model

**Decision:** Single orchestrator daemon with intelligent sync.

**Architecture:**
```
┌─────────────────────────────────────────────────────────┐
│                      isq daemon                         │
│                   (one per user)                        │
├─────────────────────────────────────────────────────────┤
│  Rate limit budget: 5000 req/hr                         │
│  Used this hour: 847                                    │
│                                                         │
│  Repo                    Priority    Last Sync    Next  │
│  ───────────────────────────────────────────────────    │
│  rails/rails             high        12s ago      48s   │
│  myorg/myapp             high        34s ago      26s   │
│  old-project/thing       low         2hr ago      4hr   │
└─────────────────────────────────────────────────────────┘
```

**How it works:**
1. Single daemon starts on `isq auth`
2. Daemon installs as user-level service (survives reboot)
   - macOS: `~/Library/LaunchAgents/com.isq.daemon.plist`
   - Linux: `~/.config/systemd/user/isq.service`
   - Windows: Task Scheduler (current user)
3. When CLI runs in a repo → tells daemon "watch this repo"
4. Daemon orchestrates ALL syncs with single rate limit budget
5. CLI always reads from local cache (instant)

**Priority rules:**
| Repo activity | Sync interval |
|---------------|---------------|
| Accessed in last 5 min | Every 30s |
| Accessed in last hour | Every 2 min |
| Accessed today | Every 10 min |
| Not accessed in days | Every hour (or drop) |

**Rate limit management:**
- Single budget across all repos (5000 req/hr)
- Orchestrator picks highest priority repo that fits budget
- Auto-backoff when approaching limit
- Warn user if too many repos

**CLI ↔ Daemon IPC:** Unix socket (macOS/Linux), named pipe (Windows)

**Commands:**
```bash
isq auth              # installs + starts daemon
isq daemon status     # show sync state, watched repos
isq daemon restart    # restart if misbehaving
isq auth --logout     # stops daemon + removes service
```

**UX result:** User never thinks about sync. Cache is always fresh. No `isq sync` command needed.

**Status:** ✅ Decided

---

## 3. Conflict Resolution

**Decision:** Do as much as possible. Server wins. User informed.

**Principle:** Partial success is better than total failure. Apply what can be applied, report what failed, let user re-assert if needed.

**How Linear does it:** Server is authority. Last-writer-wins. No OT/CRDTs. Simple.

**How isq does it:** GitHub is the server. Same model.

```
$ isq sync
✓ Created #424 "Bug"
✓ Comment posted to #423
⚠️ Label 'foo' could not be applied (deleted from repository)
⚠️ Close #423 skipped (reopened by @maintainer with context)
```

**Rules by operation type:**

| Operation | Sync behavior |
|-----------|---------------|
| Comment | Always succeeds (append-only) |
| Create issue | Succeeds, reports failed sub-ops (labels, assignee) |
| State change (close/reopen) | Server wins if diverged, user informed |
| Label add/remove | Server wins if diverged, user informed |
| Assign | Server wins if diverged, user informed |

**User can re-assert:**
```
$ isq close 423    # if they still want it closed
```

**No prompts. No blocking. Clear reporting.**

Specific UX per command to be designed during implementation—principle is sufficient for now.

**Status:** ✅ Decided

---

## 4. Multi-repo

**Decision:** Auto-detect + explicit flag + aliases. Follow `gh` CLI patterns.

**Default (auto-detect):**
```bash
$ cd ~/src/rails
$ isq list          # infers rails/rails from .git remote
```

**Explicit flag (any repo, no clone needed):**
```bash
$ isq list -R rails/rails
$ isq list --repo owner/repo
```

**First `-R` use registers with daemon:**
```
$ isq list -R rails/rails
Adding rails/rails to sync...
✓ Synced 523 issues
```

**Aliases for frequent repos:**
```toml
# ~/.config/isq/config.toml
[aliases]
rails = "rails/rails"
react = "facebook/react"
```
```bash
$ isq list -R rails    # expands to rails/rails
```

**Cross-repo queries:**
```bash
$ isq list --assignee=@me --all-repos
$ isq list --label=bug -R rails -R react
```

**Status:** ✅ Decided

---

## 5. First Forge

**Decision:** GitHub only for v0.1. Define `Forge` trait upfront.

**Why low risk:** Core concepts (issues, comments, labels, assignees, milestones) are nearly identical across GitHub, GitLab, and Forgejo. The abstraction is obvious.

```rust
trait Forge {
    fn list_issues(&self, filters: Filters) -> Result<Vec<Issue>>;
    fn get_issue(&self, id: &str) -> Result<Issue>;
    fn create_issue(&self, req: CreateIssue) -> Result<Issue>;
    fn create_comment(&self, issue: &str, body: &str) -> Result<Comment>;
    fn close_issue(&self, id: &str) -> Result<()>;
    // ...
}
```

**Mitigation against GitHub-isms:**
- Define trait first, even with only GitHub impl
- CLI code uses trait, never calls GitHub directly
- Gut check: "would this command make sense on Forgejo?"

**What actually differs (handle later):**
- API pagination → abstract iterator
- Rate limits → already per-forge in daemon
- Search syntax → normalize in CLI, translate per forge

**Status:** ✅ Decided

---

## 6. Distribution

**Decision:** Curl one-liner + pre-built binaries. Like uv.

**Tier 1 (day one):**
```bash
# macOS/Linux
curl -LsSf https://isq.dev/install.sh | sh

# Windows
powershell -c "irm https://isq.dev/install.ps1 | iex"

# Or direct download from GitHub Releases
# (macOS arm64/x64, Linux arm64/x64, Windows x64)
```

**Tier 2+ (later):** Homebrew, Scoop, Winget, apt, AUR, Nix, etc. Figure out as we grow.

**NOT the primary method:**
```bash
cargo install isq  # fine to support, never advertise
```

**Status:** ✅ Decided

---

## 7. Update Strategy

**Decision:** Claude Code's model. Standalone auto-updates. Package managers defer.

**Standalone installer (recommended):**
- Auto-updates silently in background
- Checks on startup, applies without prompting
- User never thinks about updates

**Package manager installs:**
- Detect install method
- Disable auto-update
- Guide user to correct command

```bash
$ isq --version
isq 0.1.0 (standalone, auto-updates enabled)

$ isq --version
isq 0.1.0 (homebrew)
Note: Run `brew upgrade isq` to update.
```

**Detection:** Standalone installer writes receipt to `~/.config/isq/install.json`. If no receipt or path matches known package manager locations, assume manual/package manager.

**Why this model wins:**
- uv requires explicit `uv self update`
- Codex has chaos (multiple install methods conflict)
- Claude Code: standalone just works, package managers guided

**Status:** ✅ Decided

---

## 8. Name

**Decision:** `isq`

- Stands for "issue queue" (if anyone asks)
- QQQ-style: fast to type, memorable because unusual
- 3 letters, all lowercase, no shift key
- Passes A Hundred Monkeys test: "What's that?" → curiosity
- Not taken by any significant CLI tool

```bash
isq list
isq show 423
isq comment 423 "Fixed in abc123"
isq close 423
```

**Status:** ✅ Decided

---

## 9. CLI UX Details

**Decision:** Subcommand style with namespaces.

```bash
isq issue list
isq issue show 423
isq issue close 423

# Future
isq pr list
isq pr review 456
isq notification list
```

**Rationale:**
1. **Extensibility** - Roadmap includes PRs, notifications, milestones. Without namespaces we'd have inconsistent commands or awkward `--type` flags.
2. **LLM-friendly** - `isq issue list` costs nothing extra for an LLM to generate. "Shorter" only matters for human muscle memory.
3. **Discoverability** - `isq issue --help` shows all issue commands. `isq pr --help` shows all PR commands.

**ID format:**
```bash
# Just number (assumes current repo)
isq issue show 423

# Full reference (explicit)
isq issue show rails/rails#423
```

Both supported. Number for current repo, full ref for cross-repo.

**Status:** ✅ Decided

---

## 10. Cache Location

**Decision:** XDG standard via `directories` crate.

| Platform | Cache Location |
|----------|----------------|
| Linux | `~/.cache/isq/` |
| macOS | `~/Library/Caches/isq/` |
| Windows | `%LOCALAPPDATA%\isq\cache` |

Don't pollute repo dirs. Don't pollute `~/` with dotfiles.

**Status:** ✅ Decided

