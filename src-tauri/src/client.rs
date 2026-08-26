//! Databricks MLflow REST client.
//!
//! One pooled `reqwest::Client` is shared for the whole session. With ~40 tiny
//! requests per sweep, connection and TLS reuse dominates wall time, so the
//! client is built once and cloned rather than per request.

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::creds::SessionCreds;
use crate::error::{AppError, AppResult, Secrets};

const MAX_ATTEMPTS: u32 = 3;
const RUNS_PAGE_SIZE: u32 = 100;

/// Artifact paths every child run needs, and the only two v1 downloads.
pub const COMMANDS_PATH: &str = "commands.txt";

/// Directory holding the benchmark output, one level above the harness directory.
pub const BENCHMARK_DIR: &str = "benchmark_results";

/// The benchmark file inside the harness directory. Named `yaml`, contains JSON.
pub const BENCHMARK_FILE: &str = "yaml";

/// First guess at the benchmark artifact, matching what the CLI hardcodes.
///
/// The harness directory is **not** fixed: `vllm bench serve` writes
/// `0_vllm_bench_serve` while `sglang.bench_serving` writes `0_sglang_bench_serve`,
/// and a sweep can use either. [`AdbClient::discover_benchmark_path`] resolves it
/// per run rather than assuming.
pub const BENCHMARK_PATH: &str = "benchmark_results/0_vllm_bench_serve/yaml";

pub fn benchmark_path_for(harness_dir: &str) -> String {
    format!("{BENCHMARK_DIR}/{harness_dir}/{BENCHMARK_FILE}")
}

