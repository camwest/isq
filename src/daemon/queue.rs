//! Processing of pending operations (offline-first write queue).

use anyhow::Result;
use std::collections::HashMap;

use crate::db;
use crate::forges::{CreateIssueRequest, Forge};
use crate::repo::Repo;

/// Classification of operation errors for conflict resolution.
///
/// This classification is forge-agnostic, using semantic patterns that work
/// across GitHub (REST), Linear (GraphQL), and JIRA (REST).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Resource doesn't exist (issue deleted, label removed, etc.)
    /// Server wins: discard the operation.
    NotFound,

    /// State conflict (issue already closed, already assigned, etc.)
    /// Server wins: discard the operation.
    StateConflict,

    /// Validation error (invalid field, missing required data)
    /// Unrecoverable: discard the operation.
    ValidationError,
}

/// Check if an error represents a conflict that should discard the operation.
///
/// Returns `Some(ConflictKind)` for conflicts (server wins, discard op),
/// or `None` for transient errors (keep in queue, retry later).
///
/// This function uses semantic pattern matching that works across forges:
/// - GitHub: "GitHub API error 404: ..." or "GitHub API error 422: ..."
/// - Linear: "Linear GraphQL errors: Entity not found" or similar
/// - JIRA: "JIRA API error (404): ..." or "JIRA error: Issue does not exist"
pub fn classify_error(err: &anyhow::Error) -> Option<ConflictKind> {
    let err_str = err.to_string().to_lowercase();

    // HTTP status codes (GitHub, JIRA)
    // 404 = Not Found, 410 = Gone
    if err_str.contains(" 404") || err_str.contains("(404)") || err_str.contains(" 410") {
        return Some(ConflictKind::NotFound);
    }

    // 409 = Conflict (state changed)
    if err_str.contains(" 409") || err_str.contains("(409)") {
        return Some(ConflictKind::StateConflict);
    }

    // 422 = Unprocessable Entity (validation or state issue)
    if err_str.contains(" 422") || err_str.contains("(422)") {
        return Some(ConflictKind::ValidationError);
    }

    // Semantic patterns (work across all forges including GraphQL)
    // Not found patterns
    if err_str.contains("not found")
        || err_str.contains("does not exist")
        || err_str.contains("doesn't exist")
        || err_str.contains("no such")
        || err_str.contains("entity not found")
        || err_str.contains("resource not found")
    {
        return Some(ConflictKind::NotFound);
    }

    // State conflict patterns
    if err_str.contains("already closed")
        || err_str.contains("already open")
        || err_str.contains("already exists")
        || err_str.contains("state has changed")
        || err_str.contains("was modified")
        || err_str.contains("concurrent modification")
    {
        return Some(ConflictKind::StateConflict);
    }

    // JIRA-specific transition errors (issue in wrong state)
    if err_str.contains("no 'done' transition")
        || err_str.contains("no 'reopen' transition")
        || err_str.contains("transition is not valid")
    {
        return Some(ConflictKind::StateConflict);
    }

    // None = transient error, keep in queue for retry
    None
}

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
                if let Some(conflict_kind) = classify_error(&e) {
                    // Conflict detected - server wins, discard operation
                    eprintln!(
                        "[daemon] {:?} for {} op on {}: {} (discarding)",
                        conflict_kind,
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
