//! Time formatting utilities for display.

use chrono::{DateTime, Datelike, Utc};

/// Format a timestamp as relative time (e.g., "5d ago", "2h ago", "just now")
pub fn relative_time(timestamp: &str) -> String {
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
pub fn compact_date(timestamp: &str) -> String {
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
