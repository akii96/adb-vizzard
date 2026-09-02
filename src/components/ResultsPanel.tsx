import { ArrowLeftRight, BarChart3, Loader2 } from "lucide-react";
import { Suspense, lazy, useCallback, useMemo, useRef, useState } from "react";

import { ComparisonTable } from "@/components/ComparisonTable";
import { ExportMenu } from "@/components/ExportMenu";
import { MetricPicker } from "@/components/MetricPicker";
import { RawRuns } from "@/components/RawRuns";
import {
  Button,
  Checkbox,
  EmptyState,
  Input,
  Select,
  Spinner,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components/ui/primitives";
import { availableCases, effectiveLabels, taggedLabels, useStore } from "@/store/session";
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
  { value: "interactivity", label: "interactivity (1000 / TPOT)" },
];

const AGGREGATION_OPTIONS: Array<{ value: Aggregation; label: string }> = [
  { value: "median", label: "median" },
  { value: "mean", label: "mean" },
  { value: "max", label: "max" },
  { value: "min", label: "min" },
];

export function ResultsPanel() {
  const store = useStore();
  const { table, curves, metric, xAxis, curveCase, aggregation, tableLoading, sides } = store;

  const [tab, setTab] = useState("table");
  const [logX, setLogX] = useState(false);
  const [paretoOnly, setParetoOnly] = useState(false);
  /** Kept as typed text so the field can be cleared without snapping to a number. */
  const [tpotSla, setTpotSla] = useState("");
  const chartHostRef = useRef<HTMLDivElement>(null);

  const cases = useMemo(() => availableCases(sides), [sides]);
  const caseOptions = useMemo(
    () =>
      cases.map((entry) => ({
        value: `${entry.inputLen}/${entry.outputLen}`,
        label: `in${entry.inputLen}_out${entry.outputLen}`,
      })),
    [cases],
  );

  const parsedSla = Number.parseFloat(tpotSla);
  const tpotSlaMs = Number.isFinite(parsedSla) && parsedSla > 0 ? parsedSla : undefined;

  const labels = useMemo(() => taggedLabels(sides), [sides]);
  // The footer names both sides in prose, where the A/B tags read as noise.
  const footerLabels = useMemo(() => effectiveLabels(sides), [sides]);

  // Series names come from the backend's derived labels, which collide when both
  // sides are the same run or share a run name. ECharts would then merge them
  // into one legend entry, so the tagged names are substituted here.
  const namedCurves = useMemo(() => {
    if (!curves) return null;
    const names = [labels.a, labels.b ?? ""];
    return curves.map((entry, index) => ({ ...entry, label: names[index] || entry.label }));
  }, [curves, labels]);

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
      link.download = `${effectiveLabels(sides).a || "chart"}.png`.replace(/[^\w.-]+/g, "_");
      link.click();
      URL.revokeObjectURL(url);
      store.toast("success", "Chart saved to your downloads folder.");
    }, "image/png");
  }, [store, sides]);

  const hasData = sides.a.data !== null;
  // A one-sided swap would just unload the side that was there.
  const canSwap =
    sides.a.data !== null && sides.b.data !== null && !sides.a.loading && !sides.b.loading;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Tabs value={tab} onValueChange={setTab} className="min-h-0 flex-1">
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
          <TabsList>
            <TabsTrigger value="table">Table</TabsTrigger>
            <TabsTrigger value="curves">Curves</TabsTrigger>
            <TabsTrigger value="raw">Raw runs</TabsTrigger>
          </TabsList>

          <Button
            size="sm"
            variant="ghost"
            disabled={!canSwap}
            onClick={() => void store.swap()}
            title={
              canSwap
                ? "Swap A and B, including their case selections and labels"
                : "Load both sides to swap them"
            }
          >
            <ArrowLeftRight className="h-3.5 w-3.5" />
            Swap
          </Button>

          <div className="ml-auto flex items-center gap-2">
            {tableLoading && <Spinner className="h-3.5 w-3.5 text-accent" />}
            {tab === "table" && <MetricPicker />}
            <label className="flex items-center gap-1.5 text-[11px] text-muted">
              {tab === "curves" ? "y" : "metric"}
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
            {tab === "curves" && caseOptions.length > 0 && (
              <label className="flex items-center gap-1.5 text-[11px] text-muted">
                case
                <Select
                  value={
                    curveCase ? `${curveCase.inputLen}/${curveCase.outputLen}` : caseOptions[0]!.value
                  }
                  options={caseOptions}
                  onChange={(event) => {
                    const [inputLen, outputLen] = event.target.value.split("/").map(Number);
                    store.setCurveCase({ inputLen: inputLen!, outputLen: outputLen! });
                  }}
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
                  label={
                    <span
                      className="text-xs"
                      title="A config is dominated when another one is better on both axes at once, so it is never worth picking. This hides them and leaves the Pareto frontier."
                    >
                      hide dominated configs
                    </span>
                  }
                />
                {xAxis === "interactivity" && (
                  <label className="flex items-center gap-1.5 text-[11px] text-muted">
                    TPOT SLA (ms)
                    <Input
                      type="number"
                      min={1}
                      step={1}
                      value={tpotSla}
                      placeholder="none"
                      onChange={(event) => setTpotSla(event.target.value)}
                      className="h-8 w-20 px-2 text-xs"
                    />
                  </label>
                )}
                <span className="ml-auto text-[11px] text-muted">
                  drag to zoom · double-click to reset · ringed points are on the Pareto frontier
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
                    series={namedCurves ?? curves}
                    metric={metric}
                    xAxis={xAxis}
                    logX={logX}
                    paretoOnly={paretoOnly}
                    tpotSlaMs={tpotSlaMs}
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
            {footerLabels.b ? `${footerLabels.a}  vs  ${footerLabels.b}` : footerLabels.a}
          </span>
        )}
        <div className="ml-auto">
          <ExportMenu onExportChart={exportChart} />
        </div>
      </div>
    </div>
  );
}
