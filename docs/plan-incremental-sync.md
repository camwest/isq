# Incremental Sync Using updated_at

## Overview

Replace full-replace sync with incremental sync using `updated_at` timestamps to reduce API calls, improve sync speed, and lower rate limit pressure.

**Current behavior**: Every sync cycle deletes all cached issues and re-fetches everything from the API.

**Proposed behavior**: Track `last_sync_at` timestamp per repo; use `?since=` (GitHub) or `updatedAt: { gte: ... }` (Linear) to fetch only changed issues; merge changes into cache via UPSERT.

## Design Decisions

### Sync Strategy

| Decision | Choice |
|----------|--------|
| Sync granularity | Per-repository (each repo tracks its own cursor) |
| Default sync type | Incremental using `since`/`updatedAt` filters |
| Timestamp buffer | Subtract 1 second from `last_sync_at` to avoid sub-second race conditions |
| Explicit `isq sync` | Full sync by default (user-triggered implies wanting fresh data) |

### Deletion Handling

Deleted issues won't appear in `since` queries. Solution: **periodic full reconciliation**.

| Decision | Choice |
|----------|--------|
| Reconciliation frequency | 1 hour default, configurable via `full_sync_interval` in config |
| Deletion detection | During full sync, issues in cache but not in API response are marked deleted |
| Deletion behavior | Tombstone with 7-day TTL; completely hidden from all queries |
| Deletion visibility | Never shown to users; purely internal for sync reconciliation |

### Data Types

All data types use the same incremental pattern:

| Data Type | Incremental Support |
|-----------|---------------------|
| Issues | Yes - `since` (GitHub) / `updatedAt: { gte }` (Linear) |
| Comments | Yes - same pattern |
| Goals (milestones/projects) | Yes - same pattern for consistency |

### Error Handling

| Scenario | Behavior |
|----------|----------|
| Incremental sync fails | Retry incremental (timestamp remains valid) |
| Full sync fails mid-fetch | Retry full once, then fall back to incremental |
| Rate limit hit | Existing backoff mechanism applies |
| Stale timestamp (> 1 week old) | Still use incremental (always <= full in data volume) |

### Daemon Configuration

| Setting | Current | New |
|---------|---------|-----|
| Sync interval | 30 seconds | 15 seconds (incremental is cheaper) |
| Full reconciliation | N/A | Every 1 hour (configurable) |

## Database Schema Changes

### sync_state table

Add columns to track incremental vs full sync:

```sql
ALTER TABLE sync_state ADD COLUMN last_full_sync_at TEXT;
```

- `last_sync` - timestamp of most recent sync (any type)
- `last_full_sync_at` - timestamp of most recent full reconciliation

### issues table

Add soft-delete support:

```sql
ALTER TABLE issues ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE issues ADD COLUMN deleted_at TEXT;
```

- `deleted` - 1 if issue was deleted upstream, 0 otherwise
- `deleted_at` - timestamp when deletion was detected

### sync_stats table (new)

Track sync statistics for debugging:

```sql
CREATE TABLE IF NOT EXISTS sync_stats (
    id INTEGER PRIMARY KEY,
    repo TEXT NOT NULL,
    sync_type TEXT NOT NULL,  -- 'incremental' or 'full'
    started_at TEXT NOT NULL,
    completed_at TEXT,
    duration_ms INTEGER,
    issues_fetched INTEGER,
    issues_inserted INTEGER,
    issues_updated INTEGER,
    issues_deleted INTEGER,
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
GET /repos/{owner}/{repo}/issues?state=all&per_page=100&since={last_sync_at - 1s}
```

### Linear

Current GraphQL query fetches all issues with team filter.

New for incremental:
```graphql
query($teamId: ID!, $since: DateTime!, $after: String) {
  issues(
    filter: {
      team: { id: { eq: $teamId } },
      updatedAt: { gte: $since }
    },
    first: 250,
    after: $after
  ) {
    pageInfo { hasNextPage, endCursor }
    nodes { ... }
  }
}
```

## Database Functions

### Modified: `save_issues()`

Change from DELETE ALL + INSERT to UPSERT semantics:

```rust
pub fn save_issues(conn: &Connection, repo: &str, issues: &[Issue], full_sync: bool) -> Result<SyncResult> {
    let tx = conn.unchecked_transaction()?;

    let mut inserted = 0;
    let mut updated = 0;
    let mut deleted = 0;

    // UPSERT each issue
    for issue in issues {
        let result = tx.execute(
            "INSERT INTO issues (...) VALUES (...)
             ON CONFLICT(repo, number) DO UPDATE SET
                title = excluded.title,
                body = excluded.body,
                ...
                deleted = 0,
                deleted_at = NULL",
            params![...],
        )?;

        if result == 1 { inserted += 1; } else { updated += 1; }
    }

    // During full sync: mark missing issues as deleted
    if full_sync {
        let issue_numbers: Vec<i64> = issues.iter().map(|i| i.number as i64).collect();
        // Mark issues not in the API response as deleted
        deleted = tx.execute(
            "UPDATE issues SET deleted = 1, deleted_at = datetime('now')
             WHERE repo = ? AND number NOT IN (...) AND deleted = 0",
            params![repo, ...],
        )?;
    }

    // Update sync timestamps
    if full_sync {
        tx.execute(
            "UPDATE sync_state SET last_sync = datetime('now'), last_full_sync_at = datetime('now')
             WHERE repo = ?",
            params![repo],
        )?;
    } else {
        tx.execute(
            "UPDATE sync_state SET last_sync = datetime('now') WHERE repo = ?",
            params![repo],
        )?;
    }

    tx.commit()?;
    Ok(SyncResult { inserted, updated, deleted })
}
```

