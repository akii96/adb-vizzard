import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef } from "react";

import { Badge, EmptyState } from "@/components/ui/primitives";
import { cn, formatMetric, formatPercent } from "@/lib/utils";
import { taggedLabels, useStore } from "@/store/session";
import {
  METRIC_LABELS,
  type CellGroup,
  type ComparisonRow,
  type ComparisonTable as Table,
  type MetricKey,
} from "@/types";

const ROW_HEIGHT = 30;
/** Extra rows rendered outside the viewport, to avoid blank flashes on scroll. */
const OVERSCAN = 12;

/** Lower is better for the latency metrics; used to colour ratio cells. */
const LOWER_IS_BETTER: ReadonlySet<string> = new Set([
  "median_itl_ms",
  "median_ttft_ms",
  "median_tpot_ms",
  "median_e2el_ms",
]);

export function ComparisonTable({ table }: { table: Table }) {
  const { metric, sides } = useStore();
  const scrollRef = useRef<HTMLDivElement>(null);
  const labels = useMemo(() => taggedLabels(sides), [sides]);

  const hasB = table.label_b !== null;
  const fieldCount = table.field_headers.length;
  const metricCount = table.metric_keys.length;

  // Only the excluded set changes often, so keep the filter memoized on it.
  const rows = useMemo(() => {
    const excludedA = sides.a.excluded;
    const excludedB = sides.b.excluded;
    if (excludedA.size === 0 && excludedB.size === 0) return table.rows;

    // A row survives if at least one of its sides still has an included run.
    return table.rows.filter((row) => {
      const aLives = row.a ? row.a.run_ids.some((id) => !excludedA.has(id)) : false;
      const bLives = row.b ? row.b.run_ids.some((id) => !excludedB.has(id)) : false;
      return aLives || bLives;
    });
  }, [table.rows, sides.a.excluded, sides.b.excluded]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: OVERSCAN,
  });

  if (rows.length === 0) {
    return <EmptyState title="No rows to compare" hint="Every case was unticked." />;
  }

  const totalColumns = 3 + fieldCount + metricCount + (hasB ? fieldCount + metricCount + 1 : 0);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1.5">
        <Badge tone="good">{table.matched_rows} matched</Badge>
        {table.a_only_rows > 0 && <Badge tone="warn">{table.a_only_rows} A only</Badge>}
        {table.b_only_rows > 0 && <Badge tone="warn">{table.b_only_rows} B only</Badge>}
        <span className="ml-auto text-[11px] text-muted">
          ratio column shows A / B for {METRIC_LABELS[metric]}
        </span>
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto">
        <table
          className="w-full border-separate border-spacing-0 text-xs"
          style={{ minWidth: `${totalColumns * 88}px` }}
        >
          <thead className="sticky top-0 z-20">
            {/* Row 1: grouped headers, one cell spanning each side's block. */}
            <tr>
              <Th colSpan={3} rowSpan={2} className="z-30 bg-raised text-left">
                case
              </Th>
              <Th colSpan={fieldCount + metricCount} className="th-a-group text-accent">
                <span className="truncate" title={labels.a}>
                  {labels.a}
                </span>
              </Th>
              {hasB && (
                <>
                  <Th colSpan={fieldCount + metricCount} className="bg-raised">
                    <span className="truncate" title={labels.b ?? ""}>
                      {labels.b}
                    </span>
                  </Th>
                  <Th rowSpan={2} className="bg-raised">
                    A vs B
                  </Th>
                </>
              )}
            </tr>
            {/* Row 2: per-column names. */}
            <tr>
              {table.field_headers.map((header) => (
                <Th key={`a-${header}`} className="th-a-col text-accent/90">
                  {header}
                </Th>
              ))}
              {table.metric_keys.map((key) => (
                <Th key={`a-${key}`} className="th-a-col text-accent/90">
                  {METRIC_LABELS[key]}
                </Th>
              ))}
              {hasB &&
                table.field_headers.map((header) => (
                  <Th key={`b-${header}`} className="bg-raised">
                    {header}
                  </Th>
                ))}
              {hasB &&
                table.metric_keys.map((key) => (
                  <Th key={`b-${key}`} className="bg-raised">
                    {METRIC_LABELS[key]}
                  </Th>
                ))}
            </tr>
          </thead>

          <tbody>
            {/* Spacer rows give the scroll container its full height while only
                the visible window is actually rendered. */}
            {virtualizer.getVirtualItems()[0] && (
              <tr style={{ height: virtualizer.getVirtualItems()[0]!.start }} />
            )}

            {virtualizer.getVirtualItems().map((item) => {
              const row = rows[item.index]!;
              return (
                <Row
                  key={`${row.group}-${row.concurrency}`}
                  row={row}
                  metricKeys={table.metric_keys}
                  fieldHeaders={table.field_headers}
                  hasB={hasB}
                  primaryMetric={metric}
                />
              );
            })}

            <tr
              style={{
                height:
                  virtualizer.getTotalSize() -
                  (virtualizer.getVirtualItems().at(-1)?.end ?? 0),
              }}
            />
          </tbody>
        </table>
      </div>
    </div>
  );
}

