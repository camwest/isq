//! Priority configuration from labels
//!
//! Maps GitHub labels to priority levels based on repo configuration.

use crate::forges::Issue;

/// Apply priority from label configuration to issues.
/// This is a pure function extracted for testability.
pub fn apply_priority_from_labels(issues: &mut [Issue], config: &toml::Value) {
    // Parse priority config: { "P0" = 0, "bug" = 1, ... }
    let priority_labels: std::collections::HashMap<String, u8> = config
        .as_table()
        .map(|table| {
            table
                .iter()
                .filter_map(|(label, value)| {
                    let priority = value.as_integer()?;
                    if (0..=4).contains(&priority) {
                        Some((label.clone(), priority as u8))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if priority_labels.is_empty() {
        return;
    }

    // Apply priority from labels to issues
    for issue in issues.iter_mut() {
        // Only apply if priority hasn't been set (default is 4/none)
        if issue.priority == 4 {
            // Find the highest priority label (lowest number)
            let mut best_priority = 4u8;
            let mut best_label: Option<String> = None;

            for label in &issue.labels {
                if let Some(&priority) = priority_labels.get(&label.name)
                    && priority < best_priority
                {
                    best_priority = priority;
                    best_label = Some(label.name.clone());
                }
            }

            if best_priority < 4 {
                issue.priority = best_priority;
                issue.priority_label = best_label;
            }
        }
    }
}
