# Implementation Plan: User Configuration Infrastructure

**Closes:** #46 (filter presets)
**Related:** #38 (descoped to defaults only, closed)

## Problem Statement

1. **Repeated flags:** Users consistently want `--json` output or specific sort orders, forcing repetition
2. **Common filter combos:** Power users run identical filter combinations ("my open bugs", "needs review") but must retype flags each time

## Solution

Global user configuration at `~/.config/isq/config.toml`:

```toml
[defaults]
json = true
sort = "priority"

[presets]
bugs = "--label=bug --state=open --mine"
review = "--label=needs-review --unassigned"
p0 = "--label=P0 --state=open"
```

**Usage:**
```bash
isq issue list              # Uses defaults (json=true, sort=priority)
isq issue list @bugs        # Expands preset + applies defaults
isq issue list @p0 --json   # CLI flags override/merge with preset
```

---

## Architecture

### File Locations

| Type | Path | Purpose |
|------|------|---------|
| User config | `~/.config/isq/config.toml` | Defaults, presets (personal) |
| Repo config | `.config/isq.toml` (in repo) | Worktree setup, on_start (team) |
| Cache DB | `~/.cache/isq/cache.db` | Issues, sync state |

Uses `directories::ProjectDirs` for cross-platform paths (same pattern as cache DB).

---

## Implementation Steps

### Step 1: Create User Config Module

**File:** `src/user_config.rs` (NEW)

```rust
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub presets: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Default JSON output (default: false)
    #[serde(default)]
    pub json: bool,
    /// Default sort order (default: "priority")
    pub sort: Option<String>,
    /// Default state filter (default: none, shows all)
    pub state: Option<String>,
}

/// Get user config directory
pub fn config_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Get user config file path
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Load user configuration (returns default if file missing)
pub fn load() -> Result<UserConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let config: UserConfig = toml::from_str(&content)?;
    Ok(config)
}

/// Get a preset by name (without @ prefix)
pub fn get_preset(config: &UserConfig, name: &str) -> Option<&String> {
    config.presets.get(name)
}
```

---

### Step 2: Add Preset Expansion to CLI

**File:** `src/cli/args.rs`

Add a positional argument to `IssueCommands::List` for preset:

```rust
List {
    /// Preset name (e.g., @bugs expands to saved filter)
    #[arg(value_parser = parse_preset)]
    preset: Option<String>,

    // ... existing args ...
}

/// Parse @preset syntax, stripping the @ prefix
fn parse_preset(s: &str) -> Result<String, String> {
    if s.starts_with('@') {
        Ok(s[1..].to_string())
    } else {
        Err(format!("Preset must start with @, got: {}", s))
    }
}
```

---

### Step 3: Implement Preset Expansion Logic

**File:** `src/cli/preset.rs` (NEW)

```rust
use crate::user_config::UserConfig;
use anyhow::{bail, Result};
use std::collections::HashMap;

/// Parsed filter options from a preset string
#[derive(Debug, Default)]
pub struct PresetFilters {
    pub label: Option<String>,
    pub state: Option<String>,
    pub mine: bool,
    pub unassigned: bool,
    pub goal: Option<String>,
    pub sort: Option<String>,
    pub json: bool,
}

/// Expand a preset string into filter options
pub fn expand_preset(config: &UserConfig, preset_name: &str) -> Result<PresetFilters> {
    let preset_str = config.presets.get(preset_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown preset: @{}", preset_name))?;

    parse_preset_string(preset_str)
}

/// Parse a preset string like "--label=bug --state=open --mine"
fn parse_preset_string(s: &str) -> Result<PresetFilters> {
    let mut filters = PresetFilters::default();

    let args: Vec<&str> = s.split_whitespace().collect();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if let Some(value) = arg.strip_prefix("--label=") {
            filters.label = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--state=") {
            filters.state = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--goal=") {
            filters.goal = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--sort=") {
            filters.sort = Some(value.to_string());
        } else if arg == "--mine" {
            filters.mine = true;
        } else if arg == "--unassigned" {
            filters.unassigned = true;
        } else if arg == "--json" {
            filters.json = true;
        } else if arg == "--open" {
            filters.state = Some("open".to_string());
        } else {
            bail!("Unknown preset flag: {}", arg);
        }
        i += 1;
    }

    Ok(filters)
}

/// Merge CLI args with preset and defaults. Priority: CLI > preset > defaults
pub fn merge_filters(
    cli_label: Option<&str>,
    cli_state: Option<&str>,
    cli_mine: bool,
    cli_unassigned: bool,
    cli_goal: Option<&str>,
    cli_sort: &str,
    cli_json: bool,
    preset: Option<PresetFilters>,
    defaults: &crate::user_config::Defaults,
) -> PresetFilters {
    let preset = preset.unwrap_or_default();

    PresetFilters {
        label: cli_label.map(String::from).or(preset.label),
        state: cli_state.map(String::from)
            .or(preset.state)
            .or(defaults.state.clone()),
        mine: cli_mine || preset.mine,
        unassigned: cli_unassigned || preset.unassigned,
        goal: cli_goal.map(String::from).or(preset.goal),
        sort: if cli_sort != "priority" {
            Some(cli_sort.to_string())
        } else {
            preset.sort.or(defaults.sort.clone())
        },
        json: cli_json || preset.json || defaults.json,
    }
}
```

