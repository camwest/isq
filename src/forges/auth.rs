//! Authentication configuration for forges.

use std::process::Command;

use anyhow::{Result, anyhow};

use crate::credentials;

/// Authentication configuration for a forge.
///
/// Each forge defines its auth config as a const. The auth logic is generic
/// and works with any AuthConfig, following the open/closed principle.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Service name for keyring storage (e.g., "github", "linear")
    pub keyring_service: &'static str,
    /// Environment variable name for token fallback
    pub env_var: &'static str,
    /// CLI command to get token, if any (e.g., &["gh", "auth", "token"])
    pub cli_command: Option<&'static [&'static str]>,
    /// Human-readable forge name for error messages
    pub display_name: &'static str,
    /// Command to authenticate (shown in error messages)
    pub link_command: &'static str,
}

impl AuthConfig {
    /// Get a token using the fallback chain: CLI → keyring → env var
    pub fn get_token(&self) -> Result<String> {
        // 1. Try CLI command if configured
        if let Some(cmd) = self.cli_command
            && let Ok(token) = self.try_cli_token(cmd)
        {
            return Ok(token);
        }

        // 2. Try stored credentials from OS keyring
        if let Ok(Some(cred)) = credentials::get_credential(self.keyring_service) {
            return Ok(cred.access_token);
        }

        // 3. Try environment variable
        if let Ok(token) = std::env::var(self.env_var) {
            return Ok(token);
        }

        // No token available - build helpful error message
        Err(self.auth_error())
    }

    /// Check if credentials are available (without detailed errors)
    pub fn has_credentials(&self) -> bool {
        // Check CLI
        if let Some(cmd) = self.cli_command
            && self.try_cli_token(cmd).is_ok()
        {
            return true;
        }

        // Check keyring
        if let Ok(Some(_)) = credentials::get_credential(self.keyring_service) {
            return true;
        }
        // Check scoped keyring entries (e.g., linear:<scope>)
        let scoped_prefix = format!("{}:", self.keyring_service);
        if credentials::list_services()
            .is_ok_and(|keys| keys.iter().any(|k| k.starts_with(&scoped_prefix)))
        {
            return true;
        }

        // Check env var
        std::env::var(self.env_var).is_ok()
    }

    /// Store a credential in the OS keyring
    pub fn store_credential(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<&str>,
    ) -> Result<()> {
        credentials::set_credential(
            self.keyring_service,
            access_token,
            refresh_token,
            expires_at,
        )
    }

    /// Get the full credential (including refresh token) from keyring
    pub fn get_credential(&self) -> Result<Option<credentials::Credential>> {
        credentials::get_credential(self.keyring_service)
    }

    /// Try to get a token from a CLI command
    fn try_cli_token(&self, cmd: &[&str]) -> Result<String> {
        let output = Command::new(cmd[0])
            .args(&cmd[1..])
            .output()
            .map_err(|_| anyhow!("{} CLI not found", self.display_name))?;

        if !output.status.success() {
            return Err(anyhow!("{} CLI not authenticated", self.display_name));
        }

        let token = String::from_utf8(output.stdout)?.trim().to_string();
        if token.is_empty() {
            return Err(anyhow!("{} CLI returned empty token", self.display_name));
        }

        Ok(token)
    }

    /// Build a helpful error message when no auth is available
    fn auth_error(&self) -> anyhow::Error {
        let mut msg = format!("{} not authenticated.\n\n", self.display_name);

        let mut option = 1;

        // CLI option (if available)
        if let Some(cmd) = self.cli_command {
            msg.push_str(&format!(
                "Option {}: Install {} CLI and authenticate\n",
                option, cmd[0]
            ));
            option += 1;
        }

        // OAuth option
        msg.push_str(&format!("Option {}: Run: {}\n", option, self.link_command));
        option += 1;

        // Env var option
        msg.push_str(&format!(
            "Option {}: Set {} environment variable",
            option, self.env_var
        ));

        anyhow!(msg)
    }
}
