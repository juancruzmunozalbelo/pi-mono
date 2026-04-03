use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::AuthError;

/// Credentials stored on disk for reuse across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredentials {
    pub github_token: String,
    pub enterprise_domain: Option<String>,
}

/// Returns the path where credentials are persisted.
/// `~/.pi/credentials.toml`
pub fn credentials_path() -> PathBuf {
    dirs::home_dir()
        .expect("home directory must be locatable")
        .join(".pi")
        .join("credentials.toml")
}

/// Serialize `creds` to TOML and write to [`credentials_path()`].
/// On Unix the file permissions are set to 0600 so only the owner can read it.
pub fn save_credentials(creds: &StoredCredentials) -> Result<(), AuthError> {
    let path = credentials_path();

    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let contents = toml::to_string_pretty(creds)
        .map_err(|e| AuthError::Failed(format!("TOML serialisation error: {e}")))?;

    std::fs::write(&path, &contents)?;

    // Restrict permissions to owner-only on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

/// Load credentials from [`credentials_path()`].
/// Returns `None` if the file does not exist.
pub fn load_credentials() -> Result<Option<StoredCredentials>, AuthError> {
    let path = credentials_path();

    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(&path)?;
    let creds: StoredCredentials = toml::from_str(&contents)
        .map_err(|e| AuthError::Failed(format!("TOML deserialisation error: {e}")))?;

    Ok(Some(creds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_path_ends_with_expected_components() {
        let path = credentials_path();
        let mut components = path.components().rev();
        assert_eq!(
            components.next().unwrap().as_os_str(),
            "credentials.toml",
            "last component should be credentials.toml"
        );
        assert_eq!(
            components.next().unwrap().as_os_str(),
            ".pi",
            "second-to-last component should be .pi"
        );
    }

    #[test]
    fn stored_credentials_round_trips_through_toml() {
        let original = StoredCredentials {
            github_token: "ghp_test_token_abc123".to_string(),
            enterprise_domain: Some("github.example.com".to_string()),
        };

        let serialised = toml::to_string_pretty(&original).expect("serialisation must succeed");
        let deserialised: StoredCredentials =
            toml::from_str(&serialised).expect("deserialisation must succeed");

        assert_eq!(deserialised.github_token, original.github_token);
        assert_eq!(deserialised.enterprise_domain, original.enterprise_domain);
    }

    #[test]
    fn stored_credentials_round_trips_without_enterprise_domain() {
        let original = StoredCredentials {
            github_token: "ghp_no_enterprise".to_string(),
            enterprise_domain: None,
        };

        let serialised = toml::to_string_pretty(&original).expect("serialisation must succeed");
        let deserialised: StoredCredentials =
            toml::from_str(&serialised).expect("deserialisation must succeed");

        assert_eq!(deserialised.github_token, original.github_token);
        assert!(deserialised.enterprise_domain.is_none());
    }
}
