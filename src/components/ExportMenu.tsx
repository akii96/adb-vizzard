import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Download, FileSpreadsheet, FileText, Image, Table2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button, Input } from "@/components/ui/primitives";
import * as ipc from "@/lib/ipc";
import { useStore } from "@/store/session";
import type { ExportKind } from "@/types";

const OPTIONS: Array<{
  kind: ExportKind;
  label: string;
  hint: string;
  extension: string;
  filterName: string;
  icon: typeof FileText;
}> = [
  {
    kind: "comparison_xlsx",
    label: "Comparison XLSX",
    hint: "merged grouped headers",
    extension: "xlsx",
    filterName: "Excel workbook",
    icon: FileSpreadsheet,
  },
  {
    kind: "comparison_csv",
    label: "Comparison CSV",
    hint: "two-row header",
    extension: "csv",
    filterName: "CSV",
    icon: Table2,
  },
  {
    kind: "raw_csv",
    label: "Raw runs CSV",
    hint: "one row per child run",
    extension: "csv",
    filterName: "CSV",
    icon: FileText,
  },
];

export function ExportMenu({ onExportChart }: { onExportChart: () => void }) {
  const { sides, aggregation, settings, table, toast } = useStore();
  const [open, setOpen] = useState(false);
  const [labelA, setLabelA] = useState<string | null>(null);
  const [labelB, setLabelB] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const disabled = !sides.a.data;

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  async function run(option: (typeof OPTIONS)[number]) {
    try {
      const suggested = `${suggestName(option.kind)}.${option.extension}`;
      const path = await saveDialog({
        defaultPath: suggested,
        filters: [{ name: option.filterName, extensions: [option.extension] }],
      });
      if (!path) return;

      const written = await ipc.exportFile({
        kind: option.kind,
        path,
        compareFields: settings?.compare_fields,
        aggregation,
        options: {
          label_a: labelA,
          label_b: labelB,
          ratio_metrics: [],
        },
      });
      setOpen(false);
      toast("success", `Saved ${written}`);
    } catch (error) {
      toast("error", ipc.normalizeError(error).message);
    }
  }

  function suggestName(kind: ExportKind): string {
    const base = (labelA ?? table?.label_a ?? "comparison")
      .replace(/[^\w.-]+/g, "_")
      .slice(0, 60);
    return kind === "raw_csv" ? `${base}_raw` : `${base}_compare`;
  }

  return (
    <div className="relative" ref={menuRef}>
      <Button
        variant="secondary"
        size="sm"
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
      >
        <Download className="h-3.5 w-3.5" />
        Export
      </Button>

      {open && (
        <div className="absolute bottom-full right-0 z-40 mb-1.5 w-80 overflow-hidden rounded-lg border border-border bg-surface shadow-xl animate-scale-in">
          <div className="space-y-2 border-b border-border p-2.5">
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted">
              Side labels
            </p>
            <Input
              value={labelA ?? table?.label_a ?? ""}
              onChange={(event) => setLabelA(event.target.value)}
              placeholder="Side A label"
              className="h-8 text-xs"
            />
            {table?.label_b !== null && table?.label_b !== undefined && (
              <Input
                value={labelB ?? table.label_b}
                onChange={(event) => setLabelB(event.target.value)}
                placeholder="Side B label"
                className="h-8 text-xs"
              />
            )}
          </div>

          <ul>
            {OPTIONS.map((option) => {
              const Icon = option.icon;
              return (
                <li key={option.kind}>
                  <button
                    onClick={() => void run(option)}
                    className="flex w-full items-center gap-2.5 px-2.5 py-2 text-left transition-colors hover:bg-raised"
                  >
                    <Icon className="h-4 w-4 shrink-0 text-muted" />
                    <span className="min-w-0 flex-1">
                      <span className="block text-xs text-fg">{option.label}</span>
                      <span className="block text-[11px] text-muted">{option.hint}</span>
                    </span>
                  </button>
                </li>
              );
            })}
            <li className="border-t border-border">
              <button
                onClick={() => {
                  setOpen(false);
                  onExportChart();
                }}
                className="flex w-full items-center gap-2.5 px-2.5 py-2 text-left transition-colors hover:bg-raised"
              >
                <Image className="h-4 w-4 shrink-0 text-muted" />
                <span className="min-w-0 flex-1">
                  <span className="block text-xs text-fg">Chart PNG</span>
                  <span className="block text-[11px] text-muted">current curve view</span>
                </span>
              </button>
            </li>
          </ul>
        </div>
      )}
    </div>
  );
}
