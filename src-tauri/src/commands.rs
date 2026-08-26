//! Tauri command surface.
//!
//! Everything the frontend can ask for. Two rules hold throughout: the plaintext
//! token only leaves via `reveal_token`, and every error is redacted before it
//! crosses the boundary.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::cache::CacheStats;
use crate::client::AdbClient;
use crate::comparator::{self, Aggregation, ComparisonTable, CurveSeries, XAxis};
use crate::creds::{ConnectionInfo, SessionCreds};
use crate::error::{AppError, AppResult};
use crate::exporter::{self, ExportOptions};
use crate::fetcher::{LocalDirSource, ProgressSink, RemoteSource};
use crate::model::{LoadProgress, Side, SideData};
use crate::settings::{self, BootCreds, SettingsView};
use crate::state::{AppState, Session};
use crate::{parser, runref};

/// Event name for load progress.
const PROGRESS_EVENT: &str = "load_progress";

/// Minimum gap between progress emissions, in milliseconds.
///
/// Roughly 10 Hz. Emitting per file would be ~40 events in two seconds, which
/// costs more in IPC than it conveys.
const PROGRESS_THROTTLE_MS: u64 = 100;

#[derive(Debug, Serialize)]
pub struct BootInfo {
    pub settings: SettingsView,
    pub boot: BootCreds,
    pub connection: Option<ConnectionInfo>,
    pub cache: CacheStats,
}

/// What the boot screen needs, in one round trip.
#[tauri::command]
pub async fn boot_info(state: State<'_, AppState>) -> AppResult<BootInfo> {
    let settings = state.settings_snapshot().await;
    let boot = settings::resolve_boot_creds(&settings, &state.cwd);
    let connection = state
        .session
        .read()
        .await
        .as_ref()
        .map(|session| ConnectionInfo {
            host: session.creds.host().to_string(),
            masked_token: session.creds.masked_token(),
            user: session.user.clone(),
            host_in_policy: session.creds.host_in_policy(),
        });

    Ok(BootInfo {
        settings: SettingsView::from_store(&state.settings_store, &settings),
        boot,
        connection,
        cache: state.cache.read().await.stats(),
    })
}

#[derive(Debug, Deserialize)]
pub struct ConnectArgs {
    pub host: String,
    /// Omitted when reusing a token already discovered at boot, so the plaintext
    /// never has to make a round trip through the webview.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub remember: bool,
}

/// Validates credentials and opens a session.
#[tauri::command]
pub async fn connect(state: State<'_, AppState>, args: ConnectArgs) -> AppResult<ConnectionInfo> {
    let settings = state.settings_snapshot().await;

    // An absent token means "use whatever boot resolved", which keeps a
    // remembered or environment-provided secret entirely backend-side.
    let token = match args
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(token) => token.to_string(),
        None => {
            let boot = settings::resolve_boot_creds(&settings, &state.cwd);
            settings::boot_token(&settings, &state.cwd, boot.source)
                .filter(|t| !t.trim().is_empty())
                .ok_or_else(|| AppError::config("no token was provided"))?
        }
    };

    let creds = Arc::new(SessionCreds::new(&args.host, &token)?);
    let client = Arc::new(AdbClient::new(state.http.clone(), Arc::clone(&creds)));

    let user = client.verify().await?;

    let info = ConnectionInfo {
        host: creds.host().to_string(),
        masked_token: creds.masked_token(),
        user: user.clone(),
        host_in_policy: creds.host_in_policy(),
    };

    *state.session.write().await = Some(Arc::new(Session {
        creds: Arc::clone(&creds),
        client,
        user,
        transport: tokio::sync::RwLock::new(None),
    }));

    {
        let mut settings = state.settings.write().await;
        settings.host = creds.host().to_string();
        settings.remember = args.remember;
        settings.token = if args.remember { Some(token) } else { None };
    }
    state.persist_settings().await?;

    Ok(info)
}

/// Closes the session and drops the credentials.
#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> AppResult<()> {
    for side in [Side::A, Side::B] {
        state.cancel_load(side).await;
        state.clear_side(side).await;
    }
    // Dropping the Session drops the Arc<SessionCreds>, whose Drop zeroizes.
    *state.session.write().await = None;
    Ok(())
}

