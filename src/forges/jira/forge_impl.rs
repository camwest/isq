//! Forge trait implementation for JIRA

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::adf::markdown_to_adf;
use super::client::JiraClient;
use super::types;
use crate::db;
use crate::forges::{
    CreateGoalRequest, CreateIssueRequest, FetchResult, Forge, Goal, GoalState, Issue, Label,
    RateLimitInfo,
};
use crate::repo::Repo;

/// JIRA-specific on_start configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct JiraOnStartConfig {
    /// Workflow transition name (e.g., "In Progress", "Start Progress")
    transition: Option<String>,
    /// Assign the issue to yourself
    #[serde(default)]
    assign_self: bool,
}

/// JIRA-specific on_cleanup configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct JiraOnCleanupConfig {
    /// Workflow transition name (e.g., "To Do", "Done")
    transition: Option<String>,
}

#[async_trait]
impl Forge for JiraClient {
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, None).await
    }

    async fn list_issues_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, Some(since)).await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        // Get issue type from opts, or default to "Task"
        let issue_type = req.opts.get("type").map(|s| s.as_str()).unwrap_or("Task");
        self.create_issue(
            repo,
            &req.title,
            req.body.as_deref(),
            &req.labels,
            issue_type,
        )
        .await
    }

    async fn create_comment(&self, _repo: &Repo, issue_id: &str, body: &str) -> Result<()> {
        let path = format!("/issue/{}/comment", issue_id);

        let comment_body = serde_json::json!({
            "body": markdown_to_adf(body)
        });

        self.post_no_response(&path, &comment_body).await
    }

    async fn close_issue(&self, _repo: &Repo, issue_id: &str) -> Result<()> {
        // Get available transitions
        let path = format!("/issue/{}/transitions", issue_id);
        let response: types::JiraTransitionsResponse = self.get(&path).await?;

        // Find a "Done" transition
        let done_transition = response
            .transitions
            .iter()
            .find(|t| {
                let name_lower = t.name.to_lowercase();
                name_lower == "done" || name_lower.contains("done") || name_lower.contains("close")
            })
            .ok_or_else(|| anyhow!("No 'Done' transition available for this issue"))?;

        let body = serde_json::json!({
            "transition": { "id": done_transition.id }
        });

        self.post_no_response(&path, &body).await
    }

    async fn reopen_issue(&self, _repo: &Repo, issue_id: &str) -> Result<()> {
        // Get available transitions
        let path = format!("/issue/{}/transitions", issue_id);
        let response: types::JiraTransitionsResponse = self.get(&path).await?;

        // Find a "To Do" or "Reopen" transition
        let reopen_transition = response
            .transitions
            .iter()
            .find(|t| {
                let name_lower = t.name.to_lowercase();
                name_lower == "to do"
                    || name_lower.contains("reopen")
                    || name_lower.contains("backlog")
            })
            .ok_or_else(|| anyhow!("No 'Reopen' transition available for this issue"))?;

        let body = serde_json::json!({
            "transition": { "id": reopen_transition.id }
        });

        self.post_no_response(&path, &body).await
    }

    async fn add_label(&self, _repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let path = format!("/issue/{}", issue_id);

        let body = serde_json::json!({
            "update": {
                "labels": [{ "add": label }]
            }
        });

        self.put(&path, &body).await
    }

    async fn remove_label(&self, _repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let path = format!("/issue/{}", issue_id);

        let body = serde_json::json!({
            "update": {
                "labels": [{ "remove": label }]
            }
        });

        self.put(&path, &body).await
    }

    async fn assign_issue(&self, _repo: &Repo, issue_id: &str, assignee: &str) -> Result<()> {
        let path = format!("/issue/{}/assignee", issue_id);

        // Handle unassign case
        let body = if assignee.is_empty() || assignee == "null" {
            serde_json::json!({ "accountId": null })
        } else {
            // assignee should be an account ID
            serde_json::json!({ "accountId": assignee })
        };

        self.put(&path, &body).await
    }

    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<db::Comment>> {
        self.list_all_comments_internal(repo, None).await
    }

    async fn list_comments_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<db::Comment>> {
        self.list_all_comments_internal(repo, Some(since)).await
    }

    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        self.list_goals(repo).await
    }

    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal> {
        let project_key = &repo.name;

        // First, get project ID from key
        let project_path = format!("/project/{}", project_key);
        let project: types::JiraProject = self.get(&project_path).await?;

        let project_id: i64 = project.id.parse().map_err(|_| {
            anyhow!(
                "Invalid project ID '{}' - expected numeric value",
                project.id
            )
        })?;

        let body = serde_json::json!({
            "name": req.name,
            "description": req.description,
            "releaseDate": req.target_date,
            "projectId": project_id
        });

        let version: types::JiraVersion = self.post("/version", &body).await?;

        Ok(Goal {
            id: version.id,
            name: version.name,
            description: version.description,
            target_date: version.release_date,
            state: GoalState::Open,
            progress: 0.0,
            open_count: None,
            closed_count: None,
            created_at: String::new(),
            updated_at: String::new(),
            html_url: None,
        })
    }

    async fn close_goal(&self, _repo: &Repo, goal_id: &str) -> Result<()> {
        let path = format!("/version/{}", goal_id);
        let body = serde_json::json!({ "released": true });
        self.put(&path, &body).await
    }

    async fn assign_to_goal(&self, _repo: &Repo, issue_id: &str, goal_id: &str) -> Result<()> {
        let path = format!("/issue/{}", issue_id);

        // Get version name from ID
        let version_path = format!("/version/{}", goal_id);
        let version: types::JiraVersion = self.get(&version_path).await?;

        let body = serde_json::json!({
            "update": {
                "fixVersions": [{ "add": { "name": version.name } }]
            }
        });

        self.put(&path, &body).await
    }

    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
        // JIRA doesn't expose rate limit info in a simple endpoint
        Ok(None)
    }

    async fn list_labels(&self, _repo: &Repo) -> Result<Vec<Label>> {
        // JIRA labels are freeform and not stored separately
        Ok(Vec::new())
    }

    async fn create_label(
        &self,
        _repo: &Repo,
        name: &str,
        _color: Option<&str>,
        _description: Option<&str>,
    ) -> Result<Label> {
        // JIRA labels are created implicitly when added to issues
        Ok(Label::name_only(name.to_string()))
    }

    async fn handle_on_start(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        username: Option<&str>,
    ) -> Result<()> {
        let on_start: JiraOnStartConfig = config.clone().try_into()?;

        // Handle transition
        if let Some(transition_name) = &on_start.transition {
            let path = format!("/issue/{}/transitions", issue_id);
            let response: types::JiraTransitionsResponse = self.get(&path).await?;

            let transition = response
                .transitions
                .iter()
                .find(|t| t.name.to_lowercase() == transition_name.to_lowercase())
                .ok_or_else(|| {
                    let available: Vec<_> = response.transitions.iter().map(|t| &t.name).collect();
                    anyhow!(
                        "Transition '{}' not available. Available transitions: {:?}",
                        transition_name,
                        available
                    )
                })?;

            let body = serde_json::json!({
                "transition": { "id": transition.id }
            });

            self.post_no_response(&path, &body).await?;
        }

        // Handle assign_self
        if on_start.assign_self
            && let Some(account_id) = username
        {
            Forge::assign_issue(self, repo, issue_id, account_id).await?;
        }

        Ok(())
    }

    fn validate_on_start_config(&self, config: &toml::Value) -> Result<()> {
        let _: JiraOnStartConfig = config.clone().try_into().context(
            "Invalid [on_start] config for JIRA. Expected: transition = \"In Progress\", assign_self = true",
        )?;
        Ok(())
    }

    async fn handle_on_cleanup(
        &self,
        _repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        _username: Option<&str>,
    ) -> Result<()> {
        let cfg: JiraOnCleanupConfig = config.clone().try_into().unwrap_or_default();

        // Handle transition (optional)
        if let Some(transition_name) = &cfg.transition {
            let path = format!("/issue/{}/transitions", issue_id);
            let response: types::JiraTransitionsResponse = self.get(&path).await?;

            if let Some(transition) = response
                .transitions
                .iter()
                .find(|t| t.name.to_lowercase() == transition_name.to_lowercase())
            {
                let path = format!("/issue/{}/transitions", issue_id);
                let body = serde_json::json!({ "transition": { "id": transition.id } });
                if let Err(e) = self.post_no_response(&path, &body).await {
                    eprintln!("Warning: could not transition issue: {}", e);
                }
            } else {
                let available: Vec<_> = response.transitions.iter().map(|t| &t.name).collect();
                eprintln!(
                    "Warning: Transition '{}' not available. Available transitions: {:?}",
                    transition_name, available
                );
            }
        }

        Ok(())
    }

    fn validate_on_cleanup_config(&self, config: &toml::Value) -> Result<()> {
        let _: JiraOnCleanupConfig = config
            .clone()
            .try_into()
            .context("Invalid [on_cleanup] config for JIRA. Expected: transition = \"To Do\"")?;
        Ok(())
    }

    async fn handle_command(&self, command: &str, _args: &[String]) -> Result<()> {
        match command {
            "list-fields" => self.list_fields().await,
            _ => Err(anyhow!(
                "Unknown command: {}. Available commands: list-fields",
                command
            )),
        }
    }

    async fn query_issues_with_opts(
        &self,
        repo: &Repo,
        opts: &std::collections::HashMap<String, String>,
    ) -> Result<Option<Vec<Issue>>> {
        let jql_opt = opts.get("jql");
        let type_opt = opts.get("type");

        // If no JIRA-specific options, use cache
        if jql_opt.is_none() && type_opt.is_none() {
            return Ok(None);
        }

        let project_key = &repo.name;

        // Build JQL from options
        let mut conditions = vec![format!("project = {}", project_key)];

        if let Some(jql) = jql_opt {
            conditions.push(format!("({})", jql));
        }

        if let Some(issue_type) = type_opt {
            conditions.push(format!("issuetype = \"{}\"", issue_type));
        }

        let full_jql = conditions.join(" AND ");

        let mut all_issues = Vec::new();
        let mut next_page_token: Option<String> = None;

        loop {
            let body = serde_json::json!({
                "jql": full_jql,
                "maxResults": 100,
                "fields": ["*all"],
                "nextPageToken": next_page_token
            });

            let response: types::JiraSearchResponse = self.post("/search/jql", &body).await?;

            for jira_issue in &response.issues {
                all_issues.push(self.convert_issue(jira_issue));
            }

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        Ok(Some(all_issues))
    }
}
