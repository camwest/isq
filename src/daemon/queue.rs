//! Processing of pending operations (offline-first write queue).

use anyhow::Result;
use std::collections::HashMap;

use crate::db;
use crate::forges::{CreateIssueRequest, Forge};
use crate::repo::Repo;

/// Process pending operations and return count of successful syncs
pub async fn process_pending_ops(
    forge: &dyn Forge,
    repo: &Repo,
    conn: &rusqlite::Connection,
    ops: &[db::PendingOp],
) -> usize {
    let mut synced = 0;

    for op in ops {
        let result = execute_pending_op(forge, repo, op).await;

        match result {
            Ok(()) => {
                // Operation succeeded, remove from queue
                if let Err(e) = db::complete_op(conn, op.id) {
                    eprintln!("[daemon] Failed to mark op {} complete: {}", op.id, e);
                }
                synced += 1;
            }
            Err(e) => {
                // Check if this is a conflict (server state changed)
                let err_str = e.to_string();
                if err_str.contains("404") || err_str.contains("422") || err_str.contains("409") {
                    // Conflict or resource not found - server wins, discard operation
                    eprintln!(
                        "[daemon] Conflict for {} op on {}: {} (discarding)",
                        op.op_type,
                        repo.full_name(),
                        e
                    );
                    if let Err(e) = db::complete_op(conn, op.id) {
                        eprintln!("[daemon] Failed to discard op {}: {}", op.id, e);
                    }
                    synced += 1; // Count as processed
                } else {
                    // Network or other transient error - leave in queue for retry
                    eprintln!("[daemon] Failed {} op, will retry: {}", op.op_type, e);
                }
            }
        }
    }

    synced
}

/// Execute a single pending operation
async fn execute_pending_op(forge: &dyn Forge, repo: &Repo, op: &db::PendingOp) -> Result<()> {
    let payload: serde_json::Value = serde_json::from_str(&op.payload)?;

    match op.op_type.as_str() {
        "create" => {
            let req = CreateIssueRequest {
                title: payload["title"].as_str().unwrap_or("").to_string(),
                body: payload["body"].as_str().map(|s| s.to_string()),
                labels: payload["labels"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                goal_id: payload["goal_id"].as_str().map(|s| s.to_string()),
                opts: HashMap::new(),
            };
            let issue = forge.create_issue(repo, req).await?;
            let issue_display = crate::display::format_issue_id(&issue.id);
            eprintln!("[daemon] Created {} {}", issue_display, issue.title);
        }
        "comment" => {
            // Support both old issue_number and new issue_id keys
            let issue_id = payload["issue_id"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let body = payload["body"].as_str().unwrap_or("");
            forge.create_comment(repo, &issue_id, body).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Added comment to {}", issue_display);
        }
        "close" => {
            let issue_id = payload["issue_id"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            forge.close_issue(repo, &issue_id).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Closed {}", issue_display);
        }
        "reopen" => {
            let issue_id = payload["issue_id"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            forge.reopen_issue(repo, &issue_id).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Reopened {}", issue_display);
        }
        "label_add" => {
            let issue_id = payload["issue_id"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let label = payload["label"].as_str().unwrap_or("");
            forge.add_label(repo, &issue_id, label).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Added label '{}' to {}", label, issue_display);
        }
        "label_remove" => {
            let issue_id = payload["issue_id"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let label = payload["label"].as_str().unwrap_or("");
            forge.remove_label(repo, &issue_id, label).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Removed label '{}' from {}", label, issue_display);
        }
        "assign" => {
            let issue_id = payload["issue_id"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let assignee = payload["assignee"].as_str().unwrap_or("");
            forge.assign_issue(repo, &issue_id, assignee).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Assigned @{} to {}", assignee, issue_display);
        }
        _ => {
            anyhow::bail!("Unknown op type: {}", op.op_type);
        }
    }

    Ok(())
}
