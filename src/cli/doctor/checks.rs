//! Individual diagnostic check implementations

use super::types::{CheckDetails, CheckResult, DiagnosticCheck};
use crate::db::{self, SyncHealth};
use crate::forges::ALL_FORGE_TYPES;
use crate::install::{self, InstallMethod};
use crate::repo;
use crate::service;

/// Check authentication status for all forges
pub fn check_authentication(verbose: bool) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    for forge_type in ALL_FORGE_TYPES {
        let auth = forge_type.auth();
        let name = auth.display_name.to_string();

        let (result, details) = if auth.has_credentials() {
            let source = detect_auth_source(auth);
            (
                CheckResult::Pass,
                Some(CheckDetails {
                    source: if verbose { Some(source) } else { None },
                    path: None,
                    value: Some("ready".to_string()),
                    latency_ms: None,
                }),
            )
        } else {
            (
                CheckResult::Warn {
                    reason: "not configured".to_string(),
                    guidance: format!("Run: {}", auth.link_command),
                },
                None,
            )
        };

        checks.push(DiagnosticCheck {
            category: "Authentication".to_string(),
            name,
            result,
            details,
        });
    }

    checks
}

/// Detect the source of authentication credentials
fn detect_auth_source(auth: &crate::forges::AuthConfig) -> String {
    // Check CLI first
    if let Some(cmd) = auth.cli_command
        && std::process::Command::new(cmd[0])
            .args(&cmd[1..])
            .output()
            .is_ok_and(|o| o.status.success())
    {
        return format!("{} CLI", cmd[0]);
    }

    // Check keyring
    if crate::credentials::get_credential(auth.keyring_service).is_ok_and(|c| c.is_some()) {
        return "keyring".to_string();
    }

    // Check env var
    if std::env::var(auth.env_var).is_ok() {
        return format!("${}", auth.env_var);
    }

    "unknown".to_string()
}

/// Check repository link status
pub fn check_repository(verbose: bool) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    match repo::detect_repo_path() {
        Ok(repo_path) => {
            match db::open().and_then(|conn| db::get_repo_link(&conn, &repo_path)) {
                Ok(Some(link)) => {
                    let display = link.display_name.as_deref().unwrap_or(&link.forge_repo);
                    checks.push(DiagnosticCheck {
                        category: "Repository".to_string(),
                        name: "Link".to_string(),
                        result: CheckResult::Pass,
                        details: Some(CheckDetails {
                            source: None,
                            path: if verbose { Some(repo_path) } else { None },
                            value: Some(format!("{} ({})", display, link.forge_type)),
                            latency_ms: None,
                        }),
                    });

                    // Also report issue count if available
                    if let Ok(conn) = db::open()
                        && let Ok(Some(state)) = db::get_sync_state(&conn, &link.forge_repo)
                    {
                        checks.push(DiagnosticCheck {
                            category: "Repository".to_string(),
                            name: "Cached issues".to_string(),
                            result: CheckResult::Pass,
                            details: Some(CheckDetails {
                                source: None,
                                path: None,
                                value: Some(format!("{}", state.issue_count)),
                                latency_ms: None,
                            }),
                        });
                    }
                }
                Ok(None) => {
                    checks.push(DiagnosticCheck {
                        category: "Repository".to_string(),
                        name: "Link".to_string(),
                        result: CheckResult::Warn {
                            reason: "not linked".to_string(),
                            guidance: "Run: isq link <forge>".to_string(),
                        },
                        details: None,
                    });
                }
                Err(e) => {
                    checks.push(DiagnosticCheck {
                        category: "Repository".to_string(),
                        name: "Link".to_string(),
                        result: CheckResult::Fail {
                            reason: format!("database error: {}", e),
                            guidance: "Check database with: isq doctor --check=database"
                                .to_string(),
                        },
                        details: None,
                    });
                }
            }
        }
        Err(_) => {
            checks.push(DiagnosticCheck {
                category: "Repository".to_string(),
                name: "Git repository".to_string(),
                result: CheckResult::Warn {
                    reason: "not in a git repository".to_string(),
                    guidance: "Run isq from within a git repository".to_string(),
                },
                details: None,
            });
        }
    }

    checks
}

