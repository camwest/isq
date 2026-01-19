//! Linear comment operations

use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::{debug, trace, warn};

use super::LinearClient;
use super::types::{CommentsResponse, PageInfo};
use crate::db;
use crate::forges::FetchResult;

impl LinearClient {
    /// List all comments for a team with optional since filter
    /// Uses direct comments query with pagination
    pub async fn list_comments_internal(
        &self,
        team_id: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<db::Comment>> {
        let mut all_comments = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        debug!(team_id, since = ?since, "Fetching Linear comments");

        loop {
            let query = if since.is_some() {
                r#"
                query($teamId: ID!, $after: String, $since: DateTimeOrDuration!) {
                    comments(
                        filter: { issue: { team: { id: { eq: $teamId } } }, updatedAt: { gte: $since } },
                        orderBy: updatedAt,
                        first: 250,
                        after: $after
                    ) {
                        pageInfo {
                            hasNextPage
                            endCursor
                        }
                        nodes {
                            id
                            body
                            user {
                                name
                            }
                            issue {
                                identifier
                            }
                            createdAt
                            updatedAt
                        }
                    }
                }
            "#
            } else {
                r#"
                query($teamId: ID!, $after: String) {
                    comments(
                        filter: { issue: { team: { id: { eq: $teamId } } } },
                        first: 250,
                        after: $after
                    ) {
                        pageInfo {
                            hasNextPage
                            endCursor
                        }
                        nodes {
                            id
                            body
                            user {
                                name
                            }
                            issue {
                                identifier
                            }
                            createdAt
                            updatedAt
                        }
                    }
                }
            "#
            };

            let variables = match since {
                Some(ts) => serde_json::json!({
                    "teamId": team_id,
                    "after": cursor,
                    "since": ts.to_rfc3339()
                }),
                None => serde_json::json!({
                    "teamId": team_id,
                    "after": cursor
                }),
            };

            match self.query::<CommentsResponse>(query, Some(variables)).await {
                Ok(response) => {
                    for comment in response.comments.nodes {
                        all_comments.push(db::Comment {
                            comment_id: comment.id,
                            issue_id: comment.issue.identifier.clone(),
                            body: comment.body,
                            author: comment
                                .user
                                .map(|u| u.name)
                                .unwrap_or_else(|| "unknown".to_string()),
                            created_at: comment.created_at,
                            updated_at: Some(comment.updated_at),
                        });
                    }

                    page += 1;
                    trace!(page, total = all_comments.len(), "Fetched Linear comments page");

                    let page_info = response.comments.page_info.unwrap_or(PageInfo {
                        has_next_page: false,
                        end_cursor: None,
                    });

                    if !page_info.has_next_page {
                        break;
                    }
                    cursor = page_info.end_cursor;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    // Rate limit errors (429) should be propagated, not swallowed
                    if err_str.contains("429") {
                        return Err(e);
                    }
                    // Other errors: return partial data
                    warn!(error = %err_str, "Linear comments page fetch failed");
                    return Ok(FetchResult::incomplete(all_comments));
                }
            }
        }

        debug!(total = all_comments.len(), "Linear comments fetch complete");
        Ok(FetchResult::complete(all_comments))
    }
}
