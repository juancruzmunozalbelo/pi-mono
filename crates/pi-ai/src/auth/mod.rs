//! GitHub Copilot OAuth authentication.
//!
//! # Flow
//! 1. Call [`copilot::start_device_flow`] to obtain a `DeviceFlowState`.
//! 2. Show [`DeviceFlowState::user_code`] and [`DeviceFlowState::verification_uri`] to the user.
//! 3. Call [`copilot::poll_for_token`] to wait for the GitHub OAuth token.
//! 4. Persist the token with [`credentials::save_credentials`].
//! 5. Build a [`copilot::CopilotAuthManager`] for automatic Copilot token refresh.

pub mod copilot;
pub mod credentials;

pub use copilot::{
    CopilotAuthManager, CopilotToken, DeviceFlowState, exchange_copilot_token, poll_for_token,
    start_device_flow,
};
pub use credentials::{StoredCredentials, credentials_path, load_credentials, save_credentials};

/// Errors that can occur during authentication.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Device flow expired")]
    Expired,

    #[error("Device flow cancelled")]
    Cancelled,

    #[error("Auth failed: {0}")]
    Failed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
