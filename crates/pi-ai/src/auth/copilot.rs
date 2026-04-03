use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::AuthError;

// ─── Constants ────────────────────────────────────────────────────────────────

/// GitHub OAuth app client ID used by VS Code Copilot Chat 0.35.0.
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// Copilot-compatible User-Agent string.
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";

/// VS Code editor version reported to GitHub.
const EDITOR_VERSION: &str = "vscode/1.107.0";

// ─── Task 4.1 — Device code flow ─────────────────────────────────────────────

/// State returned by the GitHub device code flow initiation.
#[derive(Debug, Clone)]
pub struct DeviceFlowState {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    /// Minimum polling interval in seconds.
    pub interval: u64,
    /// Seconds until the device code expires.
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// Start the GitHub device code OAuth flow.
///
/// Sends a POST to `{github_base}/login/device/code` and returns the state
/// needed to display the user-facing prompt and begin polling.
pub async fn start_device_flow(
    client: &reqwest::Client,
    github_base: &str,
) -> Result<DeviceFlowState, AuthError> {
    let url = format!("{github_base}/login/device/code");

    let response = client
        .post(&url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", USER_AGENT)
        .body(format!("client_id={CLIENT_ID}&scope=read:user"))
        .send()
        .await?;

    let body: DeviceCodeResponse = response.json().await?;

    Ok(DeviceFlowState {
        user_code: body.user_code,
        verification_uri: body.verification_uri,
        device_code: body.device_code,
        interval: body.interval,
        expires_in: body.expires_in,
    })
}

// ─── Task 4.2 — Poll for GitHub access token ─────────────────────────────────

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Poll the GitHub OAuth token endpoint until we receive a token, the code
/// expires, or `cancel` is triggered.
///
/// Handles the standard device-flow errors:
/// - `authorization_pending` → keep waiting
/// - `slow_down` → increase the polling interval by 5 s
/// - `expired_token` → return [`AuthError::Expired`]
pub async fn poll_for_token(
    client: &reqwest::Client,
    github_base: &str,
    device_code: &str,
    interval: u64,
    expires_in: u64,
    cancel: CancellationToken,
) -> Result<String, AuthError> {
    let url = format!("{github_base}/login/oauth/access_token");
    let started_at = unix_now();
    let mut poll_interval = interval;

    loop {
        // Check for cancellation before sleeping.
        if cancel.is_cancelled() {
            return Err(AuthError::Cancelled);
        }

        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(poll_interval)) => {},
            _ = cancel.cancelled() => {
                return Err(AuthError::Cancelled);
            }
        }

        // Check wall-clock expiry.
        if unix_now().saturating_sub(started_at) >= expires_in {
            return Err(AuthError::Expired);
        }

        let body = format!(
            "client_id={CLIENT_ID}&device_code={device_code}\
             &grant_type=urn:ietf:params:oauth:grant-type:device_code"
        );

        let response = client
            .post(&url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", USER_AGENT)
            .body(body)
            .send()
            .await?;

        let payload: AccessTokenResponse = response.json().await?;

        if let Some(token) = payload.access_token {
            if !token.is_empty() {
                return Ok(token);
            }
        }

        match payload.error.as_deref() {
            Some("authorization_pending") => {
                // Normal — keep waiting.
            }
            Some("slow_down") => {
                poll_interval += 5;
            }
            Some("expired_token") => {
                return Err(AuthError::Expired);
            }
            Some(other) => {
                return Err(AuthError::Failed(format!("OAuth error: {other}")));
            }
            None => {
                // No error and no token — treat as pending.
            }
        }
    }
}

// ─── Task 4.3 — Copilot token exchange ───────────────────────────────────────

/// A short-lived GitHub Copilot API token.
#[derive(Debug, Clone)]
pub struct CopilotToken {
    pub token: String,
    /// Unix timestamp (seconds) at which this token expires.
    pub expires_at: u64,
    /// Base URL for Copilot API requests, e.g. `https://api.individual.githubcopilot.com`.
    pub base_url: String,
}

impl CopilotToken {
    /// Returns `true` if the token has expired (with a 60-second safety margin).
    pub fn is_expired(&self) -> bool {
        unix_now() + 60 >= self.expires_at
    }
}

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: u64,
}