pub fn build_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .pool_max_idle_per_host(32)
        .tcp_keepalive(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .http2_adaptive_window(true)
        .gzip(true)
        .user_agent(concat!("adb-vizzard/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::config(format!("could not build HTTP client: {e}")))
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct RunInfo {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub run_uuid: Option<String>,
    #[serde(default)]
    pub experiment_id: String,
    #[serde(default)]
    pub run_name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub start_time: Option<i64>,
    #[serde(default)]
    pub end_time: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunData {
    #[serde(default)]
    pub tags: Vec<KeyValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeyValue {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MlflowRun {
    pub info: RunInfo,
    #[serde(default)]
    pub data: RunData,
}

impl MlflowRun {
    pub fn run_id(&self) -> &str {
        if !self.info.run_id.is_empty() {
            &self.info.run_id
        } else {
            self.info.run_uuid.as_deref().unwrap_or_default()
        }
    }

    /// Prefers `info.run_name`, falling back to the `mlflow.runName` tag that
    /// older MLflow versions use, then to the run ID.
    pub fn run_name(&self) -> String {
        if let Some(name) = self.info.run_name.as_deref().filter(|n| !n.is_empty()) {
            return name.to_string();
        }
        self.tag("mlflow.runName")
            .map(str::to_string)
            .unwrap_or_else(|| self.run_id().to_string())
    }

    pub fn tag(&self, key: &str) -> Option<&str> {
        self.data
            .tags
            .iter()
            .find(|kv| kv.key == key)
            .map(|kv| kv.value.as_str())
    }

    /// Only a finished run is safe to cache forever; anything still going can
    /// gain artifacts later.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.info.status.as_deref(),
            Some("FINISHED") | Some("FAILED") | Some("KILLED")
        )
    }
}

#[derive(Debug, Deserialize)]
struct GetRunResponse {
    run: MlflowRun,
}

#[derive(Debug, Deserialize, Default)]
struct SearchRunsResponse {
    #[serde(default)]
    runs: Vec<MlflowRun>,
    #[serde(default)]
    next_page_token: Option<String>,
    /// Some deployments return `token` instead.
    #[serde(default)]
    token: Option<String>,
}

impl SearchRunsResponse {
    fn page_token(&self) -> Option<&str> {
        self.next_page_token
            .as_deref()
            .or(self.token.as_deref())
            .filter(|t| !t.is_empty())
    }
}

#[derive(Debug, Serialize)]
struct SearchRunsRequest<'a> {
    experiment_ids: Vec<&'a str>,
    filter: &'a str,
    max_results: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_token: Option<&'a str>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileInfo {
    pub path: String,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub file_size: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct ListArtifactsResponse {
    #[serde(default)]
    files: Vec<FileInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ExperimentInfo {
    #[serde(default)]
    pub experiment_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetExperimentResponse {
    experiment: ExperimentInfo,
}

#[derive(Debug, Serialize)]
struct CredentialsRequest<'a> {
    run_id: &'a str,
    path: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    page_token: Option<&'a str>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactCredential {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub signed_uri: String,
    #[serde(default)]
    pub headers: Vec<KeyValue>,
}

#[derive(Debug, Deserialize, Default)]
struct CredentialsResponse {
    #[serde(default)]
    credential_infos: Vec<ArtifactCredential>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScimMe {
    #[serde(default, rename = "userName")]
    user_name: Option<String>,
}

/// Which artifact transport this workspace supports.
///
/// Probed once per session, so the transport is a startup decision rather than a
/// per-request branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactTransport {
    /// Batched SAS vending, then bytes straight from blob storage.
    SignedUri,
    /// `artifacts/get` streamed through the control plane.
    ProxyGet,
}

pub struct AdbClient {
    http: reqwest::Client,
    creds: Arc<SessionCreds>,
    secrets: Secrets,
}

impl AdbClient {
    pub fn new(http: reqwest::Client, creds: Arc<SessionCreds>) -> Self {
        let secrets = creds.secrets();
        Self {
            http,
            creds,
            secrets,
        }
    }

    pub fn host(&self) -> &str {
        self.creds.host()
    }

    pub fn secrets(&self) -> &Secrets {
        &self.secrets
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(self.creds.url(path))
            .bearer_auth(self.creds.token())
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .post(self.creds.url(path))
            .bearer_auth(self.creds.token())
    }

    /// Validates the token and returns the caller's identity when available.
    ///
    /// SCIM is the cheap check because it needs no run ID; if workspace policy
    /// blocks it we fall back to a trivial MLflow call, which still proves the
    /// token works even though it cannot name the user.
    pub async fn verify(&self) -> AppResult<Option<String>> {
        match self
            .send_json::<ScimMe>(|| self.get("/api/2.0/preview/scim/v2/Me"))
            .await
        {
            Ok(me) => Ok(me.user_name),
            Err(AppError::Unauthorized { host }) => Err(AppError::Unauthorized { host }),
            Err(_) => {
                self.send_json::<serde_json::Value>(|| {
                    self.get("/api/2.0/mlflow/experiments/search")
                        .query(&[("max_results", "1")])
                })
                .await?;
                Ok(None)
            }
        }
    }

    pub async fn get_run(&self, run_id: &str) -> AppResult<MlflowRun> {
        let response = self
            .send_json::<GetRunResponse>(|| {
                self.get("/api/2.0/mlflow/runs/get")
                    .query(&[("run_id", run_id)])
            })
            .await
            .map_err(|err| match err {
                AppError::Api(msg) if msg.contains("RESOURCE_DOES_NOT_EXIST") => {
                    AppError::RunNotFound {
                        run_id: run_id.to_string(),
                    }
                }
                other => other,
            })?;
        Ok(response.run)
    }

    pub async fn get_experiment(&self, experiment_id: &str) -> AppResult<ExperimentInfo> {
        let response = self
            .send_json::<GetExperimentResponse>(|| {
                self.get("/api/2.0/mlflow/experiments/get")
                    .query(&[("experiment_id", experiment_id)])
            })
            .await?;
        Ok(response.experiment)
    }

    /// Lists child runs, following pagination to the end.
    pub async fn search_child_runs(
        &self,
        experiment_id: &str,
        parent_run_id: &str,
    ) -> AppResult<Vec<MlflowRun>> {
        let filter = format!(
            "tags.mlflow.parentRunId = '{}'",
            escape_filter_value(parent_run_id)
        );

        let mut runs = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let body = SearchRunsRequest {
                experiment_ids: vec![experiment_id],
                filter: &filter,
                max_results: RUNS_PAGE_SIZE,
                page_token: page_token.as_deref(),
            };

            let page = self
                .send_json::<SearchRunsResponse>(|| {
                    self.post("/api/2.0/mlflow/runs/search").json(&body)
                })
                .await?;

            let next = page.page_token().map(str::to_string);
            runs.extend(page.runs);

            match next {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }

        Ok(runs)
    }

    /// Lists one artifact directory level. Deliberately not recursive: v1 knows
    /// the two paths it wants, so a full walk is pure latency.
    pub async fn list_artifacts(
        &self,
        run_id: &str,
        path: Option<&str>,
    ) -> AppResult<Vec<FileInfo>> {
        let response = self
            .send_json::<ListArtifactsResponse>(|| {
                let mut req = self
                    .get("/api/2.0/mlflow/artifacts/list")
                    .query(&[("run_id", run_id)]);
                if let Some(path) = path {
                    req = req.query(&[("path", path)]);
                }
                req
            })
            .await?;
        Ok(response.files)
    }

    /// Resolves the benchmark artifact path for a run by listing the harness
    /// directories, instead of assuming which benchmark tool produced the run.
    ///
    /// One extra round trip per side, done once and reused across that side's
    /// children, since a sweep runs a single harness throughout.
    pub async fn discover_benchmark_path(&self, run_id: &str) -> AppResult<String> {
        let entries = self.list_artifacts(run_id, Some(BENCHMARK_DIR)).await?;

        let mut harness_dirs: Vec<String> = entries
            .into_iter()
            .filter(|entry| entry.is_dir)
            .filter_map(|entry| {
                // Entries come back as full paths like `benchmark_results/0_x`.
                entry
                    .path
                    .rsplit('/')
                    .next()
                    .map(str::to_string)
                    .filter(|name| !name.is_empty())
            })
            .collect();

        if harness_dirs.is_empty() {
            return Err(AppError::MissingArtifact {
                run_id: run_id.to_string(),
                path: format!("{BENCHMARK_DIR}/*"),
            });
        }

        // Names are numbered (`0_vllm_bench_serve`); take the lowest so a run with
        // several benchmark stages resolves to the first, matching the CLI.
        harness_dirs.sort();
        let chosen = harness_dirs.remove(0);
        Ok(benchmark_path_for(&chosen))
    }

    /// Vends signed URIs for many paths in one call, following pagination.
    ///
    /// Undocumented but is what the Databricks MLflow client uses internally.
    pub async fn credentials_for_read(
        &self,
        run_id: &str,
        paths: &[String],
    ) -> AppResult<Vec<ArtifactCredential>> {
        let mut out = Vec::with_capacity(paths.len());
        let mut page_token: Option<String> = None;

        loop {
            let body = CredentialsRequest {
                run_id,
                path: paths,
                page_token: page_token.as_deref(),
            };

            let page = self
                .send_json::<CredentialsResponse>(|| {
                    self.post("/api/2.0/mlflow/artifacts/credentials-for-read")
                        .json(&body)
                })
                .await?;

            let next = page.next_page_token.clone().filter(|t| !t.is_empty());
            out.extend(page.credential_infos);

            match next {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }

        Ok(out)
    }

    /// Fetches bytes from a signed URI. No bearer token: the signature is the
    /// credential, and attaching ours would leak it to storage.
    ///
    /// `credentials-for-read` signs a blob path without checking that it exists, so
    /// a 404 here means the artifact is genuinely absent — the usual cause being a
    /// path guessed from the wrong benchmark harness.
    pub async fn fetch_signed(
        &self,
        run_id: &str,
        credential: &ArtifactCredential,
    ) -> AppResult<Vec<u8>> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut req = self.http.get(&credential.signed_uri);
            for header in &credential.headers {
                req = req.header(&header.key, &header.value);
            }

            match req.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return response
                            .bytes()
                            .await
                            .map(|b| b.to_vec())
                            .map_err(|e| self.network_error(&e));
                    }
                    // 403 usually means the SAS expired. The caller re-vends, so
                    // surface it rather than burning retries here.
                    if status.as_u16() == 403 {
                        return Err(AppError::SasExpired);
                    }
                    if status.as_u16() == 404 {
                        return Err(AppError::MissingArtifact {
                            run_id: run_id.to_string(),
                            path: credential.path.clone(),
                        });
                    }
                    if !is_retryable(status.as_u16()) || attempt >= MAX_ATTEMPTS {
                        return Err(AppError::api(format!(
                            "artifact download failed with HTTP {}",
                            status.as_u16()
                        )));
                    }
                }
                Err(err) => {
                    if attempt >= MAX_ATTEMPTS {
                        return Err(self.network_error(&err));
                    }
                }
            }

            tokio::time::sleep(backoff_delay(attempt)).await;
        }
    }

    /// Fallback rung: stream the artifact through the control plane.
    pub async fn fetch_via_proxy(&self, run_id: &str, path: &str) -> AppResult<Vec<u8>> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let result = self
                .get("/api/2.0/mlflow/artifacts/get")
                .query(&[("run_id", run_id), ("path", path)])
                .send()
                .await;

            match result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return response
                            .bytes()
                            .await
                            .map(|b| b.to_vec())
                            .map_err(|e| self.network_error(&e));
                    }
                    if status.as_u16() == 404 {
                        return Err(AppError::MissingArtifact {
                            run_id: run_id.to_string(),
                            path: path.to_string(),
                        });
                    }
                    if status.as_u16() == 401 || status.as_u16() == 403 {
                        return Err(AppError::Unauthorized {
                            host: self.creds.host().to_string(),
                        });
                    }
                    if !is_retryable(status.as_u16()) || attempt >= MAX_ATTEMPTS {
                        return Err(AppError::api(format!(
                            "artifact download failed with HTTP {}",
                            status.as_u16()
                        )));
                    }
                }
                Err(err) => {
                    if attempt >= MAX_ATTEMPTS {
                        return Err(self.network_error(&err));
                    }
                }
            }

            tokio::time::sleep(backoff_delay(attempt)).await;
        }
    }

    /// Decides which transport to use, once per session.
    pub async fn probe_transport(&self, run_id: &str) -> ArtifactTransport {
        let paths = vec![COMMANDS_PATH.to_string()];
        match self.credentials_for_read(run_id, &paths).await {
            Ok(creds) if creds.iter().any(|c| !c.signed_uri.is_empty()) => {
                ArtifactTransport::SignedUri
            }
            _ => {
                eprintln!(
                    "adb-vizzard: credentials-for-read unavailable, falling back to the artifact proxy"
                );
                ArtifactTransport::ProxyGet
            }
        }
    }

    /// Sends a JSON request with retry on transient failures.
    ///
    /// Takes a closure because a `RequestBuilder` is consumed by `send`, so each
    /// attempt needs a fresh one.
    async fn send_json<T: serde::de::DeserializeOwned>(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> AppResult<T> {
        let mut attempt = 0;

        loop {
            attempt += 1;
            match build().send().await {
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();

                    if status.is_success() {
                        return serde_json::from_str::<T>(&body).map_err(|e| {
                            AppError::parse(format!(
                                "unexpected response from {}: {}",
                                self.creds.host(),
                                self.secrets.redact(&e.to_string())
                            ))
                        });
                    }

                    match status.as_u16() {
                        401 | 403 => {
                            return Err(AppError::Unauthorized {
                                host: self.creds.host().to_string(),
                            })
                        }
                        code if !is_retryable(code) || attempt >= MAX_ATTEMPTS => {
                            return Err(AppError::api(format!(
                                "HTTP {code} from {}: {}",
                                self.creds.host(),
                                self.secrets.redact(&summarize_body(&body))
                            )))
                        }
                        _ => {}
                    }
                }
                Err(err) => {
                    if attempt >= MAX_ATTEMPTS {
                        return Err(self.network_error(&err));
                    }
                }
            }

            tokio::time::sleep(backoff_delay(attempt)).await;
        }
    }

    fn network_error(&self, err: &reqwest::Error) -> AppError {
        AppError::Network {
            host: self.creds.host().to_string(),
            detail: self.secrets.redact(&describe_error(err)),
        }
    }
}

