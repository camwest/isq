//! GitHub OAuth device flow authentication

use std::io::Write;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use super::client::create_http_client;

const GITHUB_CLIENT_ID: &str = "Ov23liZ4bn4Ag8Zx7XI2";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// Token response from GitHub OAuth
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct GitHubErrorResponse {
    error: String,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// Run the GitHub Device Flow for authentication
/// Shows a code for the user to enter at github.com/login/device
pub async fn oauth_flow() -> Result<TokenResponse> {
    let client = create_http_client();

    // Step 1: Request device code
    let params = [
        ("client_id", GITHUB_CLIENT_ID),
        ("scope", "repo read:user"),
    ];

    let response = client
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await?;

    let body = response.text().await?;
    let device: DeviceCodeResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow!("Failed to parse device code response: {}\nBody: {}", e, body))?;

    // Step 2: Show code to user and open browser
    println!();
    println!("  Enter code: {}", device.user_code);
    println!("  At: {}", device.verification_uri);
    println!();

    // Try to open browser (but don't fail if it doesn't work)
    let _ = open::that(&device.verification_uri);

    print!("Waiting for authorization...");
    std::io::stdout().flush()?;

    // Step 3: Poll for token
    let interval = std::time::Duration::from_secs(device.interval.max(5));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(device.expires_in);

    while std::time::Instant::now() < deadline {
        std::thread::sleep(interval);

        let params = [
            ("client_id", GITHUB_CLIENT_ID),
            ("device_code", device.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];

        let response = client
            .post(GITHUB_TOKEN_URL)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await?;

        let body = response.text().await?;

        // Check for error response
        if let Ok(error_resp) = serde_json::from_str::<GitHubErrorResponse>(&body) {
            match error_resp.error.as_str() {
                "authorization_pending" => {
                    print!(".");
                    std::io::stdout().flush()?;
                    continue;
                }
                "slow_down" => {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    continue;
                }
                "expired_token" => {
                    println!();
                    return Err(anyhow!("Authorization timed out. Please try again."));
                }
                "access_denied" => {
                    println!();
                    return Err(anyhow!("Authorization was denied."));
                }
                _ => {
                    println!();
                    let desc = error_resp.error_description.unwrap_or_default();
                    return Err(anyhow!("GitHub error: {} - {}", error_resp.error, desc));
                }
            }
        }

        // Success - parse token
        if let Ok(token) = serde_json::from_str::<TokenResponse>(&body) {
            println!(" ✓");
            return Ok(token);
        }
    }

    println!();
    Err(anyhow!("Authorization timed out. Please try again."))
}
