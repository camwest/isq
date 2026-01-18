//! Issue write operation commands (create, comment, close, reopen, label, assign)

use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::display;
use crate::forges::{CreateIssueRequest, get_forge_for_repo};
use crate::repo;

use super::super::utils::{
    WriteResult, is_offline_error, parse_forge_repo, validate_jira_issue_prefix,
};

pub async fn cmd_create(
    title: String,
    body: Option<String>,
    labels: Vec<String>,
    goal: Option<String>,
    opts: Vec<String>,
    json: bool,
    cli_quiet: bool,
) -> Result<()> {
    // Resolve body: explicit arg takes precedence, then stdin
    let body = body.or(super::super::utils::read_stdin_if_piped()?);

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
            anyhow::anyhow!("Goal '{}' not found. Run `isq sync` to refresh.", goal_name)
        })?;
        Some(g.id)
    } else {
        None
    };

    let repo_struct = parse_forge_repo(&link.forge_repo)?;

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
            let id_display = display::format_issue_id(&issue.id);
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(issue.id.clone()),
                    message: format!("Created {} {}", id_display, issue.title),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Created {} {} ({:.0}ms)",
                    id_display,
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

pub async fn cmd_comment(
    id: &str,
    message: Option<String>,
    json: bool,
    cli_quiet: bool,
) -> Result<()> {
    // Resolve message: explicit arg takes precedence, then stdin, then error
    let message = message
        .or(super::super::utils::read_stdin_if_piped()?)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Comment message required. Provide as argument or pipe via stdin.\n\
                 Usage: isq issue comment <ID> \"message\"\n\
                    or: echo \"message\" | isq issue comment <ID>"
            )
        })?;

    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let repo_struct = parse_forge_repo(&link.forge_repo)?;
    validate_jira_issue_prefix(id, &repo_struct.name, &link.forge_type)?;

    let issue_display = display::format_issue_id(id);

    match forge.create_comment(&repo_struct, id, &message).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(id.to_string()),
                    message: format!("Comment added to {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Comment added to {} ({:.0}ms)",
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_id": id,
                "body": message,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "comment", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(id.to_string()),
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

    let repo_struct = parse_forge_repo(&link.forge_repo)?;
    validate_jira_issue_prefix(id, &repo_struct.name, &link.forge_type)?;

    let issue_display = display::format_issue_id(id);

    match forge.close_issue(&repo_struct, id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(id.to_string()),
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
            let payload = serde_json::json!({ "issue_id": id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "close", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(id.to_string()),
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

    let repo_struct = parse_forge_repo(&link.forge_repo)?;
    validate_jira_issue_prefix(id, &repo_struct.name, &link.forge_type)?;

    let issue_display = display::format_issue_id(id);

    match forge.reopen_issue(&repo_struct, id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(id.to_string()),
                    message: format!("Reopened {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Reopened {} ({:.0}ms)",
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({ "issue_id": id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "reopen", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(id.to_string()),
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

pub async fn cmd_label(
    id: &str,
    action: String,
    label: String,
    json: bool,
    cli_quiet: bool,
) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let repo_struct = parse_forge_repo(&link.forge_repo)?;
    validate_jira_issue_prefix(id, &repo_struct.name, &link.forge_type)?;

    let issue_display = display::format_issue_id(id);

    match action.as_str() {
        "add" => match forge.add_label(&repo_struct, id, &label).await {
            Ok(()) => {
                let elapsed = start.elapsed();
                if json {
                    let result = WriteResult {
                        success: true,
                        queued: false,
                        issue_id: Some(id.to_string()),
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
                    "issue_id": id,
                    "label": label,
                });
                let conn = db::open()?;
                db::queue_op(&conn, &link.forge_repo, "label_add", &payload.to_string())?;
                if json {
                    let result = WriteResult {
                        success: true,
                        queued: true,
                        issue_id: Some(id.to_string()),
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
        },
        "remove" => match forge.remove_label(&repo_struct, id, &label).await {
            Ok(()) => {
                let elapsed = start.elapsed();
                if json {
                    let result = WriteResult {
                        success: true,
                        queued: false,
                        issue_id: Some(id.to_string()),
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
                    "issue_id": id,
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
                        issue_id: Some(id.to_string()),
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
        },
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

    let repo_struct = parse_forge_repo(&link.forge_repo)?;
    validate_jira_issue_prefix(id, &repo_struct.name, &link.forge_type)?;

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
