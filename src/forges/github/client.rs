//! GitHub API client implementation

use crate::forges::create_http_client as create_base_client;

/// Create HTTP client with appropriate settings
pub fn create_http_client() -> reqwest::Client {
    create_base_client()
}

#[derive(Clone)]
pub struct GitHubClient {
    client: reqwest::Client,
    token: String,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self {
            client: create_http_client(),
            token,
        }
    }

    /// Get the HTTP client (for use by submodules)
    pub(super) fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Get the token (for use by submodules)
    pub(super) fn token(&self) -> &str {
        &self.token
    }
}
