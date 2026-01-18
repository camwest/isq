//! View command argument definitions

use clap::Subcommand;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // CLI enums are constructed once, perf doesn't matter
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

        /// Display as hierarchical tree
        #[arg(long)]
        tree: bool,

        /// Show flat list including all sub-issues
        #[arg(long)]
        flat: bool,

        /// Show only root issues (those without a parent)
        #[arg(long)]
        root_only: bool,

        /// Show only children of a specific issue ID
        #[arg(long)]
        children_of: Option<String>,
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
