//! JIRA comment operations

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::adf::adf_to_markdown;
use super::client::JiraClient;
use super::types::*;
use crate::db;
use crate::forges::FetchResult;
use crate::repo::Repo;

impl JiraClient {
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
}
