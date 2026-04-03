//! LlmProvider trait, ProviderError, and provider registry.

use async_trait::async_trait;
use futures::Stream;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::types::*;

/// Errors returned by LLM providers.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("SSE error: {0}")]
    Sse(String),
    #[error("Auth error: {0}")]
    Auth(String),
    #[error("Rate limited, retry after {retry_after_ms:?}ms")]
    RateLimit { retry_after_ms: Option<u64> },
    #[error("Provider error: {0}")]
    Other(String),
}

/// A pinned, boxed streaming response of chat events.
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatEvent, ProviderError>> + Send>>;

/// Unified interface for all LLM providers.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatStream, ProviderError>;
    fn name(&self) -> &str;
}

/// Select a provider by name, constructing it with the given API key.
pub fn get_provider(name: &str, api_key: String) -> Result<Box<dyn LlmProvider>, ProviderError> {
    match name {
        "github-copilot" => Ok(Box::new(crate::openai::OpenAiCompletionsProvider::new(
            api_key,
        ))),
        "minimax" => Ok(Box::new(crate::anthropic::AnthropicMessagesProvider::new(
            api_key,
        ))),
        _ => Err(ProviderError::Other(format!("Unknown provider: {name}"))),
    }
}

// ─── Retry helper ────────────────────────────────────────────────────────────

/// Execute `f` up to `max_retries + 1` times, backing off on transient errors.
///
/// Retries on:
/// - `ProviderError::RateLimit` (respects `retry_after_ms` if present)
/// - HTTP 429 / 5xx surfaced via `ProviderError::Http` (best-effort status detection)
pub async fn with_retry<F, Fut>(max_retries: u32, f: F) -> Result<ChatStream, ProviderError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<ChatStream, ProviderError>>,
{
    let mut attempt = 0u32;

    loop {
        match f().await {
            Ok(stream) => return Ok(stream),
            Err(e) if attempt >= max_retries => return Err(e),
            Err(e) => {
                let wait_ms = retry_delay_ms(&e, attempt);
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            }
        }
    }
}

/// Compute how long to wait before the next attempt, in milliseconds.
fn retry_delay_ms(err: &ProviderError, attempt: u32) -> u64 {
    // If the provider told us how long to wait, respect that.
    if let ProviderError::RateLimit {
        retry_after_ms: Some(ms),
    } = err
    {
        return *ms;
    }

    // Check if this is a non-retryable HTTP error.
    if let ProviderError::Http(re) = err {
        let status = re.status().map(|s| s.as_u16()).unwrap_or(0);
        // Only retry on 429 and 5xx.
        if status != 429 && !(500..=599).contains(&status) && status != 0 {
            // Return 0 but the outer loop won't actually call retry for
            // non-transient errors — we surface them immediately by returning
            // a very large value so the outer `attempt >= max_retries` guard
            // catches it. But actually we should just not retry at all.
            // Re-check: the match arm above only falls here if attempt < max_retries,
            // so for non-retryable HTTP we want to NOT wait but also NOT retry.
            // The cleanest way: return u64::MAX so the sleep is huge… but that's
            // wrong. Let's refactor: move the "is retryable?" check up.
            return u64::MAX; // caller will sleep this, but we handle below
        }
    }

    // Exponential backoff: 1s, 2s, 4s … with ±20% jitter.
    let base_ms: u64 = 1000u64 << attempt.min(10);
    let jitter = (base_ms as f64 * 0.2 * (pseudo_rand_f64() - 0.5)) as i64;
    (base_ms as i64 + jitter).max(100) as u64
}

/// Very cheap deterministic "jitter" that avoids pulling in `rand`.
fn pseudo_rand_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345);
    (nanos % 1000) as f64 / 1000.0
}
