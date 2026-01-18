//! JIRA API client implementation

use std::sync::RwLock;

use anyhow::{Result, anyhow};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::adf::{adf_to_markdown, markdown_to_adf};
use super::oauth::{JiraAuthMode, JiraCredentials, refresh_token};
use super::types::*;
use super::{AUTH, map_jira_priority, parse_jira_error, truncate};
use crate::db;
use crate::forges::{FetchResult, Goal, GoalState, Issue, Label};
use crate::repo::Repo;

/// JIRA API client
pub struct JiraClient {
    client: reqwest::Client,
    pub(super) creds: RwLock<JiraCredentials>,
}

impl JiraClient {
    pub fn new(creds: JiraCredentials) -> Self {
        Self {
            client: reqwest::Client::new(),
            creds: RwLock::new(creds),
        }
    }

    /// Get the base URL for JIRA REST API v3
    fn api_base(&self) -> String {
        let creds = self.creds.read().unwrap();
        match &creds.auth_mode {
            JiraAuthMode::OAuth { cloud_id } => {
                format!("https://api.atlassian.com/ex/jira/{}/rest/api/3", cloud_id)
            }
            JiraAuthMode::ApiToken { .. } => {
                format!("{}/rest/api/3", creds.site_url)
            }
        }
    }

    /// Get the auth header value
    fn auth_header(&self) -> (String, String) {
        let creds = self.creds.read().unwrap();
        match &creds.auth_mode {
            JiraAuthMode::OAuth { .. } => ("Bearer".to_string(), creds.access_token.clone()),
            JiraAuthMode::ApiToken { email } => {
                // Basic auth: base64(email:token)
                let basic = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", email, creds.access_token));
                ("Basic".to_string(), basic)
            }
        }
    }

    /// Get the site URL for building browse links
    pub fn site_url(&self) -> String {
        let creds = self.creds.read().unwrap();
        creds.site_url.clone()
    }

