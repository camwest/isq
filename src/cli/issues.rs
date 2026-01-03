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
    id: Option<String>,
    label: Option<String>,
    state: Option<String>,
    mine: bool,
    unassigned: bool,
    open: bool,
    goal: Option<String>,
    sort: String,
    opts: Vec<String>,
    json_output: bool,
) -> Result<()> {
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

    // --open is a shorthand for --state=open
    let state = if open && state.is_none() {
        Some("open".to_string())
    } else {
        state
    };

    // Determine user_name for --mine filter (matches issue.assignees)
    let user_name = if mine { link.user_name.clone() } else { None };

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
            filtered
        } else {
            // Forge doesn't handle these opts, fall back to cache
            let ids_refs: Option<Vec<&str>> = ids.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());
            db::load_issues_filtered(
                &conn,
                &link.forge_repo,
                ids_refs.as_deref(),
                label.as_deref(),
                state.as_deref(),
                user_name.as_deref(),
                unassigned,
                goal.as_deref(),
                &sort,
            )?
        }
    } else {
        // No opts, use normal cache path
        let ids_refs: Option<Vec<&str>> = ids.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());
        db::load_issues_filtered(
            &conn,
            &link.forge_repo,
            ids_refs.as_deref(),
            label.as_deref(),
            state.as_deref(),
            user_name.as_deref(),
            unassigned,
            goal.as_deref(),
            &sort,
        )?
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
) -> Result<()> {
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
            } else {
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
            } else {
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

pub async fn cmd_comment(id: &str, message: String, json: bool) -> Result<()> {
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
            } else {
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
            } else {
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

pub async fn cmd_close(id: &str, json: bool) -> Result<()> {
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
            } else {
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
            } else {
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

pub async fn cmd_reopen(id: &str, json: bool) -> Result<()> {
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
            } else {
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
            } else {
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

pub async fn cmd_label(id: &str, action: String, label: String, json: bool) -> Result<()> {
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
                    } else {
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
                    } else {
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
                    } else {
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
                    } else {
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

pub async fn cmd_assign(id: &str, user: String, json: bool) -> Result<()> {
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
            } else {
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
            } else {
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
