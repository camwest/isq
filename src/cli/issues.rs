//! Issue-related CLI commands

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;

use crate::config;
use crate::db;
use crate::display;
use crate::forges::{get_forge_for_repo, not_linked_error, CreateIssueRequest, Issue};
use crate::repo;

use super::utils::{is_offline_error, parse_issue_number, WriteResult};

pub async fn cmd_list(
    view: Option<String>,
    id: Option<String>,
    label: Option<String>,
    state: Option<String>,
    all: bool,
    mine: bool,
    unassigned: bool,
    open: bool,
    goal: Option<String>,
    sort: String,
    opts: Vec<String>,
    json_output: bool,
) -> Result<()> {
    // Load user config for views and defaults
    let user_config = crate::user_config::load()?;
    // Apply json default from user config (CLI flag overrides)
    let json_output = json_output || user_config.defaults.json;

    // Expand view if specified, merging with CLI args (CLI wins)
    // View fields that don't have CLI equivalents are passed through directly
    let (label, label_not, label_any, state, mine, unassigned, goal, sort, priority, priority_lte, priority_gte, updated_before, updated_after, created_before, created_after) =
        if let Some(ref view_name) = view {
            let view_def = user_config
                .views
                .get(view_name)
                .ok_or_else(|| anyhow::anyhow!("Unknown view: @{}. Use 'isq view list' to see available views.", view_name))?;

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

            (merged_label, merged_label_not, merged_label_any, merged_state, merged_mine, merged_unassigned,
             merged_goal, merged_sort, merged_priority, merged_priority_lte, merged_priority_gte,
             merged_updated_before, merged_updated_after, merged_created_before, merged_created_after)
        } else {
            (label, None, None, state, mine, unassigned, goal, sort, None, None, None, None, None, None, None)
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
    } else if open && state.is_none() {
        Some("open".to_string())
    } else if state.is_none() {
        Some("open".to_string()) // Default to open
    } else {
        state
    };

    // Determine user_name for --mine filter (matches issue.assignees)
    let user_name = if mine { link.user_name.clone() } else { None };

    // Convert ids from Vec<String> to Vec<&str> for filter
    let ids_strs: Option<Vec<&str>> = ids.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());

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
            if mine {
                if let Some(ref username) = link.user_name {
                    filtered.retain(|i| i.assignees.iter().any(|a| a == username));
                }
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
    } else {
        print_issues(&issues, &comment_counts);
        eprintln!("\n{} issues in {:.0}ms", issues.len(), elapsed.as_millis());
    }

    Ok(())
}

