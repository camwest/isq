//! Goal display formatting.

use colored::Colorize;

use crate::forges::{Goal, GoalState};

use super::terminal::{is_tty, term_width};

/// Wrap text with consistent indentation (duplicated from issues.rs for independence)
fn wrap_indented(text: &str, indent: &str, width: usize) -> String {
    use textwrap::{Options, wrap};

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
    if let Some(desc) = &goal.description
        && !desc.trim().is_empty()
    {
        println!();
        print!("{}", wrap_indented(desc, "", width));
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