/// Check service/daemon status
pub fn check_service(
    verbose: bool,
    svc_status: &Option<service::ServiceStatus>,
) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    match svc_status {
        Some(status) => {
            // Installed check
            if !status.installed {
                checks.push(DiagnosticCheck {
                    category: "Service".to_string(),
                    name: "Installation".to_string(),
                    result: CheckResult::Warn {
                        reason: "daemon not installed".to_string(),
                        guidance: "Run: isq daemon start".to_string(),
                    },
                    details: None,
                });
            } else {
                checks.push(DiagnosticCheck {
                    category: "Service".to_string(),
                    name: "Installation".to_string(),
                    result: CheckResult::Pass,
                    details: Some(CheckDetails {
                        source: None,
                        path: None,
                        value: Some("installed".to_string()),
                        latency_ms: None,
                    }),
                });
            }

            // Running check
            if let Some(pid) = status.pid {
                checks.push(DiagnosticCheck {
                    category: "Service".to_string(),
                    name: "Daemon".to_string(),
                    result: CheckResult::Pass,
                    details: Some(CheckDetails {
                        source: None,
                        path: None,
                        value: Some(format!("running (PID {})", pid)),
                        latency_ms: None,
                    }),
                });

                // Check watched repos count
                if verbose
                    && let Ok(conn) = db::open()
                    && let Ok(repos) = db::list_watched_repos(&conn)
                {
                    checks.push(DiagnosticCheck {
                        category: "Service".to_string(),
                        name: "Watched repos".to_string(),
                        result: CheckResult::Pass,
                        details: Some(CheckDetails {
                            source: None,
                            path: None,
                            value: Some(format!("{}", repos.len())),
                            latency_ms: None,
                        }),
                    });
                }
            } else if status.installed {
                checks.push(DiagnosticCheck {
                    category: "Service".to_string(),
                    name: "Daemon".to_string(),
                    result: CheckResult::Fail {
                        reason: "not running".to_string(),
                        guidance: "Run: isq daemon start".to_string(),
                    },
                    details: None,
                });
            }
        }
        None => {
            checks.push(DiagnosticCheck {
                category: "Service".to_string(),
                name: "Status".to_string(),
                result: CheckResult::Fail {
                    reason: "could not check service status".to_string(),
                    guidance: "Service management may not be supported on this platform"
                        .to_string(),
                },
                details: None,
            });
        }
    }

    checks
}

/// Check sync health for linked repository
pub fn check_sync_health(verbose: bool, daemon_running: bool) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    // Only check sync health if we're in a linked repo
    let repo_path = match repo::detect_repo_path() {
        Ok(p) => p,
        Err(_) => return checks, // Skip if not in a repo
    };

    let conn = match db::open() {
        Ok(c) => c,
        Err(_) => return checks,
    };

    let link = match db::get_repo_link(&conn, &repo_path) {
        Ok(Some(l)) => l,
        _ => return checks, // Skip if not linked
    };

    let sync_state = db::get_sync_state(&conn, &link.forge_repo).ok().flatten();
    let rate_limit_state = db::get_rate_limit_state(&conn, &link.forge_type)
        .ok()
        .flatten();

    let health = db::calculate_sync_health(
        sync_state.as_ref(),
        rate_limit_state.as_ref(),
        daemon_running,
    );

    let (result, details) = match health {
        SyncHealth::Healthy => {
            let last_sync = sync_state
                .as_ref()
                .and_then(|s| s.last_full_sync_at.as_ref().or(s.issues_last_sync.as_ref()))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            (
                CheckResult::Pass,
                Some(CheckDetails {
                    source: None,
                    path: None,
                    value: Some(if verbose {
                        format!("healthy (last sync: {})", last_sync)
                    } else {
                        "healthy".to_string()
                    }),
                    latency_ms: None,
                }),
            )
        }
        SyncHealth::Degraded { reason, guidance } => (CheckResult::Warn { reason, guidance }, None),
        SyncHealth::Unhealthy { reason, guidance } => {
            (CheckResult::Fail { reason, guidance }, None)
        }
    };

    checks.push(DiagnosticCheck {
        category: "Sync".to_string(),
        name: "Health".to_string(),
        result,
        details,
    });

    // Show pending ops if any
    if let Ok(pending) = db::count_pending_ops(&conn, &link.forge_repo)
        && pending > 0
    {
        checks.push(DiagnosticCheck {
            category: "Sync".to_string(),
            name: "Pending operations".to_string(),
            result: CheckResult::Warn {
                reason: format!("{} operations queued", pending),
                guidance: "These will sync when daemon reconnects".to_string(),
            },
            details: None,
        });
    }

    checks
}