### Modified: `load_issues_filtered()`

Add filter to exclude deleted issues:

```rust
// Add to WHERE clause
sql.push_str(" AND deleted = 0");
```

### New: `purge_deleted_issues()`

Remove tombstones older than TTL:

```rust
pub fn purge_deleted_issues(conn: &Connection, ttl_days: i64) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM issues
         WHERE deleted = 1
         AND deleted_at < datetime('now', ? || ' days')",
        params![format!("-{}", ttl_days)],
    )?;
    Ok(deleted)
}
```

## Daemon Changes

### sync_once()

```rust
async fn sync_once(repo_path: &str) -> Result<()> {
    let (forge, link) = get_forge_for_repo(repo_path)?;
    let conn = db::open()?;

    // Determine sync type
    let sync_state = db::get_sync_state(&conn, &link.forge_repo)?;
    let needs_full_sync = should_do_full_sync(&sync_state);

    // Get timestamp for incremental (subtract 1 second buffer)
    let since = if needs_full_sync {
        None
    } else {
        sync_state.as_ref().map(|s| {
            // Parse and subtract 1 second
            parse_timestamp(&s.last_sync) - Duration::from_secs(1)
        })
    };

    // Fetch issues (incremental or full)
    let issues = if let Some(since) = since {
        forge.list_issues_since(&repo, since).await?
    } else {
        forge.list_issues(&repo).await?
    };

    // Save with appropriate mode
    let result = db::save_issues(&conn, &link.forge_repo, &issues, needs_full_sync)?;

    // Record stats
    db::record_sync_stats(&conn, &link.forge_repo, SyncStats {
        sync_type: if needs_full_sync { "full" } else { "incremental" },
        issues_fetched: issues.len(),
        issues_inserted: result.inserted,
        issues_updated: result.updated,
        issues_deleted: result.deleted,
        ..
    })?;

    // Purge old tombstones periodically
    if needs_full_sync {
        db::purge_deleted_issues(&conn, 7)?;
    }

    Ok(())
}

fn should_do_full_sync(sync_state: &Option<SyncState>) -> bool {
    match sync_state {
        None => true,  // First sync
        Some(state) => {
            // Check if last full sync was > 1 hour ago
            let last_full = state.last_full_sync_at.as_ref()
                .and_then(|t| parse_timestamp(t).ok());
            match last_full {
                None => true,  // Never done full sync
                Some(ts) => Utc::now() - ts > Duration::hours(1),
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

## Forge Trait Changes

Add method for incremental fetch:

```rust
#[async_trait]
pub trait Forge: Send + Sync {
    // Existing
    async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>>;

    // New: fetch issues updated since timestamp
    async fn list_issues_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<Vec<Issue>>;

    // Same pattern for comments and goals
    async fn list_comments_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<Vec<Comment>>;
    async fn list_goals_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<Vec<Goal>>;
}
```

## Implementation Order

1. **Database schema changes** - Add columns, create sync_stats table
2. **Modify `save_issues()`** - UPSERT semantics with full_sync flag
3. **Add `list_issues_since()` to Forge trait** - Define interface
4. **Implement for GitHub** - Add `since` parameter to API calls
5. **Implement for Linear** - Add `updatedAt` filter to GraphQL
6. **Update daemon `sync_once()`** - Use incremental by default, full on schedule
7. **Apply same pattern to comments and goals**
8. **Add configuration options** - `full_sync_interval`, `sync_interval`
9. **Update `isq sync` command** - Default to full sync
10. **Add tombstone purge logic** - Clean up old deleted records

## Testing Strategy

### Unit Tests
- `save_issues()` UPSERT behavior (insert new, update existing)
- `save_issues()` deletion marking during full sync
- `purge_deleted_issues()` TTL logic
- `should_do_full_sync()` timing logic

### Integration Tests
- GitHub `since` parameter produces correct API calls
- Linear `updatedAt` filter produces correct GraphQL
- End-to-end: create issue externally, verify it appears after incremental sync
- End-to-end: delete issue externally, verify it disappears after full reconciliation

### Manual Testing
- Monitor rate limit usage before/after
- Verify daemon logs show "incremental" vs "full" appropriately
- Test with large repo (1000+ issues) to verify performance improvement

## Metrics to Track (sync_stats table)

- Sync type distribution (incremental vs full)
- Average sync duration by type
- Issues fetched per sync (should be lower for incremental)
- Delete detection count (during full syncs)
- Error rates by type

## Rollout

Ship as default behavior. No feature flags needed (single developer, pre-v1).

## Future Considerations

Not implementing now, but worth noting for v2:

- **Adaptive reconciliation**: Start with frequent full syncs, decay over time as cache matures
- **Repo-size-aware scheduling**: Smaller repos get more frequent full syncs
- **User-activity-aware**: Prioritize full sync for repos with recent `isq start` activity
- **Webhook integration**: Real-time updates instead of polling (eliminates need for frequent syncs)
