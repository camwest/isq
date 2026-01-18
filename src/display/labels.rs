//! Label rendering utilities for display.

use colored::{ColoredString, Colorize};

use crate::forges::Label;

use super::terminal::{luminance, parse_hex_color, supports_truecolor};

/// Render a label with its color (background + auto-contrast text)
pub fn render_label(label: &Label, tty: bool) -> ColoredString {
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
pub fn format_labels(labels: &[Label], tty: bool) -> String {
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
