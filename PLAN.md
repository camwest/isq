# Implementation Plan: Filter Presets & User Configuration

**Closes:** #46 (filter presets)
**Related:** #38 (descoped, closed)

## Problem Statement

Power users run identical filter combinations repeatedly ("my open bugs", "needs review"). Currently they must retype flags each time or create shell aliases outside isq.

## Solution

Named filter presets managed via CLI, stored in portable TOML config:

```bash
# Create a preset
isq preset create bugs --label=bug --state=open --mine

# List presets
isq preset list

# Use preset
isq issue list @bugs

# Inspect preset
isq preset show bugs

# Delete preset
isq preset delete bugs
```

Stored in `~/.config/isq/config.toml`:

```toml
[presets.bugs]
label = "bug"
state = "open"
mine = true

[presets.stale]
state = "open"
updated_before = "30 days"

[presets.urgent]
priority_lte = 1
label_not = "wontfix"
```

---

## Design Decisions

### Why CLI commands (not just file editing)?

Per strategy: "AI agents are the primary interface." Agents run commands, not edit files.

```bash
# Agent-friendly
isq preset create bugs --label=bug --state=open --mine

# vs. requiring file editing (agent-hostile)
echo '[presets.bugs]\nlabel = "bug"' >> ~/.config/isq/config.toml
```

### Why TOML file (not SQLite)?

| Storage | Portable | Agent-friendly |
|---------|----------|----------------|
| SQLite table | No (local DB only) | Yes |
| TOML config | Yes (dotfiles sync) | Yes (via CLI) |

Presets are personal workflow shortcuts. Users expect config to travel across machines via dotfiles.

### Why structured TOML (not flag strings)?

We query SQLite directly. Structured config maps cleanly to SQL:

```toml
# Structured (maps to SQL)
[presets.bugs]
label = "bug"
state = "open"
priority_lte = 1

# vs. flag string (requires parsing)
bugs = "--label=bug --state=open"
```

Structured format enables:
- Rich operators (`label_not`, `priority_lte`, `updated_before`)
- Validation at parse time
- Direct SQL generation

### SQLite Views Considered

We evaluated SQLite views (`CREATE VIEW preset_bugs AS ...`):

| Approach | Pros | Cons |
|----------|------|------|
| SQLite views | Full SQL power, DB-optimized | Can't parameterize `repo`, `@me` varies per forge, not portable |
| TOML → SQL | Portable, agent-friendly CLI, validates input | Parse + generate SQL at runtime |

**Decision:** TOML config with SQL generation. The `repo` parameter and `@me` resolution require runtime context that views can't provide.

---

## Filter Operators

Leverage SQLite's query power:

| Config Key | SQL Generated | Example |
|------------|---------------|---------|
| `state = "open"` | `state = 'open'` | Open issues |
| `label = "bug"` | `labels LIKE '%"bug"%'` | Has label |
| `label_not = "wontfix"` | `labels NOT LIKE '%"wontfix"%'` | Excludes label |
| `label_any = ["bug", "defect"]` | `(labels LIKE '%"bug"%' OR labels LIKE '%"defect"%')` | Any of labels |
| `mine = true` | `assignees LIKE '%"username"%'` | Assigned to me |
| `unassigned = true` | `assignees = '[]'` | No assignee |
| `priority = 1` | `priority = 1` | Exact priority |
| `priority_lte = 1` | `priority <= 1` | Urgent + high |
| `priority_gte = 2` | `priority >= 2` | Medium or lower |
| `goal = "v1"` | `milestone = 'v1'` | In milestone |
| `updated_before = "30 days"` | `updated_at < datetime('now', '-30 days')` | Stale issues |
| `updated_after = "7 days"` | `updated_at > datetime('now', '-7 days')` | Recent activity |
| `created_before = "2024-01-01"` | `created_at < '2024-01-01'` | Absolute date |

All filters combine with AND. OR logic deferred to future.

---

## Implementation Steps

### Step 1: Create User Config Module

**File:** `src/user_config.rs` (NEW)

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub presets: HashMap<String, Preset>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default)]
    pub json: bool,
    pub sort: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub label: Option<String>,
    pub label_not: Option<String>,
    pub label_any: Option<Vec<String>>,
    pub state: Option<String>,
    #[serde(default)]
    pub mine: bool,
    #[serde(default)]
    pub unassigned: bool,
    pub goal: Option<String>,
    pub priority: Option<u8>,
    pub priority_lte: Option<u8>,
    pub priority_gte: Option<u8>,
    pub updated_before: Option<String>,
    pub updated_after: Option<String>,
    pub created_before: Option<String>,
    pub created_after: Option<String>,
    pub sort: Option<String>,
}

