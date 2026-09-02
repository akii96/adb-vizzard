import { Columns3 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button, Checkbox } from "@/components/ui/primitives";
import { ALL_METRICS, useStore } from "@/store/session";
import { METRIC_LABELS, type MetricKey } from "@/types";

/**
 * Chooses which of the six metrics the table shows.
 *
 * A selection applies everywhere at once: the metric's column under A, its
 * column under B, and its A/B ratio column. Narrowing to two or three is what
 * makes the table small enough to paste into a pull request.
 */
export function MetricPicker() {
  const { visibleMetrics, setVisibleMetrics } = useStore();
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  const isLastChecked = (key: MetricKey) =>
    visibleMetrics.length === 1 && visibleMetrics[0] === key;

  function toggle(key: MetricKey) {
    setVisibleMetrics(
      visibleMetrics.includes(key)
        ? visibleMetrics.filter((entry) => entry !== key)
        : [...visibleMetrics, key],
    );
  }

  return (
    <div className="relative" ref={menuRef}>
      <Button size="sm" onClick={() => setOpen((value) => !value)} title="Choose table columns">
        <Columns3 className="h-3.5 w-3.5" />
        Metrics {visibleMetrics.length}/{ALL_METRICS.length}
      </Button>

      {open && (
        <div className="absolute right-0 top-full z-40 mt-1.5 w-64 overflow-hidden rounded-lg border border-border bg-surface shadow-xl animate-scale-in">
          <div className="flex items-center gap-2 border-b border-border px-2.5 py-2">
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted">
              Table columns
            </p>
            <button
              onClick={() => setVisibleMetrics(ALL_METRICS)}
              disabled={visibleMetrics.length === ALL_METRICS.length}
              className="ml-auto text-[11px] text-accent transition-opacity hover:underline disabled:opacity-40 disabled:no-underline"
            >
              select all
            </button>
          </div>

          <ul className="p-1">
            {ALL_METRICS.map((key) => {
              const checked = visibleMetrics.includes(key);
              return (
                <li key={key} className="group/row flex items-center gap-2 rounded-md px-1.5 py-1 hover:bg-raised">
                  <Checkbox
                    checked={checked}
                    // Zero columns would leave a table of nothing but case keys.
                    disabled={isLastChecked(key)}
                    onChange={() => toggle(key)}
                    label={<span className="text-xs">{METRIC_LABELS[key]}</span>}
                    className="min-w-0 flex-1 items-center"
                  />
                  <button
                    onClick={() => setVisibleMetrics([key])}
                    className="shrink-0 text-[11px] text-muted opacity-0 transition-opacity hover:text-accent hover:underline group-hover/row:opacity-100"
                  >
                    only
                  </button>
                </li>
              );
            })}
          </ul>

          <p className="border-t border-border px-2.5 py-2 text-[11px] leading-relaxed text-muted">
            Each selected metric gets a column under A, under B, and an A/B ratio
            column.
          </p>
        </div>
      )}
    </div>
  );
}