---

### Step 4: Update Issue List Command

**File:** `src/cli/issues.rs`

Modify `cmd_list` to:
1. Accept user config
2. Expand preset if provided
3. Merge with CLI args and defaults

```rust
pub async fn cmd_list(
    preset_name: Option<&str>,  // NEW
    id: Option<&str>,
    label: Option<&str>,
    state: Option<&str>,
    mine: bool,
    unassigned: bool,
    open: bool,
    goal: Option<&str>,
    sort: &str,
    opt: &[String],
    json_output: bool,
    user_config: &UserConfig,  // NEW
) -> Result<()> {
    // Expand preset if provided
    let preset_filters = match preset_name {
        Some(name) => Some(preset::expand_preset(user_config, name)?),
        None => None,
    };

    // Merge: CLI > preset > defaults
    let effective_state = if open { Some("open") } else { state };
    let filters = preset::merge_filters(
        label,
        effective_state,
        mine,
        unassigned,
        goal,
        sort,
        json_output,
        preset_filters,
        &user_config.defaults,
    );

    // Use filters.* instead of raw CLI args
    // ...
}
```

---

### Step 5: Add `--list-presets` Flag

**File:** `src/cli/args.rs`

```rust
List {
    /// List available presets and exit
    #[arg(long)]
    list_presets: bool,

    // ... rest ...
}
```

**File:** `src/cli/issues.rs`

```rust
if list_presets {
    if user_config.presets.is_empty() {
        println!("No presets defined.");
        println!("\nAdd presets to ~/.config/isq/config.toml:");
        println!("  [presets]");
        println!("  bugs = \"--label=bug --state=open --mine\"");
    } else {
        println!("Available presets:\n");
        for (name, expansion) in &user_config.presets {
            println!("  @{:<12} {}", name, expansion);
        }
    }
    return Ok(());
}
```

---

### Step 6: Update main.rs

**File:** `src/main.rs`

Load config once at startup, pass to commands:

```rust
mod user_config;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load user config (errors logged, falls back to default)
    let user_config = user_config::load().unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load config: {}", e);
        user_config::UserConfig::default()
    });

    match cli.command {
        Some(Commands::Issue { command }) => match command {
            IssueCommands::List { preset, ... } => {
                cmd_list(preset.as_deref(), ..., &user_config).await?
            }
            // ...
        }
        // ...
    }
}
```

---

## File Changes Summary

| File | Change |
|------|--------|
| `src/user_config.rs` | NEW - Config loading, types |
| `src/cli/preset.rs` | NEW - Preset parsing and merging |
| `src/cli/args.rs` | Add `preset` positional arg, `--list-presets` flag |
| `src/cli/issues.rs` | Integrate preset expansion and defaults |
| `src/cli/mod.rs` | Add `pub mod preset;` |
| `src/main.rs` | Load user config, pass to commands |
| `src/lib.rs` | Add `pub mod user_config;` |

---

## Merge Priority

**CLI args always win**, then preset, then defaults:

```
isq issue list @bugs --state=closed
                      ^^^^^^^^^^^^^ CLI wins (overrides preset's --state=open)
               ^^^^^ preset provides --label=bug --mine
                     defaults provide json=true if configured
```

---

## Testing Strategy

1. **Unit tests (user_config.rs):**
   - Parse valid config
   - Missing file returns defaults
   - Invalid TOML logs warning, returns defaults

