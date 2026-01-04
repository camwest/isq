# Implementation Plan: Custom Views & User Configuration

**Closes:** #46 (filter presets)
**Related:** #38 (descoped, closed)

## Problem Statement

Power users run identical filter combinations repeatedly ("my open bugs", "needs review"). Currently they must retype flags each time or create shell aliases outside isq.

## Solution

Named views managed via CLI, stored in portable TOML config:

```bash
# Create a view
isq view create bugs --label=bug --state=open --mine

# List views
isq view list

# Use view
isq issue list @bugs

# Inspect view
isq view show bugs

# Delete view
isq view delete bugs
```

Stored in `~/.config/isq/config.toml`:

```toml
[views.bugs]
label = "bug"
state = "open"
mine = true

[views.stale]
state = "open"
updated_before = "30 days"

[views.urgent]
priority_lte = 1
label_not = "wontfix"
```

---

## Design Decisions

### Why CLI commands (not just file editing)?

Per strategy: "AI agents are the primary interface." Agents run commands, not edit files.

```bash
# Agent-friendly
isq view create bugs --label=bug --state=open --mine

# vs. requiring file editing (agent-hostile)
echo '[views.bugs]\nlabel = "bug"' >> ~/.config/isq/config.toml
```

### Why TOML file (not SQLite)?

| Storage | Portable | Agent-friendly |
|---------|----------|----------------|
| SQLite table | No (local DB only) | Yes |
| TOML config | Yes (dotfiles sync) | Yes (via CLI) |

Views are personal workflow shortcuts. Users expect config to travel across machines via dotfiles.

### Why structured TOML (not flag strings)?

We query SQLite directly. Structured config maps cleanly to SQL:

```toml
# Structured (maps to SQL)
[views.bugs]
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

We evaluated SQLite views (`CREATE VIEW view_bugs AS ...`):

| Approach | Pros | Cons |
|----------|------|------|
| SQLite views | Full SQL power, DB-optimized | Can't parameterize `repo`, `@me` varies per forge, not portable |
| TOML → SQL | Portable, agent-friendly CLI, validates input | Parse + generate SQL at runtime |

**Decision:** TOML config with SQL generation. The `repo` parameter and `@me` resolution require runtime context that SQLite views can't provide.

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
    pub views: HashMap<String, View>,
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
pub struct View {
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
pub fn save(config: &UserConfig) -> Result<()> { ... }  // NEW: for view commands
```

---

### Step 2: Add View Subcommand

**File:** `src/cli/args.rs`

```rust
#[derive(Subcommand)]
pub enum Commands {
    // ... existing commands ...

    /// Manage custom views
    View {
        #[command(subcommand)]
        command: ViewCommands,
    },
}

#[derive(Subcommand)]
pub enum ViewCommands {
    /// Create a new view
    Create {
        /// View name
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

    /// List all views
    List {
        #[arg(long)]
        json: bool,
    },

    /// Show view details
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },

    /// Delete a view
    Delete {
        name: String,
    },
}
```

---

### Step 3: Implement View Commands

**File:** `src/cli/views.rs` (NEW)

```rust
pub async fn cmd_create(name: &str, view: View) -> Result<()> {
    let mut config = user_config::load()?;
    config.views.insert(name.to_string(), view);
    user_config::save(&config)?;
    println!("Created view @{}", name);
    Ok(())
}

pub async fn cmd_list(json: bool) -> Result<()> {
    let config = user_config::load()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&config.views)?);
    } else {
        for (name, view) in &config.views {
            println!("@{}: {}", name, view.to_filter_string());
        }
    }
    Ok(())
}

pub async fn cmd_show(name: &str, json: bool) -> Result<()> { ... }
pub async fn cmd_delete(name: &str) -> Result<()> { ... }
```

---

### Step 4: SQL Generation from Views

**File:** `src/db/filters.rs` (NEW)

