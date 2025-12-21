use anyhow::Result;
use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::repo::Repo;

// TODO: Abstract into Forge trait per DESIGN.md
// trait Forge {
//     fn list_issues(&self, filters: Filters) -> Result<Vec<Issue>>;
//     fn get_issue(&self, id: &str) -> Result<Issue>;
//     // ...
// }

const PER_PAGE: usize = 100;

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

#[derive(Clone)]
pub struct GitHubClient {
    client: reqwest::Client,
    token: String,
}

#[derive(Deserialize)]
struct SearchResult {
    total_count: usize,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            token,
        }
    }

    /// Fetch all open issues for a repo (parallel pagination)
    pub async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>> {
        // Get total count from search API
        let total = self.get_issue_count(repo).await?;

        if total == 0 {
            return Ok(Vec::new());
        }

        let total_pages = (total + PER_PAGE - 1) / PER_PAGE;
        eprintln!("Fetching {} issues across {} pages...", total, total_pages);

        // Fetch all pages in parallel - HTTP/2 handles stream limits automatically
        let futures: Vec<_> = (1..=total_pages)
            .map(|page| self.fetch_page(repo, page))
            .collect();

        let results = join_all(futures).await;

        let mut all_issues = Vec::with_capacity(total);
        for result in results {
            match result {
                Ok(issues) => all_issues.extend(issues),
                Err(e) => eprintln!("Warning: page fetch failed: {}", e),
            }
        }

        Ok(all_issues)
    }

    /// Get total open issue count via search API
    async fn get_issue_count(&self, repo: &Repo) -> Result<usize> {
        let url = format!(
            "https://api.github.com/search/issues?q=repo:{}/{}+state:open+is:issue&per_page=1",
            repo.owner, repo.name
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
            anyhow::bail!("GitHub search API error {}: {}", status, body);
        }

        let result: SearchResult = response.json().await?;
        Ok(result.total_count)
    }

    /// Fetch a single page of issues
    async fn fetch_page(&self, repo: &Repo, page: usize) -> Result<Vec<Issue>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=open&per_page={}&page={}",
            repo.owner, repo.name, PER_PAGE, page
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
        Ok(issues)
    }

    /// Fetch a single issue by number
    #[allow(dead_code)]
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
