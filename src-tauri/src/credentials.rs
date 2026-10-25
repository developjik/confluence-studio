//! Connection credentials stored in the OS keychain (macOS Keychain / Windows Credential Manager).
//! The API token NEVER touches disk. base_url/email are non-secret but stored in the same
//! keychain entry set for a single load path.

use crate::confluence::ConfluenceAuth;
use keyring::Entry;
use serde::{Deserialize, Serialize};

pub const SERVICE: &str = "com.developjik.confluencestudio";
const ACCOUNT_TOKEN: &str = "confluence-api-token";
const ACCOUNT_EMAIL: &str = "confluence-email";
const ACCOUNT_BASE_URL: &str = "confluence-base-url";

/// Connection metadata (no token). serde camelCase for the Tauri boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub base_url: String,
    pub email: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("keychain error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("no connection configured")]
    NotConfigured,
}

fn entry(account: &str) -> Result<Entry, CredentialError> {
    Ok(Entry::new(SERVICE, account)?)
}

/// Save connection + token. Overwrites previous values.
pub fn save(base_url: &str, email: &str, token: &str) -> Result<(), CredentialError> {
    entry(ACCOUNT_BASE_URL)?.set_password(base_url)?;
    entry(ACCOUNT_EMAIL)?.set_password(email)?;
    entry(ACCOUNT_TOKEN)?.set_password(token)?;
    Ok(())
}

/// Full credentials including token — in-process use only, never return to the webview.
pub fn load_full() -> Result<ConfluenceAuth, CredentialError> {
    let base_url = entry(ACCOUNT_BASE_URL)?
        .get_password()
        .map_err(|_| CredentialError::NotConfigured)?;
    let email = entry(ACCOUNT_EMAIL)?
        .get_password()
        .map_err(|_| CredentialError::NotConfigured)?;
    let token = entry(ACCOUNT_TOKEN)?
        .get_password()
        .map_err(|_| CredentialError::NotConfigured)?;
    Ok(ConfluenceAuth {
        base_url,
        email,
        token,
    })
}

/// Non-secret connection info for the UI.
pub fn load_info() -> Result<Option<ConnectionInfo>, CredentialError> {
    let base_url = entry(ACCOUNT_BASE_URL)?.get_password().ok();
    let email = entry(ACCOUNT_EMAIL)?.get_password().ok();
    Ok(match (base_url, email) {
        (Some(base_url), Some(email)) => Some(ConnectionInfo { base_url, email }),
        _ => None,
    })
}

/// Forget everything.
pub fn clear() -> Result<(), CredentialError> {
    for account in [ACCOUNT_TOKEN, ACCOUNT_EMAIL, ACCOUNT_BASE_URL] {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_is_none_when_missing() {
        clear().ok();
        assert!(load_info().unwrap().is_none());
    }
}
