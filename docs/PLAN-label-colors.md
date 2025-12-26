# Implementation Plan: Label Colors

Display labels in CLI with their original forge colors using background color rendering.

## Overview

- Fetch and store label colors from GitHub/Linear
- Render labels with colored backgrounds and auto-contrast text
- Graceful fallback for terminals without true color support

## Implementation Steps

### Step 1: Update Data Model

**File: `src/forges/mod.rs`**

Create a `Label` struct:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: Option<String>,  // Hex color without #, e.g., "fc2929"
}
```

Update `Issue` struct:
```rust
pub labels: Vec<Label>,  // Change from Vec<String>
```

### Step 2: Update GitHub Forge

**File: `src/forges/github.rs`**

Add color to `GitHubLabel`:
```rust
#[derive(Debug, Clone, Deserialize)]
struct GitHubLabel {
    name: String,
    color: String,  // GitHub provides hex without #
}
```

Update `into_issue()` conversion to create `Label` structs with color.

### Step 3: Update Linear Forge

**File: `src/forges/linear.rs`**

The `LinearLabel` struct already has `color`. Update the conversion in the issue mapping (around line 1104) to preserve color instead of just extracting name.

### Step 4: Update Database Layer

**File: `src/db.rs`**

Labels are stored as JSON. The format change from `["bug", "feature"]` to `[{"name": "bug", "color": "fc2929"}, ...]` will be automatic via serde.

For reading old data, add fallback parsing:
```rust
// Try new format first
let labels: Vec<Label> = serde_json::from_str(&labels_json)
    .or_else(|_| {
        // Fall back to old Vec<String> format
        let names: Vec<String> = serde_json::from_str(&labels_json)?;
        Ok(names.into_iter().map(|name| Label { name, color: None }).collect())
    })
    .unwrap_or_default();
```

### Step 5: Add Color Utilities

**File: `src/display.rs`**

Add helper functions:
```rust
/// Parse hex color string to RGB tuple
fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 { return None; }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Calculate relative luminance (0.0 = black, 1.0 = white)
fn luminance(r: u8, g: u8, b: u8) -> f64 {
    0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64)
}

/// Check if terminal supports true color
fn supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v == "truecolor" || v == "24bit")
        .unwrap_or(false)
}
```

### Step 6: Update Label Rendering

**File: `src/display.rs`**

Create a function to render a colored label:
```rust
fn render_label(label: &Label, tty: bool) -> String {
    if !tty {
        return label.name.clone();
    }

    match &label.color {
        Some(hex) if supports_truecolor() => {
            if let Some((r, g, b)) = parse_hex_color(hex) {
                // Choose black or white text based on background luminance
                let lum = luminance(r, g, b);
                if lum > 127.5 {
                    // Light background -> black text
                    format!(" {} ", label.name)
                        .on_truecolor(r, g, b)
                        .truecolor(0, 0, 0)
                        .to_string()
                } else {
                    // Dark background -> white text
                    format!(" {} ", label.name)
                        .on_truecolor(r, g, b)
                        .truecolor(255, 255, 255)
                        .to_string()
                }
            } else {
                label.name.yellow().to_string()  // Invalid hex, fallback
            }
        }
        _ => label.name.yellow().to_string(),  // No color or no truecolor support
    }
}
```

Update `print_issue_row()` and `print_issue()` to use this function.

### Step 7: Update Label Filtering

**File: `src/db.rs`**

The LIKE query for label filtering should still work since it searches for the label name within the JSON string. May need minor adjustment if the JSON structure breaks the pattern.

### Step 8: Update CLI Label Commands

**File: `src/main.rs`**

The `label add/remove` commands work with label names, so they should continue to work without changes. The forge APIs accept label names for add/remove operations.

## Files Changed Summary

| File | Changes |
|------|---------|
| `src/forges/mod.rs` | Add `Label` struct, update `Issue.labels` type |
| `src/forges/github.rs` | Add `color` to `GitHubLabel`, update conversion |
| `src/forges/linear.rs` | Preserve color in issue conversion |
| `src/db.rs` | Update label serialization with backward compat |
| `src/display.rs` | Add color utils, update label rendering |

## Testing

1. Verify labels sync with colors from GitHub
2. Verify labels sync with colors from Linear
3. Test display on dark terminal
4. Test display on light terminal
5. Test fallback when `$COLORTERM` is not set
6. Test with old database format (name-only labels)

## Migration

User can delete `~/.local/share/isq/` to force fresh sync with colors, or the backward-compatible parsing will handle old data gracefully (labels without colors render in yellow).
