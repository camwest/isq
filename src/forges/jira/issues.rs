//! JIRA issue operations

use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::{debug, trace};

use super::adf::{adf_to_markdown, markdown_to_adf};
use super::client::JiraClient;
use super::map_jira_priority;
use super::types::*;
use crate::forges::{FetchResult, Issue, Label};
use crate::repo::Repo;

impl JiraClient {
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
            parent_id: jira_issue.fields.parent.as_ref().map(|p| p.key.clone()),
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

        debug!(project_key, since = ?since, "Fetching JIRA issues");

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
            trace!(page, total = all_issues.len(), "Fetched JIRA issues page");

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        debug!(total = all_issues.len(), "JIRA issues fetch complete");
        Ok(FetchResult::complete(all_issues))
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