pub fn cmd_show(id: &str, json_output: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json_output = crate::user_config::resolve_json_default(json_output)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if repo is linked
    let link = db::get_repo_link(&conn, &repo_path)?.ok_or_else(not_linked_error)?;

    // For JIRA, validate issue key prefix matches linked project
    if link.forge_type == "jira" {
        let project_key = link.forge_repo.split('/').last().unwrap_or("");
        if id.contains('-') {
            let prefix = id.split('-').next().unwrap_or("");
            if !prefix.eq_ignore_ascii_case(project_key) {
                anyhow::bail!(
                    "Issue '{}' belongs to project '{}', but you're linked to '{}'.",
                    id, prefix, project_key
                );
            }
        }
    }
    let issue_id = id;

    // Touch repo to update last_accessed for daemon priority
    db::touch_repo(&conn, &repo_path)?;

    let issue = db::load_issue(&conn, &link.forge_repo, issue_id)?;
    let comments = db::load_comments(&conn, &link.forge_repo, issue_id)?;
    let elapsed = start.elapsed();

    match issue {
        Some(issue) => {
            if json_output {
                // Include comments in JSON output
                let output = serde_json::json!({
                    "issue": issue,
                    "comments": comments.iter().map(|c| {
                        serde_json::json!({
                            "id": c.comment_id,
                            "body": c.body,
                            "author": c.author,
                            "created_at": c.created_at
                        })
                    }).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                // Use styled display
                display::print_issue(&issue, &comments, elapsed.as_millis() as u64);
            }
        }
        None => {
            anyhow::bail!(
                "Issue {} not found in cache. Run `isq sync` to refresh.",
                id
            );
        }
    }

    Ok(())
}

pub async fn cmd_create(
    title: String,
    body: Option<String>,
    labels: Vec<String>,
    goal: Option<String>,
    opts: Vec<String>,
    json: bool,
    cli_quiet: bool,
) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    // Parse forge-specific options
    let opts = crate::forges::parse_opts(&opts);

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;
    let conn = db::open()?;

    // Resolve goal name to goal_id if provided
    let goal_id = if let Some(goal_name) = &goal {
        let g = db::load_goal_by_name(&conn, &link.forge_repo, goal_name)?.ok_or_else(|| {
            anyhow::anyhow!(
                "Goal '{}' not found. Run `isq sync` to refresh.",
                goal_name
            )
        })?;
        Some(g.id)
    } else {
        None
    };

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let req = CreateIssueRequest {
        title: title.clone(),
        body: body.clone(),
        labels: labels.clone(),
        goal_id: goal_id.clone(),
        opts: opts.clone(),
    };

    match forge.create_issue(&repo_struct, req).await {
        Ok(issue) => {
            let elapsed = start.elapsed();
            let issue_id_display = display::format_issue_id(&issue.id);
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(issue.id.clone()),
                    message: format!("Created {} {}", issue_id_display, issue.title),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Created {} {} ({:.0}ms)",
                    issue_id_display,
                    issue.title,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "title": title,
                "body": body,
                "labels": labels,
                "goal_id": goal_id,
            });
            db::queue_op(&conn, &link.forge_repo, "create", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: None,
                    message: format!("Queued: {}", title),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: {} (offline, {:.0}ms)",
                    title,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

pub async fn cmd_comment(id: &str, message: String, json: bool, cli_quiet: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    // For JIRA, validate issue key prefix matches linked project
    if link.forge_type == "jira" {
        let project_key = repo_struct.name.as_str();
        // Validate prefix if ID contains one
        if id.contains('-') {
            let prefix = id.split('-').next().unwrap_or("");
            if !prefix.eq_ignore_ascii_case(project_key) {
                anyhow::bail!(
                    "Issue '{}' belongs to project '{}', but you're linked to '{}'.",
                    id, prefix, project_key
                );
            }
        }
    }
    let issue_id = id;

    match forge.create_comment(&repo_struct, issue_id, &message).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            let issue_display = display::format_issue_id(issue_id);
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Comment added to {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!("✓ Comment added to {} ({:.0}ms)", issue_display, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_id": issue_id,
                "body": message,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "comment", &payload.to_string())?;
            let issue_display = display::format_issue_id(issue_id);
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Queued: comment on {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: comment on {} (offline, {:.0}ms)",
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

pub async fn cmd_close(id: &str, json: bool, cli_quiet: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    // For JIRA, validate issue key prefix matches linked project
    if link.forge_type == "jira" {
        let project_key = repo_struct.name.as_str();
        if id.contains('-') {
            let prefix = id.split('-').next().unwrap_or("");
            if !prefix.eq_ignore_ascii_case(project_key) {
                anyhow::bail!(
                    "Issue '{}' belongs to project '{}', but you're linked to '{}'.",
                    id, prefix, project_key
                );
            }
        }
    }
    let issue_id = id;
    let issue_display = display::format_issue_id(issue_id);

    match forge.close_issue(&repo_struct, issue_id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Closed {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!("✓ Closed {} ({:.0}ms)", issue_display, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({ "issue_id": issue_id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "close", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Queued: close {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: close {} (offline, {:.0}ms)",
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

pub async fn cmd_reopen(id: &str, json: bool, cli_quiet: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    // For JIRA, validate issue key prefix matches linked project
    if link.forge_type == "jira" {
        let project_key = repo_struct.name.as_str();
        if id.contains('-') {
            let prefix = id.split('-').next().unwrap_or("");
            if !prefix.eq_ignore_ascii_case(project_key) {
                anyhow::bail!(
                    "Issue '{}' belongs to project '{}', but you're linked to '{}'.",
                    id, prefix, project_key
                );
            }
        }
    }
    let issue_id = id;
    let issue_display = display::format_issue_id(issue_id);

    match forge.reopen_issue(&repo_struct, issue_id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Reopened {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!("✓ Reopened {} ({:.0}ms)", issue_display, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({ "issue_id": issue_id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "reopen", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Queued: reopen {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: reopen {} (offline, {:.0}ms)",
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

pub async fn cmd_label(id: &str, action: String, label: String, json: bool, cli_quiet: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    // For JIRA, validate issue key prefix matches linked project
    if link.forge_type == "jira" {
        let project_key = repo_struct.name.as_str();
        if id.contains('-') {
            let prefix = id.split('-').next().unwrap_or("");
            if !prefix.eq_ignore_ascii_case(project_key) {
                anyhow::bail!(
                    "Issue '{}' belongs to project '{}', but you're linked to '{}'.",
                    id, prefix, project_key
                );
            }
        }
    }
    let issue_id = id;
    let issue_display = display::format_issue_id(issue_id);

    match action.as_str() {
        "add" => {
            match forge.add_label(&repo_struct, issue_id, &label).await {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: false,
                            issue_id: Some(issue_id.to_string()),
                            message: format!("Added label '{}' to {}", label, issue_display),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else if !quiet {
                        println!(
                            "✓ Added label '{}' to {} ({:.0}ms)",
                            label,
                            issue_display,
                            elapsed.as_millis()
                        );
                    }
                }
                Err(e) if is_offline_error(&e) => {
                    let elapsed = start.elapsed();
                    let payload = serde_json::json!({
                        "issue_id": issue_id,
                        "label": label,
                    });
                    let conn = db::open()?;
                    db::queue_op(&conn, &link.forge_repo, "label_add", &payload.to_string())?;
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: true,
                            issue_id: Some(issue_id.to_string()),
                            message: format!("Queued: add label '{}' to {}", label, issue_display),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else if !quiet {
                        println!(
                            "✓ Queued: add label '{}' to {} (offline, {:.0}ms)",
                            label,
                            issue_display,
                            elapsed.as_millis()
                        );
                    }
                }
                Err(e) => return Err(e),
            }
        }
        "remove" => {
            match forge.remove_label(&repo_struct, issue_id, &label).await {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: false,
                            issue_id: Some(issue_id.to_string()),
                            message: format!("Removed label '{}' from {}", label, issue_display),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else if !quiet {
                        println!(
                            "✓ Removed label '{}' from {} ({:.0}ms)",
                            label,
                            issue_display,
                            elapsed.as_millis()
                        );
                    }
                }
                Err(e) if is_offline_error(&e) => {
                    let elapsed = start.elapsed();
                    let payload = serde_json::json!({
                        "issue_id": issue_id,
                        "label": label,
                    });
                    let conn = db::open()?;
                    db::queue_op(
                        &conn,
                        &link.forge_repo,
                        "label_remove",
                        &payload.to_string(),
                    )?;
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: true,
                            issue_id: Some(issue_id.to_string()),
                            message: format!("Queued: remove label '{}' from {}", label, issue_display),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else if !quiet {
                        println!(
                            "✓ Queued: remove label '{}' from {} (offline, {:.0}ms)",
                            label,
                            issue_display,
                            elapsed.as_millis()
                        );
                    }
                }
                Err(e) => return Err(e),
            }
        }
        _ => {
            anyhow::bail!("Invalid action '{}'. Use 'add' or 'remove'.", action);
        }
    }

    Ok(())
}

pub async fn cmd_assign(id: &str, user: String, json: bool, cli_quiet: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    // For JIRA, validate issue key prefix matches linked project
    if link.forge_type == "jira" {
        let project_key = repo_struct.name.as_str();
        // Validate the issue key prefix matches the linked project
        parse_issue_number(id, Some(project_key))?;
    }

    let issue_display = display::format_issue_id(id);

    match forge.assign_issue(&repo_struct, id, &user).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(id.to_string()),
                    message: format!("Assigned @{} to {}", user, issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Assigned @{} to {} ({:.0}ms)",
                    user,
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_id": id,
                "assignee": user,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "assign", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(id.to_string()),
                    message: format!("Queued: assign @{} to {}", user, issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: assign @{} to {} (offline, {:.0}ms)",
                    user,
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

fn print_issues(issues: &[Issue], comment_counts: &HashMap<String, usize>) {
    if issues.is_empty() {
        println!("No open issues.");
        return;
    }

    for issue in issues {
        let count = comment_counts.get(&issue.id).copied();
        display::print_issue_row(issue, count);
    }
}
