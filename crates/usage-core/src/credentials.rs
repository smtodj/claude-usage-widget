use std::path::PathBuf;

use serde::Deserialize;

use crate::UsageError;

/// Keychain item Claude Code writes its OAuth credentials to on macOS.
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialsSource {
    Env,
    Keychain,
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_token: String,
    /// Unix time in milliseconds, when known.
    pub expires_at_ms: Option<i64>,
    /// "pro", "max", "team", ... when known.
    pub subscription_type: Option<String>,
    pub source: CredentialsSource,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    claude_ai_oauth: Option<OAuthEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthEntry {
    access_token: String,
    expires_at: Option<i64>,
    subscription_type: Option<String>,
}

/// Finds Claude Code's OAuth token. Order:
/// 1. `CLAUDE_USAGE_TOKEN` environment variable (manual override)
/// 2. macOS Keychain item `Claude Code-credentials`
/// 3. `$CLAUDE_CONFIG_DIR/.credentials.json`, then `~/.claude/.credentials.json`
///
/// The token is only read, never refreshed or written: Claude Code owns the
/// rotating refresh token, and refreshing it here could log the CLI out.
pub fn read_credentials() -> Result<Credentials, UsageError> {
    if let Ok(token) = std::env::var("CLAUDE_USAGE_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(Credentials {
                access_token: token,
                expires_at_ms: None,
                subscription_type: None,
                source: CredentialsSource::Env,
            });
        }
    }

    let mut last_err = None;

    #[cfg(target_os = "macos")]
    match read_keychain() {
        Ok(Some(json)) => match parse_credentials(&json, CredentialsSource::Keychain) {
            Ok(c) => return check_expiry(c),
            Err(e) => last_err = Some(e),
        },
        Ok(None) => {}
        Err(e) => last_err = Some(e),
    }

    for path in credential_files() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            match parse_credentials(&json, CredentialsSource::File(path)) {
                Ok(c) => return check_expiry(c),
                Err(e) => last_err = Some(e),
            }
        }
    }

    Err(last_err.unwrap_or(UsageError::NotLoggedIn))
}

fn check_expiry(c: Credentials) -> Result<Credentials, UsageError> {
    if let Some(exp) = c.expires_at_ms {
        if exp <= chrono::Utc::now().timestamp_millis() {
            return Err(UsageError::TokenExpired);
        }
    }
    Ok(c)
}

pub(crate) fn parse_credentials(
    json: &str,
    source: CredentialsSource,
) -> Result<Credentials, UsageError> {
    let file: CredentialsFile =
        serde_json::from_str(json).map_err(|e| UsageError::Credentials(e.to_string()))?;
    let entry = file.claude_ai_oauth.ok_or(UsageError::NotLoggedIn)?;
    if entry.access_token.is_empty() {
        return Err(UsageError::NotLoggedIn);
    }
    Ok(Credentials {
        access_token: entry.access_token,
        expires_at_ms: entry.expires_at,
        subscription_type: entry.subscription_type,
        source,
    })
}

fn credential_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(dirs) = std::env::var("CLAUDE_CONFIG_DIR") {
        for dir in dirs.split(',').map(str::trim).filter(|d| !d.is_empty()) {
            paths.push(PathBuf::from(dir).join(".credentials.json"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".claude").join(".credentials.json"));
    }
    paths
}

/// Returns the keychain item's JSON, or `None` if there is no such item.
#[cfg(target_os = "macos")]
fn read_keychain() -> Result<Option<String>, UsageError> {
    let output = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .map_err(|e| UsageError::Credentials(e.to_string()))?;
    if !output.status.success() {
        // Exit code 44: item not found. Anything else (e.g. the user denied
        // the keychain prompt) is reported so the menu can say why.
        if output.status.code() == Some(44) {
            return Ok(None);
        }
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(UsageError::Credentials(format!("Keychain: {msg}")));
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn read_keychain() -> Result<Option<String>, UsageError> {
    let _ = KEYCHAIN_SERVICE;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_code_credentials() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-abc","refreshToken":"sk-ant-ort01-def","expiresAt":1900000000000,"scopes":["user:inference","user:profile"],"subscriptionType":"max"}}"#;
        let c = parse_credentials(json, CredentialsSource::Keychain).unwrap();
        assert_eq!(c.access_token, "sk-ant-oat01-abc");
        assert_eq!(c.expires_at_ms, Some(1_900_000_000_000));
        assert_eq!(c.subscription_type.as_deref(), Some("max"));
    }

    #[test]
    fn missing_oauth_entry_means_not_logged_in() {
        let err = parse_credentials("{}", CredentialsSource::Keychain).unwrap_err();
        assert!(matches!(err, UsageError::NotLoggedIn));
    }

    #[test]
    fn expired_token_is_rejected() {
        let c = Credentials {
            access_token: "x".into(),
            expires_at_ms: Some(1),
            subscription_type: None,
            source: CredentialsSource::Env,
        };
        assert!(matches!(check_expiry(c), Err(UsageError::TokenExpired)));
    }
}
