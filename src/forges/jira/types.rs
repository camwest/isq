//! JIRA API response types

use serde::Deserialize;

/// JIRA project from API
#[derive(Debug, Deserialize)]
pub struct JiraProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// JIRA issue type from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssueType {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    #[allow(dead_code)]
    pub subtask: bool,
}

/// JIRA user from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraUser {
    pub account_id: String,
    pub display_name: Option<String>,
    #[allow(dead_code)]
    pub email_address: Option<String>,
}

/// JIRA status from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraStatus {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub name: String,
    pub status_category: JiraStatusCategory,
}

/// JIRA status category from API
#[derive(Debug, Deserialize)]
pub struct JiraStatusCategory {
    #[allow(dead_code)]
    pub id: u64,
    pub key: String,
    #[allow(dead_code)]
    pub name: String,
}

/// JIRA priority from API
#[derive(Debug, Deserialize)]
pub struct JiraPriority {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
}

/// JIRA parent issue reference
#[derive(Debug, Deserialize)]
pub struct JiraParentRef {
    pub key: String,
}

/// JIRA issue fields from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssueFields {
    pub summary: String,
    pub description: Option<serde_json::Value>, // ADF format
    pub status: JiraStatus,
    pub issuetype: JiraIssueType,
    pub priority: Option<JiraPriority>,
    pub reporter: Option<JiraUser>,
    pub assignee: Option<JiraUser>,
    pub labels: Option<Vec<String>>,
    pub created: String,
    pub updated: String,
    #[serde(rename = "fixVersions")]
    pub fix_versions: Option<Vec<JiraVersion>>,
    /// Parent issue (for subtasks)
    pub parent: Option<JiraParentRef>,
}

/// JIRA issue from API
#[derive(Debug, Deserialize)]
pub struct JiraIssue {
    #[allow(dead_code)]
    pub id: String,
    pub key: String,
    #[serde(rename = "self")]
    #[allow(dead_code)]
    pub self_url: String,
    pub fields: JiraIssueFields,
}

/// JIRA create issue response (minimal - just id, key, self)
#[derive(Debug, Deserialize)]
pub struct JiraCreateResponse {
    #[allow(dead_code)]
    pub id: String,
    pub key: String,
}

/// Minimal JIRA issue (for key-only queries)
#[derive(Debug, Deserialize)]
pub struct JiraIssueMinimal {
    pub key: String,
}

/// JIRA search response for minimal queries
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraSearchResponseMinimal {
    pub issues: Vec<JiraIssueMinimal>,
    pub next_page_token: Option<String>,
}

/// JIRA search response (new /search/jql format)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraSearchResponse {
    pub issues: Vec<JiraIssue>,
    pub next_page_token: Option<String>,
}

/// JIRA version (for goals)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraVersion {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub released: Option<bool>,
    pub archived: Option<bool>,
    pub release_date: Option<String>,
}

/// JIRA transition from API
#[derive(Debug, Deserialize)]
pub struct JiraTransition {
    pub id: String,
    pub name: String,
}

/// JIRA transitions response
#[derive(Debug, Deserialize)]
pub struct JiraTransitionsResponse {
    pub transitions: Vec<JiraTransition>,
}

/// JIRA comment from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraComment {
    pub id: String,
    pub body: Option<serde_json::Value>, // ADF format
    pub author: Option<JiraUser>,
    pub created: String,
    pub updated: String,
}

/// JIRA comments response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraCommentsResponse {
    pub comments: Vec<JiraComment>,
    pub total: u64,
    #[allow(dead_code)]
    pub start_at: u64,
    #[allow(dead_code)]
    pub max_results: u64,
}
