## ADDED Requirements

### Requirement: Device Code Flow Initiation
The `github-copilot-auth` module SHALL initiate the OAuth device authorization flow by posting to `https://github.com/login/device/code` (or the enterprise equivalent) with the Copilot client ID. The response MUST yield a `device_code`, `user_code`, `verification_uri`, and `interval`.

#### Scenario: Successful initiation
- **WHEN** `start_device_flow()` is called with valid client credentials
- **THEN** a `DeviceFlowState` is returned containing `user_code` and `verification_uri` for display to the user

#### Scenario: Network failure during initiation
- **WHEN** the GitHub endpoint is unreachable
- **THEN** `start_device_flow()` returns an `Err` with a descriptive network error; no credentials are stored

---

### Requirement: Access Token Polling
The module SHALL poll `https://github.com/login/oauth/access_token` at the interval specified by the device flow until the user completes authorization or the `expires_in` deadline is reached. Polling MUST stop immediately on `authorization_pending` turning to success or on a terminal error.

#### Scenario: User completes authorization
- **WHEN** polling receives a response containing `access_token`
- **THEN** polling stops, the token is returned to the caller, and the `DeviceFlowState` is marked complete

#### Scenario: Polling expires
- **WHEN** the `expires_in` deadline passes without the user authorizing
- **THEN** polling stops and returns `Err(AuthError::Expired)`

---

### Requirement: Token Exchange for Copilot API Token
After obtaining a GitHub OAuth access token the module SHALL exchange it for a short-lived Copilot API token by calling the GitHub Copilot token endpoint. The Copilot token and its expiry time MUST be stored and returned.

#### Scenario: Successful token exchange
- **WHEN** a valid GitHub OAuth token is presented to the Copilot token endpoint
- **THEN** a Copilot API token and its expiry timestamp are returned and cached in memory

#### Scenario: Expired Copilot token
- **WHEN** the cached Copilot token's expiry has passed
- **THEN** the module automatically re-exchanges the stored OAuth token for a fresh Copilot token before the next request

---

### Requirement: Credential Persistence
The module SHALL persist OAuth credentials to `~/.pi/credentials.toml` after a successful device flow. The file MUST be created with mode `0600` (owner read/write only). On subsequent startups credentials MUST be loaded from this file without re-running the device flow.

#### Scenario: Credentials written after authorization
- **WHEN** the user completes the device flow
- **THEN** `~/.pi/credentials.toml` is created or updated with the access token and file permissions are set to `0600`

#### Scenario: Credentials loaded on startup
- **WHEN** `~/.pi/credentials.toml` exists with valid credentials
- **THEN** the module loads them and skips the device flow, returning the stored token to the caller

---

### Requirement: Token Refresh
The module SHALL provide a `get_token()` function that returns a valid Copilot API token, refreshing it transparently if it is expired or absent. Callers MUST NOT need to manage token expiry themselves.

#### Scenario: Valid cached token returned
- **WHEN** `get_token()` is called and the cached Copilot token is still valid
- **THEN** the token is returned immediately without any HTTP call

#### Scenario: Refresh on expiry
- **WHEN** `get_token()` is called and the cached Copilot token is expired
- **THEN** the module exchanges the stored OAuth token for a new Copilot token, updates the in-memory cache, and returns the new token

---

### Requirement: Enterprise GitHub Support
The module SHALL accept an optional `github_api_base_url` configuration value. When set, all OAuth and Copilot token endpoint URLs MUST use that base URL instead of `https://github.com`.

#### Scenario: Enterprise base URL configured
- **WHEN** `github_api_base_url = "https://github.example.com"` is present in `~/.pi/config.toml`
- **THEN** the device flow initiation request is sent to `https://github.example.com/login/device/code`

#### Scenario: Default public GitHub
- **WHEN** no `github_api_base_url` is configured
- **THEN** all requests target `https://github.com` endpoints
