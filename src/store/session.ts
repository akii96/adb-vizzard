import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type {
  Aggregation,
  ComparisonTable,
  ConnectionInfo,
  CredSource,
  CurveCase,
  CurveSeries,
  LoadProgress,
  MetricKey,
  SettingsView,
  Side,
  SideData,
  XAxis,
} from "@/types";

export interface Toast {
  id: number;
  kind: "error" | "success" | "info";
  message: string;
}

interface SideState {
  input: string;
  data: SideData | null;
  loading: boolean;
  progress: LoadProgress | null;
  error: string | null;
  /** Child run IDs the user has unticked. */
  excluded: Set<string>;
  labelOverride: string | null;
}

const emptySide = (): SideState => ({
  input: "",
  data: null,
  loading: false,
  progress: null,
  error: null,
  excluded: new Set(),
  labelOverride: null,
});

interface Store {
  ready: boolean;
  connection: ConnectionInfo | null;
  /**
   * Dismissed the boot dialog without connecting, to work from a local
   * `exp_pull_*` folder. Local-folder mode needs no credentials, so requiring a
   * token to reach it would be backwards.
   */
  offline: boolean;
  settings: SettingsView | null;
  boot: {
    host: string;
    hasToken: boolean;
    /** Masked preview of a pre-filled token; never the plaintext. */
    masked: string | null;
    source: CredSource;
  } | null;

  sides: Record<Side, SideState>;
  table: ComparisonTable | null;
  curves: CurveSeries[] | null;

  metric: MetricKey;
  xAxis: XAxis;
  /**
   * Workload plotted on the curves tab. Different input/output lengths have
   * different trade-off curves, so only one is charted at a time. Null until a
   * side loads, at which point the first case is picked.
   */
  curveCase: CurveCase | null;
  aggregation: Aggregation;
  tableLoading: boolean;

  toasts: Toast[];

  init: () => Promise<void>;
  connect: (host: string, token: string | null, remember: boolean) => Promise<void>;
  disconnect: () => Promise<void>;
  setOffline: (offline: boolean) => void;

  setInput: (side: Side, value: string) => void;
  load: (side: Side, runRef?: string) => Promise<void>;
  loadLocal: (side: Side, path: string) => Promise<void>;
  cancel: (side: Side) => Promise<void>;
  clear: (side: Side) => Promise<void>;
  toggleChild: (side: Side, runId: string) => void;
  setAllChildren: (side: Side, included: boolean) => void;
  setLabelOverride: (side: Side, label: string | null) => void;

  setMetric: (metric: MetricKey) => void;
  setXAxis: (axis: XAxis) => void;
  setCurveCase: (curveCase: CurveCase) => void;
  setAggregation: (aggregation: Aggregation) => void;
  refresh: () => Promise<void>;

  applySettings: (settings: SettingsView) => void;
  toast: (kind: Toast["kind"], message: string) => void;
  dismissToast: (id: number) => void;
}

let toastSeq = 0;

