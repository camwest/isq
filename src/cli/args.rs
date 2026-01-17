//! CLI argument definitions using clap

use clap::{Parser, Subcommand};

/// Parse @view syntax, stripping the @ prefix
fn parse_view_arg(s: &str) -> Result<String, String> {
    if s.starts_with('@') {
        Ok(s[1..].to_string())
    } else {
        Err(format!(
            "View name must start with @, got: {}. Use @{} to reference a view.",
            s, s
        ))
    }
}

#[derive(Parser)]
#[command(name = "isq")]
#[command(about = "Instant issue tracking. Offline-first. AI-agent native.")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Link this repo to an issue tracker
    Link {
        /// Forge name
        forge: Option<String>,
        /// Forge-specific options (e.g., -o team=Engineering)
        #[arg(short = 'o', long = "opt")]
        opt: Vec<String>,
    },

    /// Unlink this repo from its issue tracker
    Unlink,

    /// Remove stored credentials for an issue tracker
    Logout {
        /// Forge name (github, linear)
        forge: Option<String>,
    },

    /// Show status (auth, link, daemon)
    Status,

    /// Issue operations
    Issue {
        #[command(subcommand)]
        command: IssueCommands,
    },

    /// Daemon operations
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    /// Sync issues from remote
    Sync,

    /// Goal operations (milestones/projects)
    Goal {
        #[command(subcommand)]
        command: GoalCommands,
    },

    /// Show current issue for this worktree
    Current {
        /// Suppress output if no issue set (exit code 1)
        #[arg(short, long)]
        quiet: bool,
    },

    /// Start working on an issue (creates worktree)
    Start {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,
    },

    /// Clean up current worktree (remove worktree, clear association)
    Cleanup {
        /// Keep the worktree directory, only clear the issue association
        #[arg(long)]
        keep: bool,
    },

    /// Label operations (list/create repository labels)
    Label {
        #[command(subcommand)]
        command: LabelCommands,
    },

    /// Forge-specific commands (e.g., isq forge jira list-fields)
    Forge {
        /// Forge name (github, linear, jira)
        forge: String,
        /// Subcommand and arguments
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Manage custom views (saved filter combinations)
    View {
        #[command(subcommand)]
        command: ViewCommands,
    },
}

#[derive(Subcommand)]
pub enum IssueCommands {
    /// List issues
    List {
        /// Use a saved view (e.g., @bugs)
        #[arg(value_parser = parse_view_arg)]
        view: Option<String>,

        /// Filter by specific issue IDs (comma-separated, e.g., --id 7,12,45)
        #[arg(long)]
        id: Option<String>,

        /// Filter by label
        #[arg(long)]
        label: Option<String>,

        /// Filter by state (open, closed, all). Defaults to open.
        #[arg(long)]
        state: Option<String>,

        /// Show all issues (including closed). Shorthand for --state=all.
        #[arg(long)]
        all: bool,

        /// Show only issues assigned to me
        #[arg(long)]
        mine: bool,

        /// Show only unassigned issues
        #[arg(long)]
        unassigned: bool,

        /// Show only open issues (shorthand for --state=open)
        #[arg(long)]
        open: bool,

        /// Filter by goal/milestone name
        #[arg(long)]
        goal: Option<String>,

        /// Sort order: priority (default), newest, oldest, updated
        #[arg(long, default_value = "priority")]
        sort: String,

        /// Forge-specific options (e.g., -o jql="...", -o type=Bug)
        #[arg(short = 'o', long = "opt")]
        opt: Vec<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show a single issue
    Show {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new issue
    Create {
        /// Issue title
        #[arg(long)]
        title: String,

        /// Issue body
        #[arg(long)]
        body: Option<String>,

        /// Labels to add
        #[arg(long)]
        label: Vec<String>,

        /// Goal to assign the issue to
        #[arg(long)]
        goal: Option<String>,

        /// Forge-specific options (e.g., -o type=Bug)
        #[arg(short = 'o', long = "opt")]
        opt: Vec<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Add a comment to an issue
    Comment {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,

        /// Comment body
        message: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Close an issue
    Close {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Reopen an issue
    Reopen {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage labels on an issue
    Label {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,

        /// Action: add or remove
        action: String,

        /// Label name
        label: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Assign a user to an issue
    Assign {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,

        /// Username to assign
        user: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum GoalCommands {
    /// List goals
    List {
        /// Filter by state (open, closed, all)
        #[arg(long, default_value = "open")]
        state: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show a goal with its issues
    Show {
        /// Goal name or ID
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new goal
    Create {
        /// Goal name
        name: String,

        /// Target date (YYYY-MM-DD)
        #[arg(long)]
        target: Option<String>,

        /// Description
        #[arg(long)]
        body: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Assign an issue to a goal
    Assign {
        /// Issue ID (e.g., 123 or DEV-123)
        issue: String,

        /// Goal name or ID
        goal: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Close a goal
    Close {
        /// Goal name or ID
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum DaemonCommands {
    /// Show daemon status and watched repos
    Status,

    /// Start the daemon
    Start,

    /// Stop the daemon
    Stop,

    /// Add current repo to watch list
    Watch,

    /// Remove current repo from watch list
    Unwatch,

    /// Run the sync loop (internal, called by spawn)
    #[command(hide = true)]
    Run,
}

#[derive(Subcommand)]
pub enum LabelCommands {
    /// List all labels in the repository
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new label
    Create {
        /// Label name
        name: String,

        /// Label color (hex, e.g., "ff0000" or "#ff0000")
        #[arg(long)]
        color: Option<String>,

        /// Label description
        #[arg(long)]
        description: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ViewCommands {
    /// Create a new view
    Create {
        /// View name
        name: String,

        /// Filter by label
        #[arg(long)]
        label: Option<String>,

        /// Exclude issues with this label
        #[arg(long)]
        label_not: Option<String>,

        /// Include issues with any of these labels (comma-separated)
        #[arg(long)]
        label_any: Option<String>,

        /// Filter by state (open, closed)
        #[arg(long)]
        state: Option<String>,

        /// Show only issues assigned to me
        #[arg(long)]
        mine: bool,

        /// Show only unassigned issues
        #[arg(long)]
        unassigned: bool,

        /// Filter by goal/milestone
        #[arg(long)]
        goal: Option<String>,

        /// Filter by exact priority
        #[arg(long)]
        priority: Option<u8>,

        /// Filter by priority <= value
        #[arg(long)]
        priority_lte: Option<u8>,

        /// Filter by priority >= value
        #[arg(long)]
        priority_gte: Option<u8>,

        /// Filter issues not updated in this duration (e.g., "30 days")
        #[arg(long)]
        updated_before: Option<String>,

        /// Filter issues updated within this duration
        #[arg(long)]
        updated_after: Option<String>,

        /// Filter issues created before this duration (e.g., "30 days")
        #[arg(long)]
        created_before: Option<String>,

        /// Filter issues created within this duration
        #[arg(long)]
        created_after: Option<String>,

        /// Sort order (priority, newest, oldest, updated)
        #[arg(long)]
        sort: Option<String>,
    },

    /// List all views
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show view details
    Show {
        /// View name
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Delete a view
    Delete {
        /// View name
        name: String,
    },
}
