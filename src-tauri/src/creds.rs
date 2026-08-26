//! Session credentials.
//!
//! The plaintext token lives here and nowhere else. It never crosses back into
//! the webview except through the explicit `reveal_token` command, and it is
//! zeroized when the session ends.

use serde::Serialize;
use zeroize::Zeroize;

use crate::error::{AppError, AppResult, Secrets};

/// Hosts we consider in-policy. A token pasted against anything else is the one
/// realistic way it leaves the workspace, so it earns a warning.
const ALLOWED_HOST_SUFFIX: &str = ".azuredatabricks.net";

/// How many leading/trailing characters of the token stay visible in previews.
const PREVIEW_EDGE: usize = 4;

pub struct SessionCreds {
    host: String,
    token: String,
}

impl SessionCreds {
    /// Normalizes the host (adds `https://`, drops a trailing slash) and
    /// rejects empty input.
    pub fn new(host: &str, token: &str) -> AppResult<Self> {
        let host = normalize_host(host)?;
        let token = token.trim().to_string();
        if token.is_empty() {
            return Err(AppError::config("token is empty"));
        }
        Ok(Self { host, token })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Builds a URL for a workspace API path.
    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.host, path.trim_start_matches('/'))
    }

    pub fn secrets(&self) -> Secrets {
        let mut secrets = Secrets::new();
        secrets.add(&self.token);
        secrets
    }

    /// `dapi••••••••3f2a` — enough to tell two tokens apart, not enough to use.
    pub fn masked_token(&self) -> String {
        mask_token(&self.token)
    }

    pub fn host_in_policy(&self) -> bool {
        host_in_policy(&self.host)
    }
}

// Never print the token, even by accident.
impl std::fmt::Debug for SessionCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCreds")
            .field("host", &self.host)
            .field("token", &"***")
            .finish()
    }
}

impl std::fmt::Display for SessionCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (token ***)", self.host)
    }
}

impl Drop for SessionCreds {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

/// Where the credentials on the boot screen came from, so the UI can say so
/// rather than leaving the user guessing which token is in play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredSource {
    Environment,
    DotEnv,
    Remembered,
    None,
}

/// What the frontend is allowed to know about the connection.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInfo {
    pub host: String,
    pub masked_token: String,
    pub user: Option<String>,
    pub host_in_policy: bool,
}

pub fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= PREVIEW_EDGE * 2 {
        return "•".repeat(chars.len().max(4));
    }
    let head: String = chars[..PREVIEW_EDGE].iter().collect();
    let tail: String = chars[chars.len() - PREVIEW_EDGE..].iter().collect();
    format!("{head}{}{tail}", "•".repeat(8))
}

pub fn host_in_policy(host: &str) -> bool {
    match host
        .strip_prefix("https://")
        .or_else(|| host.strip_prefix("http://"))
    {
        Some(rest) => {
            let authority = rest.split('/').next().unwrap_or(rest);
            let hostname = authority.split(':').next().unwrap_or(authority);
            hostname.ends_with(ALLOWED_HOST_SUFFIX)
        }
        None => false,
    }
}

/// Accepts what people actually paste: with or without a scheme, with or without
/// a trailing slash, and with stray whitespace.
pub fn normalize_host(host: &str) -> AppResult<String> {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::config("host is empty"));
    }

    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    // Reject anything with whitespace or an obviously broken authority early,
    // rather than letting it fail later as an opaque DNS error.
    let authority = with_scheme
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .unwrap_or_default();
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return Err(AppError::config(format!("{host} is not a valid host")));
    }

    Ok(with_scheme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_scheme_and_trims_trailing_slash() {
        assert_eq!(
            normalize_host("adb-123.4.azuredatabricks.net/").unwrap(),
            "https://adb-123.4.azuredatabricks.net"
        );
        assert_eq!(
            normalize_host("  https://adb-123.4.azuredatabricks.net  ").unwrap(),
            "https://adb-123.4.azuredatabricks.net"
        );
    }

    #[test]
    fn rejects_empty_or_whitespace_hosts() {
        assert!(normalize_host("").is_err());
        assert!(normalize_host("   ").is_err());
        assert!(normalize_host("host with spaces").is_err());
    }

    #[test]
    fn recognises_in_policy_hosts() {
        assert!(host_in_policy("https://adb-123.4.azuredatabricks.net"));
        assert!(host_in_policy("https://adb-123.4.azuredatabricks.net:443"));
        assert!(!host_in_policy("https://evil.example.com"));
        // A lookalike suffix must not pass.
        assert!(!host_in_policy("https://azuredatabricks.net.evil.com"));
    }

    #[test]
    fn masks_all_but_the_edges() {
        let masked = mask_token("dapiabcdefghijklmnop3f2a");
        assert!(masked.starts_with("dapi"));
        assert!(masked.ends_with("3f2a"));
        assert!(!masked.contains("efghij"));
    }

    #[test]
    fn masks_short_tokens_entirely() {
        assert!(!mask_token("abcd").contains('a'));
    }

    #[test]
    fn debug_output_hides_the_token() {
        let creds = SessionCreds::new("adb-1.2.azuredatabricks.net", "dapisecrettoken").unwrap();
        let debug = format!("{creds:?}");
        assert!(!debug.contains("dapisecrettoken"));
        assert!(debug.contains("***"));
    }

    #[test]
    fn builds_urls_without_double_slashes() {
        let creds = SessionCreds::new("https://adb-1.2.azuredatabricks.net", "tok12345").unwrap();
        assert_eq!(
            creds.url("/api/2.0/mlflow/runs/get"),
            "https://adb-1.2.azuredatabricks.net/api/2.0/mlflow/runs/get"
        );
    }
}
