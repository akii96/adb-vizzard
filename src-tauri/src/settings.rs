//! Persisted settings and the boot credential precedence.
//!
//! Storage is a plain JSON file under the app config dir. The token is written
//! only when the user opts in; see docs/DESIGN.md for the accepted risk and the
//! mitigations implemented here (opt-in default, cloud-sync refusal, explicit
//! forget).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::creds::CredSource;
use crate::error::{AppError, AppResult};

const SETTINGS_FILE: &str = "settings.json";
const MAX_RECENTS: usize = 10;

/// Folder names that indicate a sync client would replicate whatever we write.
const SYNC_ROOT_MARKERS: [&str; 4] = ["onedrive", "dropbox", "google drive", "box sync"];

fn default_metric() -> String {
    "output_throughput".to_string()
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_control_concurrency() -> usize {
    8
}

fn default_blob_concurrency() -> usize {
    24
}

fn default_cache_cap_mb() -> u64 {
    crate::cache::DEFAULT_CAP_MB
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub host: String,

    /// Present only when `remember` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,

    #[serde(default)]
    pub remember: bool,

    #[serde(default = "default_theme")]
    pub theme: String,

    #[serde(default = "default_metric")]
    pub default_metric: String,

    #[serde(default)]
    pub compare_fields: Vec<String>,

    #[serde(default)]
    pub recent_runs: Vec<RecentRun>,

    #[serde(default = "default_control_concurrency")]
    pub control_concurrency: usize,

    #[serde(default = "default_blob_concurrency")]
    pub blob_concurrency: usize,

    /// Ceiling on the in-memory artifact cache, in MB. Nothing is written to disk;
    /// see `cache.rs`.
    #[serde(default = "default_cache_cap_mb")]
    pub cache_cap_mb: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            host: String::new(),
            token: None,
            remember: false,
            theme: default_theme(),
            default_metric: default_metric(),
            compare_fields: vec![
                "tensor_parallel_size".to_string(),
                "env:VLLM_ROCM_USE_AITER".to_string(),
            ],
            recent_runs: Vec::new(),
            control_concurrency: default_control_concurrency(),
            blob_concurrency: default_blob_concurrency(),
            cache_cap_mb: default_cache_cap_mb(),
        }
    }
}

impl AppSettings {
    /// Clamps values that would otherwise let a hand-edited file hang the app or
    /// hammer the workspace.
    pub fn sanitized(mut self) -> Self {
        self.control_concurrency = self.control_concurrency.clamp(1, 32);
        self.blob_concurrency = self.blob_concurrency.clamp(1, 64);
        // Upper bound is deliberately modest: this is resident memory, not disk.
        self.cache_cap_mb = self.cache_cap_mb.clamp(8, 4_096);
        self.recent_runs.truncate(MAX_RECENTS);
        if self.theme != "light" && self.theme != "dark" {
            self.theme = default_theme();
        }
        if !self.remember {
            self.token = None;
        }
        self
    }

