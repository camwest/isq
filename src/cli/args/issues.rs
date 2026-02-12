//! Issue command argument definitions

use clap::Subcommand;

#[derive(Subcommand)]
pub enum IssueCommands {
    /// List issues
    List {
        /// Use a saved view (e.g., @bugs)
        #[arg(value_parser = super::parse_view_arg)]
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

        /// Display as hierarchical tree (indented by parent-child relationships)
        #[arg(long)]
        tree: bool,

        /// Show flat list including all sub-issues (overrides default root-only behavior)
        #[arg(long)]
        flat: bool,

        /// Show only root issues (those without a parent). This is the default when hierarchy exists.
        #[arg(long)]
        root_only: bool,

        /// Show only children of a specific issue ID
        #[arg(long)]
        children_of: Option<String>,

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
    #[command(after_help = "\
Examples:
  isq issue create --title \"Bug: login fails\"
  isq issue create --title \"Feature\" --body \"Details here\" --label enhancement
  isq issue create --title \"Urgent fix\" --label bug --label critical
  isq issue create --title \"Sprint task\" --goal \"v1.0\"
  isq issue create --title \"Sub-task\" --parent 123
")]
    Create {
        /// Issue title
        #[arg(long)]
        title: String,

        /// Issue body (can also be piped via stdin)
        #[arg(long)]
        body: Option<String>,

        /// Labels to add
        #[arg(long)]
        label: Vec<String>,

        /// Goal to assign the issue to
        #[arg(long)]
        goal: Option<String>,

        /// Parent issue ID to create a sub-issue under
        #[arg(long)]
        parent: Option<String>,

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

        /// Comment body (can also be piped via stdin)
        message: Option<String>,

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

    /// Edit issue fields (alias: update)
    #[command(alias = "update")]
    Edit {
        /// Issue ID (e.g., 123 or DEV-123)
        id: String,

        /// New title
        #[arg(long)]
        title: Option<String>,

        /// New body text (use "-" to read from stdin)
        #[arg(long)]
        body: Option<String>,

        /// Priority: 0=urgent, 1=high, 2=medium, 3=low, 4=none
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=4))]
        priority: Option<u8>,

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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::args::{Cli, Commands, IssueCommands};

    #[test]
    fn parses_issue_edit_command() {
        let cli = Cli::try_parse_from([
            "isq",
            "issue",
            "edit",
            "WRK-123",
            "--title",
            "Updated title",
            "--priority",
            "2",
        ])
        .expect("expected issue edit command to parse");

        let Some(Commands::Issue { command }) = cli.command else {
            panic!("expected issue command");
        };
        let IssueCommands::Edit {
            id,
            title,
            body,
            priority,
            ..
        } = command
        else {
            panic!("expected edit subcommand");
        };

        assert_eq!(id, "WRK-123");
        assert_eq!(title, Some("Updated title".to_string()));
        assert_eq!(body, None);
        assert_eq!(priority, Some(2));
    }

    #[test]
    fn parses_issue_update_alias_as_edit() {
        let cli = Cli::try_parse_from(["isq", "issue", "update", "123", "--body", "-"])
            .expect("expected issue update alias to parse");

        let Some(Commands::Issue { command }) = cli.command else {
            panic!("expected issue command");
        };
        let IssueCommands::Edit { id, body, .. } = command else {
            panic!("expected edit subcommand");
        };

        assert_eq!(id, "123");
        assert_eq!(body, Some("-".to_string()));
    }
}
