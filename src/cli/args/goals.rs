//! Goal command argument definitions

use clap::Subcommand;

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