2. **Unit tests (preset.rs):**
   - Parse preset string
   - Merge priority (CLI > preset > defaults)
   - Unknown preset errors

3. **Integration tests:**
   - `isq issue list @preset` works
   - `isq issue list --list-presets` shows presets
   - Defaults apply when no flags

---

## Edge Cases

| Case | Behavior |
|------|----------|
| Unknown preset `@foo` | Error: "Unknown preset: @foo" |
| Empty presets section | `--list-presets` shows help message |
| Config parse error | Log warning, use empty defaults |
| No config file | Use empty defaults (no presets, json=false) |
| Preset has invalid flag | Error when expanding preset |

---

## Step 7: Documentation Updates

Per strategy: "AI-agent native" — agents are first-class users. Documentation must teach both humans AND LLMs.

### 7a: Update SKILL.md (LLM Education)

**File:** `skills/isq/SKILL.md`

Add new section after "List Issues":

```markdown
### Filter Presets

Users can define named filter presets in `~/.config/isq/config.toml`:

```toml
[presets]
bugs = "--label=bug --state=open --mine"
review = "--label=needs-review --unassigned"
p0 = "--label=P0 --state=open"
```

Use presets with the `@` prefix:

```bash
isq issue list @bugs           # Expands to: --label=bug --state=open --mine
isq issue list @review         # Expands to: --label=needs-review --unassigned
isq issue list @p0 --sort=newest  # CLI flags override/merge with preset
isq issue list --list-presets  # Show available presets
```

**Merge priority:** CLI args > preset > user defaults

### User Defaults

Users can set defaults in `~/.config/isq/config.toml`:

```toml
[defaults]
json = true        # Always output JSON
sort = "priority"  # Default sort order
state = "open"     # Default state filter
```

Defaults apply when flags aren't explicitly provided.
```

Update Command Reference table:

```markdown
| `isq issue list @preset` | Expand named filter preset |
| `isq issue list --list-presets` | Show available presets |
```

Add to Guidance section:

```markdown
- **Use presets** when the user has common filter patterns—check `isq issue list --list-presets` first
- **Don't assume presets exist**—they're user-defined. Check before using.
- **Presets are personal**—defined in user's home directory, not repo
```

Add new Common Workflow:

```markdown
### Using Presets
```bash
# Check what presets the user has defined
isq issue list --list-presets

# If @bugs preset exists, use it
isq issue list @bugs

# If no presets, help user create one
# Edit ~/.config/isq/config.toml:
# [presets]
# bugs = "--label=bug --state=open --mine"
```
```

---

### 7b: Update README.md (Human Documentation)

**File:** `README.md`

Add to Configuration section:

```markdown
### User Configuration

Personal settings live in `~/.config/isq/config.toml`:

```toml
[defaults]
json = true        # Always output JSON
sort = "priority"  # Default sort order

[presets]
bugs = "--label=bug --state=open --mine"
review = "--label=needs-review --unassigned"
```

Use presets with `@`:

```bash
isq issue list @bugs           # Expands preset
isq issue list --list-presets  # Show available
```
```

Update Commands table:

```markdown
| `isq issue list @preset` | Use a named filter preset |
| `isq issue list --list-presets` | List available presets |
```

---

### 7c: File Changes (Documentation)

| File | Change |
|------|--------|
| `skills/isq/SKILL.md` | Add presets section, update guidance, add workflow |
| `README.md` | Add user config section, update commands table |

---

## Summary: All File Changes

### Code (7 files)

| File | Change |
|------|--------|
| `src/user_config.rs` | NEW - Config loading, types |
| `src/cli/preset.rs` | NEW - Preset parsing and merging |
| `src/cli/args.rs` | Add `preset` positional arg, `--list-presets` flag |
| `src/cli/issues.rs` | Integrate preset expansion and defaults |
| `src/cli/mod.rs` | Add `pub mod preset;` |
| `src/main.rs` | Load user config, pass to commands |
| `src/lib.rs` | Add `pub mod user_config;` |

### Documentation (2 files)

| File | Change |
|------|--------|
| `skills/isq/SKILL.md` | Teach LLMs about presets and defaults |
| `README.md` | Document user config for humans |

---

## Out of Scope (Future)

- `-R/--repo` flag and `[aliases]` — deferred to multi-repo milestone
- `isq config` subcommand for managing config
- Preset inheritance (`@bugs` extends `@open`)
- Per-repo presets (team-shared filters)