/// Returns the plaintext token. The only path by which it reaches the frontend,
/// and only ever in response to an explicit click.
#[tauri::command]
pub async fn reveal_token(state: State<'_, AppState>) -> AppResult<String> {
    if let Some(session) = state.session.read().await.as_ref() {
        return Ok(session.creds.token().to_string());
    }

    // Not connected yet: reveal a remembered token so the boot screen can show
    // what it pre-filled.
    let settings = state.settings_snapshot().await;
    let boot = settings::resolve_boot_creds(&settings, &state.cwd);
    settings::boot_token(&settings, &state.cwd, boot.source)
        .ok_or_else(|| AppError::config("there is no stored token to reveal"))
}

/// Clears the stored token and forgets the session.
#[tauri::command]
pub async fn forget_credentials(state: State<'_, AppState>) -> AppResult<SettingsView> {
    let updated = state.settings_store.forget_token()?;
    {
        let mut settings = state.settings.write().await;
        settings.token = None;
        settings.remember = false;
    }
    *state.session.write().await = None;

    Ok(SettingsView::from_store(&state.settings_store, &updated))
}

/// Builds a throttled progress emitter.
///
/// The terminal `Done` phase always emits, so the bar cannot be left stuck just
/// short of complete by the throttle.
fn progress_sink(app: AppHandle) -> ProgressSink {
    let last = Arc::new(AtomicU64::new(0));

    Arc::new(move |progress: LoadProgress| {
        let now = chrono::Utc::now().timestamp_millis() as u64;
        let is_terminal = matches!(progress.phase, crate::model::LoadPhase::Done);

        if !is_terminal {
            let previous = last.load(Ordering::Relaxed);
            if now.saturating_sub(previous) < PROGRESS_THROTTLE_MS {
                return;
            }
        }
        last.store(now, Ordering::Relaxed);

        let _ = app.emit(PROGRESS_EVENT, &progress);
    })
}

#[derive(Debug, Deserialize)]
pub struct LoadArgs {
    pub side: Side,
    /// A run ID or any Databricks run URL.
    pub run_ref: String,
}

/// Resolves a run and loads every child's metrics.
#[tauri::command]
pub async fn load_side(
    app: AppHandle,
    state: State<'_, AppState>,
    args: LoadArgs,
) -> AppResult<SideData> {
    let run_id = runref::extract_run_id(&args.run_ref)?;
    let session = state.require_session().await?;
    let settings = state.settings_snapshot().await;

    let transport = state.transport_for(&session, &run_id).await;
    let cache = state.cache.read().await.clone();

    let source = RemoteSource::new(
        Arc::clone(&session.client),
        cache,
        transport,
        settings.control_concurrency,
        settings.blob_concurrency,
    );

    let cancel = state.begin_load(args.side).await;
    let sink = progress_sink(app);

    let result = source.load(args.side, &run_id, &sink, &cancel).await;
    state.finish_load(args.side).await;

    let data = match result {
        Ok(data) => data,
        Err(err) => {
            // Redact defensively: a network error can embed the request URL.
            return Err(redact_error(&session.client, err));
        }
    };

    {
        let mut settings = state.settings.write().await;
        settings.push_recent(&data.run_id, &data.run_name);
    }
    let _ = state.persist_settings().await;

    state.store_side(args.side, data.clone()).await;
    Ok(data)
}

#[derive(Debug, Deserialize)]
pub struct LoadLocalArgs {
    pub side: Side,
    pub path: String,
}

/// Loads a side from an already-pulled `exp_pull_*` directory. No network, no
/// credentials.
#[tauri::command]
pub async fn load_local_side(
    app: AppHandle,
    state: State<'_, AppState>,
    args: LoadLocalArgs,
) -> AppResult<SideData> {
    let root = PathBuf::from(&args.path);
    let sink = progress_sink(app);

    // Filesystem walking is blocking work; keep it off the async runtime's
    // worker threads so the UI stays responsive on a large tree.
    let data =
        tokio::task::spawn_blocking(move || LocalDirSource::new(root).load(args.side, &sink))
            .await
            .map_err(|e| AppError::io(format!("local load failed: {e}")))??;

    state.store_side(args.side, data.clone()).await;
    Ok(data)
}

