//! Database schema definitions and migrations

use anyhow::Result;
use rusqlite::Connection;

/// Initialize database schema with all tables and run migrations
pub(crate) fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS issues (
            id INTEGER PRIMARY KEY,
            repo TEXT NOT NULL,
            number INTEGER NOT NULL,
            title TEXT NOT NULL,
            body TEXT,
            state TEXT NOT NULL,
            author TEXT NOT NULL,
            labels TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            html_url TEXT,
            UNIQUE(repo, number)
        );

        CREATE INDEX IF NOT EXISTS idx_issues_repo ON issues(repo);
        CREATE INDEX IF NOT EXISTS idx_issues_repo_number ON issues(repo, number);

        CREATE TABLE IF NOT EXISTS sync_state (
            repo TEXT PRIMARY KEY,
            last_sync TEXT NOT NULL,
            issue_count INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pending_ops (
            id INTEGER PRIMARY KEY,
            repo TEXT NOT NULL,
            op_type TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_pending_ops_repo ON pending_ops(repo);

        CREATE TABLE IF NOT EXISTS watched_repos (
            repo TEXT PRIMARY KEY,
            last_accessed TEXT NOT NULL,
            added_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS repo_links (
            repo_path TEXT PRIMARY KEY,
            forge_type TEXT NOT NULL,
            forge_repo TEXT NOT NULL,
            display_name TEXT,
            username TEXT,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS comments (
            id INTEGER PRIMARY KEY,
            forge_repo TEXT NOT NULL,
            issue_number INTEGER NOT NULL,
            comment_id TEXT NOT NULL,
            body TEXT NOT NULL,
            author TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(forge_repo, comment_id)
        );

        CREATE INDEX IF NOT EXISTS idx_comments_issue ON comments(forge_repo, issue_number);

        CREATE TABLE IF NOT EXISTS goals (
            id INTEGER PRIMARY KEY,
            forge_repo TEXT NOT NULL,
            goal_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            target_date TEXT,
            state TEXT NOT NULL,
            open_count INTEGER DEFAULT 0,
            closed_count INTEGER DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            html_url TEXT,
            UNIQUE(forge_repo, goal_id)
        );

        CREATE INDEX IF NOT EXISTS idx_goals_repo ON goals(forge_repo);
        CREATE INDEX IF NOT EXISTS idx_goals_state ON goals(forge_repo, state);

        CREATE TABLE IF NOT EXISTS rate_limit_state (
            forge TEXT PRIMARY KEY,
            rate_limit INTEGER,
            remaining INTEGER,
            reset_at INTEGER,
            last_error TEXT,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS worktree_issues (
            git_dir TEXT NOT NULL,
            repo TEXT NOT NULL,
            issue_number INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (git_dir, issue_number)
        );

        CREATE INDEX IF NOT EXISTS idx_worktree_issues_git_dir ON worktree_issues(git_dir);
        ",
    )?;

    run_migrations(conn)?;

    // Create sync_stats table for tracking sync history
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sync_stats (
            id INTEGER PRIMARY KEY,
            repo TEXT NOT NULL,
            data_type TEXT NOT NULL,
            sync_type TEXT NOT NULL,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            duration_ms INTEGER,
            items_fetched INTEGER,
            items_inserted INTEGER,
            items_updated INTEGER,
            items_deleted INTEGER,
            is_complete INTEGER,
            error TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_sync_stats_repo ON sync_stats(repo);
        CREATE INDEX IF NOT EXISTS idx_sync_stats_started ON sync_stats(started_at);
        ",
    )?;

    Ok(())
}

/// Run all schema migrations
fn run_migrations(conn: &Connection) -> Result<()> {
    // Migration: add display_name column if it doesn't exist
    // SQLite doesn't have IF NOT EXISTS for ALTER TABLE, so we check the schema
    let has_display_name: bool = conn
        .prepare("SELECT display_name FROM repo_links LIMIT 0")
        .is_ok();
    if !has_display_name {
        conn.execute("ALTER TABLE repo_links ADD COLUMN display_name TEXT", [])?;
    }

    // Migration: add html_url column to issues if it doesn't exist
    let has_html_url: bool = conn
        .prepare("SELECT html_url FROM issues LIMIT 0")
        .is_ok();
    if !has_html_url {
        conn.execute("ALTER TABLE issues ADD COLUMN html_url TEXT", [])?;
    }

    // Migration: add milestone column to issues if it doesn't exist
    let has_milestone: bool = conn
        .prepare("SELECT milestone FROM issues LIMIT 0")
        .is_ok();
    if !has_milestone {
        conn.execute("ALTER TABLE issues ADD COLUMN milestone TEXT", [])?;
    }

    // Migration: add progress column to goals if it doesn't exist
    let has_progress: bool = conn
        .prepare("SELECT progress FROM goals LIMIT 0")
        .is_ok();
    if !has_progress {
        conn.execute(
            "ALTER TABLE goals ADD COLUMN progress REAL DEFAULT 0.0",
            [],
        )?;
    }

    // Migration: add rate_limit and remaining columns to rate_limit_state if they don't exist
    let has_rate_limit: bool = conn
        .prepare("SELECT rate_limit FROM rate_limit_state LIMIT 0")
        .is_ok();
    if !has_rate_limit {
        conn.execute(
            "ALTER TABLE rate_limit_state ADD COLUMN rate_limit INTEGER",
            [],
        )?;
    }
    let has_remaining: bool = conn
        .prepare("SELECT remaining FROM rate_limit_state LIMIT 0")
        .is_ok();
    if !has_remaining {
        conn.execute(
            "ALTER TABLE rate_limit_state ADD COLUMN remaining INTEGER",
            [],
        )?;
    }

    // Migration: rename username column to user_id (or add user_id if neither exists)
    let has_user_id: bool = conn
        .prepare("SELECT user_id FROM repo_links LIMIT 0")
        .is_ok();
    if !has_user_id {
        let has_username: bool = conn
            .prepare("SELECT username FROM repo_links LIMIT 0")
            .is_ok();
        if has_username {
            // Rename existing column
            conn.execute(
                "ALTER TABLE repo_links RENAME COLUMN username TO user_id",
                [],
            )?;
        } else {
            // Add new column
            conn.execute("ALTER TABLE repo_links ADD COLUMN user_id TEXT", [])?;
        }
    }

    // Migration: add user_name column to repo_links if it doesn't exist
    let has_user_name: bool = conn
        .prepare("SELECT user_name FROM repo_links LIMIT 0")
        .is_ok();
    if !has_user_name {
        conn.execute("ALTER TABLE repo_links ADD COLUMN user_name TEXT", [])?;
    }

    // Migration: add assignees column to issues if it doesn't exist
    let has_assignees: bool = conn
        .prepare("SELECT assignees FROM issues LIMIT 0")
        .is_ok();
    if !has_assignees {
        conn.execute(
            "ALTER TABLE issues ADD COLUMN assignees TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }

    // Migration: add priority column to issues if it doesn't exist
    let has_priority: bool = conn
        .prepare("SELECT priority FROM issues LIMIT 0")
        .is_ok();
    if !has_priority {
        conn.execute(
            "ALTER TABLE issues ADD COLUMN priority INTEGER NOT NULL DEFAULT 4",
            [],
        )?;
    }

    // Migration: add priority_label column to issues if it doesn't exist
    let has_priority_label: bool = conn
        .prepare("SELECT priority_label FROM issues LIMIT 0")
        .is_ok();
    if !has_priority_label {
        conn.execute("ALTER TABLE issues ADD COLUMN priority_label TEXT", [])?;
    }

    // ========================================================================
    // Incremental sync migrations
    // ========================================================================

    // Migration: add per-type sync cursors to sync_state
    let has_issues_last_sync: bool = conn
        .prepare("SELECT issues_last_sync FROM sync_state LIMIT 0")
        .is_ok();
    if !has_issues_last_sync {
        conn.execute(
            "ALTER TABLE sync_state ADD COLUMN issues_last_sync TEXT",
            [],
        )?;
    }

    let has_comments_last_sync: bool = conn
        .prepare("SELECT comments_last_sync FROM sync_state LIMIT 0")
        .is_ok();
    if !has_comments_last_sync {
        conn.execute(
            "ALTER TABLE sync_state ADD COLUMN comments_last_sync TEXT",
            [],
        )?;
    }

    let has_goals_last_sync: bool = conn
        .prepare("SELECT goals_last_sync FROM sync_state LIMIT 0")
        .is_ok();
    if !has_goals_last_sync {
        conn.execute(
            "ALTER TABLE sync_state ADD COLUMN goals_last_sync TEXT",
            [],
        )?;
    }

    let has_last_full_sync_at: bool = conn
        .prepare("SELECT last_full_sync_at FROM sync_state LIMIT 0")
        .is_ok();
    if !has_last_full_sync_at {
        conn.execute(
            "ALTER TABLE sync_state ADD COLUMN last_full_sync_at TEXT",
            [],
        )?;
    }

    // Migration: add soft-delete columns to issues
    let has_issues_deleted: bool = conn
        .prepare("SELECT deleted FROM issues LIMIT 0")
        .is_ok();
    if !has_issues_deleted {
        conn.execute(
            "ALTER TABLE issues ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    let has_issues_deleted_at: bool = conn
        .prepare("SELECT deleted_at FROM issues LIMIT 0")
        .is_ok();
    if !has_issues_deleted_at {
        conn.execute("ALTER TABLE issues ADD COLUMN deleted_at TEXT", [])?;
    }

    // Migration: add issue_id column to issues if it doesn't exist
    // This replaces number with a string ID
    let has_issue_id: bool = conn.prepare("SELECT issue_id FROM issues LIMIT 0").is_ok();
    if !has_issue_id {
        // Add the new column
        conn.execute("ALTER TABLE issues ADD COLUMN issue_id TEXT", [])?;
        // Migrate existing data: convert number to string
        conn.execute(
            "UPDATE issues SET issue_id = CAST(number AS TEXT) WHERE issue_id IS NULL",
            [],
        )?;
    }

    // Create unique index for issue_id (needed for ON CONFLICT clause)
    // Use IF NOT EXISTS so this is idempotent
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_repo_issue_id ON issues(repo, issue_id)",
        [],
    )?;

    // Migration: add issue_id column to comments if it doesn't exist
    let has_comments_issue_id: bool = conn
        .prepare("SELECT issue_id FROM comments LIMIT 0")
        .is_ok();
    if !has_comments_issue_id {
        conn.execute("ALTER TABLE comments ADD COLUMN issue_id TEXT", [])?;
        // Migrate existing data: convert issue_number to string
        conn.execute(
            "UPDATE comments SET issue_id = CAST(issue_number AS TEXT) WHERE issue_id IS NULL",
            [],
        )?;
        // Create new index
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_comments_issue_id ON comments(forge_repo, issue_id)",
            [],
        )?;
    }

    // Migration: add issue_id column to worktree_issues if it doesn't exist
    let has_worktree_issue_id: bool = conn
        .prepare("SELECT issue_id FROM worktree_issues LIMIT 0")
        .is_ok();
    if !has_worktree_issue_id {
        conn.execute("ALTER TABLE worktree_issues ADD COLUMN issue_id TEXT", [])?;
        // Migrate existing data: convert issue_number to string
        conn.execute(
            "UPDATE worktree_issues SET issue_id = CAST(issue_number AS TEXT) WHERE issue_id IS NULL",
            [],
        )?;
    }

    // Migration: add updated_at and soft-delete columns to comments
    let has_comments_updated_at: bool = conn
        .prepare("SELECT updated_at FROM comments LIMIT 0")
        .is_ok();
    if !has_comments_updated_at {
        conn.execute("ALTER TABLE comments ADD COLUMN updated_at TEXT", [])?;
    }

    let has_comments_deleted: bool = conn
        .prepare("SELECT deleted FROM comments LIMIT 0")
        .is_ok();
    if !has_comments_deleted {
        conn.execute(
            "ALTER TABLE comments ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    let has_comments_deleted_at: bool = conn
        .prepare("SELECT deleted_at FROM comments LIMIT 0")
        .is_ok();
    if !has_comments_deleted_at {
        conn.execute("ALTER TABLE comments ADD COLUMN deleted_at TEXT", [])?;
    }

    // Migration: recreate issues table without UNIQUE(repo, number) constraint
    // The (repo, issue_id) unique index is the correct constraint for string IDs.
    // The old UNIQUE(repo, number) causes conflicts when Linear issues like "WRK-360"
    // are synced because the extracted number (360) may collide with migrated data.
    let table_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='issues'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if table_sql.contains("UNIQUE(repo, number)") {
        conn.execute_batch(
            "
            -- Create new table without UNIQUE(repo, number)
            CREATE TABLE issues_new (
                id INTEGER PRIMARY KEY,
                repo TEXT NOT NULL,
                number INTEGER NOT NULL,
                issue_id TEXT,
                title TEXT NOT NULL,
                body TEXT,
                state TEXT NOT NULL,
                author TEXT NOT NULL,
                labels TEXT NOT NULL,
                assignees TEXT NOT NULL DEFAULT '[]',
                priority INTEGER NOT NULL DEFAULT 4,
                priority_label TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                html_url TEXT,
                milestone TEXT,
                deleted INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT
            );

            -- Copy data
            INSERT INTO issues_new SELECT
                id, repo, number, issue_id, title, body, state, author,
                labels, assignees, priority, priority_label,
                created_at, updated_at, html_url, milestone, deleted, deleted_at
            FROM issues;

            -- Swap tables
            DROP TABLE issues;
            ALTER TABLE issues_new RENAME TO issues;

            -- Recreate indexes
            CREATE INDEX idx_issues_repo ON issues(repo);
            CREATE INDEX idx_issues_repo_number ON issues(repo, number);
            CREATE UNIQUE INDEX idx_issues_repo_issue_id ON issues(repo, issue_id);
            ",
        )?;
    }

    Ok(())
}
