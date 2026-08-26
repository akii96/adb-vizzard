// Charts live behind a dynamic import (see ResultsPanel), so nothing in this
// module is in the boot bundle. Importing from echarts/core and registering only
// what we use keeps it around 250 KB instead of ~1 MB.

import { LineChart, ScatterChart } from "echarts/charts";
import {
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkLineComponent,
  TooltipComponent,
} from "echarts/components";
import * as echarts from "echarts/core";
import { CanvasRenderer } from "echarts/renderers";
import { useEffect, useMemo, useRef } from "react";

import { METRIC_LABELS, type CurveSeries, type MetricKey, type XAxis } from "@/types";

echarts.use([
  LineChart,
  ScatterChart,
  GridComponent,
  TooltipComponent,
  DataZoomComponent,
  LegendComponent,
  MarkLineComponent,
  CanvasRenderer,
]);

const X_LABELS: Record<XAxis, string> = {
  concurrency: "max_concurrency",
  input_len: "random_input_len",
  output_len: "random_output_len",
};

const SERIES_COLORS = ["#22d3ee", "#fbbf24"];

/** What each data point carries so the tooltip can name its benchmark case. */
interface PointPayload {
  value: [number, number];
  meta: {
    isl: number;
    osl: number;
    concurrency: number;
    runCount: number;
    isPareto: boolean;
  };
}

function formatValue(value: number): string {
  return value.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

/** Series labels are user-derived, and the tooltip renders as HTML. */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export interface ParetoChartProps {
  series: CurveSeries[];
  metric: MetricKey;
  xAxis: XAxis;
  logX: boolean;
  paretoOnly: boolean;
}

export default function ParetoChart({
  series,
  metric,
  xAxis,
  logX,
  paretoOnly,
}: ParetoChartProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<echarts.ECharts | null>(null);

  const isDark = document.documentElement.classList.contains("dark");

  const option = useMemo<echarts.EChartsCoreOption>(() => {
    const axisColor = isDark ? "#94a3b8" : "#475569";
    const splitColor = isDark ? "rgba(148,163,184,0.15)" : "rgba(71,85,105,0.12)";

    return {
      animationDuration: 220,
      grid: { left: 56, right: 20, top: 36, bottom: 56 },
      legend: {
        top: 4,
        textStyle: { color: axisColor, fontSize: 11 },
        icon: "roundRect",
      },
      tooltip: {
        trigger: "item",
        confine: true,
        backgroundColor: isDark ? "#1e293b" : "#ffffff",
        borderColor: splitColor,
        textStyle: { color: isDark ? "#e2e8f0" : "#0f172a", fontSize: 11 },
        // Names the benchmark case, so a point is identifiable without
        // cross-referencing the table. ISL/OSL/concurrency are always shown, not
        // just whichever one is currently on the x axis.
        formatter: (params: unknown) => {
          const item = Array.isArray(params) ? params[0] : params;
          const point = (item as { data?: PointPayload })?.data;
          if (!point?.meta) return "";

          const { isl, osl, concurrency, runCount, isPareto } = point.meta;
          const seriesName = (item as { seriesName?: string }).seriesName ?? "";
          const value = point.value[1];

          const rows = [
            `<div style="font-weight:600;margin-bottom:2px">ISL ${isl} / OSL ${osl} / mc ${concurrency}</div>`,
            `<div style="opacity:.75">${escapeHtml(seriesName)}</div>`,
            `<div style="margin-top:3px">${METRIC_LABELS[metric]}: <b>${formatValue(value)}</b></div>`,
          ];
          if (runCount > 1) {
            rows.push(`<div style="opacity:.6">median of ${runCount} runs</div>`);
          }
          if (isPareto) {
            rows.push(`<div style="opacity:.6">on the Pareto frontier</div>`);
          }
          return rows.join("");
        },
      },
      xAxis: {
        type: logX ? "log" : "value",
        name: X_LABELS[xAxis],
        nameLocation: "middle",
        nameGap: 30,
        nameTextStyle: { color: axisColor, fontSize: 11 },
        axisLabel: { color: axisColor, fontSize: 10 },
        axisLine: { lineStyle: { color: splitColor } },
        splitLine: { lineStyle: { color: splitColor } },
      },
      yAxis: {
        type: "value",
        name: METRIC_LABELS[metric],
        nameLocation: "middle",
        nameGap: 42,
        nameTextStyle: { color: axisColor, fontSize: 11 },
        axisLabel: { color: axisColor, fontSize: 10 },
        axisLine: { lineStyle: { color: splitColor } },
        splitLine: { lineStyle: { color: splitColor } },
        scale: true,
      },
      // Drag to zoom, double-click to reset.
      dataZoom: [
        { type: "inside", xAxisIndex: 0, filterMode: "none" },
        { type: "inside", yAxisIndex: 0, filterMode: "none" },
      ],
      series: series.flatMap((entry, index) => {
        const color = SERIES_COLORS[index % SERIES_COLORS.length];
        const points = paretoOnly
          ? entry.points.filter((point) => point.is_pareto)
          : entry.points;

        const toPayload = (point: (typeof points)[number]): PointPayload => ({
          value: [point.x, point.y],
          meta: {
            isl: point.input_len,
            osl: point.output_len,
            concurrency: point.concurrency,
            runCount: point.run_count,
            isPareto: point.is_pareto,
          },
        });

        const line = {
          name: entry.label,
          type: "line" as const,
          smooth: false,
          symbol: "circle",
          symbolSize: 7,
          // A generous hit area: the points are small and the data is sparse.
          emphasis: { scale: 1.6 },
          itemStyle: { color },
          lineStyle: { color, width: 2 },
          data: points.map(toPayload),
        };

        if (paretoOnly) return [line];

        // Ring the frontier points so they read as special without needing a
        // second legend entry. Silent, so hovering still hits the line series
        // underneath and shows one tooltip rather than two.
        const frontier = entry.points.filter((point) => point.is_pareto);
        return [
          line,
          {
            name: entry.label,
            type: "scatter" as const,
            symbolSize: 13,
            silent: true,
            legendHoverLink: false,
            tooltip: { show: false },
            itemStyle: {
              color: "transparent",
              borderColor: color,
              borderWidth: 2,
            },
            data: frontier.map(toPayload),
          },
        ];
      }),
    };
  }, [series, metric, xAxis, logX, paretoOnly, isDark]);

  useEffect(() => {
    if (!containerRef.current) return;
    const chart = echarts.init(containerRef.current, undefined, { renderer: "canvas" });
    chartRef.current = chart;

    const observer = new ResizeObserver(() => chart.resize());
    observer.observe(containerRef.current);

    const onDoubleClick = () => chart.dispatchAction({ type: "dataZoom", start: 0, end: 100 });
    containerRef.current.addEventListener("dblclick", onDoubleClick);
    const element = containerRef.current;

    return () => {
      observer.disconnect();
      element.removeEventListener("dblclick", onDoubleClick);
      chart.dispose();
      chartRef.current = null;
    };
  }, []);

  // Patch rather than replace, so zoom state survives a metric change.
  useEffect(() => {
    chartRef.current?.setOption(option, { notMerge: false, lazyUpdate: true });
  }, [option]);

  return <div ref={containerRef} className="h-full w-full" />;
}
