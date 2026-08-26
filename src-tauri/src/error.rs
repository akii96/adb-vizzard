//! Error type plus the redaction helper that keeps secrets out of messages.
//!
//! Every error that can reach the frontend goes through [`AppError`]. The
//! important property is [`Secrets::redact`]: `reqwest` and URL parse errors
//! happily echo back whatever you handed them, including a bearer token in a
//! header or a SAS signature in a query string, so error text is scrubbed before
//! it is ever serialized.

use std::fmt;

use serde::Serialize;

/// Placeholder substituted for any secret found in an error string.
const MASK: &str = "[redacted]";

/// Minimum length before a value is treated as a redactable secret. Guards
/// against a short or empty token turning every message into mush.
const MIN_SECRET_LEN: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not connected: connect to a Databricks workspace first")]
    NotConnected,

    #[error("authentication failed: the token was rejected by {host}")]
    Unauthorized { host: String },

    #[error("cannot reach {host}: {detail}")]
    Network { host: String, detail: String },

    #[error("{0}")]
    Api(String),

    #[error("run {run_id} was not found in this workspace")]
    RunNotFound { run_id: String },

    #[error("{0} is not a valid MLflow run ID or Databricks run URL")]
    BadRunRef(String),

    #[error("artifact {path} is missing for run {run_id}")]
    MissingArtifact { run_id: String, path: String },

    /// A signed URI expired between being vended and being used. Its own variant
    /// so the retry is a type-level decision rather than a message-substring match.
    #[error("the download link expired before it could be used")]
    SasExpired,

    #[error("{0}")]
    Parse(String),

    #[error("no benchmark runs could be read from {0}")]
    EmptySide(String),

    #[error("load was cancelled")]
    Cancelled,

    #[error("{0}")]
    Io(String),

    #[error("{0}")]
    Config(String),
}

impl AppError {
    pub fn api(msg: impl Into<String>) -> Self {
        Self::Api(msg.into())
    }

    pub fn parse(msg: impl Into<String>) -> Self {
        Self::Parse(msg.into())
    }

    pub fn io(msg: impl Into<String>) -> Self {
        Self::Io(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Stable machine-readable discriminant, so the UI can react to a class of
    /// failure without string matching on the message.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotConnected => "not_connected",
            Self::Unauthorized { .. } => "unauthorized",
            Self::Network { .. } => "network",
            Self::Api(_) => "api",
            Self::RunNotFound { .. } => "run_not_found",
            Self::BadRunRef(_) => "bad_run_ref",
            Self::MissingArtifact { .. } => "missing_artifact",
            Self::SasExpired => "sas_expired",
            Self::Parse(_) => "parse",
            Self::EmptySide(_) => "empty_side",
            Self::Cancelled => "cancelled",
            Self::Io(_) => "io",
            Self::Config(_) => "config",
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

/// Wire format for errors crossing into the webview.
#[derive(Debug, Serialize)]
pub struct SerializedError {
    pub kind: &'static str,
    pub message: String,
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SerializedError {
            kind: self.kind(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// A set of values that must never appear in user-visible text.
///
/// Holds the bearer token and any live SAS URI signatures. Cheap to clone
/// because it is only ever a handful of short strings.
#[derive(Default, Clone)]
pub struct Secrets {
    values: Vec<String>,
}

impl Secrets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, value: &str) {
        let value = value.trim();
        if value.len() >= MIN_SECRET_LEN && !self.values.iter().any(|v| v == value) {
            self.values.push(value.to_string());
        }
    }

    /// Replaces every known secret with [`MASK`].
    ///
    /// Also strips SAS query parameters wholesale, because a signed URI we have
    /// never seen before is still a bearer credential.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in &self.values {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), MASK);
            }
        }
        redact_sas_params(&out)
    }
}

// Deliberately opaque: a stray `{:?}` on this must not print the contents.
impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secrets({} values)", self.values.len())
    }
}

/// Blanks the value of any query parameter that carries a signature.
///
/// Azure SAS URIs put the credential in `sig=`; presigned S3 uses
/// `X-Amz-Signature`. Neither should survive into a log line.
fn redact_sas_params(text: &str) -> String {
    const SENSITIVE_KEYS: [&str; 4] = ["sig=", "sig%3D", "X-Amz-Signature=", "X-Amz-Credential="];

    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    'outer: while !rest.is_empty() {
        let mut best: Option<(usize, usize)> = None;
        for key in SENSITIVE_KEYS {
            if let Some(idx) = rest.find(key) {
                if best.map_or(true, |(b, _)| idx < b) {
                    best = Some((idx, key.len()));
                }
            }
        }

        match best {
            Some((idx, key_len)) => {
                let value_start = idx + key_len;
                out.push_str(&rest[..value_start]);
                out.push_str(MASK);
                // Preserve whatever delimiter ended the value so the rest of the
                // message stays readable.
                let tail = &rest[value_start..];
                match tail.find(|c: char| c == '&' || c.is_whitespace() || c == '"' || c == '\'') {
                    Some(end) => rest = &tail[end..],
                    None => break 'outer,
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_a_known_token() {
        let mut secrets = Secrets::new();
        secrets.add("dapi0123456789abcdef");
        let msg = "GET https://x.net failed: header Authorization: Bearer dapi0123456789abcdef";
        let out = secrets.redact(msg);
        assert!(!out.contains("dapi0123456789abcdef"));
        assert!(out.contains(MASK));
    }

    #[test]
    fn ignores_values_that_are_too_short_to_be_secrets() {
        let mut secrets = Secrets::new();
        secrets.add("abc");
        assert_eq!(secrets.redact("abc def"), "abc def");
    }

    #[test]
    fn redacts_an_unseen_sas_signature() {
        let secrets = Secrets::new();
        let msg = "error fetching https://acct.blob.core.windows.net/a/b?sig=AbC%2FdEf123&se=2026";
        let out = secrets.redact(msg);
        assert!(!out.contains("AbC%2FdEf123"));
        assert!(
            out.contains("se=2026"),
            "non-secret params should survive: {out}"
        );
    }

    #[test]
    fn redacts_a_trailing_signature_with_no_delimiter() {
        let secrets = Secrets::new();
        let out = secrets.redact("uri https://x/y?sp=r&sig=TRAILINGSECRET");
        assert!(!out.contains("TRAILINGSECRET"));
    }

    #[test]
    fn debug_impl_does_not_leak() {
        let mut secrets = Secrets::new();
        secrets.add("dapi0123456789abcdef");
        assert!(!format!("{secrets:?}").contains("dapi"));
    }
}
