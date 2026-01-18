# Implementation Plan: Install, Upgrade, and Uninstall UX

**Closes:** #30

## Problem Statement

Installation and upgrade procedures vary depending on install method. Uninstalling may leave behind daemon processes and residual configuration without clear explanation.

**Goal**: Establish transparent, dependable processes for all three deployment phases.

**Success criteria:**
- Users can confidently proceed through installation and upgrade workflows with explicit step-by-step guidance
- The uninstall process restores the system to a clean baseline state with transparent communication about any changes

---

## Current State Analysis

### What Works Well

| Feature | Status | Notes |
|---------|--------|-------|
| Install script (curl) | Good | Checksum verification, auto-detect platform, receipt writing |
| `isq update check` | Exists | Checks for newer version |
| `isq update install` | Exists | Downloads and installs latest |
| `isq doctor` | Exists | Diagnoses common issues |
| Service uninstall API | Exists | `service::uninstall()` stops daemon, removes service file |

### Gaps

| Gap | Impact | Priority |
|-----|--------|----------|
| No uninstall documentation | Users don't know how to cleanly remove isq | P0 |
| Upgrade not prominent in README | Users don't discover self-update capability | P1 |
| No `isq uninstall` command | Users must manually stop daemon + delete files | P1 |
| No shell completions documentation | Reduces discoverability | P2 |
| Install troubleshooting sparse | Users stuck when things fail | P2 |

---

## Solution Design

### 1. Add `isq uninstall` Command

A guided uninstall that removes all isq components with clear feedback.

**Behavior:**

```
$ isq uninstall
This will remove isq and its associated files:

  Binary:    /usr/local/bin/isq
  Config:    ~/.config/isq/ (contains views, credentials)
  Cache:     ~/Library/Caches/isq/ (contains issue database)
  Daemon:    com.isq.daemon (will be stopped)
  Commit hook: .git/hooks/prepare-commit-msg (in linked repos)

Proceed? [y/N] y

Stopping daemon... done
Removing service... done
→ To complete uninstall, run:
  sudo rm /usr/local/bin/isq  # or just: rm ~/.local/bin/isq

Config and cache preserved. Remove manually if desired:
  rm -rf ~/.config/isq ~/Library/Caches/isq
```

**Flags:**
- `--keep-config` — Skip config removal prompt
- `--keep-cache` — Skip cache removal prompt
- `-y, --yes` — Skip confirmation prompts
- `--dry-run` — Show what would be removed without removing

**Why not auto-delete binary?**
Binary may be in `/usr/local/bin` (requires sudo) or managed by package manager. Safer to instruct user than assume permissions.

**Implementation:**

```rust
// src/cli/uninstall.rs (NEW)

pub async fn cmd_uninstall(
    keep_config: bool,
    keep_cache: bool,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    let receipt = install::load_receipt()?;
    let config_dir = user_config::config_dir()?;
    let cache_dir = cache_dir()?;

    // Detect what exists
    let items = UninstallItems {
        binary_path: receipt.map(|r| r.binary_path),
        config_dir: config_dir.exists().then_some(config_dir),
        cache_dir: cache_dir.exists().then_some(cache_dir),
        daemon_running: service::status().map(|s| s.running).unwrap_or(false),
        service_installed: service::status().map(|s| s.installed).unwrap_or(false),
    };

    // Show plan
    print_uninstall_plan(&items, keep_config, keep_cache);

    if dry_run {
        return Ok(());
    }

    if !yes && !confirm("Proceed?")? {
        return Ok(());
    }

    // Execute
    if items.daemon_running {
        print_step("Stopping daemon...");
        service::stop()?;
    }
    if items.service_installed {
        print_step("Removing service...");
        service::uninstall()?;
    }

    // Data directories (credentials are in config_dir)
    if !keep_config {
        if let Some(dir) = items.config_dir {
            fs::remove_dir_all(&dir)?;
            println!("  Removed {}", dir.display());
        }
    }
    if !keep_cache {
        if let Some(dir) = items.cache_dir {
            fs::remove_dir_all(&dir)?;
            println!("  Removed {}", dir.display());
        }
    }

    // Binary removal instruction
    if let Some(path) = items.binary_path {
        if path.starts_with("/usr") {
            println!("\n→ To complete uninstall, run:\n  sudo rm {}", path.display());
        } else {
            println!("\n→ To complete uninstall, run:\n  rm {}", path.display());
        }
    }

    Ok(())
}
```

---

### 2. Documentation Improvements

#### 2a. README.md Updates

Add new sections and improve existing ones:

**Add "Updating" section after Install:**

```markdown
## Updating

isq can update itself:

```bash
# Check if update is available
isq update check

# Install latest version
isq update install
```

If installed via Homebrew: `brew upgrade isq`
```

**Add "Uninstalling" section:**

```markdown
## Uninstalling

```bash
# Guided uninstall (stops daemon, removes config)
isq uninstall

# Keep your configuration
isq uninstall --keep-config
```

**Manual uninstall:**

macOS/Linux:
```bash
# Stop and remove daemon
launchctl unload ~/Library/LaunchAgents/com.isq.daemon.plist 2>/dev/null
rm -f ~/Library/LaunchAgents/com.isq.daemon.plist
# Or on Linux:
systemctl --user stop isq-daemon
systemctl --user disable isq-daemon

# Remove binary
sudo rm /usr/local/bin/isq  # or ~/.local/bin/isq

# Remove data (optional)
rm -rf ~/.config/isq ~/Library/Caches/isq  # macOS
rm -rf ~/.config/isq ~/.cache/isq          # Linux
```

Windows:
```powershell
# Remove scheduled task
schtasks /delete /tn "isq-daemon" /f

