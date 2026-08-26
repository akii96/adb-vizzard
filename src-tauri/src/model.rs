//! Domain types shared between the fetch pipeline, the comparator and the UI.

use serde::Serialize;

use crate::parser::{BenchmarkMetrics, CommandMetadata};

/// Which panel a side occupies. Kept in the backend so progress events and
/// errors can be routed without the frontend correlating requests itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::A => "a",
            Side::B => "b",
        }
    }
}

/// One benchmark case: a child run reduced to a group, a concurrency and metrics.
#[derive(Debug, Clone, Serialize)]
pub struct ChildRun {
    pub run_id: String,
    pub run_name: String,
    pub status: Option<String>,
    pub start_time: Option<i64>,

    pub group: String,
    pub input_len: i64,
    pub output_len: i64,
    pub concurrency: i64,

    pub metrics: BenchmarkMetrics,

    /// Flags and env vars, for the raw-runs tab and comparison fields.
    pub metadata: CommandMetadata,

    /// Where the benchmark artifact came from, for traceability in exports.
    pub benchmark_path: String,
}

/// A child that could not be read. Surfaced rather than silently dropped, and
/// deliberately non-fatal: one malformed run should not lose the other nineteen.
#[derive(Debug, Clone, Serialize)]
pub struct ChildFailure {
    pub run_id: String,
    pub run_name: String,
    pub reason: String,
}

/// Everything loaded for one panel.
#[derive(Debug, Clone, Serialize)]
pub struct SideData {
    pub run_id: String,
    pub run_name: String,
    pub experiment_id: String,
    pub experiment_name: Option<String>,
    pub is_parent: bool,
    /// Auto-derived display label, editable in the UI before export.
    pub label: String,
    pub children: Vec<ChildRun>,
    pub failures: Vec<ChildFailure>,
    /// True when every child came from the on-disk cache, so the UI can show
    /// that the load cost no network.
    pub from_cache: bool,
    pub source: SideSource,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SideSource {
    Remote,
    LocalDir { path: String },
}

/// Progress for a running load, emitted as a throttled Tauri event.
#[derive(Debug, Clone, Serialize)]
pub struct LoadProgress {
    pub side: Side,
    pub done: usize,
    pub total: usize,
    pub current_run_name: Option<String>,
    pub phase: LoadPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadPhase {
    Resolving,
    ListingChildren,
    FetchingArtifacts,
    Done,
}

/// Derives a human label for a side from the run name plus the most
/// distinguishing config, so the export header is meaningful without the user
/// typing anything.
pub fn derive_label(run_name: &str, children: &[ChildRun]) -> String {
    let Some(first) = children.first() else {
        return run_name.to_string();
    };

    let mut parts: Vec<String> = Vec::new();

    if let Ok(tp) = first.metadata.flag("tensor_parallel_size") {
        parts.push(format!("TP{tp}"));
    }

    // The image tag is the most useful discriminator between two sweeps of the
    // same model, and it is the one thing not already in the run name.
    if let Some(image) = short_image(first.metadata.image.as_deref()) {
        parts.push(image);
    }

    if parts.is_empty() {
        run_name.to_string()
    } else {
        format!("{run_name} ({})", parts.join(", "))
    }
}

/// Shortens a container image reference for display.
///
/// Only the last path segment: the registry and org prefix are usually shared
/// across the runs being compared, and the trailing tag is what differs.
fn short_image(image: Option<&str>) -> Option<String> {
    let image = image?.trim();
    if image.is_empty() {
        return None;
    }

    let segment = image.rsplit('/').next().unwrap_or(image);
    // Guard against a pathological header; a label is not worth a wall of text.
    const MAX: usize = 60;
    if segment.chars().count() > MAX {
        return Some(segment.chars().take(MAX).collect());
    }
    Some(segment.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_commands;

    fn child_with(commands: &str) -> ChildRun {
        ChildRun {
            run_id: "r1".into(),
            run_name: "child".into(),
            status: Some("FINISHED".into()),
            start_time: None,
            group: "in1000_out100".into(),
            input_len: 1000,
            output_len: 100,
            concurrency: 16,
            metrics: BenchmarkMetrics {
                median_itl_ms: 1.0,
                median_ttft_ms: 2.0,
                median_tpot_ms: 3.0,
                median_e2el_ms: 4.0,
                output_throughput: 5.0,
                total_token_throughput: 6.0,
            },
            metadata: parse_commands(commands),
            benchmark_path: "benchmark_results/0_vllm_bench_serve/yaml".into(),
        }
    }

    #[test]
    fn label_includes_tensor_parallel_size() {
        let child = child_with("--tensor_parallel_size 4\n");
        let label = derive_label("oob_vllm_tp4_mxfp4", &[child]);
        assert!(label.contains("oob_vllm_tp4_mxfp4"));
        assert!(label.contains("TP4"), "{label}");
    }

    #[test]
    fn label_falls_back_to_the_run_name_alone() {
        assert_eq!(derive_label("solo_run", &[]), "solo_run");
        let bare = child_with("--num_prompts 16\n");
        assert_eq!(derive_label("solo_run", &[bare]), "solo_run");
    }

    #[test]
    fn side_serializes_lowercase_for_event_routing() {
        assert_eq!(serde_json::to_string(&Side::A).unwrap(), "\"a\"");
        assert_eq!(Side::B.as_str(), "b");
    }
}
