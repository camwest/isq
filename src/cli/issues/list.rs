//! Issue list command

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use anyhow::Result;

use crate::config;
use crate::db::{self, ChildProgress};
use crate::forges::{get_forge_for_repo, not_linked_error};
use crate::repo;

use super::print_issues;

#[allow(clippy::too_many_arguments)]
pub async fn cmd_list(
    view: Option<String>,
    id: Option<String>,
    label: Option<String>,
    state: Option<String>,
    all: bool,
    mine: bool,
    unassigned: bool,
    _open: bool,
    goal: Option<String>,
    sort: String,
    tree: bool,
    flat: bool,
    root_only: bool,
    children_of: Option<String>,
    opts: Vec<String>,
    json_output: bool,
) -> Result<()> {
    // Load user config for views and defaults
    let user_config = crate::user_config::load()?;
    // Apply json default from user config (CLI flag overrides)
    let json_output = json_output || user_config.defaults.json;

    // Expand view if specified, merging with CLI args (CLI wins)
    // View fields that don't have CLI equivalents are passed through directly
    let (
        label,
        label_not,
        label_any,
        state,
        mine,
        unassigned,
        goal,
        sort,
        priority,
        priority_lte,
        priority_gte,
        updated_before,
        updated_after,
        created_before,
        created_after,
    ) = if let Some(ref view_name) = view {
        let view_def = user_config.views.get(view_name).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown view: @{}. Use 'isq view list' to see available views.",
                view_name
            )
        })?;

        // Merge: CLI args override view settings
        let merged_label = label.or_else(|| view_def.label.clone());
        let merged_label_not = view_def.label_not.clone();
        let merged_label_any = view_def.label_any.clone();
        let merged_state = state.or_else(|| view_def.state.clone());
        let merged_mine = mine || view_def.mine;
        let merged_unassigned = unassigned || view_def.unassigned;
        let merged_goal = goal.or_else(|| view_def.goal.clone());
        let merged_sort = if sort != "priority" {
            sort // CLI provided explicit sort
        } else {
            view_def.sort.clone().unwrap_or(sort)
        };
        // Priority filters from view
        let merged_priority = view_def.priority;
        let merged_priority_lte = view_def.priority_lte;
        let merged_priority_gte = view_def.priority_gte;
        // Date filters from view
        let merged_updated_before = view_def.updated_before.clone();
        let merged_updated_after = view_def.updated_after.clone();
        let merged_created_before = view_def.created_before.clone();
        let merged_created_after = view_def.created_after.clone();

        (
            merged_label,
            merged_label_not,
            merged_label_any,
            merged_state,
            merged_mine,
            merged_unassigned,
            merged_goal,
            merged_sort,
            merged_priority,
            merged_priority_lte,
            merged_priority_gte,
            merged_updated_before,
            merged_updated_after,
            merged_created_before,
            merged_created_after,
        )
    } else {
        (
            label, None, None, state, mine, unassigned, goal, sort, None, None, None, None, None,
            None, None,
        )
    };

    // Parse forge-specific options
    let opts = crate::forges::parse_opts(&opts);
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if repo is linked
    let link = db::get_repo_link(&conn, &repo_path)?.ok_or_else(not_linked_error)?;

    // Parse IDs if provided (can be numeric like "123" or string like "DEV-123")
    let ids: Option<Vec<String>> = id.map(|s| {
        s.split(',')
            .map(|id_str| id_str.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    // Auto-sync if no cached data
    let sync_state = db::get_sync_state(&conn, &link.forge_repo)?;
    if sync_state.is_none() {
        eprintln!("No cache for {}. Syncing...", link.forge_repo);
        let (forge, _) = get_forge_for_repo(&repo_path)?;

        // Parse forge_repo to create Repo struct
        let parts: Vec<&str> = link.forge_repo.split('/').collect();
        if parts.len() == 2 {
            let repo_struct = repo::Repo {
                owner: parts[0].to_string(),
                name: parts[1].to_string(),
            };
            let issues_result = forge.list_issues(&repo_struct).await?;
            let mut issues = issues_result.items;

            // Apply priority from repo config (each forge handles its own logic)
            if let Ok(Some(config)) = config::load_repo_config(std::path::Path::new(&repo_path)) {
                forge.apply_priority_config(&mut issues, &config.priority);
            }

            db::save_issues(
                &conn,
                &link.forge_repo,
                &issues,
                true,
                issues_result.is_complete,
            )?;
            eprintln!("✓ Synced {} issues", issues.len());
        }
    }

    // Touch repo to update last_accessed for daemon priority
    db::touch_repo(&conn, &repo_path)?;

    // Default to open unless --all, --state=all, or explicit --state provided
    let state = if all || state.as_deref() == Some("all") {
        None // No state filtering
    } else if state.is_none() {
        Some("open".to_string()) // Default to open
    } else {
        state
    };

    // Determine user_name for --mine filter (matches issue.assignees)
    let user_name = if mine { link.user_name.clone() } else { None };

    // Convert ids from Vec<String> to Vec<&str> for filter
    let ids_strs: Option<Vec<&str>> = ids.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());

    // Get hierarchy info to determine if we should default to root-only
    let (child_progress, issues_with_parent): (HashMap<String, ChildProgress>, HashSet<String>) =
        db::count_children_by_parent(&conn, &link.forge_repo)?;
    let has_hierarchy = !child_progress.is_empty();

    // Determine effective root_only:
    // - Explicit --root-only always wins
    // - Explicit --flat forces flat view
    // - --children-of implies we want children, not root-only
    // - --tree shows all issues in tree format
    // - Default: root-only when hierarchy exists (Linear-style)
    let effective_root_only = if root_only {
        true
    } else if flat || children_of.is_some() || tree {
        false
    } else {
        has_hierarchy // Default to root-only when hierarchy exists
    };

    // Build the filter struct with all parameters
    let filter = db::IssueFilter {
        ids: ids_strs.as_deref(),
        label: label.as_deref(),
        label_not: label_not.as_deref(),
        label_any: label_any.as_deref(),
        state: state.as_deref(),
        assignee: user_name.as_deref(),
        unassigned,
        goal: goal.as_deref(),
        priority,
        priority_lte,
        priority_gte,
        updated_before: updated_before.as_deref(),
        updated_after: updated_after.as_deref(),
        created_before: created_before.as_deref(),
        created_after: created_after.as_deref(),
        sort: &sort,
        root_only: effective_root_only,
        children_of: children_of.as_deref(),
    };

    // Check for forge-specific query options (e.g., JQL for JIRA)
    let issues = if !opts.is_empty() {
        let (forge, _) = get_forge_for_repo(&repo_path)?;
        let parts: Vec<&str> = link.forge_repo.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
        }
        let repo_struct = repo::Repo {
            owner: parts[0].to_string(),
            name: parts[1].to_string(),
        };

        // Try forge-specific query
        if let Some(issues) = forge.query_issues_with_opts(&repo_struct, &opts).await? {
            // Direct API query - apply local filters too
            let mut filtered = issues;
            if let Some(ref label_filter) = label {
                filtered.retain(|i| i.labels.iter().any(|l| l.name == *label_filter));
            }
            if let Some(ref label_not_filter) = label_not {
                filtered.retain(|i| !i.labels.iter().any(|l| l.name == *label_not_filter));
            }
            if let Some(ref state_filter) = state {
                filtered.retain(|i| i.state == *state_filter);
            }
            if mine && let Some(ref username) = link.user_name {
                filtered.retain(|i| i.assignees.iter().any(|a| a == username));
            }
            if unassigned {
                filtered.retain(|i| i.assignees.is_empty());
            }
            // Apply priority filters
            if let Some(p) = priority {
                filtered.retain(|i| i.priority == p);
            }
            if let Some(p) = priority_lte {
                filtered.retain(|i| i.priority <= p);
            }
            if let Some(p) = priority_gte {
                filtered.retain(|i| i.priority >= p);
            }
            filtered
        } else {
            // Forge doesn't handle these opts, fall back to cache with full filter
            db::load_issues_with_filter(&conn, &link.forge_repo, &filter)?
        }
    } else {
        // No opts, use normal cache path with full filter
        db::load_issues_with_filter(&conn, &link.forge_repo, &filter)?
    };
    let comment_counts = db::count_comments_by_issue(&conn, &link.forge_repo)?;
    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else if tree {
        super::print_issues_tree(&issues, &comment_counts, &child_progress);
        print_footer(
            issues.len(),
            elapsed.as_millis(),
            has_hierarchy,
            false,
            false,
        );
    } else {
        print_issues(
            &issues,
            &comment_counts,
            &child_progress,
            &issues_with_parent,
        );
        let show_tree_hint = has_hierarchy && effective_root_only;
        let show_flat_hint = has_hierarchy && effective_root_only;
        print_footer(
            issues.len(),
            elapsed.as_millis(),
            has_hierarchy,
            show_tree_hint,
            show_flat_hint,
        );
    }

    Ok(())
}

/// Print the footer with issue count, timing, and optional hierarchy hints
fn print_footer(
    count: usize,
    elapsed_ms: u128,
    has_hierarchy: bool,
    show_tree_hint: bool,
    show_flat_hint: bool,
) {
    use colored::Colorize;
    use std::io::IsTerminal;

    let tty = std::io::stderr().is_terminal();

    // Build hint parts
    let mut hints = Vec::new();
    if show_tree_hint {
        hints.push("--tree");
    }
    if show_flat_hint {
        hints.push("--flat");
    }

    let hint_str = if !hints.is_empty() && has_hierarchy {
        format!(" ({})", hints.join(", "))
    } else {
        String::new()
    };

    if tty {
        eprintln!(
            "\n{}",
            format!("{} issues in {:.0}ms{}", count, elapsed_ms, hint_str).dimmed()
        );
    } else {
        eprintln!("\n{} issues in {:.0}ms{}", count, elapsed_ms, hint_str);
    }
}