/// Exchange a GitHub OAuth token for a short-lived Copilot token.
///
/// Uses `enterprise_domain` (e.g. `github.example.com`) when provided,
/// otherwise falls back to `api.github.com`.
pub async fn exchange_copilot_token(
    client: &reqwest::Client,
    github_token: &str,
    enterprise_domain: Option<&str>,
) -> Result<CopilotToken, AuthError> {
    let api_domain = enterprise_domain.unwrap_or("github.com");
    let url = format!("https://api.{api_domain}/copilot_internal/v2/token");

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {github_token}"))
        .header("User-Agent", USER_AGENT)
        .header("Editor-Version", EDITOR_VERSION)
        .header("Editor-Plugin-Version", "copilot-chat/0.35.0")
        .header("Openai-Intent", "conversation-panel")
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(AuthError::Failed(format!(
            "Copilot token exchange failed ({status}): {text}"
        )));
    }

    let payload: CopilotTokenResponse = response.json().await?;

    // Extract the proxy endpoint from the JWT.
    // The token contains annotations like `proxy-ep=proxy.individual.githubcopilot.com`.
    let base_url = extract_base_url_from_token(&payload.token);

    Ok(CopilotToken {
        token: payload.token,
        expires_at: payload.expires_at,
        base_url,
    })
}

/// Parse `proxy-ep=<host>` out of the JWT payload annotations and convert it
/// to a full HTTPS URL:
/// `proxy.individual.githubcopilot.com` → `https://api.individual.githubcopilot.com`
///
/// Falls back to `https://api.githubcopilot.com` if the claim is absent.
fn extract_base_url_from_token(token: &str) -> String {
    // JWT: header.payload.signature — we only need the payload (index 1).
    if let Some(payload_b64) = token.split('.').nth(1) {
        // JWT uses base64url (no padding).
        use std::str;

        // Pad to a multiple of 4.
        let padding = (4 - payload_b64.len() % 4) % 4;
        let padded = format!("{payload_b64}{}", "=".repeat(padding));

        if let Ok(decoded) = base64_url_decode(&padded) {
            if let Ok(text) = str::from_utf8(&decoded) {
                // Look for proxy-ep claim: `"proxy-ep":"proxy.individual.githubcopilot.com"`
                // or as part of annotations string `proxy-ep=proxy.individual.githubcopilot.com`
                if let Some(host) = extract_proxy_ep(text) {
                    // Strip the leading "proxy." to get the API subdomain.
                    let api_host = host
                        .strip_prefix("proxy.")
                        .map(|s| format!("api.{s}"))
                        .unwrap_or_else(|| host.to_string());
                    return format!("https://{api_host}");
                }
            }
        }
    }

    "https://api.githubcopilot.com".to_string()
}

fn base64_url_decode(s: &str) -> Result<Vec<u8>, ()> {
    // Convert base64url to standard base64.
    let standard = s.replace('-', "+").replace('_', "/");
    // Simple decode without an external dep — use std only.
    // We rely on the `base64` feature NOT being available and do a manual decode.
    // Since reqwest already pulls in base64 transitively this is fine, but to
    // avoid adding a dep we can just decode inline or use a small helper.
    decode_base64_standard(&standard)
}

fn decode_base64_standard(s: &str) -> Result<Vec<u8>, ()> {
    const CHARS: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|&b| b != b'=')
        .map(|b| {
            CHARS
                .iter()
                .position(|&c| c == b)
                .map(|p| p as u8)
                .unwrap_or(0)
        })
        .collect();

    for chunk in bytes.chunks(4) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let b3 = *chunk.get(3).unwrap_or(&0);

        out.push((b0 << 2) | (b1 >> 4));
        if chunk.len() > 2 {
            out.push((b1 << 4) | (b2 >> 2));
        }
        if chunk.len() > 3 {
            out.push((b2 << 6) | b3);
        }
    }

    Ok(out)
}

