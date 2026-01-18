//! Issue display formatting.

use colored::Colorize;
use textwrap::{Options, wrap};

use crate::db::Comment;
use crate::forges::Issue;

use super::labels::format_labels;
use super::terminal::{is_tty, term_width};
use super::time::{compact_date, relative_time};

/// Format an issue ID for display
/// Returns "#123" for numeric IDs (GitHub), or the ID as-is for string IDs (Linear/JIRA)
pub fn format_issue_id(id: &str) -> String {
    // If the ID is purely numeric, prefix with #
    if id.chars().all(|c| c.is_ascii_digit()) {
        format!("#{}", id)
    } else {
        id.to_string()
    }
}

/// Format a state indicator (filled/empty circle) with appropriate color
fn state_indicator(state: &str, tty: bool) -> String {
    match (state, tty) {
        ("open", true) => "●".green().to_string(),
        ("open", false) => "●".to_string(),
        (_, true) => "○".red().to_string(),
        (_, false) => "○".to_string(),
    }
}

/// Wrap text with consistent indentation
fn wrap_indented(text: &str, indent: &str, width: usize) -> String {
    let effective_width = width.saturating_sub(indent.len());
    let opts = Options::new(effective_width);

    let mut result = String::new();
    for line in text.lines() {
        if line.is_empty() {
            result.push('\n');
        } else {
            for wrapped in wrap(line, &opts) {
                result.push_str(indent);
                result.push_str(&wrapped);
                result.push('\n');
            }
        }
    }
    result
}

/// Format an issue detail view as a string (without timing footer)
pub fn format_issue(issue: &Issue, comments: &[Comment]) -> String {
    let tty = is_tty();
    let mut output = String::new();

    // Title line - format ID: "#123" for GitHub, "DEV-123" for Linear/JIRA
    let issue_id_display = format_issue_id(&issue.id);
    let title_line = format!("  {} {}", issue_id_display, issue.title);
    if tty {
        output.push_str(&format!("{}\n", title_line.bold()));
    } else {
        output.push_str(&format!("{}\n", title_line));
    }

    // Heavy separator
    let separator = "━".repeat(60);
    if tty {
        output.push_str(&format!(" {}\n", separator.dimmed()));
    } else {
        output.push_str(&format!(" {}\n", separator));
    }

    // State + author + labels line
    let state_ind = state_indicator(&issue.state, tty);

    let author = format!("@{}", issue.author);
    let labels_str = format_labels(&issue.labels, tty);

    let mut meta_parts = vec![state_ind, issue.state.clone()];

    if tty {
        meta_parts.push(author.cyan().to_string());
    } else {
        meta_parts.push(author);
    }

    if !labels_str.is_empty() {
        meta_parts.push(labels_str);
    }

    // Add milestone/goal if present
    if let Some(milestone) = &issue.milestone {
        let goal_str = format!("→ {}", milestone);
        if tty {
            meta_parts.push(goal_str.cyan().to_string());
        } else {
            meta_parts.push(goal_str);
        }
    }

    let meta_line = format!("  {}", meta_parts.join("   "));
    output.push_str(&format!("{}\n", meta_line));

    // Timestamps line
    let created = relative_time(&issue.created_at);
    let updated = relative_time(&issue.updated_at);
    let time_line = format!("  {} · updated {}", created, updated);
    if tty {
        output.push_str(&format!("{}\n", time_line.dimmed()));
    } else {
        output.push_str(&format!("{}\n", time_line));
    }

    // URL line (in header, not footer) - keep https:// for terminal clickability
    if let Some(url) = &issue.url {
        if tty {
            output.push_str(&format!(
                "  {} {}\n",
                "↗".dimmed(),
                url.dimmed().underline()
            ));
        } else {
            output.push_str(&format!("  {}\n", url));
        }
    }

    // Body (wrapped to terminal width with indent)
    if let Some(body) = &issue.body
        && !body.trim().is_empty()
    {
        output.push('\n');
        let width = term_width();
        output.push_str(&wrap_indented(body, "  ", width));
    }

    // Comments section
    if !comments.is_empty() {
        output.push('\n');
        let light_separator = "─".repeat(60);
        if tty {
            output.push_str(&format!(" {}\n", light_separator.dimmed()));
        } else {
            output.push_str(&format!(" {}\n", light_separator));
        }

        let comments_header = format!(
            "  {} comment{}",
            comments.len(),
            if comments.len() == 1 { "" } else { "s" }
        );
        if tty {
            output.push_str(&format!("{}\n", comments_header.bold()));
        } else {
            output.push_str(&format!("{}\n", comments_header));
        }
        output.push('\n');

        for c in comments {
            let comment_author = format!("@{}", c.author);
            let comment_time = relative_time(&c.created_at);

            if tty {
                output.push_str(&format!(
                    "  {} · {}\n",
                    comment_author.cyan(),
                    comment_time.dimmed()
                ));
            } else {
                output.push_str(&format!("  {} · {}\n", comment_author, comment_time));
            }

            // Indent comment body (wrapped)
            let width = term_width();
            output.push_str(&wrap_indented(&c.body, "  ", width));
            output.push('\n');
        }
    }

    output
}

/// Print timing footer to stderr
pub fn print_timing_footer(elapsed_ms: u64) {
    eprintln!();
    if is_tty() {
        eprintln!("{}", format!("  Loaded in {}ms", elapsed_ms).dimmed());
    } else {
        eprintln!("  Loaded in {}ms", elapsed_ms);
    }
}

/// Print a styled issue detail view (convenience wrapper)
pub fn print_issue(issue: &Issue, comments: &[Comment], elapsed_ms: u64) {
    let output = format_issue(issue, comments);
    print!("{}", output);
    print_timing_footer(elapsed_ms);
}

/// Convert priority level to display indicator (Linear-style)
/// 0=urgent, 1=high, 2=medium, 3=low, 4=none
pub(crate) fn priority_indicator(priority: u8) -> &'static str {
    match priority {
        0 => "[!]", // Urgent
        1 => "▰▰▰", // High
        2 => "▰▰▱", // Medium
        3 => "▰▱▱", // Low
        _ => "---", // None or unknown
    }
}

/// Print a compact issue list row with optional comment count
pub fn print_issue_row(issue: &Issue, comment_count: Option<usize>) {
    let tty = is_tty();

    let priority_str = priority_indicator(issue.priority);

    let state_char = state_indicator(&issue.state, tty);

    let labels_str = format_labels(&issue.labels, tty);

    // Format goal/milestone (if present)
    let goal_str = issue
        .milestone
        .as_ref()
        .map(|m| format!(" → {}", m))
        .unwrap_or_default();

    // Format comment count
    let comment_str = match comment_count {
        Some(0) | None => String::new(),
        Some(count) => format!(" 💬{}", count),
    };

    // Format created date
    let date_str = compact_date(&issue.created_at);

    // Format issue ID: "#123" for GitHub, "DEV-123" for Linear/JIRA
    let issue_id = format_issue_id(&issue.id);

    if tty {
        println!(
            "{} {}  {:>10}  {}{}{}  {}{}",
            state_char,
            priority_str,
            issue_id.dimmed(),
            issue.title,
            labels_str,
            goal_str.cyan(),
            date_str.dimmed(),
            comment_str.dimmed()
        );
    } else {
        println!(
            "{} {}  {:<10}  {}{}{}  {}{}",
            state_char,
            priority_str,
            issue_id,
            issue.title,
            labels_str,
            goal_str,
            date_str,
            comment_str
        );
    }
}
