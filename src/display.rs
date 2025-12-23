//! Styled terminal output for issue display
//!
//! Design principles:
//! - Visual hierarchy: title prominent, metadata dimmed
//! - Semantic colors: green=open, red=closed
//! - Relative timestamps: "5d ago" vs ISO format
//! - Graceful degradation: plain text when not a TTY

use std::io::IsTerminal;

use chrono::{DateTime, Utc};
use colored::Colorize;
use textwrap::{wrap, Options};

use crate::db::Comment;
use crate::forges::{Goal, GoalState, Issue};

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

/// Check if stdout is a terminal (for color support)
fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Get terminal width, defaulting to 80 if unavailable
fn term_width() -> usize {
    // Try to get terminal size, fall back to 80
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
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

/// Print a styled issue detail view
pub fn print_issue(issue: &Issue, comments: &[Comment], elapsed_ms: u64) {
    let tty = is_tty();

    // Title line
    let title_line = format!("  #{} {}", issue.number, issue.title);
    if tty {
        println!("{}", title_line.bold());
    } else {
        println!("{}", title_line);
    }

    // Heavy separator
    let separator = "━".repeat(60);
    if tty {
        println!(" {}", separator.dimmed());
    } else {
        println!(" {}", separator);
    }

    // State + author + labels line
    let state_indicator = if issue.state == "open" {
        if tty {
            "●".green().to_string()
        } else {
            "●".to_string()
        }
    } else {
        if tty {
            "●".red().to_string()
        } else {
            "○".to_string()
        }
    };

    let author = format!("@{}", issue.author);
    let labels: Vec<&str> = issue.labels.iter().map(|s| s.as_str()).collect();
    let labels_str = labels.join(", ");

    let mut meta_parts = vec![
        state_indicator,
        issue.state.clone(),
    ];

    if tty {
        meta_parts.push(author.cyan().to_string());
    } else {
        meta_parts.push(author);
    }

    if !labels_str.is_empty() {
        if tty {
            meta_parts.push(labels_str.yellow().to_string());
        } else {
            meta_parts.push(labels_str);
        }
    }

    let meta_line = format!("  {}", meta_parts.join("   "));
    println!("{}", meta_line);

    // Timestamps line
    let created = relative_time(&issue.created_at);
    let updated = relative_time(&issue.updated_at);
    let time_line = format!("  {} · updated {}", created, updated);
    if tty {
        println!("{}", time_line.dimmed());
    } else {
        println!("{}", time_line);
    }

    // URL line (in header, not footer) - keep https:// for terminal clickability
    if let Some(url) = &issue.url {
        if tty {
            println!("  {} {}", "↗".dimmed(), url.dimmed().underline());
        } else {
            println!("  {}", url);
        }
    }

    // Body (wrapped to terminal width with indent)
    if let Some(body) = &issue.body {
        if !body.trim().is_empty() {
            println!();
            let width = term_width();
            print!("{}", wrap_indented(body, "  ", width));
        }
    }

    // Comments section
    if !comments.is_empty() {
        println!();
        let light_separator = "─".repeat(60);
        if tty {
            println!(" {}", light_separator.dimmed());
        } else {
            println!(" {}", light_separator);
        }

        let comments_header = format!("  {} comment{}", comments.len(), if comments.len() == 1 { "" } else { "s" });
        if tty {
            println!("{}", comments_header.bold());
        } else {
            println!("{}", comments_header);
        }
        println!();

        for c in comments {
            let comment_author = format!("@{}", c.author);
            let comment_time = relative_time(&c.created_at);

            if tty {
                println!("  {} · {}", comment_author.cyan(), comment_time.dimmed());
            } else {
                println!("  {} · {}", comment_author, comment_time);
            }

            // Indent comment body (wrapped)
            let width = term_width();
            print!("{}", wrap_indented(&c.body, "  ", width));
            println!();
        }
    }

    // Timing footer
    if tty {
        eprintln!();
        eprintln!("{}", format!("  Loaded in {}ms", elapsed_ms).dimmed());
    } else {
        eprintln!();
        eprintln!("  Loaded in {}ms", elapsed_ms);
    }
}

/// Print a compact issue list row with optional comment count
pub fn print_issue_row(issue: &Issue, comment_count: Option<usize>) {
    let tty = is_tty();

    let state_char = if issue.state == "open" {
        if tty {
            "●".green().to_string()
        } else {
            "●".to_string()
        }
    } else {
        if tty {
            "○".red().to_string()
        } else {
            "○".to_string()
        }
    };

    let labels: Vec<&str> = issue.labels.iter().map(|s| s.as_str()).collect();
    let labels_str = if labels.is_empty() {
        String::new()
    } else {
        format!(" [{}]", labels.join(", "))
    };

    // Format comment count (dimmed)
    let comment_str = match comment_count {
        Some(0) | None => String::new(),
        Some(count) => format!(" 💬{}", count),
    };

    if tty {
        println!(
            "{} {:>5}  {}{}{}",
            state_char,
            format!("#{}", issue.number).dimmed(),
            issue.title,
            labels_str.yellow(),
            comment_str.dimmed()
        );
    } else {
        println!(
            "{} #{:<5}  {}{}{}",
            state_char,
            issue.number,
            issue.title,
            labels_str,
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

        let total = goal.open_count + goal.closed_count;
        let progress = if total > 0 {
            format!("{}/{}", goal.closed_count, total)
        } else {
            "0/0".to_string()
        };

        let target = goal
            .target_date
            .as_ref()
            .map(|d| format!("→ {}", d))
            .unwrap_or_default();

        // Avoid dimmed colors - they're unreadable on light terminals
        println!(
            "{} {:>8}  {}  {}",
            status_char,
            progress,
            goal.name,
            target
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
    let total = goal.open_count + goal.closed_count;
    let pct = if total > 0 {
        (goal.closed_count * 100 / total) as usize
    } else {
        0
    };
    let filled = pct / 10;
    let bar = format!(
        "[{}{}] {}% ({}/{})",
        "=".repeat(filled),
        "-".repeat(10 - filled),
        pct,
        goal.closed_count,
        total
    );

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
}