fn extract_proxy_ep(jwt_payload_text: &str) -> Option<String> {
    // Try JSON key: "proxy-ep":"<value>"
    if let Some(idx) = jwt_payload_text.find("proxy-ep") {
        let rest = &jwt_payload_text[idx + "proxy-ep".len()..];
        // Skip past `":"` or `=`
        let value_start = rest.find(|c| !matches!(c, ':' | '"' | ' ' | '='))?;
        let value = &rest[value_start..];
        let end = value
            .find(['"', ',', '}', ';', ' '])
            .unwrap_or(value.len());
        let host = value[..end].trim().to_string();
        if !host.is_empty() {
            return Some(host);
        }
    }
    None
}

// ─── Task 4.4 — Token manager with RwLock ────────────────────────────────────

/// Manages a cached Copilot token, refreshing it automatically when it expires.
pub struct CopilotAuthManager {
    client: reqwest::Client,
    github_token: String,
    enterprise_domain: Option<String>,
    cached_token: RwLock<Option<CopilotToken>>,
}

impl CopilotAuthManager {
    /// Create a new manager.
    pub fn new(
        client: reqwest::Client,
        github_token: String,
        enterprise_domain: Option<String>,
    ) -> Self {
        Self {
            client,
            github_token,
            enterprise_domain,
            cached_token: RwLock::new(None),
        }
    }

    /// Return a valid Copilot token, refreshing if necessary.
    ///
    /// Uses a double-checked lock pattern:
    /// 1. Check under a read lock — if valid, return immediately.
    /// 2. If expired/missing, acquire write lock, re-check, then refresh.
    pub async fn get_token(&self) -> Result<String, AuthError> {
        // Fast path: read lock.
        {
            let guard = self.cached_token.read().await;
            if let Some(ref ct) = *guard {
                if !ct.is_expired() {
                    return Ok(ct.token.clone());
                }
            }
        }

        // Slow path: write lock with re-check.
        let mut guard = self.cached_token.write().await;
        if let Some(ref ct) = *guard {
            if !ct.is_expired() {
                return Ok(ct.token.clone());
            }
        }

        let fresh = exchange_copilot_token(
            &self.client,
            &self.github_token,
            self.enterprise_domain.as_deref(),
        )
        .await?;

        let token_str = fresh.token.clone();
        *guard = Some(fresh);
        Ok(token_str)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_secs()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_token_not_expired_when_far_future() {
        let token = CopilotToken {
            token: "tok".to_string(),
            expires_at: unix_now() + 3600,
            base_url: "https://api.individual.githubcopilot.com".to_string(),
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn copilot_token_expired_when_in_past() {
        let token = CopilotToken {
            token: "tok".to_string(),
            expires_at: unix_now() - 1,
            base_url: "https://api.individual.githubcopilot.com".to_string(),
        };
        assert!(token.is_expired());
    }

    #[test]
    fn copilot_token_expired_within_safety_margin() {
        // expires_at is 30 seconds from now — within the 60-second margin.
        let token = CopilotToken {
            token: "tok".to_string(),
            expires_at: unix_now() + 30,
            base_url: "https://api.individual.githubcopilot.com".to_string(),
        };
        assert!(token.is_expired());
    }

    #[test]
    fn extract_base_url_falls_back_when_no_proxy_ep() {
        let fake_jwt = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ0ZXN0In0.";
        let url = extract_base_url_from_token(fake_jwt);
        assert_eq!(url, "https://api.githubcopilot.com");
    }

    #[test]
    fn extract_proxy_ep_finds_value() {
        let payload = r#"{"proxy-ep":"proxy.individual.githubcopilot.com","sku":"copilot_for_business"}"#;
        let result = extract_proxy_ep(payload);
        assert_eq!(
            result.as_deref(),
            Some("proxy.individual.githubcopilot.com")
        );
    }

    #[test]
    fn extract_proxy_ep_returns_none_when_absent() {
        let payload = r#"{"sub":"user","iat":1234567890}"#;
        assert!(extract_proxy_ep(payload).is_none());
    }
}
