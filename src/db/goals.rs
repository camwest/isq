//! Goal/milestone storage and retrieval

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::forges::{Goal, GoalState};

/// Save goals for a repo (replaces all existing goals)
pub fn save_goals(conn: &Connection, forge_repo: &str, goals: &[Goal]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Delete existing goals for this repo
    tx.execute("DELETE FROM goals WHERE forge_repo = ?", params![forge_repo])?;

    // Insert new goals
    let mut stmt = tx.prepare(
        "INSERT INTO goals (forge_repo, goal_id, name, description, target_date, state, progress, open_count, closed_count, created_at, updated_at, html_url)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;

    for goal in goals {
        stmt.execute(params![
            forge_repo,
            goal.id,
            goal.name,
            goal.description,
            goal.target_date,
            goal.state.as_str(),
            goal.progress,
            goal.open_count.map(|c| c as i64),
            goal.closed_count.map(|c| c as i64),
            goal.created_at,
            goal.updated_at,
            goal.html_url,
        ])?;
    }

    drop(stmt);
    tx.commit()?;
    Ok(())
}

/// Save a single goal (insert or update)
pub fn save_goal(conn: &Connection, forge_repo: &str, goal: &Goal) -> Result<()> {
    conn.execute(
        "INSERT INTO goals (forge_repo, goal_id, name, description, target_date, state, progress, open_count, closed_count, created_at, updated_at, html_url)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(forge_repo, goal_id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            target_date = excluded.target_date,
            state = excluded.state,
            progress = excluded.progress,
            open_count = excluded.open_count,
            closed_count = excluded.closed_count,
            updated_at = excluded.updated_at,
            html_url = excluded.html_url",
        params![
            forge_repo,
            goal.id,
            goal.name,
            goal.description,
            goal.target_date,
            goal.state.as_str(),
            goal.progress,
            goal.open_count.map(|c| c as i64),
            goal.closed_count.map(|c| c as i64),
            goal.created_at,
            goal.updated_at,
            goal.html_url,
        ],
    )?;
    Ok(())
}

/// Load all goals for a repo from cache
pub fn load_goals(conn: &Connection, forge_repo: &str, state: Option<&str>) -> Result<Vec<Goal>> {
    let mut sql = String::from(
        "SELECT goal_id, name, description, target_date, state, progress, open_count, closed_count, created_at, updated_at, html_url
         FROM goals WHERE forge_repo = ?",
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(forge_repo.to_string())];

    if let Some(s) = state {
        sql.push_str(" AND state = ?");
        params_vec.push(Box::new(s.to_string()));
    }

    sql.push_str(" ORDER BY target_date ASC NULLS LAST, name ASC");

    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let goals = stmt
        .query_map(params_refs.as_slice(), |row| {
            let state_str: String = row.get(4)?;
            let progress: f64 = row.get::<_, Option<f64>>(5)?.unwrap_or(0.0);
            let open: Option<i64> = row.get(6)?;
            let closed: Option<i64> = row.get(7)?;

            Ok(Goal {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                target_date: row.get(3)?,
                state: GoalState::from_str(&state_str),
                progress,
                open_count: open.map(|c| c as u64),
                closed_count: closed.map(|c| c as u64),
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                html_url: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(goals)
}

/// Load a single goal by name or ID
pub fn load_goal_by_name(
    conn: &Connection,
    forge_repo: &str,
    name: &str,
) -> Result<Option<Goal>> {
    let mut stmt = conn.prepare(
        "SELECT goal_id, name, description, target_date, state, progress, open_count, closed_count, created_at, updated_at, html_url
         FROM goals WHERE forge_repo = ? AND (name = ? OR goal_id = ?)",
    )?;

    let mut rows = stmt.query(params![forge_repo, name, name])?;

    if let Some(row) = rows.next()? {
        let state_str: String = row.get(4)?;
        let progress: f64 = row.get::<_, Option<f64>>(5)?.unwrap_or(0.0);
        let open: Option<i64> = row.get(6)?;
        let closed: Option<i64> = row.get(7)?;

        Ok(Some(Goal {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            target_date: row.get(3)?,
            state: GoalState::from_str(&state_str),
            progress,
            open_count: open.map(|c| c as u64),
            closed_count: closed.map(|c| c as u64),
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            html_url: row.get(10)?,
        }))
    } else {
        Ok(None)
    }
}

/// Count goals for a repo
pub fn count_goals(conn: &Connection, forge_repo: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM goals WHERE forge_repo = ?",
        params![forge_repo],
        |row| row.get(0),
    )?;
    Ok(count)
}
