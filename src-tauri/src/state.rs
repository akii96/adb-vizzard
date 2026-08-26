//! Managed application state.
//!
//! Holds the session (credentials, HTTP client, chosen artifact transport), the
//! settings, the artifact cache, and whatever each side has loaded.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::cache::ArtifactCache;
use crate::client::{AdbClient, ArtifactTransport};
use crate::creds::SessionCreds;
use crate::error::{AppError, AppResult};
use crate::model::{Side, SideData};
use crate::settings::{AppSettings, SettingsStore};

pub struct Session {
    pub creds: Arc<SessionCreds>,
    pub client: Arc<AdbClient>,
    pub user: Option<String>,
    /// Resolved on first artifact fetch, then reused for the session.
    pub transport: RwLock<Option<ArtifactTransport>>,
}

pub struct AppState {
    pub http: reqwest::Client,
    pub settings_store: SettingsStore,
    pub settings: RwLock<AppSettings>,
    pub session: RwLock<Option<Arc<Session>>>,
    pub cache: RwLock<Arc<ArtifactCache>>,
    /// Loaded data per side, kept so comparisons and exports do not refetch.
    pub sides: RwLock<HashMap<&'static str, SideData>>,
    /// In-flight load per side, so a new load or an explicit cancel aborts the
    /// previous one instead of racing it.
    pub cancels: Mutex<HashMap<&'static str, CancellationToken>>,
    /// Working directory captured at startup, used to find a `.env`.
    pub cwd: PathBuf,
}

impl AppState {
    pub fn new(http: reqwest::Client, settings_store: SettingsStore, cwd: PathBuf) -> Self {
        let settings = settings_store.load();
        let cache = ArtifactCache::new(settings.cache_cap_mb);

        Self {
            http,
            settings_store,
            settings: RwLock::new(settings),
            session: RwLock::new(None),
            cache: RwLock::new(Arc::new(cache)),
            sides: RwLock::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            cwd,
        }
    }

    pub async fn require_session(&self) -> AppResult<Arc<Session>> {
        self.session
            .read()
            .await
            .clone()
            .ok_or(AppError::NotConnected)
    }

    pub async fn settings_snapshot(&self) -> AppSettings {
        self.settings.read().await.clone()
    }

    /// Persists the current settings.
    pub async fn persist_settings(&self) -> AppResult<()> {
        let settings = self.settings.read().await.clone();
        self.settings_store.save(&settings)
    }

    /// Replaces the cancellation token for a side and returns the new one,
    /// cancelling whatever was previously running there.
    pub async fn begin_load(&self, side: Side) -> CancellationToken {
        let token = CancellationToken::new();
        let mut cancels = self.cancels.lock().await;
        if let Some(previous) = cancels.insert(side.as_str(), token.clone()) {
            previous.cancel();
        }
        token
    }

    pub async fn cancel_load(&self, side: Side) {
        if let Some(token) = self.cancels.lock().await.remove(side.as_str()) {
            token.cancel();
        }
    }

    pub async fn finish_load(&self, side: Side) {
        self.cancels.lock().await.remove(side.as_str());
    }

    pub async fn store_side(&self, side: Side, data: SideData) {
        self.sides.write().await.insert(side.as_str(), data);
    }

    pub async fn take_side(&self, side: Side) -> Option<SideData> {
        self.sides.read().await.get(side.as_str()).cloned()
    }

    pub async fn clear_side(&self, side: Side) {
        self.sides.write().await.remove(side.as_str());
    }

    /// Resolves the artifact transport once per session by probing the batched
    /// credential endpoint, then caches the answer.
    pub async fn transport_for(&self, session: &Session, run_id: &str) -> ArtifactTransport {
        if let Some(existing) = *session.transport.read().await {
            return existing;
        }

        let resolved = session.client.probe_transport(run_id).await;
        *session.transport.write().await = Some(resolved);
        resolved
    }

    /// Applies a new cache ceiling in place, so a settings change does not throw
    /// away artifacts the session already fetched.
    pub async fn apply_cache_cap(&self, cap_mb: u64) {
        self.cache.read().await.set_cap_mb(cap_mb);
    }
}