/// Lists the parent directories inside a pull, so the UI can offer a choice when
/// there is more than one.
#[tauri::command]
pub async fn list_local_parents(path: String) -> AppResult<Vec<String>> {
    let root = PathBuf::from(path);
    let dirs = tokio::task::spawn_blocking(move || LocalDirSource::list_parent_dirs(&root))
        .await
        .map_err(|e| AppError::io(format!("could not scan directory: {e}")))?;

    Ok(dirs.into_iter().map(|p| p.display().to_string()).collect())
}

#[tauri::command]
pub async fn cancel_load(state: State<'_, AppState>, side: Side) -> AppResult<()> {
    state.cancel_load(side).await;
    Ok(())
}

#[tauri::command]
pub async fn clear_side(state: State<'_, AppState>, side: Side) -> AppResult<()> {
    state.cancel_load(side).await;
    state.clear_side(side).await;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct TableArgs {
    #[serde(default)]
    pub compare_fields: Option<Vec<String>>,
    #[serde(default)]
    pub aggregation: Aggregation,
}

/// Joins whatever is loaded into the comparison table.
#[tauri::command]
pub async fn build_comparison(
    state: State<'_, AppState>,
    args: TableArgs,
) -> AppResult<ComparisonTable> {
    let a = state
        .take_side(Side::A)
        .await
        .ok_or_else(|| AppError::EmptySide("side A".to_string()))?;
    let b = state.take_side(Side::B).await;

    let compare_fields = match args.compare_fields {
        Some(fields) => fields,
        None => state.settings_snapshot().await.compare_fields,
    };

    Ok(comparator::build_table(
        &a,
        b.as_ref(),
        &compare_fields,
        args.aggregation,
    ))
}

#[derive(Debug, Deserialize)]
pub struct CurveArgs {
    pub metric: String,
    #[serde(default)]
    pub x_axis: XAxis,
    #[serde(default)]
    pub aggregation: Aggregation,
    /// Restricts the curve to one workload. Both are needed for the filter to
    /// apply; either missing means every case is plotted.
    #[serde(default)]
    pub input_len: Option<i64>,
    #[serde(default)]
    pub output_len: Option<i64>,
}

/// Builds one curve series per loaded side.
#[tauri::command]
pub async fn build_curves(
    state: State<'_, AppState>,
    args: CurveArgs,
) -> AppResult<Vec<CurveSeries>> {
    if parser::METRIC_KEYS
        .iter()
        .all(|k| *k != args.metric.as_str())
    {
        return Err(AppError::config(format!("unknown metric {}", args.metric)));
    }

    let case = args.input_len.zip(args.output_len);

    let mut series = Vec::new();
    for side in [Side::A, Side::B] {
        if let Some(data) = state.take_side(side).await {
            series.push(comparator::build_series(
                &data,
                &args.metric,
                args.x_axis,
                args.aggregation,
                case,
            ));
        }
    }

    if series.is_empty() {
        return Err(AppError::EmptySide("no runs loaded".to_string()));
    }
    Ok(series)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    ComparisonCsv,
    ComparisonXlsx,
    RawCsv,
}

#[derive(Debug, Deserialize)]
pub struct ExportArgs {
    pub kind: ExportKind,
    pub path: String,
    #[serde(default)]
    pub compare_fields: Option<Vec<String>>,
    #[serde(default)]
    pub aggregation: Aggregation,
    #[serde(default)]
    pub options: Option<ExportOptions>,
}

/// Writes an export to disk and returns the path written.
#[tauri::command]
pub async fn export(state: State<'_, AppState>, args: ExportArgs) -> AppResult<String> {
    let a = state
        .take_side(Side::A)
        .await
        .ok_or_else(|| AppError::EmptySide("side A".to_string()))?;
    let b = state.take_side(Side::B).await;

    let compare_fields = match args.compare_fields {
        Some(fields) => fields,
        None => state.settings_snapshot().await.compare_fields,
    };
    let options = args.options.unwrap_or_default();
    let path = PathBuf::from(&args.path);

    // Writers are synchronous and can touch a few MB; keep them off the runtime.
    let written = tokio::task::spawn_blocking(move || -> AppResult<String> {
        match args.kind {
            ExportKind::ComparisonCsv => {
                let table =
                    comparator::build_table(&a, b.as_ref(), &compare_fields, args.aggregation);
                exporter::write_comparison_csv(&path, &table, &options)?;
            }
            ExportKind::ComparisonXlsx => {
                let table =
                    comparator::build_table(&a, b.as_ref(), &compare_fields, args.aggregation);
                exporter::write_comparison_xlsx(&path, &table, &options)?;
            }
            ExportKind::RawCsv => {
                let mut sides: Vec<(&str, &SideData)> = vec![("A", &a)];
                if let Some(side_b) = b.as_ref() {
                    sides.push(("B", side_b));
                }
                exporter::write_raw_csv(&path, &sides)?;
            }
        }
        Ok(path.display().to_string())
    })
    .await
    .map_err(|e| AppError::io(format!("export failed: {e}")))??;

    Ok(written)
}

#[derive(Debug, Default, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub default_metric: Option<String>,
    #[serde(default)]
    pub compare_fields: Option<Vec<String>>,
    #[serde(default)]
    pub control_concurrency: Option<usize>,
    #[serde(default)]
    pub blob_concurrency: Option<usize>,
    #[serde(default)]
    pub cache_cap_mb: Option<u64>,
    #[serde(default)]
    pub clear_recents: Option<bool>,
}