/// Flattens an error's source chain into one line.
///
/// `reqwest`'s own `Display` is terse — "error sending request for url (…)" —
/// which hides whether the cause was DNS, a timeout, or a rejected certificate.
/// The chain is where the diagnosable part lives, so surface all of it.
fn describe_error(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);

    while let Some(cause) = current {
        let text = cause.to_string();
        // Wrappers often restate their source verbatim; don't repeat it.
        if !parts.iter().any(|existing| existing == &text) {
            parts.push(text);
        }
        current = cause.source();
    }

    let joined = parts.join(": ");
    match trust_hint(&joined) {
        Some(hint) => format!("{joined}. {hint}"),
        None => joined,
    }
}

/// TLS interception is the most likely cause of a certificate failure on a
/// corporate network, and it is not obvious from the raw error, so name it.
fn trust_hint(detail: &str) -> Option<&'static str> {
    const NEEDLES: [&str; 6] = [
        "certificate",
        "self signed",
        "self-signed",
        "unknown issuer",
        "untrusted",
        "chain",
    ];

    let lowered = detail.to_lowercase();
    NEEDLES
        .iter()
        .any(|needle| lowered.contains(needle))
        .then_some(
            "This looks like a TLS trust problem rather than a connectivity one. \
             If your network inspects TLS, its root certificate has to be trusted by \
             Windows — check that the same URL works in Edge.",
        )
}

