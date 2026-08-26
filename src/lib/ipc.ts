// Typed wrappers over the Tauri command surface.
//
// Backend errors arrive as { kind, message }; normalizeError turns anything
// unexpected into the same shape so callers only handle one thing.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  Aggregation,
  AppErrorPayload,
  BootInfo,
  CacheStats,
  ComparisonTable,
  ConnectionInfo,
  CurveSeries,
  ExportKind,
  ExportOptions,
  LoadProgress,
  MetricInfo,
  MetricKey,
  SettingsView,
  Side,
  SideData,
  XAxis,
} from "@/types";

export function normalizeError(error: unknown): AppErrorPayload {
  if (
    typeof error === "object" &&
    error !== null &&
    "kind" in error &&
    "message" in error
  ) {
    return error as AppErrorPayload;
  }
  return { kind: "api", message: String(error) };
}

export async function bootInfo(): Promise<BootInfo> {
  return invoke("boot_info");
}

export async function connect(args: {
  host: string;
  token?: string | null;
  remember: boolean;
}): Promise<ConnectionInfo> {
  return invoke("connect", { args });
}

export async function disconnect(): Promise<void> {
  return invoke("disconnect");
}

export async function revealToken(): Promise<string> {
  return invoke("reveal_token");
}

export async function forgetCredentials(): Promise<SettingsView> {
  return invoke("forget_credentials");
}

export async function loadSide(side: Side, runRef: string): Promise<SideData> {
  return invoke("load_side", { args: { side, run_ref: runRef } });
}

export async function loadLocalSide(side: Side, path: string): Promise<SideData> {
  return invoke("load_local_side", { args: { side, path } });
}

export async function listLocalParents(path: string): Promise<string[]> {
  return invoke("list_local_parents", { path });
}

export async function cancelLoad(side: Side): Promise<void> {
  return invoke("cancel_load", { side });
}

export async function clearSide(side: Side): Promise<void> {
  return invoke("clear_side", { side });
}

export async function buildComparison(args: {
  compareFields?: string[];
  aggregation: Aggregation;
}): Promise<ComparisonTable> {
  return invoke("build_comparison", {
    args: {
      compare_fields: args.compareFields ?? null,
      aggregation: args.aggregation,
    },
  });
}

export async function buildCurves(args: {
  metric: MetricKey;
  xAxis: XAxis;
  aggregation: Aggregation;
}): Promise<CurveSeries[]> {
  return invoke("build_curves", {
    args: { metric: args.metric, x_axis: args.xAxis, aggregation: args.aggregation },
  });
}

export async function exportFile(args: {
  kind: ExportKind;
  path: string;
  compareFields?: string[];
  aggregation: Aggregation;
  options?: ExportOptions;
}): Promise<string> {
  return invoke("export", {
    args: {
      kind: args.kind,
      path: args.path,
      compare_fields: args.compareFields ?? null,
      aggregation: args.aggregation,
      options: args.options ?? null,
    },
  });
}

export async function saveSettings(patch: {
  theme?: string;
  default_metric?: string;
  compare_fields?: string[];
  control_concurrency?: number;
  blob_concurrency?: number;
  cache_cap_mb?: number;
  clear_recents?: boolean;
}): Promise<SettingsView> {
  return invoke("save_settings", { patch });
}

export async function cacheStats(): Promise<CacheStats> {
  return invoke("cache_stats");
}

export async function clearCache(): Promise<CacheStats> {
  return invoke("clear_cache");
}

export async function parseRunRef(input: string): Promise<string> {
  return invoke("parse_run_ref", { input });
}

export async function metricCatalog(): Promise<MetricInfo[]> {
  return invoke("metric_catalog");
}

export function onLoadProgress(
  handler: (progress: LoadProgress) => void,
): Promise<UnlistenFn> {
  return listen<LoadProgress>("load_progress", (event) => handler(event.payload));
}
