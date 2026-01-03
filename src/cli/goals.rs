//! Goal-related CLI commands

use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::display;
use crate::forges::{get_forge_for_repo, not_linked_error, CreateGoalRequest};
use crate::repo;

use super::utils::{is_offline_error, WriteResult};

pub async fn cmd_list(state: String, json_output: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    let link = db::get_repo_link(&conn, &repo_path)?.ok_or_else(not_linked_error)?;

    // Load goals from cache, filtering by state if not "all"
    let state_filter = if state == "all" {
        None
    } else {
        Some(state.as_str())
    };
    let mut goals = db::load_goals(&conn, &link.forge_repo, state_filter)?;

    // If no cached goals, fetch from API
    if goals.is_empty() && db::count_goals(&conn, &link.forge_repo)? == 0 {
        eprintln!("Syncing goals...");
        let (forge, _) = get_forge_for_repo(&repo_path)?;

        // Parse forge_repo to create Repo struct
        let parts: Vec<&str> = link.forge_repo.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
        }
        let repo_struct = repo::Repo {
            owner: parts[0].to_string(),
            name: parts[1].to_string(),
        };

        let fetched = forge.list_goals(&repo_struct).await?;
        db::save_goals(&conn, &link.forge_repo, &fetched)?;

        // Re-filter after saving
        goals = db::load_goals(&conn, &link.forge_repo, state_filter)?;
    }

    db::touch_repo(&conn, &repo_path)?;
    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&goals)?);
    } else {
        display::print_goals(&goals);
        eprintln!("\n{} goals in {:.0}ms", goals.len(), elapsed.as_millis());
    }

    Ok(())
}

pub fn cmd_show(name: String, json_output: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    let link = db::get_repo_link(&conn, &repo_path)?.ok_or_else(not_linked_error)?;

    db::touch_repo(&conn, &repo_path)?;

    let goal = db::load_goal_by_name(&conn, &link.forge_repo, &name)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Goal '{}' not found. Run `isq sync` to refresh.",
            name
        )
    })?;

    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&goal)?);
    } else {
        display::print_goal_detail(&goal, elapsed.as_millis() as u64);
    }

    Ok(())
}

pub async fn cmd_create(
    name: String,
    target: Option<String>,
    body: Option<String>,
    json: bool,
) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let req = CreateGoalRequest {
        name: name.clone(),
        description: body.clone(),
        target_date: target.clone(),
    };

    match forge.create_goal(&repo_struct, req).await {
        Ok(goal) => {
            let elapsed = start.elapsed();
            // Save to local cache
            let conn = db::open()?;
            db::save_goal(&conn, &link.forge_repo, &goal)?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: None,
                    message: format!("Created goal: {}", goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Created goal: {} ({:.0}ms)",
                    goal.name,
                    elapsed.as_millis()
                );
                if let Some(url) = &goal.html_url {
                    println!("  {}", url);
                }
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "name": name,
                "target_date": target,
                "description": body,
            });
            let conn = db::open()?;
            db::queue_op(
                &conn,
                &link.forge_repo,
                "create_goal",
                &payload.to_string(),
            )?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: None,
                    message: format!("Queued: create goal {}", name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Queued: create goal {} (offline, {:.0}ms)",
                    name,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

pub async fn cmd_assign(issue_id: &str, goal_name: String, json: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;
    let conn = db::open()?;

    // Resolve goal name to ID
    let goal = db::load_goal_by_name(&conn, &link.forge_repo, &goal_name)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Goal '{}' not found. Run `isq sync` to refresh.",
            goal_name
        )
    })?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let issue_display = crate::display::format_issue_id(issue_id);
    match forge.assign_to_goal(&repo_struct, issue_id, &goal.id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Assigned {} to goal '{}'", issue_display, goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Assigned {} to goal '{}' ({:.0}ms)",
                    issue_display,
                    goal.name,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_id": issue_id,
                "goal_id": goal.id,
            });
            db::queue_op(
                &conn,
                &link.forge_repo,
                "assign_goal",
                &payload.to_string(),
            )?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(issue_id.to_string()),
                    message: format!("Queued: assign {} to '{}'", issue_display, goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Queued: assign {} to '{}' (offline, {:.0}ms)",
                    issue_display,
                    goal.name,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

pub async fn cmd_close(name: String, json: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;
    let conn = db::open()?;

    // Resolve goal name to ID
    let goal = db::load_goal_by_name(&conn, &link.forge_repo, &name)?.ok_or_else(|| {
        anyhow::anyhow!("Goal '{}' not found. Run `isq sync` to refresh.", name)
    })?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match forge.close_goal(&repo_struct, &goal.id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: None,
                    message: format!("Closed goal '{}'", goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Closed goal '{}' ({:.0}ms)",
                    goal.name,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "goal_id": goal.id,
            });
            db::queue_op(&conn, &link.forge_repo, "close_goal", &payload.to_string())?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: None,
                    message: format!("Queued: close goal '{}'", goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Queued: close goal '{}' (offline, {:.0}ms)",
                    goal.name,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}
