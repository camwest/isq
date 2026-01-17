//! Pager support for long output
//!
//! Pipes content through a pager (less, more, etc.) when:
//! - stdout is a TTY
//! - content exceeds terminal height
//!
//! Falls back to direct printing if pager unavailable or not a TTY.

use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

/// Pipes content through a pager if stdout is a TTY and content is long.
/// Falls back to direct printing if pager unavailable or not a TTY.
pub fn print_with_pager(content: &str) {
    // Not a TTY - print directly (for piping, LLM agents, etc.)
    if !std::io::stdout().is_terminal() {
        print!("{}", content);
        return;
    }

    // Check if content needs paging (exceeds terminal height)
    let term_height = terminal_size::terminal_size()
        .map(|(_, h)| h.0 as usize)
        .unwrap_or(24);

    let line_count = content.lines().count();
    // Leave 2 lines for prompt/footer
    if line_count <= term_height.saturating_sub(2) {
        print!("{}", content);
        return;
    }

    // Try pager from $PAGER, default to less
    let pager = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());

    // Build pager command with -R flag for less to interpret ANSI color codes
    let mut cmd = Command::new(&pager);
    if pager == "less" || pager.ends_with("/less") {
        cmd.arg("-R");
    }

    match cmd
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(content.as_bytes());
            }
            let _ = child.wait();
        }
        Err(_) => {
            // Fallback: direct print if pager fails
            print!("{}", content);
        }
    }
}
