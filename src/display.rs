//! Styled terminal output for issue display
//!
//! Design principles:
//! - Visual hierarchy: title prominent, metadata dimmed
//! - Semantic colors: green=open, red=closed
//! - Relative timestamps: "5d ago" vs ISO format
//! - Graceful degradation: plain text when not a TTY

use std::io::IsTerminal;

use chrono::{DateTime, Datelike, Utc};
use colored::{ColoredString, Colorize};
use textwrap::{Options, wrap};

use crate::db::Comment;
use crate::forges::{Goal, GoalState, Issue, Label};

/// Format a timestamp as relative time (e.g., "5d ago", "2h ago", "just now")
fn relative_time(timestamp: &str) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(timestamp) else {
        return timestamp.to_string();
    };

    let now = Utc::now();
    let duration = now.signed_duration_since(dt.with_timezone(&Utc));

    let seconds = duration.num_seconds();
    if seconds < 0 {
        return "just now".to_string();
    }

    let minutes = duration.num_minutes();
    let hours = duration.num_hours();
    let days = duration.num_days();
    let weeks = days / 7;
    let months = days / 30;
    let years = days / 365;

    if years > 0 {
        format!("{}y ago", years)
    } else if months > 0 {
        format!("{}mo ago", months)
    } else if weeks > 0 {
        format!("{}w ago", weeks)
    } else if days > 0 {
        format!("{}d ago", days)
    } else if hours > 0 {
        format!("{}h ago", hours)
    } else if minutes > 0 {
        format!("{}m ago", minutes)
    } else {
        "just now".to_string()
    }
}

/// Format a timestamp as compact date (e.g., "Dec 22" or "Dec 22 '23" for previous years)
fn compact_date(timestamp: &str) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(timestamp) else {
        return String::new();
    };
    let now = Utc::now();
    if dt.year() == now.year() {
        dt.format("%b %d").to_string()
    } else {
        dt.format("%b %d '%y").to_string()
    }
}

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

/// Check if stdout is a terminal (for color support)
fn is_tty() -> bool {
    std::io::stdout().is_terminal()
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

/// Get terminal width, defaulting to 80 if unavailable
fn term_width() -> usize {
    // Try to get terminal size, fall back to 80
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

/// Parse hex color string to RGB tuple
fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Calculate relative luminance (0.0 = black, 255.0 = white)
fn luminance(r: u8, g: u8, b: u8) -> f64 {
    0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64)
}

/// Check if terminal supports true color
fn supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v == "truecolor" || v == "24bit")
        .unwrap_or(false)
}

/// Render a label with its color (background + auto-contrast text)
fn render_label(label: &Label, tty: bool) -> ColoredString {
    if !tty {
        return label.name.normal();
    }

    match &label.color {
        Some(hex) if supports_truecolor() => {
            if let Some((r, g, b)) = parse_hex_color(hex) {
                let lum = luminance(r, g, b);
                if lum > 127.5 {
                    // Light background -> black text
                    label.name.on_truecolor(r, g, b).truecolor(0, 0, 0)
                } else {
                    // Dark background -> white text
                    label.name.on_truecolor(r, g, b).truecolor(255, 255, 255)
                }
            } else {
                // Invalid hex, fallback to yellow
                label.name.yellow()
            }
        }
        _ => {
            // No color or no truecolor support, fallback to yellow
            label.name.yellow()
        }
    }
}

