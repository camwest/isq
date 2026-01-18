//! Types for doctor diagnostics

use serde::Serialize;

/// Result of a diagnostic check
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CheckResult {
    Pass,
    Warn { reason: String, guidance: String },
    Fail { reason: String, guidance: String },
}

/// A single diagnostic check with its result
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticCheck {
    pub category: String,
    pub name: String,
    pub result: CheckResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<CheckDetails>,
}

/// Optional details for verbose mode
#[derive(Debug, Clone, Serialize)]
pub struct CheckDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

/// Summary of all checks
#[derive(Debug, Clone, Serialize)]
pub struct CheckSummary {
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
}

/// Full diagnostic report
#[derive(Debug, Serialize)]
pub struct DiagnosticReport {
    pub version: String,
    pub checks: Vec<DiagnosticCheck>,
    pub summary: CheckSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_result_variants() {
        // Verify Pass variant
        let pass = CheckResult::Pass;
        assert!(matches!(pass, CheckResult::Pass));

        // Verify Warn variant
        let warn = CheckResult::Warn {
            reason: "test".to_string(),
            guidance: "test".to_string(),
        };
        assert!(matches!(warn, CheckResult::Warn { .. }));

        // Verify Fail variant
        let fail = CheckResult::Fail {
            reason: "test".to_string(),
            guidance: "test".to_string(),
        };
        assert!(matches!(fail, CheckResult::Fail { .. }));
    }
}
