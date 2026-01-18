//! Terminal utilities for display.

use std::io::IsTerminal;

/// Check if stdout is a terminal (for color support)
pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Get terminal width, defaulting to 80 if unavailable
pub fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

/// Check if terminal supports true color
pub fn supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v == "truecolor" || v == "24bit")
        .unwrap_or(false)
}

/// Parse hex color string to RGB tuple
pub fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
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
pub fn luminance(r: u8, g: u8, b: u8) -> f64 {
    0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64)
}
