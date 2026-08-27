// Mirrors the Rust types in src-tauri/src. Kept hand-written rather than
// generated so the IPC surface stays small and reviewable.

export type Side = "a" | "b";

export type CredSource = "environment" | "dot_env" | "remembered" | "none";

export type MetricKey =
  | "median_itl_ms"
  | "median_ttft_ms"
  | "median_tpot_ms"
  | "median_e2el_ms"
  | "output_throughput"
  | "total_token_throughput";

export type Aggregation = "median" | "mean" | "max" | "min";

/**
 * Curve x axis. Only knobs that trade off against the plotted metric belong
 * here; input and output length define the workload, so they pick the case
 * instead (see `CurveCase`).
 */
export type XAxis = "concurrency" | "interactivity";

/** One workload, identified by its input and output lengths. */
export interface CurveCase {
  inputLen: number;
  outputLen: number;
}

export interface AppErrorPayload {
  kind:
    | "not_connected"
    | "unauthorized"
    | "network"
    | "api"
    | "run_not_found"
    | "bad_run_ref"
    | "missing_artifact"
    | "parse"
    | "empty_side"
    | "cancelled"
    | "io"
    | "config";
  message: string;
}

export interface BenchmarkMetrics {
  median_itl_ms: number;
  median_ttft_ms: number;
  median_tpot_ms: number;
  median_e2el_ms: number;
  /** `median_e2el_ms` was derived from TTFT and TPOT, not reported by the harness. */
  e2el_approximate: boolean;
  output_throughput: number;
  total_token_throughput: number;
}

export interface CommandMetadata {
  flags: Record<string, string[]>;
  env_vars: Record<string, string>;
}

export interface ChildRun {
  run_id: string;
  run_name: string;
  status: string | null;
  start_time: number | null;
  group: string;
  input_len: number;
  output_len: number;
  concurrency: number;
  metrics: BenchmarkMetrics;
  metadata: CommandMetadata;
  benchmark_path: string;
}

export interface ChildFailure {
  run_id: string;
  run_name: string;
  reason: string;
}

export type SideSource = "remote" | { local_dir: { path: string } };

export interface SideData {
  run_id: string;
  run_name: string;
  experiment_id: string;
  experiment_name: string | null;
  is_parent: boolean;
  label: string;
  children: ChildRun[];
  failures: ChildFailure[];
  from_cache: boolean;
  source: SideSource;
  elapsed_ms: number;
}

export type LoadPhase =
  | "resolving"
  | "listing_children"
  | "fetching_artifacts"
  | "done";

export interface LoadProgress {
  side: Side;
  done: number;
  total: number;
  current_run_name: string | null;
  phase: LoadPhase;
}

export interface CellGroup {
  metrics: BenchmarkMetrics;
  run_count: number;
  run_ids: string[];
  fields: Record<string, string>;
  benchmark_path: string;
}

export interface ComparisonRow {
  group: string;
  input_len: number;
  output_len: number;
  concurrency: number;
  a: CellGroup | null;
  b: CellGroup | null;
  ratios: Partial<Record<MetricKey, number>>;
}

export interface ComparisonTable {
  rows: ComparisonRow[];
  label_a: string;
  label_b: string | null;
  field_headers: string[];
  metric_keys: MetricKey[];
  matched_rows: number;
  a_only_rows: number;
  b_only_rows: number;
}

export interface CurvePoint {
  x: number;
  y: number;
  group: string;
  input_len: number;
  output_len: number;
  concurrency: number;
  run_count: number;
  is_pareto: boolean;
}

export interface CurveSeries {
  label: string;
  points: CurvePoint[];
}

export interface RecentRun {
  run_id: string;
  run_name: string;
  seen_at: string;
}

export interface SettingsView {
  host: string;
  has_stored_token: boolean;
  masked_token: string | null;
  remember: boolean;
  theme: "system" | "light" | "dark";
  default_metric: MetricKey;
  compare_fields: string[];
  compare_fields_enabled: boolean;
  recent_runs: RecentRun[];
  control_concurrency: number;
  blob_concurrency: number;
  cache_cap_mb: number;
  sync_root_warning: boolean;
  settings_path: string;
}

export interface BootCreds {
  host: string;
  masked_token: string | null;
  has_token: boolean;
  source: CredSource;
}

export interface ConnectionInfo {
  host: string;
  masked_token: string;
  user: string | null;
  host_in_policy: boolean;
}

export interface CacheStats {
  entries: number;
  bytes: number;
  cap_bytes: number;
}

export interface BootInfo {
  settings: SettingsView;
  boot: BootCreds;
  connection: ConnectionInfo | null;
  cache: CacheStats;
}

export interface MetricInfo {
  key: MetricKey;
  lower_is_better: boolean;
}

export type ExportKind = "comparison_csv" | "comparison_xlsx" | "raw_csv";

export interface ExportOptions {
  label_a?: string | null;
  label_b?: string | null;
  ratio_metrics?: string[];
}

/** Display names for the six metric columns. */
export const METRIC_LABELS: Record<MetricKey, string> = {
  median_itl_ms: "ITL (ms)",
  median_ttft_ms: "TTFT (ms)",
  median_tpot_ms: "TPOT (ms)",
  median_e2el_ms: "E2EL (ms)",
  output_throughput: "Output tok/s",
  total_token_throughput: "Total tok/s",
};
