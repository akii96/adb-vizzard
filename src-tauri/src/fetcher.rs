//! The fetch pipeline.
//!
//! Two sources satisfy the same shape: [`RemoteSource`] talks to Databricks,
//! [`LocalDirSource`] reads an already-pulled `exp_pull_*` tree. Everything
//! downstream is source-agnostic, which is what lets the parsers and exports be
//! validated offline against real fixtures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::cache::ArtifactCache;
use crate::client::{
    AdbClient, ArtifactTransport, BENCHMARK_DIR, BENCHMARK_FILE, BENCHMARK_PATH, COMMANDS_PATH,
};
use crate::error::{AppError, AppResult};
use crate::model::{ChildFailure, ChildRun, LoadPhase, LoadProgress, Side, SideData, SideSource};
use crate::parser::{self, CommandMetadata};

/// Reports progress upward without coupling the pipeline to Tauri.
pub type ProgressSink = Arc<dyn Fn(LoadProgress) + Send + Sync>;

/// The two artifacts v1 needs, by exact path. Asking for these directly is what
/// removes the recursive listing walk that dominates the CLI's latency.
///
/// The benchmark path is resolved per run rather than hardcoded, because the
/// harness directory differs between `vllm bench serve` and `sglang.bench_serving`.
fn required_paths(benchmark_path: &str) -> Vec<String> {
    vec![COMMANDS_PATH.to_string(), benchmark_path.to_string()]
}

struct RawChild {
    run_id: String,
    run_name: String,
    status: Option<String>,
    start_time: Option<i64>,
    commands: String,
    benchmark: String,
    benchmark_path: String,
}

/// Turns raw artifact text into a [`ChildRun`], applying the CLI's consistency
/// check between the two files.
fn build_child(raw: RawChild) -> AppResult<ChildRun> {
    let metadata: CommandMetadata = parser::parse_commands(&raw.commands);

    let metrics = parser::parse_benchmark(&raw.benchmark)?;
    let benchmark_concurrency = parser::parse_benchmark_concurrency(&raw.benchmark)?;

    // commands.txt is the primary source, matching the CLI. The sglang harness also
    // records the dimensions in the benchmark artifact, so fall back to those rather
    // than failing a run whose command extraction came out incomplete — the header
    // of every commands.txt warns that extraction is best-effort.
    let from_commands = metadata
        .flag_i64("random_input_len")
        .and_then(|input| Ok((input, metadata.flag_i64("random_output_len")?)));

    let (input_len, output_len) = match from_commands {
        Ok(dims) => dims,
        Err(commands_error) => parser::parse_benchmark_dims(&raw.benchmark)
            // Neither source has them: report the commands.txt error, since it
            // names the specific flag that was missing.
            .ok_or(commands_error)?,
    };

    let command_concurrency = metadata
        .flag_i64("max_concurrency")
        .unwrap_or(benchmark_concurrency);

    if benchmark_concurrency != command_concurrency {
        return Err(AppError::parse(format!(
            "max_concurrency mismatch: commands.txt={command_concurrency}, benchmark={benchmark_concurrency}"
        )));
    }

    Ok(ChildRun {
        run_id: raw.run_id,
        run_name: raw.run_name,
        status: raw.status,
        start_time: raw.start_time,
        group: parser::group_key(input_len, output_len),
        input_len,
        output_len,
        concurrency: benchmark_concurrency,
        metrics,
        metadata,
        benchmark_path: raw.benchmark_path,
    })
}

// ---------------------------------------------------------------------------
// Remote source
// ---------------------------------------------------------------------------

pub struct RemoteSource {
    client: Arc<AdbClient>,
    cache: Arc<ArtifactCache>,
    transport: ArtifactTransport,
    control_concurrency: usize,
    /// Global ceiling on in-flight artifact downloads. Separate from the
    /// control-plane bound so a slow workspace API cannot starve blob throughput,
    /// and a wide sweep cannot open hundreds of sockets at once.
    blob_permits: Arc<Semaphore>,
}