/// Retries only what is plausibly transient: rate limiting and server faults.
fn is_retryable(status: u16) -> bool {
    status == 429 || status == 408 || (500..600).contains(&status)
}

/// Exponential backoff with jitter, so parallel workers do not resynchronize
/// into a thundering herd after a 429.
fn backoff_delay(attempt: u32) -> Duration {
    let base_ms = 200u64 << attempt.min(4);
    let jitter = rand::thread_rng().gen_range(0..=base_ms / 2);
    Duration::from_millis(base_ms + jitter)
}

/// Keeps an error body short enough to show in a toast.
fn summarize_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty response)".to_string();
    }

    // Databricks errors are JSON with a `message`; use it when present.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
            return truncate(message, 300);
        }
        if let Some(code) = value.get("error_code").and_then(|m| m.as_str()) {
            return truncate(code, 300);
        }
    }
    truncate(trimmed, 300)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

/// Escapes a value for an MLflow filter string, matching
/// `pull_experiments.py::escape_filter_value`. Order matters: backslashes first,
/// otherwise the escapes we add get escaped again.
pub fn escape_filter_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_backslashes_before_quotes() {
        assert_eq!(escape_filter_value("plain"), "plain");
        assert_eq!(escape_filter_value("it's"), "it\\'s");
        assert_eq!(escape_filter_value(r"back\slash"), r"back\\slash");
        // A backslash followed by a quote must not double-escape.
        assert_eq!(escape_filter_value(r"a\'b"), r"a\\\'b");
    }

    #[test]
    fn classifies_retryable_statuses() {
        assert!(is_retryable(429));
        assert!(is_retryable(500));
        assert!(is_retryable(503));
        assert!(!is_retryable(400));
        assert!(!is_retryable(404));
        assert!(!is_retryable(200));
    }

    #[test]
    fn backoff_grows_and_stays_bounded() {
        let first = backoff_delay(1);
        let later = backoff_delay(4);
        assert!(first >= Duration::from_millis(400));
        assert!(later >= Duration::from_millis(3200));
        assert!(later <= Duration::from_millis(6000));
    }

    #[test]
    fn extracts_the_message_from_a_databricks_error() {
        let body = r#"{"error_code":"RESOURCE_DOES_NOT_EXIST","message":"Run 'abc' not found"}"#;
        assert_eq!(summarize_body(body), "Run 'abc' not found");
        assert_eq!(summarize_body("   "), "(empty response)");
    }

    #[test]
    fn run_name_falls_back_to_the_tag_then_the_id() {
        let json = r#"{
            "info": {"run_id": "abc123", "experiment_id": "1", "status": "FINISHED"},
            "data": {"tags": [{"key": "mlflow.runName", "value": "tagged-name"}]}
        }"#;
        let run: MlflowRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.run_name(), "tagged-name");
        assert!(run.is_terminal());

        let json = r#"{"info": {"run_id": "abc123", "experiment_id": "1"}, "data": {}}"#;
        let run: MlflowRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.run_name(), "abc123");
        assert!(!run.is_terminal());
    }

    #[derive(Debug)]
    struct Layered {
        message: String,
        cause: Option<Box<Layered>>,
    }

    impl std::fmt::Display for Layered {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl std::error::Error for Layered {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.cause
                .as_deref()
                .map(|c| c as &(dyn std::error::Error + 'static))
        }
    }

    fn layered(messages: &[&str]) -> Layered {
        let mut iter = messages.iter().rev();
        let mut current = Layered {
            message: iter.next().copied().unwrap_or_default().to_string(),
            cause: None,
        };
        for message in iter {
            current = Layered {
                message: (*message).to_string(),
                cause: Some(Box::new(current)),
            };
        }
        current
    }

    #[test]
    fn error_description_includes_the_whole_source_chain() {
        // The useful detail is always in the cause, never the top-level message.
        let err = layered(&[
            "error sending request for url (https://x.net/api)",
            "invalid peer certificate",
            "UnknownIssuer",
        ]);

        let described = describe_error(&err);
        assert!(described.contains("error sending request"));
        assert!(described.contains("invalid peer certificate"));
        assert!(described.contains("UnknownIssuer"));
    }

    #[test]
    fn error_description_does_not_repeat_a_restated_cause() {
        let err = layered(&["same text", "same text"]);
        assert_eq!(describe_error(&err), "same text");
    }

    #[test]
    fn certificate_failures_get_a_trust_hint() {
        let err = layered(&["error sending request", "invalid peer certificate"]);
        assert!(describe_error(&err).contains("TLS trust problem"));
    }

    #[test]
    fn ordinary_connectivity_failures_get_no_trust_hint() {
        let err = layered(&["error sending request", "connection timed out"]);
        let described = describe_error(&err);
        assert!(described.contains("connection timed out"));
        assert!(
            !described.contains("TLS trust problem"),
            "a timeout is not a trust problem: {described}"
        );
    }

    #[test]
    fn search_response_accepts_either_pagination_field() {
        let a: SearchRunsResponse = serde_json::from_str(r#"{"next_page_token": "t1"}"#).unwrap();
        assert_eq!(a.page_token(), Some("t1"));

        let b: SearchRunsResponse = serde_json::from_str(r#"{"token": "t2"}"#).unwrap();
        assert_eq!(b.page_token(), Some("t2"));

        let c: SearchRunsResponse = serde_json::from_str(r#"{"token": ""}"#).unwrap();
        assert_eq!(c.page_token(), None);
    }
}