pub fn config_dir() -> Result<PathBuf> { ... }
pub fn config_path() -> Result<PathBuf> { ... }
pub fn load() -> Result<UserConfig> { ... }
pub fn save(config: &UserConfig) -> Result<()> { ... }  // NEW: for preset commands
```

---

### Step 2: Add Preset Subcommand

**File:** `src/cli/args.rs`

```rust
#[derive(Subcommand)]
pub enum Commands {
    // ... existing commands ...

    /// Manage filter presets
    Preset {
        #[command(subcommand)]
        command: PresetCommands,
    },
}

#[derive(Subcommand)]
pub enum PresetCommands {
    /// Create a new preset
    Create {
        /// Preset name
        name: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        label_not: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        mine: bool,
        #[arg(long)]
        unassigned: bool,
        #[arg(long)]
        goal: Option<String>,
        #[arg(long)]
        priority: Option<u8>,
        #[arg(long)]
        priority_lte: Option<u8>,
        #[arg(long)]
        updated_before: Option<String>,
        #[arg(long)]
        sort: Option<String>,
    },

    /// List all presets
    List {
        #[arg(long)]
        json: bool,
    },

    /// Show preset details
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },

    /// Delete a preset
    Delete {
        name: String,
    },
}
```

---

### Step 3: Implement Preset Commands

**File:** `src/cli/presets.rs` (NEW)

```rust
pub async fn cmd_create(name: &str, preset: Preset) -> Result<()> {
    let mut config = user_config::load()?;
    config.presets.insert(name.to_string(), preset);
    user_config::save(&config)?;
    println!("Created preset @{}", name);
    Ok(())
}

pub async fn cmd_list(json: bool) -> Result<()> {
    let config = user_config::load()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&config.presets)?);
    } else {
        for (name, preset) in &config.presets {
            println!("@{}: {}", name, preset.to_filter_string());
        }
    }
    Ok(())
}

pub async fn cmd_show(name: &str, json: bool) -> Result<()> { ... }
pub async fn cmd_delete(name: &str) -> Result<()> { ... }
```

---

### Step 4: SQL Generation from Presets

**File:** `src/db/filters.rs` (NEW)

```rust
use crate::user_config::Preset;

pub struct SqlFilter {
    pub where_clause: String,
    pub params: Vec<Box<dyn rusqlite::ToSql>>,
}

/// Generate SQL WHERE clause from preset
pub fn preset_to_sql(preset: &Preset, username: Option<&str>) -> SqlFilter {
    let mut conditions = vec!["deleted = 0".to_string()];
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![];

    if let Some(label) = &preset.label {
        conditions.push("labels LIKE ?".to_string());
        params.push(Box::new(format!("%\"{}\"&", label)));
    }

    if let Some(label) = &preset.label_not {
        conditions.push("labels NOT LIKE ?".to_string());
        params.push(Box::new(format!("%\"{}\"&", label)));
    }

    if let Some(state) = &preset.state {
        conditions.push("state = ?".to_string());
        params.push(Box::new(state.clone()));
    }

    if preset.mine {
        if let Some(user) = username {
            conditions.push("assignees LIKE ?".to_string());
            params.push(Box::new(format!("%\"{}\"&", user)));
        }
    }

    if preset.unassigned {
        conditions.push("(assignees = '[]' OR assignees IS NULL)".to_string());
    }

    if let Some(p) = preset.priority_lte {
        conditions.push("priority <= ?".to_string());
        params.push(Box::new(p as i64));
    }

    if let Some(days) = &preset.updated_before {
        // Parse "30 days" -> "-30 days"
        conditions.push("updated_at < datetime('now', ?)".to_string());
        params.push(Box::new(format!("-{}", days)));
    }

    // ... other operators

    SqlFilter {
        where_clause: conditions.join(" AND "),
        params,
    }
}
```

---

### Step 5: Update Issue List to Use Presets

**File:** `src/cli/issues.rs`

```rust
pub async fn cmd_list(
    preset_name: Option<&str>,
    // ... existing filter args ...
    user_config: &UserConfig,
) -> Result<()> {
    // Load preset if specified
    let preset = match preset_name {
        Some(name) => {
            user_config.presets.get(name)
                .ok_or_else(|| anyhow!("Unknown preset: @{}", name))?
                .clone()
        }
        None => Preset::default(),
    };

    // Merge CLI args over preset (CLI wins)
    let effective = merge_preset_with_cli(preset, cli_args);

    // Generate SQL and query
    let sql_filter = filters::preset_to_sql(&effective, username.as_deref());
    let issues = db::load_issues_with_filter(conn, repo, &sql_filter)?;

    // Apply json default
    let json_output = cli_json || user_config.defaults.json;

    // ... render output
}
```

---

### Step 6: Update main.rs

```rust
match cli.command {
    Some(Commands::Preset { command }) => match command {
        PresetCommands::Create { name, label, ... } => {
            let preset = Preset { label, state, mine, ... };
            cmd_preset_create(&name, preset).await?
        }
        PresetCommands::List { json } => cmd_preset_list(json).await?,
        PresetCommands::Show { name, json } => cmd_preset_show(&name, json).await?,
        PresetCommands::Delete { name } => cmd_preset_delete(&name).await?,
    },
    Some(Commands::Issue { command }) => match command {
        IssueCommands::List { preset, ... } => {
            cmd_list(preset.as_deref(), ..., &user_config).await?
        }
        // ...
    },
    // ...
}
```

---

## Step 7: Apply Defaults Globally

The `[defaults].json` setting should apply to ALL commands with `--json` flag.

**Affected commands (15+):**
- `issue list/show/create/comment/close/reopen/label/assign`
- `goal list/show/create/assign/close`
- `label list/create`
- `status`

**Implementation:** Thread `user_config.defaults.json` through each command, use as fallback when `--json` not explicitly set.

---

## Step 8: Documentation Updates

### 8a: Update SKILL.md (LLM Education)

**File:** `skills/isq/SKILL.md`

Add sections:

```markdown
### Filter Presets

