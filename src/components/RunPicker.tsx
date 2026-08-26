import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  ChevronDown,
  Clock,
  FolderOpen,
  HardDriveDownload,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import {
  Badge,
  Button,
  Checkbox,
  Input,
  ProgressBar,
  Spinner,
} from "@/components/ui/primitives";
import * as ipc from "@/lib/ipc";
import { cn, formatDuration, formatRelativeTime, looksLikeRunRef } from "@/lib/utils";
import { useStore } from "@/store/session";
import type { LoadPhase, Side } from "@/types";

const PHASE_LABELS: Record<LoadPhase, string> = {
  resolving: "Resolving run…",
  listing_children: "Listing child runs…",
  fetching_artifacts: "Fetching metrics…",
  done: "Done",
};

export function RunPicker({ side }: { side: Side }) {
  const store = useStore();
  const state = store.sides[side];
  const { settings } = store;

  const [recentsOpen, setRecentsOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const recentsRef = useRef<HTMLDivElement>(null);

  const title = side === "a" ? "Left (A)" : "Right (B)";

  // Close the recents popover on an outside click.
  useEffect(() => {
    if (!recentsOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!recentsRef.current?.contains(event.target as Node)) setRecentsOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [recentsOpen]);

  // Ctrl+L / Ctrl+R focus the corresponding field.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const wanted = side === "a" ? "l" : "r";
      if (event.ctrlKey && event.key.toLowerCase() === wanted) {
        event.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [side]);

  const inputLooksValid = useMemo(
    () => state.input.trim().length === 0 || looksLikeRunRef(state.input),
    [state.input],
  );

  async function onPickFolder() {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Open a pulled exp_pull_* folder",
      });
      if (typeof selected !== "string") return;

      // A pull can hold several parents; if so, load the first and say so.
      const parents = await ipc.listLocalParents(selected);
      if (parents.length === 0) {
        store.toast("error", "No child run directories were found in that folder.");
        return;
      }
      await store.loadLocal(side, parents[0]!);
      if (parents.length > 1) {
        store.toast(
          "info",
          `That pull has ${parents.length} parent runs; loaded the first. Pick a specific parent folder to choose another.`,
        );
      }
    } catch (error) {
      store.toast("error", ipc.normalizeError(error).message);
    }
  }

  const data = state.data;
  const allIncluded = data ? state.excluded.size === 0 : false;

  return (
    <div className="flex h-full min-h-0 flex-col gap-2.5 p-3">
      <div className="flex items-center justify-between">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-muted">{title}</h2>
        {data && (
          <button
            onClick={() => void store.clear(side)}
            className="rounded p-0.5 text-muted transition-colors hover:text-fg"
            aria-label={`Clear side ${side.toUpperCase()}`}
            title="Clear"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      <div className="relative" ref={recentsRef}>
        <div className="flex gap-1.5">
          <div className="relative flex-1">
            <Input
              ref={inputRef}
              value={state.input}
              onChange={(event) => store.setInput(side, event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !state.loading) void store.load(side);
              }}
              placeholder="Run ID or Databricks URL"
              spellCheck={false}
              autoComplete="off"
              className={cn(
                "pr-8 font-mono text-xs",
                !inputLooksValid && "border-warn/60",
              )}
            />
            {(settings?.recent_runs.length ?? 0) > 0 && (
              <button
                onClick={() => setRecentsOpen((open) => !open)}
                aria-label="Recent runs"
                title="Recent runs"
                className="absolute right-1 top-1 grid h-7 w-7 place-items-center rounded-md text-muted transition-colors hover:bg-raised hover:text-fg"
              >
                <ChevronDown className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
          <Button
            variant="primary"
            onClick={() => (state.loading ? void store.cancel(side) : void store.load(side))}
            disabled={!state.loading && state.input.trim().length === 0}
          >
            {state.loading ? "Cancel" : "Load"}
          </Button>
        </div>

        {!inputLooksValid && (
          <p className="mt-1.5 flex items-start gap-1.5 text-xs text-warn">
            <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" />
            <span>Expected a 32-character run ID, or a URL containing one.</span>
          </p>
        )}

        {recentsOpen && settings && settings.recent_runs.length > 0 && (
          <div className="absolute left-0 right-0 top-full z-30 mt-1 overflow-hidden rounded-lg border border-border bg-surface shadow-xl animate-scale-in">
            <p className="flex items-center gap-1.5 border-b border-border px-2.5 py-1.5 text-[11px] font-medium uppercase tracking-wide text-muted">
              <Clock className="h-3 w-3" />
              Recent
            </p>
            <ul className="max-h-56 overflow-y-auto">
              {settings.recent_runs.map((recent) => (
                <li key={recent.run_id}>
                  <button
                    onClick={() => {
                      setRecentsOpen(false);
                      store.setInput(side, recent.run_id);
                      void store.load(side, recent.run_id);
                    }}
                    className="flex w-full items-baseline gap-2 px-2.5 py-1.5 text-left transition-colors hover:bg-raised"
                  >
                    <span className="min-w-0 flex-1 truncate text-xs text-fg">
                      {recent.run_name}
                    </span>
                    <span className="shrink-0 text-[11px] text-muted">
                      {formatRelativeTime(recent.seen_at)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>

      <button
        onClick={() => void onPickFolder()}
        className="flex items-center gap-1.5 self-start text-xs text-muted transition-colors hover:text-accent"
      >
        <FolderOpen className="h-3.5 w-3.5" />
        Open a local pull folder
      </button>

      {state.loading && (
        <div className="space-y-1.5 rounded-lg border border-border bg-raised/40 p-2.5">
          <div className="flex items-center gap-2 text-xs text-muted">
            <Spinner className="h-3.5 w-3.5 text-accent" />
            <span className="flex-1 truncate">
              {state.progress ? PHASE_LABELS[state.progress.phase] : "Starting…"}
              {state.progress?.current_run_name ? ` ${state.progress.current_run_name}` : ""}
            </span>
            {state.progress && state.progress.total > 0 && (
              <span className="tabular shrink-0">
                {state.progress.done}/{state.progress.total}
              </span>
            )}
          </div>
          <ProgressBar done={state.progress?.done ?? 0} total={state.progress?.total ?? 0} />
        </div>
      )}

      {state.error && !state.loading && (
        <div
          data-selectable
          className="rounded-lg border border-bad/30 bg-bad/10 px-2.5 py-2 text-xs leading-relaxed text-bad"
        >
          {state.error}
        </div>
      )}

      {data && (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <div className="rounded-lg border border-border bg-raised/40 p-2.5">
            <p className="truncate text-sm font-medium text-fg" title={data.run_name}>
              {data.run_name}
            </p>
            <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
              <Badge tone="accent">
                {data.children.length} {data.is_parent ? "children" : "run"}
              </Badge>
              {data.from_cache ? (
                <Badge tone="good">
                  <HardDriveDownload className="mr-1 h-3 w-3" />
                  cached
                </Badge>
              ) : (
                <Badge>{formatDuration(data.elapsed_ms)}</Badge>
              )}
              {data.failures.length > 0 && (
                <Badge tone="bad">{data.failures.length} failed</Badge>
              )}
            </div>
            {data.experiment_name && (
              <p className="mt-1.5 truncate text-xs text-muted" title={data.experiment_name}>
                {data.experiment_name}
              </p>
            )}
            {typeof data.source === "object" && (
              <p className="mt-1.5 truncate text-xs text-muted" title={data.source.local_dir.path}>
                local: {data.source.local_dir.path}
              </p>
            )}
          </div>

          <div className="flex items-center justify-between px-0.5">
            <span className="text-[11px] font-medium uppercase tracking-wide text-muted">
              Cases
            </span>
            <button
              onClick={() => store.setAllChildren(side, !allIncluded)}
              className="text-[11px] text-muted transition-colors hover:text-accent"
            >
              {allIncluded ? "none" : "all"}
            </button>
          </div>

          <ul className="min-h-0 flex-1 space-y-0.5 overflow-y-auto pr-0.5">
            {data.children.map((child) => (
              <li key={child.run_id}>
                <Checkbox
                  checked={!state.excluded.has(child.run_id)}
                  onChange={() => store.toggleChild(side, child.run_id)}
                  className="rounded px-1.5 py-1 hover:bg-raised/60"
                  label={
                    <span className="flex items-baseline gap-1.5">
                      <span className="tabular text-xs">{child.group}</span>
                      <span className="tabular text-[11px] text-muted">
                        mc{child.concurrency}
                      </span>
                    </span>
                  }
                />
              </li>
            ))}
          </ul>

          {data.failures.length > 0 && (
            <details className="rounded-lg border border-bad/25 bg-bad/5 px-2.5 py-2">
              <summary className="cursor-pointer text-xs font-medium text-bad">
                {data.failures.length} run{data.failures.length === 1 ? "" : "s"} could not be
                read
              </summary>
              <ul data-selectable className="mt-1.5 space-y-1">
                {data.failures.map((failure) => (
                  <li key={failure.run_id} className="text-[11px] leading-relaxed text-muted">
                    <span className="text-fg">{failure.run_name}</span>: {failure.reason}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}
    </div>
  );
}
