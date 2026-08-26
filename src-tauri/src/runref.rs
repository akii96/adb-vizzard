//! Extracts an MLflow run ID from whatever the user pasted.
//!
//! People copy the Databricks URL out of the browser far more often than they
//! copy the bare 32-hex ID, so both are accepted.

use std::sync::OnceLock;

use regex::Regex;

use crate::error::{AppError, AppResult};

fn runs_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)runs/([0-9a-f]{32})").unwrap())
}

fn bare_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b([0-9a-f]{32})\b").unwrap())
}

/// Returns the run ID, lowercased.
///
/// Tries the `runs/<id>` path segment first: a Databricks run URL also contains
/// an experiment ID, and matching the path avoids picking the wrong 32-hex
/// string out of a long URL.
pub fn extract_run_id(input: &str) -> AppResult<String> {
    let trimmed = input.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return Err(AppError::BadRunRef("(empty)".to_string()));
    }

    if let Some(caps) = runs_path_re().captures(trimmed) {
        if let Some(id) = caps.get(1) {
            return Ok(id.as_str().to_lowercase());
        }
    }

    if let Some(caps) = bare_id_re().captures(trimmed) {
        if let Some(id) = caps.get(1) {
            return Ok(id.as_str().to_lowercase());
        }
    }

    Err(AppError::BadRunRef(truncate_for_message(trimmed)))
}

/// Keeps a bad paste short enough to show in an inline hint.
fn truncate_for_message(input: &str) -> String {
    const MAX: usize = 60;
    if input.chars().count() <= MAX {
        return input.to_string();
    }
    let kept: String = input.chars().take(MAX).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "9fc606afefef461581f6c4d6461fcb34";

    #[test]
    fn accepts_a_bare_run_id() {
        assert_eq!(extract_run_id(ID).unwrap(), ID);
        assert_eq!(extract_run_id(&format!("  {ID}  ")).unwrap(), ID);
    }

    #[test]
    fn uppercases_are_normalized() {
        assert_eq!(extract_run_id(&ID.to_uppercase()).unwrap(), ID);
    }

    #[test]
    fn accepts_a_plain_experiments_url() {
        let url = format!("https://adb-123.4.azuredatabricks.net/ml/experiments/44/runs/{ID}");
        assert_eq!(extract_run_id(&url).unwrap(), ID);
    }

    #[test]
    fn accepts_a_hash_routed_url() {
        let url =
            format!("https://adb-123.4.azuredatabricks.net/?o=99#mlflow/experiments/44/runs/{ID}");
        assert_eq!(extract_run_id(&url).unwrap(), ID);
    }

    #[test]
    fn accepts_a_url_with_trailing_path_or_query() {
        let url = format!(
            "https://adb-1.2.azuredatabricks.net/ml/experiments/44/runs/{ID}/artifacts?x=1"
        );
        assert_eq!(extract_run_id(&url).unwrap(), ID);
    }

    #[test]
    fn prefers_the_run_id_over_another_hex_string_in_the_url() {
        // The experiment segment here is also 32 hex characters; the run path
        // must win.
        let other = "00000000000000000000000000000000";
        let url = format!("https://x/ml/experiments/{other}/runs/{ID}");
        assert_eq!(extract_run_id(&url).unwrap(), ID);
    }

    #[test]
    fn strips_surrounding_quotes_from_a_paste() {
        assert_eq!(extract_run_id(&format!("\"{ID}\"")).unwrap(), ID);
    }

    #[test]
    fn rejects_input_with_no_run_id() {
        assert!(extract_run_id("").is_err());
        assert!(extract_run_id("   ").is_err());
        assert!(extract_run_id("not-a-run-id").is_err());
        // 31 characters is not a run ID.
        assert!(extract_run_id("9fc606afefef461581f6c4d6461fcb3").is_err());
        // Non-hex characters.
        assert!(extract_run_id("9fc606afefef461581f6c4d6461fcbZZ").is_err());
    }

    #[test]
    fn error_message_truncates_a_long_paste() {
        let long = "z".repeat(500);
        let err = extract_run_id(&long).unwrap_err().to_string();
        assert!(err.len() < 120, "message should stay short: {err}");
    }
}
