import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";

import { BootDialog } from "@/components/BootDialog";
import { ResultsPanel } from "@/components/ResultsPanel";
import { RunPicker } from "@/components/RunPicker";
import { SettingsDialog } from "@/components/SettingsDialog";
import { Toasts } from "@/components/Toasts";
import { TopBar } from "@/components/TopBar";
import * as ipc from "@/lib/ipc";
import { subscribeToProgress, useStore } from "@/store/session";
import type { CacheStats } from "@/types";

export default function App() {
  const { ready, init } = useStore();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [cache, setCache] = useState<CacheStats | null>(null);

  useEffect(() => {
    void init();
    const unlisten = subscribeToProgress();
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [init]);

  // The window is created hidden so there is no flash of an unstyled or empty
  // frame; show it once the first paint has happened.
  useEffect(() => {
    if (!ready) return;
    void getCurrentWindow().show().catch(() => undefined);
  }, [ready]);

  useEffect(() => {
    if (!settingsOpen) return;
    void ipc
      .cacheStats()
      .then(setCache)
      .catch(() => undefined);
  }, [settingsOpen]);

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <TopBar onOpenSettings={() => setSettingsOpen(true)} />

      <main className="grid min-h-0 flex-1 grid-cols-[minmax(220px,17rem)_minmax(0,1fr)_minmax(220px,17rem)]">
        <aside className="min-h-0 border-r border-border bg-surface/60">
          <RunPicker side="a" />
        </aside>

        <section className="min-h-0">
          <ResultsPanel />
        </section>

        <aside className="min-h-0 border-l border-border bg-surface/60">
          <RunPicker side="b" />
        </aside>
      </main>

      {/* Rendered unconditionally: settings are useful offline too, and the
          dialog already no-ops until it has settings to show. */}
      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        cache={cache}
        onCacheChange={setCache}
      />

      <BootDialog />
      <Toasts />
    </div>
  );
}
