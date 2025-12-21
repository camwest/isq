use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::repo::Repo;

// TODO: Abstract into Forge trait per DESIGN.md
// trait Forge {
//     fn list_issues(&self, filters: Filters) -> Result<Vec<Issue>>;
//     fn get_issue(&self, id: &str) -> Result<Issue>;
//     // ...
// }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub user: User,
    pub labels: Vec<Label>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
}

pub struct GitHubClient {
    client: reqwest::Client,
    token: String,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            token,
        }
    }

    /// Fetch all open issues for a repo (handles pagination)
    pub async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>> {
        let mut all_issues = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            let url = format!(
                "https://api.github.com/repos/{}/{}/issues?state=open&per_page={}&page={}",
                repo.owner, repo.name, per_page, page
            );

            let response = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("User-Agent", "isq")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await?;
                anyhow::bail!("GitHub API error {}: {}", status, body);
            }

            let issues: Vec<Issue> = response.json().await?;
            let count = issues.len();
            all_issues.extend(issues);

            // If we got fewer than per_page, we've reached the end
            if count < per_page {
                break;
            }

            page += 1;
        }

        Ok(all_issues)
    }

    /// Fetch a single issue by number
    pub async fn get_issue(&self, repo: &Repo, number: u64) -> Result<Issue> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}",
            repo.owner, repo.name, number
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let issue: Issue = response.json().await?;
        Ok(issue)
    }
}
