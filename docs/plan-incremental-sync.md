# Incremental Sync Using updated_at

**Status: ✅ Implemented** (PR #68)

## Overview

Replace full-replace sync with incremental sync using `updated_at` timestamps to reduce API calls, improve sync speed, and lower rate limit pressure.

**Current behavior**: Every sync cycle deletes all cached issues and re-fetches everything from the API.

**Proposed behavior**: Track per-type `last_sync` timestamps per repo; use `?since=` (GitHub) or `updatedAt: { gte: ... }` (Linear) to fetch only changed items; merge changes into cache via UPSERT.

## Design Decisions

### Sync Strategy

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Sync granularity | Per-repository | Each repo tracks its own cursors |
| Cursor scope | Per data type | Separate `issues_last_sync`, `comments_last_sync`, `goals_last_sync` to avoid cross-type interference |
| Timestamp source | Server-derived | Use `max(updated_at)` from API response; cursor only advances to what we've seen |
| Timestamp buffer | Subtract 1 second | Avoid sub-second race conditions |
| Timestamp format | RFC3339 UTC | ISO 8601 format compatible with GitHub `since` and Linear `DateTime` |
| Explicit `isq sync` | Full sync by default | User-triggered implies wanting fresh, verified data |

### Deletion Handling

Deleted items won't appear in `since` queries. Solution: **periodic full reconciliation**.

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Reconciliation frequency | 1 hour default, configurable | Balance freshness vs API usage |
| Deletion detection | During full sync only | Items in cache but not in API response are marked deleted |
| Deletion query | Temp table approach | Avoids SQLite 999-parameter limit and empty list edge case |
| Partial sync safety | Completeness flag | Only run deletion reconciliation if forge confirms complete fetch |
| Tombstone behavior | 7-day TTL, completely hidden | Deleted items invisible to all queries |
| Tombstone reactivation | Reactivate if seen | If item appears in full sync, clear deleted flag (self-healing) |

### Data Types

| Data Type | Sync Strategy | Rationale |
|-----------|---------------|-----------|
| Issues | Full incremental | Primary data; frequent changes |
| Comments | Full incremental | Need edit visibility; full deletion reconciliation |
| Goals | Full replace | Few items (< 50), 1 API call, GitHub doesn't support `since` |

### GitHub-Specific

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Pull requests | Exclude client-side | Filter items with `pull_request` field; PRs aren't issues |

### Linear-Specific

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Pagination order | `orderBy: updatedAt ASC` | Ensures deterministic pagination; items updated during fetch appear at end |

### Error Handling

| Scenario | Behavior |
|----------|----------|
| Incremental sync fails | Retry incremental (cursor remains valid) |
| Full sync partial failure | UPSERT received data, skip deletion reconciliation (completeness=false) |
| Rate limit hit | Existing backoff mechanism applies |
| Stale cursor (> 1 week old) | Still use incremental (always <= full in data volume) |

### Daemon Configuration

| Setting | Current | New |
|---------|---------|-----|
| Sync interval | 30 seconds | 15 seconds (incremental is cheaper) |
| Full reconciliation | N/A | Every 1 hour (configurable) |

## Database Schema Changes

### sync_state table

Replace single `last_sync` with per-type cursors stored as RFC3339 UTC:

```sql
-- New columns for per-type cursors
ALTER TABLE sync_state ADD COLUMN issues_last_sync TEXT;      -- RFC3339 UTC
ALTER TABLE sync_state ADD COLUMN comments_last_sync TEXT;    -- RFC3339 UTC
ALTER TABLE sync_state ADD COLUMN goals_last_sync TEXT;       -- RFC3339 UTC
ALTER TABLE sync_state ADD COLUMN last_full_sync_at TEXT;     -- RFC3339 UTC
```

- `issues_last_sync` - max(updated_at) from last issues sync
- `comments_last_sync` - max(updated_at) from last comments sync
- `goals_last_sync` - timestamp of last goals sync (full replace, so client time ok)
- `last_full_sync_at` - timestamp of most recent full reconciliation
- `issue_count` - maintained via recount after each sync

### issues table

Add soft-delete support:

```sql
ALTER TABLE issues ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE issues ADD COLUMN deleted_at TEXT;
```

### comments table

Add `updated_at` for incremental sync and soft-delete support:

```sql
ALTER TABLE comments ADD COLUMN updated_at TEXT;
ALTER TABLE comments ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE comments ADD COLUMN deleted_at TEXT;
```

### sync_stats table (new)

Track sync statistics for debugging using RETURNING clause (SQLite 3.35+):

```sql
CREATE TABLE IF NOT EXISTS sync_stats (
    id INTEGER PRIMARY KEY,
    repo TEXT NOT NULL,
    data_type TEXT NOT NULL,      -- 'issues', 'comments', 'goals'
    sync_type TEXT NOT NULL,      -- 'incremental' or 'full'
    started_at TEXT NOT NULL,
    completed_at TEXT,
    duration_ms INTEGER,
    items_fetched INTEGER,
    items_inserted INTEGER,       -- accurate via RETURNING clause
    items_updated INTEGER,        -- accurate via RETURNING clause
    items_deleted INTEGER,
    is_complete INTEGER,          -- 1 if fetch was complete, 0 if partial
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_sync_stats_repo ON sync_stats(repo);
CREATE INDEX IF NOT EXISTS idx_sync_stats_started ON sync_stats(started_at);
```

## API Changes

### GitHub

Current `list_issues()`:
```
GET /repos/{owner}/{repo}/issues?state=all&per_page=100
```

New for incremental:
```
GET /repos/{owner}/{repo}/issues?state=all&per_page=100&since={issues_last_sync - 1s}
```

**PR filtering**: After fetching, filter out items where `pull_request` field is present.

Current `list_all_comments()`:
```
GET /repos/{owner}/{repo}/issues/comments?per_page=100
```

New for incremental:
```
GET /repos/{owner}/{repo}/issues/comments?per_page=100&since={comments_last_sync - 1s}
```

### Linear

Current GraphQL query fetches all issues with team filter.

New for incremental issues:
```graphql
query($teamId: ID!, $since: DateTime!, $after: String) {
  issues(
    filter: {
      team: { id: { eq: $teamId } },
      updatedAt: { gte: $since }
    },
    orderBy: { field: updatedAt, direction: ASC },
    first: 250,
    after: $after
  ) {
    pageInfo { hasNextPage, endCursor }
    nodes { ... }
  }
}
```

New for incremental comments:
```graphql
query($teamId: ID!, $since: DateTime!, $after: String) {
  comments(
    filter: {
      issue: { team: { id: { eq: $teamId } } },
      updatedAt: { gte: $since }
    },
    orderBy: { field: updatedAt, direction: ASC },
    first: 250,
    after: $after
  ) {
    pageInfo { hasNextPage, endCursor }
    nodes { ... }
  }
}
```

## Forge Trait Changes

```rust
/// Result of a fetch operation
pub struct FetchResult<T> {
    pub items: Vec<T>,
    /// True only if ALL pages succeeded; false if any partial failure
    pub is_complete: bool,
}

#[async_trait]
pub trait Forge: Send + Sync {
    // Existing (returns FetchResult for completeness tracking)
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>>;
    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<Comment>>;

    // New: fetch items updated since timestamp
    async fn list_issues_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<FetchResult<Issue>>;
    async fn list_comments_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<FetchResult<Comment>>;

    // Goals stay full-replace (no _since variant)
    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>>;
}
```

## Database Functions

### Modified: `save_issues()`

Change from DELETE ALL + INSERT to UPSERT with RETURNING for accurate stats:

```rust
pub struct SyncResult {
    pub inserted: usize,
    pub updated: usize,
    pub deleted: usize,
}

pub fn save_issues(
    conn: &Connection,
    repo: &str,
    issues: &[Issue],
    full_sync: bool,
    is_complete: bool,  // Only run deletion if true
) -> Result<SyncResult> {
    let tx = conn.unchecked_transaction()?;

    let mut inserted = 0;
    let mut updated = 0;

    // UPSERT each issue using RETURNING to detect insert vs update
    let mut stmt = tx.prepare(
        "INSERT INTO issues (repo, number, title, body, state, author, labels, ..., deleted, deleted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ..., 0, NULL)
         ON CONFLICT(repo, number) DO UPDATE SET
            title = excluded.title,
            body = excluded.body,
            state = excluded.state,
            ...
            deleted = 0,
            deleted_at = NULL
         RETURNING (SELECT COUNT(*) FROM issues WHERE repo = ?1 AND number = ?2)"
    )?;

    for issue in issues {
        // RETURNING gives 0 for insert (row didn't exist), 1 for update
        let existed: i64 = stmt.query_row(params![...], |row| row.get(0))?;
        if existed == 0 { inserted += 1; } else { updated += 1; }
    }
    drop(stmt);

    // During full sync with complete fetch: mark missing issues as deleted
    let deleted = if full_sync && is_complete {
        // Create temp table for seen issue numbers
        tx.execute("CREATE TEMP TABLE seen_issues (number INTEGER PRIMARY KEY)", [])?;

        // Batch insert seen numbers
        for chunk in issues.chunks(500) {
            let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
            let sql = format!("INSERT INTO seen_issues VALUES {}", placeholders.join(","));
            let params: Vec<i64> = chunk.iter().map(|i| i.number as i64).collect();
            tx.execute(&sql, rusqlite::params_from_iter(params))?;
        }

        // Mark unseen issues as deleted
        let count = tx.execute(
            "UPDATE issues SET deleted = 1, deleted_at = datetime('now')
             WHERE repo = ? AND deleted = 0
             AND number NOT IN (SELECT number FROM seen_issues)",
            params![repo]
        )?;

        // Temp table auto-dropped at transaction end
        count
    } else {
        0
    };

    // Update cursor to max(updated_at) from response (server-derived)
    let max_updated_at = issues.iter()
        .map(|i| &i.updated_at)
        .max()
        .cloned();

    if let Some(cursor) = max_updated_at {
        if full_sync && is_complete {
            tx.execute(
                "UPDATE sync_state SET issues_last_sync = ?, last_full_sync_at = ? WHERE repo = ?",
                params![cursor, cursor, repo],
            )?;
        } else {
            tx.execute(
                "UPDATE sync_state SET issues_last_sync = ? WHERE repo = ?",
                params![cursor, repo],
            )?;
        }
    }

    // Recount issue_count for accuracy
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM issues WHERE repo = ? AND deleted = 0",
        params![repo],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE sync_state SET issue_count = ? WHERE repo = ?",
        params![count, repo],
    )?;

    tx.commit()?;
    Ok(SyncResult { inserted, updated, deleted })
}
```

### Modified: `save_comments()`

Same UPSERT + temp table pattern as `save_issues()`, using `comments_last_sync` cursor.

### Modified: `load_issues_filtered()`

Add filter to exclude deleted issues:

```rust
// Add to WHERE clause
sql.push_str(" AND deleted = 0");
```

### Modified: `load_comments()`

Add filter to exclude deleted comments:

```rust
// Add to WHERE clause
sql.push_str(" AND deleted = 0");
```

### New: `purge_deleted_items()`

Remove tombstones older than TTL:

```rust
pub fn purge_deleted_items(conn: &Connection, ttl_days: i64) -> Result<(usize, usize)> {
    let threshold = format!("-{} days", ttl_days);

    let issues = conn.execute(
        "DELETE FROM issues WHERE deleted = 1 AND deleted_at < datetime('now', ?)",
        params![threshold],
    )?;

    let comments = conn.execute(
        "DELETE FROM comments WHERE deleted = 1 AND deleted_at < datetime('now', ?)",
        params![threshold],
    )?;

    Ok((issues, comments))
}
```

## Daemon Changes

### sync_once()

```rust
async fn sync_once(repo_path: &str) -> Result<()> {
    let (forge, link) = get_forge_for_repo(repo_path)?;
    let conn = db::open()?;

    // Determine if we need full sync
    let sync_state = db::get_sync_state(&conn, &link.forge_repo)?;
    let needs_full_sync = should_do_full_sync(&sync_state);

    // === ISSUES ===
    let issues_cursor = sync_state.as_ref()
        .and_then(|s| s.issues_last_sync.as_ref())
        .and_then(|t| parse_rfc3339(t).ok())
        .map(|t| t - chrono::Duration::seconds(1));  // 1-second buffer

    let issues_result = if needs_full_sync || issues_cursor.is_none() {
        forge.list_issues(&repo).await?
    } else {
        forge.list_issues_since(&repo, issues_cursor.unwrap()).await?
    };

    // Filter out PRs (GitHub-specific, no-op for Linear)
    let issues: Vec<Issue> = issues_result.items.into_iter()
        .filter(|i| !i.is_pull_request)
        .collect();

    let issues_stats = db::save_issues(
        &conn,
        &link.forge_repo,
        &issues,
        needs_full_sync,
        issues_result.is_complete,
    )?;

    // === COMMENTS ===
    let comments_cursor = sync_state.as_ref()
        .and_then(|s| s.comments_last_sync.as_ref())
        .and_then(|t| parse_rfc3339(t).ok())
        .map(|t| t - chrono::Duration::seconds(1));

    let comments_result = if needs_full_sync || comments_cursor.is_none() {
        forge.list_all_comments(&repo).await?
    } else {
        forge.list_comments_since(&repo, comments_cursor.unwrap()).await?
    };

    let comments_stats = db::save_comments(
        &conn,
        &link.forge_repo,
        &comments_result.items,
        needs_full_sync,
        comments_result.is_complete,
    )?;

    // === GOALS (always full replace) ===
    let goals = forge.list_goals(&repo).await?;
    db::save_goals(&conn, &link.forge_repo, &goals)?;

    // Record stats
    db::record_sync_stats(&conn, &link.forge_repo, &issues_stats, &comments_stats, needs_full_sync)?;

    // Purge old tombstones during full sync
    if needs_full_sync {
        db::purge_deleted_items(&conn, 7)?;
    }

    eprintln!(
        "[daemon] {} sync for {}: {} issues (+{} -{} ~{}), {} comments (+{} -{} ~{})",
        if needs_full_sync { "Full" } else { "Incremental" },
        link.forge_repo,
        issues.len(),
        issues_stats.inserted,
        issues_stats.deleted,
        issues_stats.updated,
        comments_result.items.len(),
        comments_stats.inserted,
        comments_stats.deleted,
        comments_stats.updated,
    );

    Ok(())
}

fn should_do_full_sync(sync_state: &Option<SyncState>) -> bool {
    match sync_state {
        None => true,  // First sync
        Some(state) => {
            let last_full = state.last_full_sync_at.as_ref()
                .and_then(|t| parse_rfc3339(t).ok());
            match last_full {
                None => true,  // Never done full sync
                Some(ts) => chrono::Utc::now() - ts > chrono::Duration::hours(1),
            }
        }
    }
}
```

### Configuration

Add to repo config (`isq.toml`):

```toml
[sync]
# Interval between full reconciliation syncs
full_sync_interval = "1h"  # default

# Daemon sync interval (how often to check for changes)
sync_interval = "15s"  # default
```

## Implementation Order

1. **Database schema changes** - Add per-type cursor columns, deleted columns, sync_stats table, updated_at for comments
2. **Add `FetchResult` wrapper** - Completeness flag for all fetch operations
3. **Modify `save_issues()`** - UPSERT with RETURNING, temp table deletion, server-derived cursor
4. **Modify `save_comments()`** - Same pattern as issues
5. **Update GitHub forge** - PR filtering, `since` parameter, completeness tracking
6. **Update Linear forge** - `orderBy` for pagination, `updatedAt` filter, fix 100-issue comment truncation
7. **Add `list_issues_since()` to forges** - Incremental fetch methods
8. **Add `list_comments_since()` to forges** - Incremental fetch methods
9. **Update daemon `sync_once()`** - Per-type cursors, completeness checks
10. **Add configuration options** - `full_sync_interval`, `sync_interval`
11. **Update `isq sync` command** - Explicit full sync
12. **Add tombstone purge logic** - Clean up old deleted records

## Testing Strategy

### Unit Tests
- UPSERT with RETURNING correctly identifies insert vs update
- Temp table deletion handles 1000+ issues without parameter limit errors
- Temp table deletion handles empty issue list (no issues in repo)
- Tombstone reactivation when deleted item reappears
- `should_do_full_sync()` timing logic
- RFC3339 timestamp parsing and formatting
- Server-derived cursor advancement (max updated_at)

### Integration Tests
- GitHub `since` parameter produces correct API calls
- GitHub PR filtering excludes PRs from results
- Linear `updatedAt` filter with `orderBy` produces correct GraphQL
- Linear comment fetch handles > 100 issues (pagination fix)
- Completeness flag is false when page fetch fails
- Deletion reconciliation skipped when is_complete=false
- End-to-end: create issue externally, verify it appears after incremental sync
- End-to-end: delete issue externally, verify it disappears after full reconciliation
- End-to-end: edit comment externally, verify edit appears after incremental sync

### Manual Testing
- Monitor rate limit usage before/after
- Verify daemon logs show "incremental" vs "full" appropriately
- Test with large repo (1000+ issues) to verify performance improvement
- Verify SQLite 3.35+ RETURNING clause works on target systems

## Metrics to Track (sync_stats table)

- Sync type distribution (incremental vs full) per data type
- Average sync duration by type
- Items fetched per sync (should be lower for incremental)
- Insert vs update ratio (useful for detecting churn)
- Delete detection count (during full syncs)
- Partial sync rate (is_complete=false)
- Error rates by type

## Rollout

Ship as default behavior. No feature flags needed (single developer, pre-v1).

**SQLite version requirement**: 3.35+ for RETURNING clause (released March 2021).

## Future Considerations

Not implementing now, but worth noting for v2:

- **Adaptive reconciliation**: Start with frequent full syncs, decay over time as cache matures
- **Repo-size-aware scheduling**: Smaller repos get more frequent full syncs
- **User-activity-aware**: Prioritize full sync for repos with recent `isq start` activity
- **Webhook integration**: Real-time updates instead of polling (eliminates need for frequent syncs)
- **Per-type sync cadences**: Issues every 15s, comments every 60s, goals every 5m
