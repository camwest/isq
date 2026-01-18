//! Rate limit state tracking for forge APIs

use anyhow::Result;
use rusqlite::{Connection, params};

/// Rate limit state for a forge
#[derive(Debug, Clone)]
pub struct RateLimitState {
    /// Total requests allowed per hour
    pub limit: Option<u32>,
    /// Requests remaining this hour
    pub remaining: Option<u32>,
    /// Unix timestamp when the limit resets
    pub reset_at: Option<i64>,
    pub last_error: Option<String>,
}

impl RateLimitState {
    /// Calculate how many requests have been used this hour
    pub fn used(&self) -> Option<u32> {
        match (self.limit, self.remaining) {
            (Some(limit), Some(remaining)) => Some(limit.saturating_sub(remaining)),
            _ => None,
        }
    }
}

/// Get rate limit state for a forge
pub fn get_rate_limit_state(conn: &Connection, forge: &str) -> Result<Option<RateLimitState>> {
    let mut stmt = conn.prepare(
        "SELECT forge, rate_limit, remaining, reset_at, last_error, updated_at FROM rate_limit_state WHERE forge = ?",
    )?;

    let mut rows = stmt.query(params![forge])?;

    if let Some(row) = rows.next()? {
        let limit: Option<i64> = row.get(1)?;
        let remaining: Option<i64> = row.get(2)?;
        Ok(Some(RateLimitState {
            limit: limit.map(|v| v as u32),
            remaining: remaining.map(|v| v as u32),
            reset_at: row.get(3)?,
            last_error: row.get(4)?,
        }))
    } else {
        Ok(None)
    }
}

/// Set rate limit state for a forge (error case - only reset_at and last_error)
pub fn set_rate_limit_state(
    conn: &Connection,
    forge: &str,
    reset_at: Option<i64>,
    last_error: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO rate_limit_state (forge, reset_at, last_error, updated_at)
         VALUES (?, ?, ?, datetime('now'))
         ON CONFLICT(forge) DO UPDATE SET
            reset_at = excluded.reset_at,
            last_error = excluded.last_error,
            updated_at = excluded.updated_at",
        params![forge, reset_at, last_error],
    )?;
    Ok(())
}

/// Update rate limit budget for a forge (called after successful API calls)
pub fn update_rate_limit_budget(
    conn: &Connection,
    forge: &str,
    limit: u32,
    remaining: u32,
    reset_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO rate_limit_state (forge, rate_limit, remaining, reset_at, updated_at)
         VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(forge) DO UPDATE SET
            rate_limit = excluded.rate_limit,
            remaining = excluded.remaining,
            reset_at = excluded.reset_at,
            last_error = NULL,
            updated_at = excluded.updated_at",
        params![forge, limit as i64, remaining as i64, reset_at],
    )?;
    Ok(())
}

/// Check if a forge is currently rate limited
pub fn is_rate_limited(conn: &Connection, forge: &str) -> Result<bool> {
    if let Some(state) = get_rate_limit_state(conn, forge)? {
        // We're rate limited if we have no remaining requests AND the reset time is in the future
        if let (Some(remaining), Some(reset_at)) = (state.remaining, state.reset_at) {
            if remaining == 0 {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                return Ok(now < reset_at);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::init_schema;

    /// Create an in-memory database for testing
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_update_rate_limit_budget() {
        let conn = test_db();

        // Initially no rate limit state
        let state = get_rate_limit_state(&conn, "github").unwrap();
        assert!(state.is_none());

        // Update rate limit budget
        update_rate_limit_budget(&conn, "github", 5000, 4900, 1700000000).unwrap();

        // Verify it was saved
        let state = get_rate_limit_state(&conn, "github").unwrap();
        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.limit, Some(5000));
        assert_eq!(state.remaining, Some(4900));
        assert_eq!(state.reset_at, Some(1700000000));
        assert!(state.last_error.is_none());
    }

    #[test]
    fn test_rate_limit_budget_used_calculation() {
        let conn = test_db();

        update_rate_limit_budget(&conn, "github", 5000, 4153, 1700000000).unwrap();

        let state = get_rate_limit_state(&conn, "github").unwrap().unwrap();
        assert_eq!(state.used(), Some(847)); // 5000 - 4153 = 847
    }

    #[test]
    fn test_rate_limit_budget_per_forge() {
        let conn = test_db();

        // Update GitHub
        update_rate_limit_budget(&conn, "github", 5000, 4500, 1700000000).unwrap();
        // Update Linear
        update_rate_limit_budget(&conn, "linear", 1500, 1400, 1700000000).unwrap();

        // Verify they're tracked independently
        let github = get_rate_limit_state(&conn, "github").unwrap().unwrap();
        let linear = get_rate_limit_state(&conn, "linear").unwrap().unwrap();

        assert_eq!(github.limit, Some(5000));
        assert_eq!(github.remaining, Some(4500));
        assert_eq!(github.used(), Some(500));

        assert_eq!(linear.limit, Some(1500));
        assert_eq!(linear.remaining, Some(1400));
        assert_eq!(linear.used(), Some(100));
    }

    #[test]
    fn test_rate_limit_budget_updates_existing() {
        let conn = test_db();

        // Initial state
        update_rate_limit_budget(&conn, "github", 5000, 5000, 1700000000).unwrap();

        // After more API calls
        update_rate_limit_budget(&conn, "github", 5000, 4500, 1700000000).unwrap();

        let state = get_rate_limit_state(&conn, "github").unwrap().unwrap();
        assert_eq!(state.remaining, Some(4500));
        assert_eq!(state.used(), Some(500));
    }

    #[test]
    fn test_rate_limit_error_clears_on_success() {
        let conn = test_db();

        // Set error state
        set_rate_limit_state(&conn, "github", Some(1700000000), Some("rate limited")).unwrap();

        let state = get_rate_limit_state(&conn, "github").unwrap().unwrap();
        assert!(state.last_error.is_some());

        // Successful API call clears error
        update_rate_limit_budget(&conn, "github", 5000, 4500, 1700001000).unwrap();

        let state = get_rate_limit_state(&conn, "github").unwrap().unwrap();
        assert!(state.last_error.is_none());
        assert_eq!(state.limit, Some(5000));
    }
}
