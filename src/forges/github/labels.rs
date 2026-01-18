//! GitHub label operations

use anyhow::Result;

use crate::forges::Label;
use crate::repo::Repo;

use super::GitHubClient;
use super::rate_limit::throttle_write;
use super::types::GitHubLabel;

impl GitHubClient {
    /// Internal add_label without auto-create (to avoid infinite recursion)
    pub async fn add_label_internal(
        &self,
        repo: &Repo,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "labels": [label] });

        let response = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
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

    /// Create a label in the repository (internal, for add_label auto-create)
    pub async fn create_label_internal(&self, repo: &Repo, label: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/labels",
            repo.owner, repo.name
        );

        // Use a nice blue color for auto-created labels
        let payload = serde_json::json!({
            "name": label,
            "color": "1d76db",
            "description": "Auto-created by isq"
        });

        let response = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        // 422 means label already exists, which is fine
        if response.status().is_success() || response.status().as_u16() == 422 {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("GitHub API error creating label {}: {}", status, body);
    }

    /// List all labels in the repository
    pub async fn list_labels(&self, repo: &Repo) -> Result<Vec<Label>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/labels?per_page=100",
            repo.owner, repo.name
        );

        let response = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error listing labels {}: {}", status, body);
        }

        let labels: Vec<GitHubLabel> = response.json().await?;
        Ok(labels
            .into_iter()
            .map(|l| Label::new(l.name, Some(l.color)))
            .collect())
    }

    /// Create a label in the repository
    pub async fn create_label(
        &self,
        repo: &Repo,
        name: &str,
        color: Option<&str>,
        description: Option<&str>,
    ) -> Result<Label> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/labels",
            repo.owner, repo.name
        );

        let color = color.unwrap_or("1d76db").trim_start_matches('#');
        let desc = description.unwrap_or("Created by isq");

        let payload = serde_json::json!({
            "name": name,
            "color": color,
            "description": desc
        });

        let response = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error creating label {}: {}", status, body);
        }

        let label: GitHubLabel = response.json().await?;
        Ok(Label::new(label.name, Some(label.color)))
    }

    /// Add label to issue (public method that auto-creates if needed)
    pub async fn add_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "labels": [label] });

        let response = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if response.status().is_success() {
            return Ok(());
        }

        // Check if label doesn't exist (422 with "Label does not exist")
        let status = response.status();
        let body = response.text().await?;

        if status.as_u16() == 422 && body.to_lowercase().contains("label") {
            // Create the label and retry
            self.create_label_internal(repo, label).await?;
            return self.add_label_internal(repo, issue_number, label).await;
        }

        anyhow::bail!("GitHub API error {}: {}", status, body);
    }

    /// Remove label from issue
    pub async fn remove_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels/{}",
            repo.owner, repo.name, issue_number, label
        );

        let response = self
            .http_client()
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
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
}