# Remove binary and data
Remove-Item "$env:LOCALAPPDATA\isq" -Recurse -Force
Remove-Item "$env:APPDATA\isq" -Recurse -Force
```
```

**Expand Install section with troubleshooting:**

```markdown
## Install

**macOS / Linux:**
```bash
curl -LsSf https://cameronwestland.com/isq/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://cameronwestland.com/isq/install.ps1 | iex
```

Or download directly from [GitHub Releases](https://github.com/camwest/isq/releases).

<details>
<summary>Troubleshooting</summary>

**"command not found" after install**

The installer places `isq` in `~/.local/bin` if `/usr/local/bin` isn't writable.
Add it to your PATH:

```bash
# bash
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc

# zsh
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.zshrc
```

**Checksum verification failed**

Re-run the installer. If it persists, download manually from GitHub Releases and verify:

```bash
shasum -a 256 isq-*.tar.gz
# Compare with checksums.txt in the release
```

**Permission denied**

If you see permission errors installing to `/usr/local/bin`:
```bash
# Option 1: Use sudo
curl -LsSf https://cameronwestland.com/isq/install.sh | sudo sh

# Option 2: Let installer use ~/.local/bin (automatic fallback)
```

</details>
```

#### 2b. Update Commands table

Add uninstall to the Commands table:

```markdown
| `isq uninstall` | Remove isq (stops daemon, guides cleanup) |
```

---

### 3. `isq doctor` Improvements

Add checks relevant to install/upgrade/uninstall:

```rust
// In src/cli/doctor.rs

fn check_install_method() -> DiagnosticResult {
    let receipt = install::load_receipt();
    match receipt {
        Ok(Some(r)) => {
            if r.method == "standalone" && r.auto_update {
                DiagnosticResult::ok("Install method", "standalone (auto-updates enabled)")
            } else {
                DiagnosticResult::ok("Install method", &format!("{} (update via package manager)", r.method))
            }
        }
        _ => DiagnosticResult::warn(
            "Install method",
            "Unknown (no receipt found). Updates may need manual installation."
        )
    }
}

fn check_orphan_daemon() -> DiagnosticResult {
    // Check if daemon is running but binary doesn't exist
    if let Ok(status) = service::status() {
        if status.running {
            if let Ok(Some(receipt)) = install::load_receipt() {
                if !Path::new(&receipt.binary_path).exists() {
                    return DiagnosticResult::error(
                        "Orphan daemon",
                        "Daemon running but isq binary not found. Run: isq uninstall"
                    );
                }
            }
        }
    }
    DiagnosticResult::ok("Daemon state", "healthy")
}
```

---

### 4. Shell Completions Documentation

Document the existing completions (if they exist) or note as future work.

**Check:** Does `isq completions <shell>` exist?

If yes, add to README:

```markdown
## Shell Completions

Generate completions for your shell:

```bash
# Bash
isq completions bash > ~/.local/share/bash-completion/completions/isq

# Zsh
isq completions zsh > ~/.zfunc/_isq

# Fish
isq completions fish > ~/.config/fish/completions/isq.fish
```
```

If no, note in roadmap as future work (out of scope for this issue).

---

## Implementation Steps

### Phase 1: Uninstall Command (Core)

1. **Create `src/cli/uninstall.rs`**
   - Implement `cmd_uninstall()` with plan display, confirmation, cleanup
   - Handle all three platforms (macOS, Linux, Windows)

2. **Add to CLI args**
   - Add `Uninstall` variant to `Commands` enum in `src/cli/args.rs`
   - Wire up in `main.rs`

3. **Test on all platforms**
   - macOS: launchd service removal
   - Linux: systemd user service removal
   - Windows: Task Scheduler removal

### Phase 2: Documentation

5. **Update README.md**
   - Add "Updating" section
   - Add "Uninstalling" section with manual fallback
   - Expand "Install" with troubleshooting accordion

6. **Update Commands table**
   - Add `isq uninstall` row

### Phase 3: Polish

7. **Enhance `isq doctor`**
   - Add install method check
   - Add orphan daemon detection

8. **Shell completions** (if time permits)
   - Document if exists
   - Skip if not implemented (out of scope)

---

## File Changes Summary

| File | Change |
|------|--------|
| `src/cli/uninstall.rs` | NEW — Uninstall command implementation |
| `src/cli/args.rs` | Add `Uninstall` subcommand |
| `src/cli/mod.rs` | Add `pub mod uninstall;` |
| `src/main.rs` | Route uninstall command |
| `src/cli/doctor.rs` | Add install/daemon checks |
| `README.md` | Add Updating, Uninstalling sections; expand Install |

---

## Testing Strategy

1. **Unit tests:** Uninstall plan generation, path detection
2. **Integration tests:**
   - Fresh install → uninstall → verify clean state
   - Uninstall with `--keep-config` preserves config
   - Uninstall with `--dry-run` changes nothing
3. **Manual testing:** All three platforms

---

## Out of Scope

- Package manager distribution (Homebrew, apt, etc.) — separate initiative
- Man page generation — future enhancement
- Auto-update in background — already working for standalone installs
- Website/hosted docs — future when user base grows

---

## Success Metrics

Post-implementation, users should be able to:

1. **Install**: `curl ... | sh` → working `isq` in PATH
2. **Upgrade**: `isq update install` → latest version, daemon restarted
3. **Uninstall**: `isq uninstall` → clean system with clear feedback

No orphan processes. No mystery files. No undocumented steps.