```rust
use crate::user_config::View;

pub struct SqlFilter {
    pub where_clause: String,
    pub params: Vec<Box<dyn rusqlite::ToSql>>,
}

/// Generate SQL WHERE clause from view
pub fn view_to_sql(view: &View, username: Option<&str>) -> SqlFilter {
    let mut conditions = vec!["deleted = 0".to_string()];
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![];

    if let Some(label) = &view.label {
        conditions.push("labels LIKE ?".to_string());
        params.push(Box::new(format!("%\"{}\"&", label)));
    }

    if let Some(label) = &view.label_not {
        conditions.push("labels NOT LIKE ?".to_string());
        params.push(Box::new(format!("%\"{}\"&", label)));
    }

    if let Some(state) = &view.state {
        conditions.push("state = ?".to_string());
        params.push(Box::new(state.clone()));
    }

    if view.mine {
        if let Some(user) = username {
            conditions.push("assignees LIKE ?".to_string());
            params.push(Box::new(format!("%\"{}\"&", user)));
        }
    }

    if view.unassigned {
        conditions.push("(assignees = '[]' OR assignees IS NULL)".to_string());
    }

    if let Some(p) = view.priority_lte {
        conditions.push("priority <= ?".to_string());
        params.push(Box::new(p as i64));
    }

    if let Some(days) = &view.updated_before {
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

### Step 5: Update Issue List to Use Views

**File:** `src/cli/issues.rs`

```rust
pub async fn cmd_list(
    view_name: Option<&str>,
    // ... existing filter args ...
    user_config: &UserConfig,
) -> Result<()> {
    // Load view if specified
    let view = match view_name {
        Some(name) => {
            user_config.views.get(name)
                .ok_or_else(|| anyhow!("Unknown view: @{}", name))?
                .clone()
        }
        None => View::default(),
    };

    // Merge CLI args over view (CLI wins)
    let effective = merge_view_with_cli(view, cli_args);

    // Generate SQL and query
    let sql_filter = filters::view_to_sql(&effective, username.as_deref());
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
    Some(Commands::View { command }) => match command {
        ViewCommands::Create { name, label, ... } => {
            let view = View { label, state, mine, ... };
            cmd_view_create(&name, view).await?
        }
        ViewCommands::List { json } => cmd_view_list(json).await?,
        ViewCommands::Show { name, json } => cmd_view_show(&name, json).await?,
        ViewCommands::Delete { name } => cmd_view_delete(&name).await?,
    },
    Some(Commands::Issue { command }) => match command {
        IssueCommands::List { view, ... } => {
            cmd_list(view.as_deref(), ..., &user_config).await?
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
### Custom Views

Create and use named filter views:

```bash
# Create a view
isq view create bugs --label=bug --state=open --mine
isq view create urgent --priority-lte=1 --state=open
isq view create stale --state=open --updated-before="30 days"

# List views
isq view list

# Use a view
isq issue list @bugs
isq issue list @urgent --sort=newest  # CLI flags merge with view

# Inspect view
isq view show bugs

# Delete view
isq view delete bugs
```

**Available filters:**
- `--label`, `--label-not` — include/exclude label
- `--state` — open, closed
- `--mine`, `--unassigned` — assignment filters
- `--priority`, `--priority-lte`, `--priority-gte` — priority filters
- `--goal` — milestone/project
- `--updated-before`, `--updated-after` — recency filters
- `--sort` — priority, newest, oldest, updated

**Merge priority:** CLI args > view > user defaults
```

Add to Guidance:

```markdown
- **Create views for users** when they have repeated filter patterns
- **Use `isq view list`** to discover existing views before creating new ones
- **Views are portable** — stored in ~/.config/isq/config.toml, syncs via dotfiles
```

### 8b: Update README.md

Add to Configuration section with view examples and command reference.

---

## File Changes Summary

### Code (8 files)

| File | Change |
|------|--------|
| `src/user_config.rs` | NEW - Config types, load/save |
| `src/db/filters.rs` | NEW - View → SQL generation |
| `src/cli/views.rs` | NEW - view create/list/show/delete |
| `src/cli/args.rs` | Add View subcommand, @view syntax |
| `src/cli/issues.rs` | Integrate view loading and merging |
| `src/cli/mod.rs` | Add `pub mod views;` |
| `src/main.rs` | Load user config, route view commands |
| `src/lib.rs` | Add `pub mod user_config;` |

### Documentation (2 files)

| File | Change |
|------|--------|
| `skills/isq/SKILL.md` | View commands, filter options, guidance |
| `README.md` | User config section, view examples |

---

## Testing Strategy

1. **Unit tests (user_config.rs):** Parse, save, round-trip
2. **Unit tests (filters.rs):** View → SQL generation for each operator
3. **Integration tests:** `isq view create` → `isq issue list @view` → correct results
4. **CLI tests:** Create, list, show, delete workflows

---

## Edge Cases

| Case | Behavior |
|------|----------|
| Unknown view `@foo` | Error: "Unknown view: @foo" |
| No views defined | `isq view list` shows empty, suggests creating one |
| View name collision | `isq view create` overwrites with warning |
| Invalid filter combo | Validate at create time, error with guidance |
| `@me` without linked repo | Error: "No repository linked" |

---

## Out of Scope (Future)

- `-R/--repo` flag and `[aliases]` — deferred to multi-repo milestone
- OR logic in filters (`--label=bug OR --label=defect`)
- View inheritance (`@bugs` extends `@open`)
- Per-repo views (team-shared filters in `.config/isq.toml`)
- `isq view edit` — interactive editing