/// Applies a partial settings update and persists it.
#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> AppResult<SettingsView> {
    let mut cache_cap_changed = false;

    {
        let mut settings = state.settings.write().await;
        if let Some(theme) = patch.theme {
            settings.theme = theme;
        }
        if let Some(metric) = patch.default_metric {
            settings.default_metric = metric;
        }
        if let Some(fields) = patch.compare_fields {
            settings.compare_fields = fields;
        }
        if let Some(value) = patch.control_concurrency {
            settings.control_concurrency = value;
        }
        if let Some(value) = patch.blob_concurrency {
            settings.blob_concurrency = value;
        }
        if let Some(value) = patch.cache_cap_mb {
            settings.cache_cap_mb = value;
            cache_cap_changed = true;
        }
        if patch.clear_recents.unwrap_or(false) {
            settings.recent_runs.clear();
        }

        // Re-run the clamps so a hand-typed value cannot escape the valid range.
        *settings = settings.clone().sanitized();
    }

    // Read the value back after clamping, rather than trusting the request.
    if cache_cap_changed {
        let clamped = state.settings_snapshot().await.cache_cap_mb;
        state.apply_cache_cap(clamped).await;
    }

    state.persist_settings().await?;
    let settings = state.settings_snapshot().await;
    Ok(SettingsView::from_store(&state.settings_store, &settings))
}

#[tauri::command]
pub async fn cache_stats(state: State<'_, AppState>) -> AppResult<CacheStats> {
    Ok(state.cache.read().await.stats())
}

/// Drops everything the session has fetched. The next load refetches.
#[tauri::command]
pub async fn clear_cache(state: State<'_, AppState>) -> AppResult<CacheStats> {
    let cache = state.cache.read().await.clone();
    cache.clear();
    Ok(cache.stats())
}

/// Normalizes a pasted run reference. Exposed so the input can validate as the
/// user types, using the same logic the loader will apply.
#[tauri::command]
pub fn parse_run_ref(input: String) -> AppResult<String> {
    runref::extract_run_id(&input)
}

/// Metric keys and their direction, so the UI does not hardcode a second copy.
#[derive(Debug, Serialize)]
pub struct MetricInfo {
    pub key: String,
    pub lower_is_better: bool,
}

#[tauri::command]
pub fn metric_catalog() -> Vec<MetricInfo> {
    parser::METRIC_KEYS
        .iter()
        .map(|key| MetricInfo {
            key: (*key).to_string(),
            lower_is_better: comparator::lower_is_better(key),
        })
        .collect()
}

fn redact_error(client: &AdbClient, err: AppError) -> AppError {
    match err {
        AppError::Network { host, detail } => AppError::Network {
            host,
            detail: client.secrets().redact(&detail),
        },
        AppError::Api(msg) => AppError::Api(client.secrets().redact(&msg)),
        AppError::Parse(msg) => AppError::Parse(client.secrets().redact(&msg)),
        other => other,
    }
}
