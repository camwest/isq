use anyhow::Result;
use async_trait::async_trait;
use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::forge::{CreateIssueRequest, Forge};
use crate::repo::Repo;

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

    /// Get authenticated user's login
    pub async fn get_user(&self) -> Result<String> {
        let response = self
            .client
            .get("https://api.github.com/user")
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

        let user: User = response.json().await?;
        Ok(user.login)
    }

    /// Fetch a single issue by number
    async fn fetch_issue(&self, repo: &Repo, number: u64) -> Result<Issue> {
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

    /// Helper for PATCH requests to update issue state
    async fn patch_issue(&self, repo: &Repo, number: u64, body: &serde_json::Value) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}",
            repo.owner, repo.name, number
        );

        let response = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }
}

#[async_trait]
impl Forge for GitHubClient {
    async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>> {
        self.list_issues(repo).await
    }

    async fn get_issue(&self, repo: &Repo, number: u64) -> Result<Issue> {
        self.fetch_issue(repo, number).await
    }

    async fn get_user(&self) -> Result<String> {
        self.get_user().await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues",
            repo.owner, repo.name
        );

        let mut body = serde_json::json!({
            "title": req.title,
        });

        if let Some(b) = &req.body {
            body["body"] = serde_json::json!(b);
        }

        if !req.labels.is_empty() {
            body["labels"] = serde_json::json!(req.labels);
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&body)
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

    async fn create_comment(&self, repo: &Repo, issue_number: u64, body: &str) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/comments",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "body": body });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    async fn close_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        self.patch_issue(repo, issue_number, &serde_json::json!({ "state": "closed" }))
            .await
    }

    async fn reopen_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        self.patch_issue(repo, issue_number, &serde_json::json!({ "state": "open" }))
            .await
    }

    async fn add_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "labels": [label] });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    async fn remove_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels/{}",
            repo.owner, repo.name, issue_number, label
        );

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        // 404 is ok - label might not exist
        if !response.status().is_success() && response.status().as_u16() != 404 {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    async fn assign_issue(&self, repo: &Repo, issue_number: u64, assignee: &str) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/assignees",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "assignees": [assignee] });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }
}
