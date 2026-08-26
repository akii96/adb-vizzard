import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";

import { cn } from "@/lib/utils";
import { useStore } from "@/store/session";
import type { Toast } from "@/store/session";

const TONES: Record<Toast["kind"], { className: string; icon: typeof Info }> = {
  error: { className: "border-bad/30 bg-bad/10 text-bad", icon: AlertCircle },
  success: { className: "border-good/30 bg-good/10 text-good", icon: CheckCircle2 },
  info: { className: "border-border bg-raised text-fg", icon: Info },
};

export function Toasts() {
  const { toasts, dismissToast } = useStore();
  if (toasts.length === 0) return null;

  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex w-full max-w-sm flex-col gap-2">
      {toasts.map((toast) => {
        const tone = TONES[toast.kind];
        const Icon = tone.icon;
        return (
          <div
            key={toast.id}
            role="status"
            className={cn(
              "pointer-events-auto flex items-start gap-2.5 rounded-lg border px-3 py-2.5 shadow-lg animate-scale-in",
              tone.className,
            )}
          >
            <Icon className="mt-px h-4 w-4 shrink-0" />
            <p data-selectable className="min-w-0 flex-1 text-xs leading-relaxed">
              {toast.message}
            </p>
            <button
              onClick={() => dismissToast(toast.id)}
              aria-label="Dismiss"
              className="shrink-0 rounded p-0.5 opacity-60 transition-opacity hover:opacity-100"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
