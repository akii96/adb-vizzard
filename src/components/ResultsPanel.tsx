import { BarChart3, Loader2 } from "lucide-react";
import { Suspense, lazy, useCallback, useRef, useState } from "react";

import { ComparisonTable } from "@/components/ComparisonTable";
import { ExportMenu } from "@/components/ExportMenu";
import { RawRuns } from "@/components/RawRuns";
import {
  Checkbox,
  EmptyState,
  Select,
  Spinner,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components/ui/primitives";
import { useStore } from "@/store/session";
import { METRIC_LABELS, type Aggregation, type MetricKey, type XAxis } from "@/types";

// Charts are the single largest dependency, and are not needed to render the
// table, so they load on demand after first paint.
const ParetoChart = lazy(() => import("@/components/ParetoChart"));

const METRIC_OPTIONS = (Object.keys(METRIC_LABELS) as MetricKey[]).map((key) => ({
  value: key,
  label: METRIC_LABELS[key],
}));

const X_AXIS_OPTIONS: Array<{ value: XAxis; label: string }> = [
  { value: "concurrency", label: "concurrency" },
  { value: "input_len", label: "input len" },
  { value: "output_len", label: "output len" },
];

const AGGREGATION_OPTIONS: Array<{ value: Aggregation; label: string }> = [
  { value: "median", label: "median" },
  { value: "mean", label: "mean" },
  { value: "max", label: "max" },
  { value: "min", label: "min" },
];

export function ResultsPanel() {
  const store = useStore();
  const { table, curves, metric, xAxis, aggregation, tableLoading, sides } = store;

  const [tab, setTab] = useState("table");
  const [logX, setLogX] = useState(false);
  const [paretoOnly, setParetoOnly] = useState(false);
  const chartHostRef = useRef<HTMLDivElement>(null);

  /**
   * Grabs the chart's canvas and saves it.
   *
   * Reading the canvas directly avoids threading a ref through the lazy
   * component just for an occasional export.
   */
  const exportChart = useCallback(() => {
    const canvas = chartHostRef.current?.querySelector("canvas");
    if (!canvas) {
      store.toast("info", "Open the Curves tab first, then export the chart.");
      return;
    }
    canvas.toBlob((blob) => {
      if (!blob) {
        store.toast("error", "Could not render the chart image.");
        return;
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = `${table?.label_a ?? "chart"}.png`.replace(/[^\w.-]+/g, "_");
      link.click();
      URL.revokeObjectURL(url);
      store.toast("success", "Chart saved to your downloads folder.");
    }, "image/png");
  }, [store, table?.label_a]);

  const hasData = sides.a.data !== null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Tabs value={tab} onValueChange={setTab} className="min-h-0 flex-1">
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
          <TabsList>
            <TabsTrigger value="table">Table</TabsTrigger>
            <TabsTrigger value="curves">Curves</TabsTrigger>
            <TabsTrigger value="raw">Raw runs</TabsTrigger>
          </TabsList>

          <div className="ml-auto flex items-center gap-2">
            {tableLoading && <Spinner className="h-3.5 w-3.5 text-accent" />}
            <label className="flex items-center gap-1.5 text-[11px] text-muted">
              metric
              <Select
                value={metric}
                options={METRIC_OPTIONS}
                onChange={(event) => store.setMetric(event.target.value as MetricKey)}
              />
            </label>
            {tab === "curves" && (
              <label className="flex items-center gap-1.5 text-[11px] text-muted">
                x
                <Select
                  value={xAxis}
                  options={X_AXIS_OPTIONS}
                  onChange={(event) => store.setXAxis(event.target.value as XAxis)}
                />
              </label>
            )}
            <label className="flex items-center gap-1.5 text-[11px] text-muted">
              agg
              <Select
                value={aggregation}
                options={AGGREGATION_OPTIONS}
                onChange={(event) => store.setAggregation(event.target.value as Aggregation)}
              />
            </label>
          </div>
        </div>

        <TabsContent value="table" className="min-h-0">
          {!hasData ? (
            <EmptyState
              icon={<BarChart3 className="h-8 w-8" />}
              title="Load a run to get started"
              hint="Paste an MLflow run ID or a Databricks run URL on the left. Add a second run on the right to compare two sweeps."
            />
          ) : table ? (
            <ComparisonTable table={table} />
          ) : (
            <EmptyState title="Building comparison…" />
          )}
        </TabsContent>

        <TabsContent value="curves" className="min-h-0">
          {!curves || curves.length === 0 ? (
            <EmptyState title="No curve data" hint="Load a run to plot its metrics." />
          ) : (
            <div className="flex h-full min-h-0 flex-col">
              <div className="flex shrink-0 items-center gap-4 px-3 py-1.5">
                <Checkbox checked={logX} onChange={setLogX} label={<span className="text-xs">log x</span>} />
                <Checkbox
                  checked={paretoOnly}
                  onChange={setParetoOnly}
                  label={<span className="text-xs">Pareto frontier only</span>}
                />
                <span className="ml-auto text-[11px] text-muted">
                  drag to zoom · double-click to reset · ringed points are non-dominated
                </span>
              </div>
              <div ref={chartHostRef} className="min-h-0 flex-1 px-1 pb-1">
                <Suspense
                  fallback={
                    <div className="grid h-full place-items-center">
                      <Loader2 className="h-5 w-5 animate-spin text-accent" />
                    </div>
                  }
                >
                  <ParetoChart
                    series={curves}
                    metric={metric}
                    xAxis={xAxis}
                    logX={logX}
                    paretoOnly={paretoOnly}
                  />
                </Suspense>
              </div>
            </div>
          )}
        </TabsContent>

        <TabsContent value="raw" className="min-h-0">
          <RawRuns />
        </TabsContent>
      </Tabs>

      <div className="flex shrink-0 items-center gap-2 border-t border-border px-3 py-2">
        {table && (
          <span className="truncate text-xs text-muted">
            {table.label_b
              ? `${table.label_a}  vs  ${table.label_b}`
              : table.label_a}
          </span>
        )}
        <div className="ml-auto">
          <ExportMenu onExportChart={exportChart} />
        </div>
      </div>
    </div>
  );
}