/// Check installation status and detect orphan daemon
pub fn check_install(
    verbose: bool,
    svc_status: &Option<service::ServiceStatus>,
) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    // Check install method and auto-update status
    let receipt = install::read_receipt().ok().flatten();

    let (method_result, method_details) = match &receipt {
        Some(r) => {
            let method_str = match r.install_method {
                InstallMethod::Standalone => "standalone",
                InstallMethod::Homebrew => "homebrew",
                InstallMethod::Scoop => "scoop",
                InstallMethod::Cargo => "cargo",
                InstallMethod::Unknown => "unknown",
            };

            let update_info = if r.auto_update {
                "auto-updates enabled"
            } else {
                match r.install_method {
                    InstallMethod::Homebrew => "update via: brew upgrade isq",
                    InstallMethod::Scoop => "update via: scoop update isq",
                    InstallMethod::Cargo => "update via: cargo install isq",
                    _ => "run: isq update install",
                }
            };

            (
                CheckResult::Pass,
                Some(CheckDetails {
                    source: if verbose {
                        Some(format!("{} ({})", method_str, update_info))
                    } else {
                        Some(method_str.to_string())
                    },
                    path: if verbose {
                        Some(r.binary_path.to_string_lossy().to_string())
                    } else {
                        None
                    },
                    value: None,
                    latency_ms: None,
                }),
            )
        }
        None => {
            // No receipt - detect from path
            let method = install::detect_install_method();
            let method_str = match method {
                InstallMethod::Standalone => "standalone (no receipt)",
                InstallMethod::Homebrew => "homebrew",
                InstallMethod::Scoop => "scoop",
                InstallMethod::Cargo => "cargo",
                InstallMethod::Unknown => "unknown",
            };

            (
                CheckResult::Warn {
                    reason: "no install receipt".to_string(),
                    guidance: "Updates may need manual installation".to_string(),
                },
                Some(CheckDetails {
                    source: Some(method_str.to_string()),
                    path: None,
                    value: None,
                    latency_ms: None,
                }),
            )
        }
    };

    checks.push(DiagnosticCheck {
        category: "Install".to_string(),
        name: "Method".to_string(),
        result: method_result,
        details: method_details,
    });

    // Check for orphan daemon (daemon running but binary missing)
    if let Some(status) = svc_status
        && status.running
        && let Some(r) = &receipt
        && !r.binary_path.exists()
    {
        checks.push(DiagnosticCheck {
            category: "Install".to_string(),
            name: "Daemon state".to_string(),
            result: CheckResult::Fail {
                reason: "daemon running but binary not found".to_string(),
                guidance: "Run: isq uninstall".to_string(),
            },
            details: Some(CheckDetails {
                source: None,
                path: Some(r.binary_path.to_string_lossy().to_string()),
                value: None,
                latency_ms: None,
            }),
        });
    }

    checks
}
