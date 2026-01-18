//! Doctor command - diagnose common issues and suggest fixes

mod checks;
mod output;
mod system;
mod types;

use anyhow::Result;

use crate::service;
use types::{CheckResult, CheckSummary, DiagnosticReport};

/// Run all diagnostic checks
pub fn cmd_doctor(verbose: bool, json: bool, check_filter: Option<&str>) -> Result<()> {
    let mut all_checks = Vec::new();

    // Determine which checks to run
    let run_auth = check_filter.is_none() || check_filter == Some("auth");
    let run_repo = check_filter.is_none() || check_filter == Some("repo");
    let run_service = check_filter.is_none() || check_filter == Some("service");
    let run_sync = check_filter.is_none() || check_filter == Some("sync");
    let run_database = check_filter.is_none() || check_filter == Some("database");
    let run_network = check_filter.is_none() || check_filter == Some("network");

    // Get service status early (needed for sync health)
    let svc_status = service::status().ok();
    let daemon_running = svc_status.as_ref().is_some_and(|s| s.pid.is_some());

    // Authentication checks
    if run_auth {
        all_checks.extend(checks::check_authentication(verbose));
    }

    // Repository link check
    if run_repo {
        all_checks.extend(checks::check_repository(verbose));
    }

    // Service checks
    if run_service {
        all_checks.extend(checks::check_service(verbose, &svc_status));
    }

    // Sync health check
    if run_sync {
        all_checks.extend(checks::check_sync_health(verbose, daemon_running));
    }

    // Database checks
    if run_database {
        all_checks.extend(system::check_database(verbose));
    }

    // Network checks
    if run_network {
        all_checks.extend(system::check_network(verbose));
    }

    // Calculate summary
    let summary = CheckSummary {
        pass: all_checks
            .iter()
            .filter(|c| matches!(c.result, CheckResult::Pass))
            .count(),
        warn: all_checks
            .iter()
            .filter(|c| matches!(c.result, CheckResult::Warn { .. }))
            .count(),
        fail: all_checks
            .iter()
            .filter(|c| matches!(c.result, CheckResult::Fail { .. }))
            .count(),
    };

    // Check for failures before potentially moving checks
    let has_failures = summary.fail > 0;

    // Output
    if json {
        let report = DiagnosticReport {
            version: env!("CARGO_PKG_VERSION").to_string(),
            checks: all_checks,
            summary,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        output::print_human_output(&all_checks, &summary, verbose);
    }

    // Exit with error if any failures
    if has_failures {
        std::process::exit(1);
    }

    Ok(())
}
