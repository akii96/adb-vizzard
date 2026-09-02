import { ChevronRight } from "lucide-react";
import { useState } from "react";

import { Badge, EmptyState } from "@/components/ui/primitives";
import { cn, formatMetric } from "@/lib/utils";
import { useStore } from "@/store/session";
import { METRIC_LABELS, type ChildRun, type MetricKey, type SideData } from "@/types";

/** Expandable tree of every loaded child run, for sanity-checking a load. */
export function RawRuns() {
  const { sides } = useStore();
  const loaded = [
    ["A", sides.a] as const,
    ["B", sides.b] as const,
  ].filter(
    (entry): entry is readonly ["A" | "B", (typeof sides)["a"] & { data: SideData }] =>
      entry[1].data !== null,
  );

  if (loaded.length === 0) {
    return <EmptyState title="Nothing loaded yet" />;
  }

  return (
    <div className="h-full overflow-y-auto p-3">
      {loaded.map(([name, side]) => {
        const { data, excluded } = side;
        const included = data.children.length - excluded.size;

        return (
          <section key={name} className="mb-4 last:mb-0">
            <div className="mb-1.5 flex items-center gap-2">
              <Badge tone={name === "A" ? "accent" : "neutral"}>{name}</Badge>
              <span className="truncate text-sm font-medium">{data.label}</span>
              <span className="shrink-0 text-xs text-muted">
                {excluded.size > 0
                  ? `${included} of ${data.children.length} runs`
                  : `${data.children.length} runs`}
              </span>
            </div>
            <ul className="space-y-1">
              {data.children.map((child) => (
                <RunNode
                  key={child.run_id}
                  child={child}
                  excluded={excluded.has(child.run_id)}
                />
              ))}
            </ul>
          </section>
        );
      })}
    </div>
  );
}

function RunNode({ child, excluded }: { child: ChildRun; excluded: boolean }) {
  const [open, setOpen] = useState(false);

  const flags = Object.entries(child.metadata.flags);
  const envVars = Object.entries(child.metadata.env_vars);

  return (
    // Unticked runs stay listed rather than disappearing, since this tab is the
    // inventory of what was loaded, but they are dimmed to show they contribute
    // to nothing downstream.
    <li className={cn("overflow-hidden rounded-lg border border-border", excluded && "opacity-40")}>
      <button
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 bg-raised/40 px-2.5 py-1.5 text-left transition-colors hover:bg-raised"
      >
        <ChevronRight
          className={cn("h-3.5 w-3.5 shrink-0 text-muted transition-transform", open && "rotate-90")}
        />
        <span className={cn("tabular shrink-0 text-xs font-medium", excluded && "line-through")}>
          {child.group}
        </span>
        <span className="tabular shrink-0 text-[11px] text-muted">mc{child.concurrency}</span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-muted" title={child.run_name}>
          {child.run_name}
        </span>
        {excluded && <span className="shrink-0 text-[11px] text-muted">excluded</span>}
      </button>

      {open && (
        <div data-selectable className="space-y-3 border-t border-border px-2.5 py-2">
          <div>
            <p className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted">
              Metrics
            </p>
            <div className="grid grid-cols-2 gap-x-4 gap-y-0.5 sm:grid-cols-3">
              {(Object.keys(METRIC_LABELS) as MetricKey[]).map((key) => (
                <div key={key} className="flex items-baseline justify-between gap-2">
                  <span className="truncate text-[11px] text-muted">{METRIC_LABELS[key]}</span>
                  <span className="tabular text-[11px]">{formatMetric(child.metrics[key])}</span>
                </div>
              ))}
            </div>
          </div>

          {envVars.length > 0 && (
            <div>
              <p className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted">
                Env vars
              </p>
              <div className="grid gap-x-4 gap-y-0.5 sm:grid-cols-2">
                {envVars.map(([name, value]) => (
                  <div key={name} className="flex items-baseline justify-between gap-2">
                    <span className="truncate font-mono text-[11px] text-muted" title={name}>
                      {name}
                    </span>
                    <span className="shrink-0 font-mono text-[11px]">{value}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {flags.length > 0 && (
            <details>
              <summary className="cursor-pointer text-[11px] font-semibold uppercase tracking-wide text-muted">
                Flags ({flags.length})
              </summary>
              <div className="mt-1 grid gap-x-4 gap-y-0.5 sm:grid-cols-2">
                {flags.map(([name, values]) => (
                  <div key={name} className="flex items-baseline justify-between gap-2">
                    <span className="truncate font-mono text-[11px] text-muted" title={name}>
                      {name}
                    </span>
                    <span className="shrink-0 truncate font-mono text-[11px]">
                      {values.join(" | ")}
                    </span>
                  </div>
                ))}
              </div>
            </details>
          )}

          <p className="truncate font-mono text-[10px] text-muted" title={child.benchmark_path}>
            {child.benchmark_path}
          </p>
        </div>
      )}
    </li>
  );
}