/// Format labels for display
fn format_labels(labels: &[Label], tty: bool) -> String {
    if labels.is_empty() {
        return String::new();
    }

    if tty && supports_truecolor() {
        // Render each label with its color
        let rendered: Vec<String> = labels
            .iter()
            .map(|l| render_label(l, tty).to_string())
            .collect();
        format!(" {}", rendered.join(" "))
    } else if tty {
        // Fallback: all labels in yellow brackets
        let names: Vec<&str> = labels.iter().map(|l| l.name.as_str()).collect();
        format!(" [{}]", names.join(", ")).yellow().to_string()
    } else {
        // Non-TTY: plain text
        let names: Vec<&str> = labels.iter().map(|l| l.name.as_str()).collect();
        format!(" [{}]", names.join(", "))
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
    if let Some(body) = &issue.body {
        if !body.trim().is_empty() {
            output.push('\n');
            let width = term_width();
            output.push_str(&wrap_indented(body, "  ", width));
        }
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
fn priority_indicator(priority: u8) -> &'static str {
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

/// Print a list of goals
pub fn print_goals(goals: &[Goal]) {
    if goals.is_empty() {
        println!("No goals found.");
        return;
    }

    let tty = is_tty();

    for goal in goals {
        let status_char = match goal.state {
            GoalState::Open => {
                if tty {
                    "●".yellow().to_string()
                } else {
                    "●".to_string()
                }
            }
            GoalState::Closed => {
                if tty {
                    "✓".green().to_string()
                } else {
                    "✓".to_string()
                }
            }
        };

        // Show counts if available, otherwise show percentage
        let progress_str = match (goal.open_count, goal.closed_count) {
            (Some(open), Some(closed)) => {
                let total = open + closed;
                if total > 0 {
                    format!("{}/{}", closed, total)
                } else {
                    "0/0".to_string()
                }
            }
            _ => format!("{}%", (goal.progress * 100.0).round() as u32),
        };

        let target = goal
            .target_date
            .as_ref()
            .map(|d| format!("→ {}", d))
            .unwrap_or_default();

        // Avoid dimmed colors - they're unreadable on light terminals
        println!(
            "{} {:>8}  {}  {}",
            status_char, progress_str, goal.name, target
        );
    }
}

/// Print goal detail view
pub fn print_goal_detail(goal: &Goal, elapsed_ms: u64) {
    let tty = is_tty();
    let width = term_width();

    // Header
    if tty {
        println!("{}", goal.name.bold());
    } else {
        println!("{}", goal.name);
    }

    // Target date
    if let Some(target) = &goal.target_date {
        println!("Target: {}", target);
    }

    // Description
    if let Some(desc) = &goal.description {
        if !desc.trim().is_empty() {
            println!();
            print!("{}", wrap_indented(desc, "", width));
        }
    }

    // Progress bar - use filled/empty that work on both dark and light
    let pct = (goal.progress * 100.0).round() as usize;
    let filled = pct / 10;

    let bar = match (goal.open_count, goal.closed_count) {
        (Some(open), Some(closed)) => {
            let total = open + closed;
            format!(
                "[{}{}] {}% ({}/{})",
                "=".repeat(filled),
                "-".repeat(10 - filled),
                pct,
                closed,
                total
            )
        }
        _ => format!(
            "[{}{}] {}%",
            "=".repeat(filled),
            "-".repeat(10 - filled),
            pct
        ),
    };

    println!();
    println!("{}", bar);

    // State
    let state_str = match goal.state {
        GoalState::Open => {
            if tty {
                format!("Status: {}", "Open".yellow())
            } else {
                "Status: Open".to_string()
            }
        }
        GoalState::Closed => {
            if tty {
                format!("Status: {}", "Closed".green())
            } else {
                "Status: Closed".to_string()
            }
        }
    };
    println!("{}", state_str);

    // URL - underline is fine, but skip dimmed
    if let Some(url) = &goal.html_url {
        println!();
        if tty {
            println!("{}", url.underline());
        } else {
            println!("{}", url);
        }
    }

    // Footer timing
    eprintln!();
    eprintln!("Loaded in {}ms", elapsed_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_time() {
        // Just test the function doesn't panic on various inputs
        assert_eq!(relative_time("invalid"), "invalid");
        assert!(!relative_time("2024-01-01T00:00:00Z").is_empty());
    }

    #[test]
    fn test_parse_hex_color_valid() {
        assert_eq!(parse_hex_color("ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("00ff00"), Some((0, 255, 0)));
        assert_eq!(parse_hex_color("0000ff"), Some((0, 0, 255)));
        assert_eq!(parse_hex_color("4EA7FC"), Some((78, 167, 252)));
    }

    #[test]
    fn test_parse_hex_color_with_hash() {
        assert_eq!(parse_hex_color("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("#4EA7FC"), Some((78, 167, 252)));
    }

    #[test]
    fn test_parse_hex_color_invalid() {
        assert_eq!(parse_hex_color(""), None);
        assert_eq!(parse_hex_color("fff"), None); // Too short
        assert_eq!(parse_hex_color("fffffff"), None); // Too long
        assert_eq!(parse_hex_color("gggggg"), None); // Invalid hex chars
    }

    #[test]
    fn test_luminance() {
        // Black
        assert_eq!(luminance(0, 0, 0), 0.0);
        // White
        assert!((luminance(255, 255, 255) - 255.0).abs() < 0.1);
        // Pure red (0.299 * 255 = 76.245)
        assert!((luminance(255, 0, 0) - 76.245).abs() < 0.1);
    }

    #[test]
    fn test_priority_indicator() {
        assert_eq!(priority_indicator(0), "[!]"); // Urgent
        assert_eq!(priority_indicator(1), "▰▰▰"); // High
        assert_eq!(priority_indicator(2), "▰▰▱"); // Medium
        assert_eq!(priority_indicator(3), "▰▱▱"); // Low
        assert_eq!(priority_indicator(4), "---"); // None
        assert_eq!(priority_indicator(5), "---"); // Unknown defaults to none
        assert_eq!(priority_indicator(255), "---"); // Edge case
    }
}