    /// Make an authenticated GET request
    pub async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "JIRA authentication failed. Please re-authenticate with: isq link jira"
            ));
        }

        if response.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                "Access denied. You may not have permission to access this JIRA project."
            ));
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(anyhow!("JIRA API error ({}): {}", status, body));
        }

        let result = response.json().await?;
        Ok(result)
    }

    /// Make an authenticated POST request
    pub async fn post<T: for<'de> Deserialize<'de>, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "JIRA authentication failed. Please re-authenticate with: isq link jira"
            ));
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(parse_jira_error(status, &body));
        }

        let result = response.json().await?;
        Ok(result)
    }

    /// Make an authenticated POST request without expecting a response body
    pub async fn post_no_response<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(parse_jira_error(status, &body));
        }

        Ok(())
    }

    /// Make an authenticated PUT request
    pub async fn put<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(anyhow!("JIRA API error ({}): {}", status, body));
        }

        Ok(())
    }

    /// Refresh the access token if needed (only for OAuth mode)
    async fn refresh_if_needed(&self) -> Result<()> {
        let needs_refresh = {
            let creds = self.creds.read().unwrap();
            // API tokens don't expire, only refresh OAuth tokens
            if matches!(creds.auth_mode, JiraAuthMode::ApiToken { .. }) {
                return Ok(());
            }
            if let Some(expires_at) = creds.expires_at {
                let now = chrono::Utc::now().timestamp();
                let remaining = expires_at - now;
                // Refresh if less than 5 minutes remaining
                remaining < 300
            } else {
                false
            }
        };

        if needs_refresh {
            self.do_refresh_token().await?;
        }

        Ok(())
    }

    /// Refresh the access token using the stored refresh token
    async fn do_refresh_token(&self) -> Result<()> {
        let stored_refresh_token = {
            let creds = self.creds.read().unwrap();
            creds.refresh_token.clone().ok_or_else(|| {
                anyhow!("No refresh token available - please re-authenticate with: isq link jira")
            })?
        };

        let new_tokens = refresh_token(&stored_refresh_token).await?;

        let expires_at = new_tokens
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs as i64);

        // Update stored credentials
        {
            let mut creds = self.creds.write().unwrap();
            creds.access_token = new_tokens.access_token.clone();
            if let Some(rt) = &new_tokens.refresh_token {
                creds.refresh_token = Some(rt.clone());
            }
            creds.expires_at = expires_at;
        }

        // Store updated credentials in keyring (only for OAuth mode)
        let creds = self.creds.read().unwrap();
        if let JiraAuthMode::OAuth { cloud_id } = &creds.auth_mode {
            let cred_json = serde_json::json!({
                "access_token": creds.access_token,
                "refresh_token": creds.refresh_token,
                "cloud_id": cloud_id,
                "site_url": creds.site_url,
                "expires_at": creds.expires_at
            });
            AUTH.store_credential(&cred_json.to_string(), None, None)?;
        }

        Ok(())
    }

    /// List projects accessible to the user
    pub async fn list_projects(&self) -> Result<Vec<JiraProject>> {
        #[derive(Deserialize)]
        struct ProjectsResponse {
            values: Vec<JiraProject>,
        }

        let response: ProjectsResponse = self.get("/project/search?maxResults=100").await?;
        Ok(response.values)
    }

    /// Get current user info
    pub async fn get_current_user(&self) -> Result<JiraUser> {
        self.get("/myself").await
    }

    /// Check if user has write permissions using /mypermissions endpoint
    pub async fn check_write_permission(&self, project_key: &str) -> Result<bool> {
        let path = format!(
            "/mypermissions?projectKey={}&permissions=CREATE_ISSUES",
            project_key
        );

        #[derive(Deserialize)]
        struct PermissionsResponse {
            permissions: std::collections::HashMap<String, Permission>,
        }

        #[derive(Deserialize)]
        struct Permission {
            #[serde(rename = "havePermission")]
            have_permission: bool,
        }

        match self.get::<PermissionsResponse>(&path).await {
            Ok(resp) => {
                let can_create = resp
                    .permissions
                    .get("CREATE_ISSUES")
                    .map(|p| p.have_permission)
                    .unwrap_or(false);
                Ok(can_create)
            }
            Err(e) if e.to_string().contains("403") || e.to_string().contains("Access denied") => {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// List available JIRA fields (for JQL queries)
    pub async fn list_fields(&self) -> Result<()> {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct JiraField {
            id: String,
            name: String,
            #[serde(default)]
            searchable: bool,
            #[serde(default)]
            clause_names: Vec<String>,
            schema: Option<JiraFieldSchema>,
        }

        #[derive(Debug, Deserialize)]
        struct JiraFieldSchema {
            #[serde(rename = "type")]
            field_type: Option<String>,
        }

        let fields: Vec<JiraField> = self.get("/field").await?;

        // Filter to searchable fields and sort by name
        let mut searchable: Vec<_> = fields.iter().filter(|f| f.searchable).collect();
        searchable.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        println!("{:<30} {:<25} {:<15} ID", "Name", "JQL Clause", "Type");
        println!("{}", "-".repeat(85));

        for field in &searchable {
            let clause = field
                .clause_names
                .first()
                .map(|s| s.as_str())
                .unwrap_or(&field.id);
            let field_type = field
                .schema
                .as_ref()
                .and_then(|s| s.field_type.as_deref())
                .unwrap_or("unknown");
            println!(
                "{:<30} {:<25} {:<15} {}",
                truncate(&field.name, 29),
                truncate(clause, 24),
                truncate(field_type, 14),
                &field.id
            );
        }

        println!("\n{} searchable fields", searchable.len());
        println!("\nExample JQL queries:");
        println!("  isq issue list -o jql=\"assignee = currentUser()\"");
        println!("  isq issue list -o jql=\"priority = High\"");
        println!("  isq issue list -o jql=\"status = 'In Progress'\"");

        Ok(())
    }

    /// Convert a JIRA issue to our Issue type
    pub fn convert_issue(&self, jira_issue: &JiraIssue) -> Issue {
        let state = match jira_issue.fields.status.status_category.key.as_str() {
            "done" => "closed",
            _ => "open",
        };

        let mut labels: Vec<Label> = jira_issue
            .fields
            .labels
            .as_ref()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|l| Label::name_only(l.clone()))
            .collect();

        // Add issue type as pseudo-label
        labels.push(Label::name_only(format!(
            "type:{}",
            jira_issue.fields.issuetype.name
        )));

        let priority =
            map_jira_priority(jira_issue.fields.priority.as_ref().map(|p| p.name.as_str()));

        let assignees: Vec<String> = jira_issue
            .fields
            .assignee
            .as_ref()
            .map(|a| a.display_name.as_ref().unwrap_or(&a.account_id).clone())
            .into_iter()
            .collect();

        let body = jira_issue.fields.description.as_ref().map(adf_to_markdown);

        let milestone = jira_issue
            .fields
            .fix_versions
            .as_ref()
            .and_then(|v| v.first())
            .map(|v| v.name.clone());

        Issue {
            id: jira_issue.key.clone(),
            title: jira_issue.fields.summary.clone(),
            body,
            state: state.to_string(),
            author: jira_issue
                .fields
                .reporter
                .as_ref()
                .and_then(|r| r.display_name.clone())
                .unwrap_or_default(),
            labels,
            assignees,
            priority,
            priority_label: None,
            created_at: jira_issue.fields.created.clone(),
            updated_at: jira_issue.fields.updated.clone(),
            url: Some(format!("{}/browse/{}", self.site_url(), jira_issue.key)),
            milestone,
        }
    }

    /// Internal list_issues with optional since filter for incremental sync
    pub async fn list_issues_internal(
        &self,
        repo: &Repo,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<Issue>> {
        let project_key = &repo.name;

        let mut all_issues = Vec::new();
        let mut next_page_token: Option<String> = None;
        let mut page = 0;

        loop {
            // Build JQL with optional updated filter for incremental sync
            let jql = match since {
                Some(ts) => format!(
                    "project = {} AND updated >= \"{}\" ORDER BY updated ASC",
                    project_key,
                    ts.format("%Y-%m-%d %H:%M")
                ),
                None => format!("project = {} ORDER BY updated DESC", project_key),
            };

            let body = serde_json::json!({
                "jql": jql,
                "maxResults": 100,
                "fields": ["*all"],
                "nextPageToken": next_page_token
            });

            let response: JiraSearchResponse = self.post("/search/jql", &body).await?;

            for jira_issue in &response.issues {
                all_issues.push(self.convert_issue(jira_issue));
            }

            page += 1;
            // Print progress every 10 pages
            if page % 10 == 0 {
                eprintln!("  {} issues...", all_issues.len());
            }

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        Ok(FetchResult::complete(all_issues))
    }

    /// Internal list_all_comments with optional since filter for incremental sync
    pub async fn list_all_comments_internal(
        &self,
        repo: &Repo,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<db::Comment>> {
        let project_key = &repo.name;

        let mut all_comments = Vec::new();
        let mut next_page_token: Option<String> = None;
        let mut page = 0;

        loop {
            // Build JQL with optional updated filter for incremental sync
            let jql = match since {
                Some(ts) => format!(
                    "project = {} AND updated >= \"{}\" ORDER BY updated ASC",
                    project_key,
                    ts.format("%Y-%m-%d %H:%M")
                ),
                None => format!("project = {} ORDER BY updated DESC", project_key),
            };

            let body = serde_json::json!({
                "jql": jql,
                "maxResults": 100,
                "fields": ["key"],
                "nextPageToken": next_page_token
            });

            let response: JiraSearchResponseMinimal = self.post("/search/jql", &body).await?;

            // For each issue, fetch comments
            for jira_issue in &response.issues {
                // Paginate through all comments for this issue
                let mut start_at: u64 = 0;
                loop {
                    let comments_path = format!(
                        "/issue/{}/comment?startAt={}&maxResults=100",
                        jira_issue.key, start_at
                    );
                    let comments_response: JiraCommentsResponse =
                        match self.get(&comments_path).await {
                            Ok(r) => r,
                            Err(_) => break, // Skip issues we can't read comments from
                        };

                    for comment in &comments_response.comments {
                        let body = comment
                            .body
                            .as_ref()
                            .map(adf_to_markdown)
                            .unwrap_or_default();

                        all_comments.push(db::Comment {
                            comment_id: comment.id.clone(),
                            issue_id: jira_issue.key.clone(),
                            body,
                            author: comment
                                .author
                                .as_ref()
                                .and_then(|a| a.display_name.clone())
                                .unwrap_or_default(),
                            created_at: comment.created.clone(),
                            updated_at: Some(comment.updated.clone()),
                        });
                    }

                    // Check if there are more pages
                    // Break if no comments returned (prevents infinite loop on restricted comments)
                    if comments_response.comments.is_empty() {
                        break;
                    }
                    let fetched = start_at + comments_response.comments.len() as u64;
                    if fetched >= comments_response.total {
                        break;
                    }
                    start_at = fetched;
                }
            }

            page += 1;
            // Print progress every 10 pages
            if page % 10 == 0 {
                eprintln!("  {} comments...", all_comments.len());
            }

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        Ok(FetchResult::complete(all_comments))
    }

    /// List goals (versions) for a project
    pub async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        let project_key = &repo.name;
        let path = format!("/project/{}/versions", project_key);

        let versions: Vec<JiraVersion> = self.get(&path).await?;

        let goals: Vec<Goal> = versions
            .into_iter()
            .map(|v| {
                let state = if v.released.unwrap_or(false) || v.archived.unwrap_or(false) {
                    GoalState::Closed
                } else {
                    GoalState::Open
                };

                Goal {
                    id: v.id,
                    name: v.name,
                    description: v.description,
                    target_date: v.release_date,
                    state,
                    progress: 0.0, // TODO: calculate from issues
                    open_count: None,
                    closed_count: None,
                    created_at: String::new(), // Versions don't have created_at
                    updated_at: String::new(),
                    html_url: None,
                }
            })
            .collect();

        Ok(goals)
    }

    /// Create an issue
    pub async fn create_issue(
        &self,
        repo: &Repo,
        title: &str,
        body: Option<&str>,
        labels: &[String],
        issue_type: &str,
    ) -> Result<Issue> {
        let project_key = &repo.name;

        let description_adf = body.map(markdown_to_adf);

        let mut fields = serde_json::json!({
            "project": { "key": project_key },
            "summary": title,
            "issuetype": { "name": issue_type }
        });

        if let Some(desc) = description_adf {
            fields["description"] = desc;
        }

        if !labels.is_empty() {
            fields["labels"] = serde_json::json!(labels);
        }

        let body = serde_json::json!({ "fields": fields });
        let created: JiraCreateResponse = self.post("/issue", &body).await?;

        // Fetch full issue to get all fields
        let path = format!("/issue/{}", created.key);
        let full_issue: JiraIssue = self.get(&path).await?;

        Ok(self.convert_issue(&full_issue))
    }
}