export const useStore = create<Store>((set, get) => ({
  ready: false,
  connection: null,
  offline: false,
  settings: null,
  boot: null,

  sides: { a: emptySide(), b: emptySide() },
  table: null,
  curves: null,

  metric: "output_throughput",
  xAxis: "concurrency",
  curveCase: null,
  aggregation: "median",
  tableLoading: false,

  toasts: [],

  async init() {
    try {
      const info = await ipc.bootInfo();
      set({
        ready: true,
        connection: info.connection,
        settings: info.settings,
        boot: {
          host: info.boot.host,
          hasToken: info.boot.has_token,
          masked: info.boot.masked_token,
          source: info.boot.source,
        },
        metric: info.settings.default_metric,
      });
      applyTheme(info.settings.theme);
    } catch (error) {
      set({ ready: true });
      get().toast("error", ipc.normalizeError(error).message);
    }
  },

  async connect(host, token, remember) {
    const connection = await ipc.connect({ host, token, remember });
    set({ connection, offline: false });
    // Pick up the freshly persisted host and remember flag.
    const info = await ipc.bootInfo();
    set({ settings: info.settings });
  },

  async disconnect() {
    await ipc.disconnect();
    set({
      connection: null,
      offline: false,
      sides: { a: emptySide(), b: emptySide() },
      table: null,
      curves: null,
    });
  },

  setOffline(offline) {
    set({ offline });
  },

  setInput(side, value) {
    set((state) => ({
      sides: { ...state.sides, [side]: { ...state.sides[side], input: value, error: null } },
    }));
  },

  async load(side, runRef) {
    const reference = (runRef ?? get().sides[side].input).trim();
    if (!reference) return;

    patchSide(set, side, { loading: true, error: null, progress: null });

    try {
      const data = await ipc.loadSide(side, reference);
      patchSide(set, side, {
        data,
        loading: false,
        progress: null,
        excluded: new Set(),
        input: reference,
      });

      if (data.failures.length > 0) {
        get().toast(
          "info",
          `${data.failures.length} of ${data.children.length + data.failures.length} runs could not be read`,
        );
      }
      await get().refresh();
    } catch (error) {
      const payload = ipc.normalizeError(error);
      patchSide(set, side, { loading: false, progress: null });
      if (payload.kind !== "cancelled") {
        patchSide(set, side, { error: payload.message });
        get().toast("error", payload.message);
      }
    }
  },

  async loadLocal(side, path) {
    patchSide(set, side, { loading: true, error: null, progress: null });
    try {
      const data = await ipc.loadLocalSide(side, path);
      patchSide(set, side, {
        data,
        loading: false,
        progress: null,
        excluded: new Set(),
        input: path,
      });
      await get().refresh();
    } catch (error) {
      const payload = ipc.normalizeError(error);
      patchSide(set, side, { loading: false, progress: null, error: payload.message });
      get().toast("error", payload.message);
    }
  },

  async cancel(side) {
    await ipc.cancelLoad(side);
    patchSide(set, side, { loading: false, progress: null });
  },

  async clear(side) {
    await ipc.clearSide(side);
    set((state) => ({ sides: { ...state.sides, [side]: emptySide() } }));
    await get().refresh();
  },

  toggleChild(side, runId) {
    set((state) => {
      const excluded = new Set(state.sides[side].excluded);
      if (excluded.has(runId)) excluded.delete(runId);
      else excluded.add(runId);
      return { sides: { ...state.sides, [side]: { ...state.sides[side], excluded } } };
    });
  },

  setAllChildren(side, included) {
    set((state) => {
      const data = state.sides[side].data;
      const excluded = included
        ? new Set<string>()
        : new Set(data?.children.map((c) => c.run_id) ?? []);
      return { sides: { ...state.sides, [side]: { ...state.sides[side], excluded } } };
    });
  },

  setLabelOverride(side, label) {
    patchSide(set, side, { labelOverride: label });
  },

  setMetric(metric) {
    set({ metric });
    void ipc.saveSettings({ default_metric: metric }).catch(() => undefined);
    void get().refresh();
  },

  setXAxis(xAxis) {
    set({ xAxis });
    void get().refresh();
  },

  setCurveCase(curveCase) {
    set({ curveCase });
    void get().refresh();
  },

  setAggregation(aggregation) {
    set({ aggregation });
    void get().refresh();
  },

  /**
   * Rebuilds the table and curves from whatever is loaded.
   *
   * The join happens in Rust, so this is one round trip rather than work on the
   * main thread.
   */
  async refresh() {
    const { sides, aggregation, metric, xAxis, settings } = get();
    if (!sides.a.data) {
      set({ table: null, curves: null, curveCase: null });
      return;
    }

    // A newly loaded run may not contain the case that was selected before, so
    // fall back to its first one rather than charting nothing.
    const cases = availableCases(sides);
    const selected = get().curveCase;
    const curveCase =
      selected && cases.some((c) => sameCase(c, selected)) ? selected : (cases[0] ?? null);
    if (curveCase !== selected) set({ curveCase });

    set({ tableLoading: true });
    try {
      const [table, curves] = await Promise.all([
        ipc.buildComparison({
          compareFields: settings?.compare_fields,
          aggregation,
        }),
        ipc.buildCurves({ metric, xAxis, aggregation, case: curveCase }),
      ]);
      set({ table, curves, tableLoading: false });
    } catch (error) {
      set({ tableLoading: false });
      const payload = ipc.normalizeError(error);
      if (payload.kind !== "empty_side") {
        get().toast("error", payload.message);
      }
    }
  },

  applySettings(settings) {
    set({ settings });
    applyTheme(settings.theme);
  },

  toast(kind, message) {
    const id = ++toastSeq;
    set((state) => ({ toasts: [...state.toasts, { id, kind, message }] }));
    // Errors stay until dismissed; transient notices clear themselves.
    if (kind !== "error") {
      window.setTimeout(() => get().dismissToast(id), 4000);
    }
  },

  dismissToast(id) {
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },
}));

type SetState = (updater: (state: Store) => Partial<Store>) => void;

function patchSide(set: SetState, side: Side, patch: Partial<SideState>) {
  set((state) => ({
    sides: { ...state.sides, [side]: { ...state.sides[side], ...patch } },
  }));
}

export function applyTheme(theme: "system" | "light" | "dark") {
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const dark = theme === "dark" || (theme === "system" && prefersDark);
  document.documentElement.classList.toggle("dark", dark);
}

/** Wires the backend progress event into per-side state. */
export function subscribeToProgress() {
  return ipc.onLoadProgress((progress) => {
    useStore.setState((state) => ({
      sides: {
        ...state.sides,
        [progress.side]: { ...state.sides[progress.side], progress },
      },
    }));
  });
}

export function sameCase(one: CurveCase, other: CurveCase) {
  return one.inputLen === other.inputLen && one.outputLen === other.outputLen;
}

/**
 * The distinct workloads across both sides, in ascending input then output
 * order. The curves tab charts one of these at a time.
 */
export function availableCases(sides: Record<Side, SideState>): CurveCase[] {
  const seen = new Map<string, CurveCase>();
  for (const side of [sides.a, sides.b]) {
    for (const child of side.data?.children ?? []) {
      const key = `${child.input_len}/${child.output_len}`;
      if (!seen.has(key)) {
        seen.set(key, { inputLen: child.input_len, outputLen: child.output_len });
      }
    }
  }
  return [...seen.values()].sort(
    (one, other) => one.inputLen - other.inputLen || one.outputLen - other.outputLen,
  );
}

/** Children remaining after the user's checkbox selection. */
export function includedChildren(side: SideState) {
  if (!side.data) return [];
  return side.data.children.filter((child) => !side.excluded.has(child.run_id));
}
