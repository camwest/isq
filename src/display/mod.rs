//! Styled terminal output for issue display
//!
//! Design principles:
//! - Visual hierarchy: title prominent, metadata dimmed
//! - Semantic colors: green=open, red=closed
//! - Relative timestamps: "5d ago" vs ISO format
//! - Graceful degradation: plain text when not a TTY

mod goals;
mod issues;
mod labels;
mod terminal;
mod time;

#[cfg(test)]
mod tests;

// Re-export public items
pub use goals::{print_goal_detail, print_goals};
pub use issues::{
    format_issue_id, format_issue_with_hierarchy, print_issue, print_issue_row,
    print_issue_row_indented, print_timing_footer,
};