    pub fn push_recent(&mut self, run_id: &str, run_name: &str) {
        self.recent_runs.retain(|r| r.run_id != run_id);
        self.recent_runs.insert(
            0,
            RecentRun {
                run_id: run_id.to_string(),
                run_name: run_name.to_string(),
                seen_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        self.recent_runs.truncate(MAX_RECENTS);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentRun {
    pub run_id: String,
    pub run_name: String,
    pub seen_at: String,
}

/// Settings as the frontend sees them: the token is replaced by a preview.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub host: String,
    pub has_stored_token: bool,
    pub masked_token: Option<String>,
    pub remember: bool,
    pub theme: String,
    pub default_metric: String,
    pub compare_fields: Vec<String>,
    pub recent_runs: Vec<RecentRun>,
    pub control_concurrency: usize,
    pub blob_concurrency: usize,
    pub cache_cap_mb: u64,
    /// True when the config dir sits inside a sync root, so the UI can explain
    /// why the token was not saved.
    pub sync_root_warning: bool,
    /// Where settings actually live. Reported rather than hardcoded in the UI,
    /// since the location is derived from the bundle identifier.
    pub settings_path: String,
}

impl SettingsView {
    pub fn from_store(store: &SettingsStore, settings: &AppSettings) -> Self {
        let mut view = Self::from_settings(settings, store.in_sync_root());
        view.settings_path = store.path().display().to_string();
        view
    }

    pub fn from_settings(settings: &AppSettings, sync_root_warning: bool) -> Self {
        Self {
            host: settings.host.clone(),
            has_stored_token: settings.token.is_some(),
            masked_token: settings.token.as_deref().map(crate::creds::mask_token),
            remember: settings.remember,
            theme: settings.theme.clone(),
            default_metric: settings.default_metric.clone(),
            compare_fields: settings.compare_fields.clone(),
            recent_runs: settings.recent_runs.clone(),
            control_concurrency: settings.control_concurrency,
            blob_concurrency: settings.blob_concurrency,
            cache_cap_mb: settings.cache_cap_mb,
            sync_root_warning,
            settings_path: String::new(),
        }
    }
}

/// Credentials discovered at boot, with their origin.
#[derive(Debug, Clone, Serialize)]
pub struct BootCreds {
    pub host: String,
    pub masked_token: Option<String>,
    pub has_token: bool,
    pub source: CredSource,
}

pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Reads settings, falling back to defaults for a missing or corrupt file.
    ///
    /// A corrupt file must not brick the app, so it is reported and replaced by
    /// defaults rather than propagated.
    pub fn load(&self) -> AppSettings {
        let path = self.path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<AppSettings>(&text) {
                Ok(settings) => settings.sanitized(),
                Err(err) => {
                    eprintln!("adb-vizzard: ignoring unreadable {}: {err}", path.display());
                    AppSettings::default()
                }
            },
            Err(_) => AppSettings::default(),
        }
    }

    /// Writes settings atomically, dropping the token if the directory is synced.
    pub fn save(&self, settings: &AppSettings) -> AppResult<()> {
        std::fs::create_dir_all(&self.dir)?;

        let mut to_write = settings.clone();
        if self.in_sync_root() {
            to_write.token = None;
        }

        let json = serde_json::to_string_pretty(&to_write)
            .map_err(|e| AppError::io(format!("could not serialize settings: {e}")))?;

        // Write to a sibling temp file then rename, so a crash mid-write cannot
        // leave a truncated settings file behind.
        let path = self.path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;

        restrict_to_current_user(&path);
        Ok(())
    }

    /// True when the config directory looks like it is inside a cloud-sync root.
    pub fn in_sync_root(&self) -> bool {
        let lowered = self.dir.to_string_lossy().to_lowercase();
        SYNC_ROOT_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker))
    }

    /// Clears the stored token but keeps everything else.
    pub fn forget_token(&self) -> AppResult<AppSettings> {
        let mut settings = self.load();
        settings.token = None;
        settings.remember = false;
        self.save(&settings)?;
        Ok(settings)
    }
}