function Th({
  children,
  className,
  colSpan,
  rowSpan,
}: {
  children?: React.ReactNode;
  className?: string;
  colSpan?: number;
  rowSpan?: number;
}) {
  return (
    <th
      colSpan={colSpan}
      rowSpan={rowSpan}
      className={cn(
        "sticky top-0 border-b border-r border-border px-2 py-1.5 text-center text-[11px] font-semibold",
        className,
      )}
    >
      {children}
    </th>
  );
}

function Row({
  row,
  metricKeys,
  fieldHeaders,
  hasB,
  primaryMetric,
}: {
  row: ComparisonRow;
  metricKeys: MetricKey[];
  fieldHeaders: string[];
  hasB: boolean;
  primaryMetric: MetricKey;
}) {
  const missing = hasB && (!row.a || !row.b);

  return (
    <tr className={cn("group", missing && "bg-warn/[0.06]")} style={{ height: ROW_HEIGHT }}>
      <Td className="tabular text-right">{row.input_len}</Td>
      <Td className="tabular text-right">{row.output_len}</Td>
      <Td className="tabular text-right font-medium">{row.concurrency}</Td>

      {fieldHeaders.map((header) => (
        <Td key={`a-${header}`} className="truncate text-center text-muted">
          {row.a?.fields[header] ?? "—"}
        </Td>
      ))}
      {metricKeys.map((key) => (
        <Td
          key={`a-${key}`}
          className={cn(
            "tabular text-right",
            key === primaryMetric && "font-medium text-fg",
          )}
        >
          {row.a ? formatMetric(row.a.metrics[key]) : "—"}
          <Estimated cell={row.a} metricKey={key} />
        </Td>
      ))}

      {hasB && (
        <>
          {fieldHeaders.map((header) => (
            <Td key={`b-${header}`} className="truncate text-center text-muted">
              {row.b?.fields[header] ?? "—"}
            </Td>
          ))}
          {metricKeys.map((key) => (
            <Td
              key={`b-${key}`}
              className={cn(
                "tabular text-right",
                key === primaryMetric && "font-medium text-fg",
              )}
            >
              {row.b ? formatMetric(row.b.metrics[key]) : "—"}
              <Estimated cell={row.b} metricKey={key} />
            </Td>
          ))}
          <RatioCell value={row.ratios[primaryMetric]} metric={primaryMetric} />
        </>
      )}
    </tr>
  );
}

/**
 * Marks an E2EL that was derived rather than measured, which happens when the
 * sweep was launched without `e2el` in `--percentile-metrics`.
 */
function Estimated({ cell, metricKey }: { cell: CellGroup | null; metricKey: MetricKey }) {
  if (metricKey !== "median_e2el_ms" || !cell?.metrics.e2el_approximate) return null;
  return (
    <span
      className="cursor-help text-warn"
      title="Estimated: this run's benchmark artifact has no end-to-end latency field, so it was derived as median TTFT + median TPOT × (output tokens per request − 1). vLLM only writes E2EL when --percentile-metrics includes e2el."
    >
      *
    </span>
  );
}

/**
 * A ratio above 100% means A's value is larger, which is good for throughput and
 * bad for latency, so the colour depends on the metric's direction.
 */
function RatioCell({ value, metric }: { value: number | undefined; metric: MetricKey }) {
  if (value === undefined) {
    return <Td className="text-center text-muted">—</Td>;
  }

  const aIsBetter = LOWER_IS_BETTER.has(metric) ? value < 100 : value > 100;
  const neutral = Math.abs(value - 100) < 0.5;

  return (
    <Td
      className={cn(
        "tabular text-right font-medium",
        neutral ? "text-muted" : aIsBetter ? "bg-good/10 text-good" : "bg-bad/10 text-bad",
      )}
    >
      {formatPercent(value)}
    </Td>
  );
}

function Td({ children, className }: { children?: React.ReactNode; className?: string }) {
  return (
    <td
      data-selectable
      className={cn(
        "max-w-[12rem] border-b border-r border-border/60 px-2 py-1",
        className,
      )}
    >
      {children}
    </td>
  );
}