impl RemoteSource {
    pub fn new(
        client: Arc<AdbClient>,
        cache: Arc<ArtifactCache>,
        transport: ArtifactTransport,
        control_concurrency: usize,
        blob_concurrency: usize,
    ) -> Self {
        Self {
            client,
            cache,
            transport,
            control_concurrency: control_concurrency.max(1),
            blob_permits: Arc::new(Semaphore::new(blob_concurrency.max(1))),
        }
    }

    /// Resolves a run, lists its children, and fetches every child's metrics.
    pub async fn load(
        &self,
        side: Side,
        run_id: &str,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> AppResult<SideData> {
        let started = std::time::Instant::now();

        progress(LoadProgress {
            side,
            done: 0,
            total: 0,
            current_run_name: None,
            phase: LoadPhase::Resolving,
        });

        let parent = self.client.get_run(run_id).await?;
        let parent_id = parent.run_id().to_string();
        let experiment_id = parent.info.experiment_id.clone();

        check_cancelled(cancel)?;

        progress(LoadProgress {
            side,
            done: 0,
            total: 0,
            current_run_name: Some(parent.run_name()),
            phase: LoadPhase::ListingChildren,
        });

        let experiment_name = self
            .client
            .get_experiment(&experiment_id)
            .await
            .ok()
            .and_then(|e| e.name);

        let children = self
            .client
            .search_child_runs(&experiment_id, &parent_id)
            .await?;

        check_cancelled(cancel)?;

        // A leaf run is its own single child, matching the CLI's fallback.
        let is_parent = !children.is_empty();
        let targets = if is_parent {
            children
        } else {
            vec![parent.clone()]
        };
        let total = targets.len();

        let mut sorted = targets;
        sorted.sort_by(|a, b| {
            a.run_name()
                .to_lowercase()
                .cmp(&b.run_name().to_lowercase())
                .then_with(|| a.run_id().cmp(b.run_id()))
        });

        // Resolve which benchmark harness this sweep used, once, from the first
        // child. Hardcoding `0_vllm_bench_serve` makes every sglang sweep 404.
        let benchmark_path = match sorted.first() {
            Some(first) => self
                .client
                .discover_benchmark_path(first.run_id())
                .await
                .unwrap_or_else(|_| BENCHMARK_PATH.to_string()),
            None => BENCHMARK_PATH.to_string(),
        };

        check_cancelled(cancel)?;

        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        // Fan out across children. Each task does one batched credential call
        // and two small GETs, so the bound is on the control plane.
        let results = stream::iter(sorted.into_iter().map(|run| {
            let done = Arc::clone(&done);
            let progress = Arc::clone(progress);
            let benchmark_path = benchmark_path.clone();
            async move {
                if cancel.is_cancelled() {
                    return (
                        run.run_id().to_string(),
                        run.run_name(),
                        Err(AppError::Cancelled),
                    );
                }

                let outcome = self.fetch_child(&run, &benchmark_path).await;

                let completed = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress(LoadProgress {
                    side,
                    done: completed,
                    total,
                    current_run_name: Some(run.run_name()),
                    phase: LoadPhase::FetchingArtifacts,
                });

                (run.run_id().to_string(), run.run_name(), outcome)
            }
        }))
        .buffer_unordered(self.control_concurrency.max(1))
        .collect::<Vec<_>>()
        .await;

        check_cancelled(cancel)?;

        let mut kids = Vec::new();
        let mut failures = Vec::new();
        let mut all_cached = true;

        for (run_id, run_name, outcome) in results {
            match outcome {
                Ok((child, from_cache)) => {
                    all_cached &= from_cache;
                    kids.push(child);
                }
                Err(err) => failures.push(ChildFailure {
                    run_id,
                    run_name,
                    reason: self.client.secrets().redact(&err.to_string()),
                }),
            }
        }

        if kids.is_empty() {
            let detail = failures
                .first()
                .map(|f| format!("{} ({})", parent.run_name(), f.reason))
                .unwrap_or_else(|| parent.run_name());
            return Err(AppError::EmptySide(detail));
        }

        sort_children(&mut kids);

        progress(LoadProgress {
            side,
            done: total,
            total,
            current_run_name: None,
            phase: LoadPhase::Done,
        });

        let label = crate::model::derive_label(&parent.run_name(), &kids);

        Ok(SideData {
            run_id: parent_id,
            run_name: parent.run_name(),
            experiment_id,
            experiment_name,
            is_parent,
            label,
            children: kids,
            failures,
            from_cache: all_cached,
            source: SideSource::Remote,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Fetches and parses one child, cache first.
    async fn fetch_child(
        &self,
        run: &crate::client::MlflowRun,
        benchmark_path: &str,
    ) -> AppResult<(ChildRun, bool)> {
        let run_id = run.run_id().to_string();
        // Only a terminal run is safe to serve from, or write to, the cache.
        let cacheable = run.is_terminal();

        let mut texts: Vec<Option<String>> = vec![None, None];
        let paths = required_paths(benchmark_path);
        let mut all_cached = true;

        if cacheable {
            for (idx, path) in paths.iter().enumerate() {
                if let Some(bytes) = self.cache.get(&run_id, path) {
                    texts[idx] = Some(decode_utf8(bytes));
                }
            }
        }

        let missing: Vec<String> = paths
            .iter()
            .enumerate()
            .filter(|(idx, _)| texts[*idx].is_none())
            .map(|(_, path)| path.clone())
            .collect();

        // Resolved path for this child, which may differ from the side-wide guess.
        let mut resolved_benchmark = benchmark_path.to_string();

        if !missing.is_empty() {
            all_cached = false;
            let fetched = match self.fetch_paths(&run_id, &missing).await {
                Ok(fetched) => fetched,
                // The side-wide harness guess does not fit this child. Re-resolve
                // for it specifically and try once more, so one odd child in a
                // mixed sweep does not fail the whole side.
                Err(AppError::MissingArtifact { .. }) => {
                    let rediscovered = self.client.discover_benchmark_path(&run_id).await?;
                    let retry: Vec<String> = missing
                        .iter()
                        .map(|path| {
                            if path == benchmark_path {
                                rediscovered.clone()
                            } else {
                                path.clone()
                            }
                        })
                        .collect();
                    resolved_benchmark = rediscovered;
                    self.fetch_paths(&run_id, &retry).await?
                }
                Err(other) => return Err(other),
            };

            for (path, bytes) in fetched {
                if cacheable {
                    self.cache.put(&run_id, &path, &bytes);
                }
                let slot = if path == COMMANDS_PATH { 0 } else { 1 };
                texts[slot] = Some(decode_utf8(bytes));
            }
        }

        let commands = texts[0].take().ok_or_else(|| AppError::MissingArtifact {
            run_id: run_id.clone(),
            path: COMMANDS_PATH.to_string(),
        })?;
        let benchmark = texts[1].take().ok_or_else(|| AppError::MissingArtifact {
            run_id: run_id.clone(),
            path: resolved_benchmark.clone(),
        })?;

        let child = build_child(RawChild {
            run_id,
            run_name: run.run_name(),
            status: run.info.status.clone(),
            start_time: run.info.start_time,
            commands,
            benchmark,
            benchmark_path: resolved_benchmark,
        })?;

        Ok((child, all_cached))
    }

    /// Fetches a set of paths for one run using the active transport.
    async fn fetch_paths(
        &self,
        run_id: &str,
        paths: &[String],
    ) -> AppResult<Vec<(String, Vec<u8>)>> {
        match self.transport {
            ArtifactTransport::ProxyGet => self.fetch_via_proxy(run_id, paths).await,
            ArtifactTransport::SignedUri => match self.fetch_via_signed(run_id, paths).await {
                Ok(out) => Ok(out),
                // A vended URI can expire between the batch call and the GET. One
                // re-vend covers that; a second failure is a real error.
                Err(AppError::SasExpired) => self.fetch_via_signed(run_id, paths).await,
                Err(other) => Err(other),
            },
        }
    }

    async fn fetch_via_signed(
        &self,
        run_id: &str,
        paths: &[String],
    ) -> AppResult<Vec<(String, Vec<u8>)>> {
        let credentials = self.client.credentials_for_read(run_id, paths).await?;

        // Match every path up front, so a missing credential fails before we spend
        // requests on the paths that are present.
        let mut pairs = Vec::with_capacity(paths.len());
        for path in paths {
            let credential = credentials
                .iter()
                .find(|c| c.path == *path)
                .ok_or_else(|| AppError::MissingArtifact {
                    run_id: run_id.to_string(),
                    path: path.clone(),
                })?;
            pairs.push((path.clone(), credential.clone()));
        }

        let fetches = pairs.into_iter().map(|(path, credential)| async move {
            let _permit = self.acquire_blob_permit().await?;
            let bytes = self.client.fetch_signed(run_id, &credential).await?;
            Ok::<(String, Vec<u8>), AppError>((path, bytes))
        });

        futures::future::try_join_all(fetches).await
    }

    async fn fetch_via_proxy(
        &self,
        run_id: &str,
        paths: &[String],
    ) -> AppResult<Vec<(String, Vec<u8>)>> {
        let fetches = paths.iter().map(|path| async move {
            let _permit = self.acquire_blob_permit().await?;
            let bytes = self.client.fetch_via_proxy(run_id, path).await?;
            Ok::<(String, Vec<u8>), AppError>((path.clone(), bytes))
        });

        futures::future::try_join_all(fetches).await
    }

    async fn acquire_blob_permit(&self) -> AppResult<tokio::sync::SemaphorePermit<'_>> {
        self.blob_permits
            .acquire()
            .await
            .map_err(|_| AppError::api("the download scheduler shut down mid-fetch"))
    }
}

// ---------------------------------------------------------------------------
// Local directory source
// ---------------------------------------------------------------------------

pub struct LocalDirSource {
    root: PathBuf,
}

impl LocalDirSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Loads one parent directory out of an `exp_pull_*` tree.
    ///
    /// Accepts being pointed at either the pull root or a single parent
    /// directory, because both are things a user will reasonably pick.
    pub fn load(&self, side: Side, progress: &ProgressSink) -> AppResult<SideData> {
        let started = std::time::Instant::now();

        progress(LoadProgress {
            side,
            done: 0,
            total: 0,
            current_run_name: None,
            phase: LoadPhase::Resolving,
        });

        let parent_dir = self.resolve_parent_dir()?;
        let child_dirs = child_dirs_of(&parent_dir);
        if child_dirs.is_empty() {
            return Err(AppError::EmptySide(parent_dir.display().to_string()));
        }

        let total = child_dirs.len();
        let mut kids = Vec::new();
        let mut failures = Vec::new();

        for (idx, child_dir) in child_dirs.iter().enumerate() {
            let name = dir_label(child_dir);

            progress(LoadProgress {
                side,
                done: idx,
                total,
                current_run_name: Some(name.clone()),
                phase: LoadPhase::FetchingArtifacts,
            });

            match self.read_child(child_dir, &name) {
                Ok(child) => kids.push(child),
                Err(err) => failures.push(ChildFailure {
                    run_id: name.clone(),
                    run_name: name,
                    reason: err.to_string(),
                }),
            }
        }

        if kids.is_empty() {
            let detail = failures
                .first()
                .map(|f| format!("{} ({})", parent_dir.display(), f.reason))
                .unwrap_or_else(|| parent_dir.display().to_string());
            return Err(AppError::EmptySide(detail));
        }

        sort_children(&mut kids);

        progress(LoadProgress {
            side,
            done: total,
            total,
            current_run_name: None,
            phase: LoadPhase::Done,
        });

        let run_name = dir_label(&parent_dir);
        let label = crate::model::derive_label(&run_name, &kids);

        Ok(SideData {
            run_id: format!("local:{}", parent_dir.display()),
            run_name,
            experiment_id: String::new(),
            experiment_name: None,
            is_parent: kids.len() > 1,
            label,
            children: kids,
            failures,
            from_cache: true,
            source: SideSource::LocalDir {
                path: parent_dir.display().to_string(),
            },
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    fn read_child(&self, child_dir: &Path, name: &str) -> AppResult<ChildRun> {
        let commands_path = child_dir.join(COMMANDS_PATH);
        let benchmark_path = find_benchmark_file(child_dir).ok_or_else(|| {
            AppError::parse(format!(
                "no {BENCHMARK_DIR}/*/{BENCHMARK_FILE} found in {}",
                child_dir.display()
            ))
        })?;

        let commands = std::fs::read_to_string(&commands_path).map_err(|_| {
            AppError::parse(format!(
                "missing {COMMANDS_PATH} in {}",
                child_dir.display()
            ))
        })?;
        let benchmark = std::fs::read_to_string(&benchmark_path)
            .map_err(|_| AppError::parse(format!("could not read {}", benchmark_path.display())))?;

        let relative = benchmark_path
            .strip_prefix(&self.root)
            .unwrap_or(&benchmark_path)
            .display()
            .to_string();

        build_child(RawChild {
            run_id: name.to_string(),
            run_name: name.to_string(),
            status: Some("FINISHED".to_string()),
            start_time: None,
            commands,
            benchmark,
            benchmark_path: relative,
        })
    }

    /// Finds the directory whose children we should read.
    ///
    /// `adb-pull` names these `parent_<slug>_<hex>`, but real pulls get renamed
    /// by hand, so detection is by structure (does it contain `child_*`?) rather
    /// than by prefix.
    fn resolve_parent_dir(&self) -> AppResult<PathBuf> {
        if !self.root.is_dir() {
            return Err(AppError::io(format!(
                "{} is not a directory",
                self.root.display()
            )));
        }

        if !child_dirs_of(&self.root).is_empty() {
            return Ok(self.root.clone());
        }

        let mut candidates: Vec<PathBuf> = read_dirs(&self.root)
            .into_iter()
            .filter(|dir| !child_dirs_of(dir).is_empty())
            .collect();
        candidates.sort();

        candidates.into_iter().next().ok_or_else(|| {
            AppError::io(format!(
                "no child run directories found under {}",
                self.root.display()
            ))
        })
    }

    /// Lists every parent directory in the tree, so the UI can offer a choice
    /// when a pull contains more than one.
    pub fn list_parent_dirs(root: &Path) -> Vec<PathBuf> {
        if !root.is_dir() {
            return Vec::new();
        }
        if !child_dirs_of(root).is_empty() {
            return vec![root.to_path_buf()];
        }
        let mut dirs: Vec<PathBuf> = read_dirs(root)
            .into_iter()
            .filter(|dir| !child_dirs_of(dir).is_empty())
            .collect();
        dirs.sort();
        dirs
    }
}

fn read_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

/// Locates the benchmark artifact inside a pulled child directory.
///
/// The harness subdirectory is not fixed — `0_vllm_bench_serve` for one benchmark
/// tool, `0_sglang_bench_serve` for another — so scan for it rather than assuming.
/// Lowest-numbered wins, matching the remote resolution.
fn find_benchmark_file(child_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = read_dirs(&child_dir.join(BENCHMARK_DIR))
        .into_iter()
        .map(|dir| dir.join(BENCHMARK_FILE))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Child directories are the ones `adb-pull` prefixes with `child_`. That prefix
/// is stable in every pull, unlike the parent naming.
fn child_dirs_of(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = read_dirs(dir)
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("child_"))
        })
        .collect();
    dirs.sort();
    dirs
}

/// Strips `adb-pull`'s `child_`/`parent_` prefix and trailing short hash for
/// display, matching how the CLI derives a slug.
fn dir_label(dir: &Path) -> String {
    let raw = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed")
        .to_string();

    let stripped = raw
        .strip_prefix("child_")
        .or_else(|| raw.strip_prefix("parent_"))
        .unwrap_or(&raw);

    match stripped.rsplit_once('_') {
        Some((head, tail))
            if tail.len() == 8
                && tail.chars().all(|c| c.is_ascii_hexdigit())
                && !head.is_empty() =>
        {
            head.to_string()
        }
        _ => stripped.to_string(),
    }
}

/// Sort order matches the CLI and the example CSV.
fn sort_children(children: &mut [ChildRun]) {
    children.sort_by(|a, b| {
        a.input_len
            .cmp(&b.input_len)
            .then(a.output_len.cmp(&b.output_len))
            .then(a.concurrency.cmp(&b.concurrency))
            .then_with(|| a.run_name.cmp(&b.run_name))
    });
}

fn check_cancelled(cancel: &CancellationToken) -> AppResult<()> {
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    Ok(())
}

/// Artifacts are text, but a truncated download can split a UTF-8 sequence;
/// lossy decoding turns that into a parse error rather than a panic.
fn decode_utf8(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_pull_prefixes_and_short_hashes() {
        assert_eq!(
            dir_label(Path::new("/x/child_unleashed-stork-87_ccf846e0")),
            "unleashed-stork-87"
        );
        assert_eq!(dir_label(Path::new("/x/parent_my-run_1a2b3c4d")), "my-run");
        // A real pull that was renamed by hand keeps its name untouched.
        assert_eq!(
            dir_label(Path::new("/x/AMD-GPT-TP1-vllm-private-rocm721-03252026")),
            "AMD-GPT-TP1-vllm-private-rocm721-03252026"
        );
        // A trailing segment that is not an 8-char hex hash is preserved.
        assert_eq!(dir_label(Path::new("/x/child_run_v2")), "run_v2");
    }

    #[test]
    fn sorts_by_input_output_then_concurrency() {
        use crate::parser::{parse_commands, BenchmarkMetrics};

        let make = |input: i64, output: i64, concurrency: i64| ChildRun {
            run_id: format!("r{input}{output}{concurrency}"),
            run_name: "c".into(),
            status: None,
            start_time: None,
            group: parser::group_key(input, output),
            input_len: input,
            output_len: output,
            concurrency,
            metrics: BenchmarkMetrics {
                median_itl_ms: 0.0,
                median_ttft_ms: 0.0,
                median_tpot_ms: 0.0,
                median_e2el_ms: 0.0,
                output_throughput: 0.0,
                total_token_throughput: 0.0,
            },
            metadata: parse_commands(""),
            benchmark_path: String::new(),
        };

        let mut children = vec![
            make(1000, 100, 8),
            make(500, 100, 4),
            make(1000, 100, 4),
            make(1000, 50, 16),
        ];
        sort_children(&mut children);

        let order: Vec<(i64, i64, i64)> = children
            .iter()
            .map(|c| (c.input_len, c.output_len, c.concurrency))
            .collect();
        assert_eq!(
            order,
            vec![
                (500, 100, 4),
                (1000, 50, 16),
                (1000, 100, 4),
                (1000, 100, 8)
            ]
        );
    }

    #[test]
    fn decodes_invalid_utf8_without_panicking() {
        let text = decode_utf8(vec![b'o', b'k', 0xff]);
        assert!(text.starts_with("ok"));
    }
}
