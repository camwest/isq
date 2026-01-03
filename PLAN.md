# Implementation Plan: Issue #38 - Repeated Flags for Common Workflows

## Problem Statement

Users working with multiple repositories must repeatedly specify `-R owner/repo` with every command. Some users also consistently want `--json` output. This creates friction in common workflows.

## Proposed Solution

Enable configuration of common patterns via a global user config file:

```toml
# ~/.config/isq/config.toml
[aliases]
rails = "rails/rails"
react = "facebook/react"

[defaults]
json = true
```

---

## Architecture Overview

### Current State

| Component | Location | Purpose |
|-----------|----------|---------|
| Per-repo config | `.config/isq.toml` in repo root | Worktree setup, on_start hooks |
| Cache DB | `~/.cache/isq/cache.db` | Issues, comments, sync state |
| CLI args | `src/cli/args.rs` | Clap-derived command structure |
| No global config | - | Does not exist |
| No `-R` flag | - | Not implemented |

### Target State

| Component | Location | Purpose |
|-----------|----------|---------|
| **NEW: Global user config** | `~/.config/isq/config.toml` | Aliases, defaults |
| Per-repo config | `.config/isq.toml` | Unchanged |
| CLI args | `src/cli/args.rs` | Add global `-R` flag |

---

## Implementation Steps

### Step 1: Create Global User Config Module

**File:** `src/user_config.rs` (NEW)

Create a new module for user-level configuration:

```rust
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub defaults: Defaults,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default)]
    pub json: bool,
}

/// Get the user config directory path
pub fn config_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Get the user config file path
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Load global user configuration
pub fn load() -> Result<UserConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let config: UserConfig = toml::from_str(&content)?;
    Ok(config)
}

/// Expand a repo alias to its full form
pub fn expand_alias(config: &UserConfig, repo: &str) -> String {
    config.aliases.get(repo).cloned().unwrap_or_else(|| repo.to_string())
}
```

**Tests:** Add unit tests for parsing, alias expansion, missing file handling.

---

### Step 2: Add Global `-R/--repo` Flag to CLI

**File:** `src/cli/args.rs`

Add a global `--repo` flag to the `Cli` struct:

```rust
#[derive(Parser)]
#[command(name = "isq")]
#[command(about = "Instant issue tracking. Offline-first. AI-agent native.")]
#[command(version)]
pub struct Cli {
    /// Repository to operate on (e.g., owner/repo or alias)
    #[arg(short = 'R', long = "repo", global = true)]
    pub repo: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}
```

The `global = true` makes this flag available to all subcommands.

---

### Step 3: Create Config Resolution Helpers

**File:** `src/cli/utils.rs` (add to existing or create)

Add functions to resolve effective configuration:

```rust
use crate::user_config::{self, UserConfig};

/// Resolved configuration for a command
pub struct ResolvedConfig {
    pub repo: Option<String>,  // Expanded alias if any
    pub json: bool,            // CLI override or default
}

/// Resolve effective configuration from CLI args and user config
pub fn resolve_config(
    cli_repo: Option<&str>,
    cli_json: bool,
    user_config: &UserConfig,
) -> ResolvedConfig {
    let repo = cli_repo.map(|r| user_config::expand_alias(user_config, r));
    let json = cli_json || user_config.defaults.json;

    ResolvedConfig { repo, json }
}
```

---

### Step 4: Update main.rs to Load Config and Pass Through

**File:** `src/main.rs`

1. Load user config once at startup
2. Pass the parsed `cli.repo` to command handlers
3. Update command functions to accept optional repo override

```rust
// In main()
let user_config = user_config::load().unwrap_or_default();

// When calling command functions, pass resolved config
let effective_repo = cli.repo.as_deref()
    .map(|r| user_config::expand_alias(&user_config, r));
```

---

### Step 5: Update Command Functions to Use Repo Override

**Files:** `src/cli/issues.rs`, `src/cli/goals.rs`, etc.

Modify command functions to accept optional repo override:

```rust
pub async fn cmd_list(
    // ... existing args ...
    repo_override: Option<&str>,  // NEW
    user_config: &UserConfig,     // NEW
) -> Result<()> {
    // Use repo_override if provided, else detect from cwd
    let repo_path = match repo_override {
        Some(repo) => {
            // For remote repo: ensure it's synced, return synthetic path
            ensure_repo_synced(repo).await?;
            repo.to_string()
        }
        None => detect_repo_path()?,
    };

    // Resolve json default
    let json = json_flag || user_config.defaults.json;

    // ... rest of function
}
```

**Key consideration:** When `-R` is used for a non-local repo, we need to:
1. Auto-register it with the daemon for syncing (if not already)
2. Use the forge_repo directly for DB queries instead of deriving from local path

---

### Step 6: Handle Non-Local Repo Operations

When `-R rails/rails` is used and user is NOT in a rails/rails checkout:

1. **Check if repo is linked:** Query `repo_links` table for this forge_repo
2. **If not linked:** Auto-link with default forge (GitHub), register with daemon
3. **Query cache:** Use forge_repo directly for issue queries
4. **Write operations:** Use forge_repo for API calls

This requires refactoring how `get_repo_link` works to support lookup by forge_repo as well as local path.

**File:** `src/db/repos.rs`

Add function:
```rust
pub fn get_repo_link_by_forge_repo(
    conn: &Connection,
    forge_repo: &str,
) -> Result<Option<RepoLink>>
```

---

### Step 7: Apply JSON Default

Currently `--json` is defined on each subcommand. Options:

**Option A: Keep per-command flag, merge with default**
- Pro: Clear, explicit, CLI flag wins
- Con: Slightly more code in each command

**Option B: Move to global flag**
- Pro: Cleaner args.rs
- Con: Breaking change if users rely on subcommand flag position

**Recommendation:** Option A - keep per-command `--json` flags, merge with config default in each command handler. CLI flag always wins (explicit `--json` enables, implicit default from config applies when flag absent).

---

## File Changes Summary

| File | Change |
|------|--------|
| `src/user_config.rs` | NEW - Global config loading, alias expansion |
| `src/cli/args.rs` | Add global `-R/--repo` flag |
| `src/main.rs` | Load user config, pass to commands |
| `src/cli/issues.rs` | Accept repo override, apply json default |
| `src/cli/goals.rs` | Accept repo override, apply json default |
| `src/cli/labels.rs` | Accept repo override, apply json default |
| `src/db/repos.rs` | Add `get_repo_link_by_forge_repo` |
| `src/lib.rs` | Add `pub mod user_config;` |

---

## Testing Strategy

1. **Unit tests for user_config.rs:**
   - Parse valid config
   - Parse empty config (defaults)
   - Alias expansion
   - Missing file returns defaults

2. **Integration tests:**
   - `-R alias` expands correctly
   - `--json` flag overrides config default
   - Config default applies when flag absent

3. **Manual testing:**
   - Create `~/.config/isq/config.toml` with aliases
   - Run `isq issue list -R alias`
   - Verify alias expansion in output

---

## Edge Cases

1. **Alias doesn't exist:** Use the literal value as repo (e.g., `-R rails/rails`)
2. **Invalid repo format:** Error from forge when attempting to sync
3. **Config file syntax error:** Log warning, use defaults
4. **Mixed: local repo + -R flag:** `-R` wins
5. **No config file:** Use empty defaults (no aliases, json=false)

---

## Future Considerations

- Add `isq config` subcommand to manage config file
- Support `--no-json` to explicitly disable when default is true
- Add more defaults (e.g., `sort = "priority"`, `state = "open"`)
- Consider environment variable overrides (`ISQ_DEFAULT_REPO`)