/// Restricts the settings file to the current user.
///
/// On Windows, files created under the per-user AppData tree already inherit a
/// user-only ACL, so this is a no-op there; on Unix it drops the mode to 0600,
/// which matters when developing on a shared machine.
fn restrict_to_current_user(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Resolves boot credentials by precedence: environment, then `.env`, then
/// remembered settings. Mirrors `pull_experiments.py::load_databricks_env` so an
/// existing CLI setup boots with no typing.
pub fn resolve_boot_creds(settings: &AppSettings, cwd: &Path) -> BootCreds {
    if let (Ok(host), Ok(token)) = (
        std::env::var("DATABRICKS_HOST"),
        std::env::var("DATABRICKS_TOKEN"),
    ) {
        if !host.trim().is_empty() && !token.trim().is_empty() {
            return BootCreds {
                host: host.trim().to_string(),
                masked_token: Some(crate::creds::mask_token(token.trim())),
                has_token: true,
                source: CredSource::Environment,
            };
        }
    }

    if let Some((host, token)) = read_dotenv(cwd) {
        return BootCreds {
            host,
            masked_token: Some(crate::creds::mask_token(&token)),
            has_token: true,
            source: CredSource::DotEnv,
        };
    }

    if settings.remember {
        if let Some(token) = settings.token.as_deref() {
            if !settings.host.is_empty() && !token.is_empty() {
                return BootCreds {
                    host: settings.host.clone(),
                    masked_token: Some(crate::creds::mask_token(token)),
                    has_token: true,
                    source: CredSource::Remembered,
                };
            }
        }
    }

    BootCreds {
        host: settings.host.clone(),
        masked_token: None,
        has_token: false,
        source: CredSource::None,
    }
}

/// Returns the actual secret for a resolved boot source. Kept separate from
/// [`resolve_boot_creds`] so the plaintext is only materialized at connect time.
pub fn boot_token(settings: &AppSettings, cwd: &Path, source: CredSource) -> Option<String> {
    match source {
        CredSource::Environment => std::env::var("DATABRICKS_TOKEN")
            .ok()
            .map(|t| t.trim().to_string()),
        CredSource::DotEnv => read_dotenv(cwd).map(|(_, token)| token),
        CredSource::Remembered => settings.token.clone(),
        CredSource::None => None,
    }
}

/// Parses `KEY=value` pairs from a `.env` in `cwd`, matching the CLI's quote
/// stripping. Returns both values only if both are present and non-empty.
fn read_dotenv(cwd: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(cwd.join(".env")).ok()?;
    let vars = parse_dotenv(&text);
    let host = vars.get("DATABRICKS_HOST")?.trim().to_string();
    let token = vars.get("DATABRICKS_TOKEN")?.trim().to_string();
    (!host.is_empty() && !token.is_empty()).then_some((host, token))
}

fn parse_dotenv(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('\'').trim_matches('"');
        out.insert(key.trim().to_string(), value.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotenv_with_quotes_and_comments() {
        let vars = parse_dotenv(
            "# a comment\nDATABRICKS_HOST=\"https://adb-1.2.azuredatabricks.net\"\n\nDATABRICKS_TOKEN='dapi123456789'\nJUNK\n",
        );
        assert_eq!(
            vars.get("DATABRICKS_HOST").map(String::as_str),
            Some("https://adb-1.2.azuredatabricks.net")
        );
        assert_eq!(
            vars.get("DATABRICKS_TOKEN").map(String::as_str),
            Some("dapi123456789")
        );
        assert!(!vars.contains_key("JUNK"));
    }

    #[test]
    fn sanitize_clamps_hostile_values() {
        let settings = AppSettings {
            control_concurrency: 9999,
            blob_concurrency: 0,
            cache_cap_mb: 1,
            theme: "chartreuse".into(),
            ..Default::default()
        }
        .sanitized();

        assert_eq!(settings.control_concurrency, 32);
        assert_eq!(settings.blob_concurrency, 1);
        assert_eq!(settings.cache_cap_mb, 8);
        assert_eq!(settings.theme, "system");
    }

    #[test]
    fn sanitize_drops_token_when_remember_is_off() {
        let settings = AppSettings {
            remember: false,
            token: Some("dapi123456789".into()),
            ..Default::default()
        }
        .sanitized();
        assert!(settings.token.is_none());
    }

    #[test]
    fn recents_are_deduped_newest_first_and_capped() {
        let mut settings = AppSettings::default();
        for i in 0..12 {
            settings.push_recent(&format!("run{i}"), &format!("name{i}"));
        }
        settings.push_recent("run0", "name0");

        assert_eq!(settings.recent_runs.len(), MAX_RECENTS);
        assert_eq!(settings.recent_runs[0].run_id, "run0");
        assert_eq!(
            settings
                .recent_runs
                .iter()
                .filter(|r| r.run_id == "run0")
                .count(),
            1
        );
    }

    #[test]
    fn remembered_creds_are_ignored_when_remember_is_off() {
        let settings = AppSettings {
            remember: false,
            host: "https://adb-1.2.azuredatabricks.net".into(),
            token: Some("dapi123456789".into()),
            ..Default::default()
        };
        let boot = resolve_boot_creds(&settings, Path::new("/nonexistent-dir-for-test"));
        assert!(!boot.has_token);
        assert_eq!(boot.source, CredSource::None);
    }

    #[test]
    fn detects_a_sync_root() {
        let store = SettingsStore::new(PathBuf::from(
            r"C:\Users\me\OneDrive - Corp\AppData\ADB Vizzard",
        ));
        assert!(store.in_sync_root());

        let store = SettingsStore::new(PathBuf::from(r"C:\Users\me\AppData\Roaming\ADB Vizzard"));
        assert!(!store.in_sync_root());
    }

    #[test]
    fn settings_view_never_exposes_the_token() {
        let settings = AppSettings {
            remember: true,
            token: Some("dapiabcdefghijkl3f2a".into()),
            ..Default::default()
        };
        let view = SettingsView::from_settings(&settings, false);
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("dapiabcdefghijkl3f2a"));
        assert!(view.has_stored_token);
    }
}