Create and use named filter presets:

```bash
# Create a preset
isq preset create bugs --label=bug --state=open --mine
isq preset create urgent --priority-lte=1 --state=open
isq preset create stale --state=open --updated-before="30 days"

# List presets
isq preset list

# Use a preset
isq issue list @bugs
isq issue list @urgent --sort=newest  # CLI flags merge with preset

# Inspect preset
isq preset show bugs

# Delete preset
isq preset delete bugs
```

**Available filters:**
- `--label`, `--label-not` — include/exclude label
- `--state` — open, closed
- `--mine`, `--unassigned` — assignment filters
- `--priority`, `--priority-lte`, `--priority-gte` — priority filters
- `--goal` — milestone/project
- `--updated-before`, `--updated-after` — recency filters
- `--sort` — priority, newest, oldest, updated

**Merge priority:** CLI args > preset > user defaults
```

Add to Guidance:

```markdown
- **Create presets for users** when they have repeated filter patterns
- **Use `isq preset list`** to discover existing presets before creating new ones
- **Presets are portable** — stored in ~/.config/isq/config.toml, syncs via dotfiles
```

### 8b: Update README.md

Add to Configuration section with preset examples and command reference.

---

## File Changes Summary

### Code (8 files)

| File | Change |
|------|--------|
| `src/user_config.rs` | NEW - Config types, load/save |
| `src/db/filters.rs` | NEW - Preset → SQL generation |
| `src/cli/presets.rs` | NEW - preset create/list/show/delete |
| `src/cli/args.rs` | Add Preset subcommand, @preset syntax |
| `src/cli/issues.rs` | Integrate preset loading and merging |
| `src/cli/mod.rs` | Add `pub mod presets;` |
| `src/main.rs` | Load user config, route preset commands |
| `src/lib.rs` | Add `pub mod user_config;` |

### Documentation (2 files)

| File | Change |
|------|--------|
| `skills/isq/SKILL.md` | Preset commands, filter options, guidance |
| `README.md` | User config section, preset examples |

---

## Testing Strategy

1. **Unit tests (user_config.rs):** Parse, save, round-trip
2. **Unit tests (filters.rs):** Preset → SQL generation for each operator
3. **Integration tests:** `isq preset create` → `isq issue list @preset` → correct results
4. **CLI tests:** Create, list, show, delete workflows

---

## Edge Cases

| Case | Behavior |
|------|----------|
| Unknown preset `@foo` | Error: "Unknown preset: @foo" |
| No presets defined | `isq preset list` shows empty, suggests creating one |
| Preset name collision | `isq preset create` overwrites with warning |
| Invalid filter combo | Validate at create time, error with guidance |
| `@me` without linked repo | Error: "No repository linked" |

---

## Out of Scope (Future)

- `-R/--repo` flag and `[aliases]` — deferred to multi-repo milestone
- OR logic in filters (`--label=bug OR --label=defect`)
- Preset inheritance (`@bugs` extends `@open`)
- Per-repo presets (team-shared filters in `.config/isq.toml`)
- `isq preset edit` — interactive editing
