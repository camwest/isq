//! CLI argument definitions using clap

mod goals;
mod issues;
mod views;

pub use goals::GoalCommands;
pub use issues::IssueCommands;
pub use views::ViewCommands;

use clap::{Parser, Subcommand};

/// Parse @view syntax, stripping the @ prefix
fn parse_view_arg(s: &str) -> Result<String, String> {
    if let Some(stripped) = s.strip_prefix('@') {
        Ok(stripped.to_string())
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
pub struct Cli {
    /// Print version information
    #[arg(short = 'V', long = "version")]
    pub version: bool,

    /// Suppress success messages (errors still shown)
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Increase logging verbosity (-v=info, -vv=debug, -vvv=trace)
    ///
    /// Can also use ISQ_LOG env var for fine-grained control:
    /// ISQ_LOG=debug or ISQ_LOG=isq::forges=trace
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Disable colored output
    #[arg(long, global = true)]
    pub no_color: bool,

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

    /// Diagnose common issues and suggest fixes
    Doctor {
        /// Show detailed diagnostics
        #[arg(short, long)]
        verbose: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Run specific check only (auth, repo, sync, service, database, network)
        #[arg(long)]
        check: Option<String>,
    },

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

    /// Install management (internal)
    #[command(hide = true)]
    Install {
        #[command(subcommand)]
        command: InstallCommands,
    },

    /// Check for and install updates
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },

    /// Uninstall isq (stops daemon, removes config/cache)
    Uninstall {
        /// Keep configuration directory (~/.config/isq)
        #[arg(long)]
        keep_config: bool,

        /// Keep cache directory (issue database)
        #[arg(long)]
        keep_cache: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,

        /// Show what would be removed without removing
        #[arg(long)]
        dry_run: bool,
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

    /// Show daemon logs
    Logs {
        /// Number of lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,

        /// Follow log output (like tail -f)
        #[arg(short = 'f', long)]
        follow: bool,
    },

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
pub enum InstallCommands {
    /// Write install receipt (called by installer scripts)
    WriteReceipt {
        /// Installation method (standalone, homebrew, scoop, cargo)
        #[arg(long)]
        method: String,

        /// Path to the isq binary
        #[arg(long)]
        binary_path: std::path::PathBuf,

        /// Enable auto-update
        #[arg(long)]
        auto_update: bool,
    },
}

#[derive(Subcommand)]
pub enum UpdateCommands {
    /// Check if a newer version is available
    Check {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Download and install the latest version
    Install,
}
