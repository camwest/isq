//! Output formatting for doctor diagnostics

use colored::Colorize;

use super::types::{CheckDetails, CheckResult, CheckSummary, DiagnosticCheck};

/// Print human-readable output grouped by category
pub fn print_human_output(checks: &[DiagnosticCheck], summary: &CheckSummary, verbose: bool) {
    let mut current_category: Option<&str> = None;

    for check in checks {
        // Print category header when it changes
        if current_category != Some(&check.category) {
            if current_category.is_some() {
                println!();
            }
            print_category_header(&check.category, checks);
            current_category = Some(&check.category);
        }

        // Print individual check
        print_check(check, verbose);
    }

    // Print summary
    println!();
    println!("{}", "━".repeat(50));
    print!("Summary: {} passed", format!("{}", summary.pass).green());
    if summary.warn > 0 {
        print!(
            ", {} warning{}",
            format!("{}", summary.warn).yellow(),
            if summary.warn == 1 { "" } else { "s" }
        );
    }
    if summary.fail > 0 {
        print!(
            ", {} failure{}",
            format!("{}", summary.fail).red(),
            if summary.fail == 1 { "" } else { "s" }
        );
    }
    println!();

    if !verbose && (summary.warn > 0 || summary.fail > 0) {
        println!();
        println!("Run {} for detailed diagnostics.", "isq doctor -v".cyan());
    }
}

/// Print category header with overall status
fn print_category_header(category: &str, checks: &[DiagnosticCheck]) {
    let category_checks: Vec<_> = checks.iter().filter(|c| c.category == category).collect();

    let has_fail = category_checks
        .iter()
        .any(|c| matches!(c.result, CheckResult::Fail { .. }));
    let has_warn = category_checks
        .iter()
        .any(|c| matches!(c.result, CheckResult::Warn { .. }));

    let symbol = if has_fail {
        format!("[{}]", "✗".red())
    } else if has_warn {
        format!("[{}]", "!".yellow())
    } else {
        format!("[{}]", "✓".green())
    };

    println!("{} {}", symbol, category.bold());
}

/// Print a single check result
fn print_check(check: &DiagnosticCheck, verbose: bool) {
    match &check.result {
        CheckResult::Pass => {
            print!("    • {}", check.name);
            if let Some(details) = &check.details {
                print_pass_details(details, verbose);
            }
            println!();
        }
        CheckResult::Warn { reason, guidance } => {
            println!("    • {}: {}", check.name, reason.yellow());
            println!("      → {}", guidance.dimmed());
        }
        CheckResult::Fail { reason, guidance } => {
            println!("    • {}: {}", check.name, reason.red());
            println!("      → {}", guidance.dimmed());
        }
    }
}

/// Print details for a passing check
fn print_pass_details(details: &CheckDetails, verbose: bool) {
    if let Some(value) = &details.value {
        print!(": {}", value.dimmed());
    }
    if verbose {
        if let Some(source) = &details.source {
            print!(" ({})", source.dimmed());
        }
        if let Some(latency) = details.latency_ms {
            print!(" [{}ms]", latency);
        }
    }
}
