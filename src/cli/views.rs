//! View management commands
//!
//! Handles creation, listing, showing, and deletion of custom views.

use anyhow::{anyhow, Result};

use crate::user_config::{self, View};

/// Create a new view
pub fn cmd_create(
    name: String,
    label: Option<String>,
    label_not: Option<String>,
    state: Option<String>,
    mine: bool,
    unassigned: bool,
    goal: Option<String>,
    priority: Option<u8>,
    priority_lte: Option<u8>,
    priority_gte: Option<u8>,
    updated_before: Option<String>,
    updated_after: Option<String>,
    sort: Option<String>,
) -> Result<()> {
    let view = View {
        label,
        label_not,
        label_any: None,
        state,
        mine,
        unassigned,
        goal,
        priority,
        priority_lte,
        priority_gte,
        updated_before,
        updated_after,
        created_before: None,
        created_after: None,
        sort,
    };

    if view.is_empty() {
        return Err(anyhow!(
            "View must have at least one filter. Use --label, --state, --mine, etc."
        ));
    }

    let mut config = user_config::load()?;
    let existed = config.views.contains_key(&name);
    config.views.insert(name.clone(), view.clone());
    user_config::save(&config)?;

    if existed {
        println!("Updated view @{}", name);
    } else {
        println!("Created view @{}", name);
    }
    println!("  {}", view.to_filter_string());

    Ok(())
}

/// List all views
pub fn cmd_list(json: bool) -> Result<()> {
    let config = user_config::load()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&config.views)?);
        return Ok(());
    }

    if config.views.is_empty() {
        println!("No views defined.\n");
        println!("Create a view:");
        println!("  isq view create bugs --label=bug --state=open --mine\n");
        println!("Use a view:");
        println!("  isq issue list @bugs");
        return Ok(());
    }

    println!("Views:\n");
    let mut views: Vec<_> = config.views.iter().collect();
    views.sort_by_key(|(name, _)| *name);

    for (name, view) in views {
        println!("  @{:<15} {}", name, view.to_filter_string());
    }

    Ok(())
}

/// Show view details
pub fn cmd_show(name: &str, json: bool) -> Result<()> {
    let config = user_config::load()?;

    let view = config
        .views
        .get(name)
        .ok_or_else(|| anyhow!("Unknown view: @{}", name))?;

    if json {
        println!("{}", serde_json::to_string_pretty(view)?);
    } else {
        println!("@{}", name);
        println!("  {}", view.to_filter_string());
    }

    Ok(())
}

/// Delete a view
pub fn cmd_delete(name: &str) -> Result<()> {
    let mut config = user_config::load()?;

    if !config.views.contains_key(name) {
        return Err(anyhow!("Unknown view: @{}", name));
    }

    config.views.remove(name);
    user_config::save(&config)?;

    println!("Deleted view @{}", name);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_empty_view_fails() {
        // This test verifies validation logic without file I/O
        let view = View::default();
        assert!(view.is_empty());

        // Manually testing the validation that would happen in cmd_create
        // (cmd_create itself requires filesystem access)
    }

    #[test]
    fn test_view_building() {
        let view = View {
            label: Some("bug".to_string()),
            label_not: None,
            label_any: None,
            state: Some("open".to_string()),
            mine: true,
            unassigned: false,
            goal: None,
            priority: None,
            priority_lte: None,
            priority_gte: None,
            updated_before: None,
            updated_after: None,
            created_before: None,
            created_after: None,
            sort: None,
        };

        assert!(!view.is_empty());
        let filter_str = view.to_filter_string();
        assert!(filter_str.contains("--label=bug"));
        assert!(filter_str.contains("--state=open"));
        assert!(filter_str.contains("--mine"));
    }

    #[test]
    fn test_view_with_priority_filters() {
        let view = View {
            priority_lte: Some(1),
            label_not: Some("wontfix".to_string()),
            ..Default::default()
        };

        assert!(!view.is_empty());
        let filter_str = view.to_filter_string();
        assert!(filter_str.contains("--priority-lte=1"));
        assert!(filter_str.contains("--label-not=wontfix"));
    }

    #[test]
    fn test_view_with_date_filters() {
        let view = View {
            state: Some("open".to_string()),
            updated_before: Some("30 days".to_string()),
            ..Default::default()
        };

        assert!(!view.is_empty());
        let filter_str = view.to_filter_string();
        assert!(filter_str.contains("--updated-before=\"30 days\""));
    }
}
