import { Database, LogOut, Moon, Plug, Settings, Sun } from "lucide-react";

import { Badge, Button } from "@/components/ui/primitives";
import * as ipc from "@/lib/ipc";
import { useStore } from "@/store/session";

export function TopBar({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { connection, offline, settings, applySettings, disconnect, setOffline, toast } =
    useStore();

  const shortHost = connection?.host.replace(/^https?:\/\//, "") ?? "";
  const isDark = document.documentElement.classList.contains("dark");

  async function toggleTheme() {
    const next = isDark ? "light" : "dark";
    try {
      applySettings(await ipc.saveSettings({ theme: next }));
    } catch (error) {
      toast("error", ipc.normalizeError(error).message);
    }
  }

  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border bg-surface px-3">
      <div className="flex items-center gap-2">
        <div className="grid h-6 w-6 place-items-center rounded-md bg-accent/15">
          <Database className="h-3.5 w-3.5 text-accent" />
        </div>
        <span className="text-sm font-semibold tracking-tight">ADB Vizzard</span>
      </div>

      {connection ? (
        <div className="flex min-w-0 items-center gap-2">
          <Badge tone={connection.host_in_policy ? "neutral" : "warn"}>
            <span className="max-w-[22rem] truncate" title={connection.host}>
              {shortHost}
            </span>
          </Badge>
          {connection.user && (
            <span className="truncate text-xs text-muted" title={connection.user}>
              {connection.user}
            </span>
          )}
        </div>
      ) : (
        offline && (
          // Offline must not be a dead end: one click gets back to the boot dialog.
          <div className="flex items-center gap-2">
            <Badge>local folders only</Badge>
            <Button variant="ghost" size="sm" onClick={() => setOffline(false)}>
              <Plug className="h-3.5 w-3.5" />
              Connect
            </Button>
          </div>
        )
      )}

      <div className="ml-auto flex items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void toggleTheme()}
          aria-label="Toggle theme"
          title={`Switch to ${isDark ? "light" : "dark"} theme`}
        >
          {isDark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={onOpenSettings}
          aria-label="Settings"
          title="Settings"
        >
          <Settings className="h-4 w-4" />
        </Button>
        {connection && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void disconnect()}
            aria-label="Disconnect"
            title={
              settings?.remember
                ? "Disconnect (your saved token stays until you forget it in Settings)"
                : "Disconnect"
            }
          >
            <LogOut className="h-4 w-4" />
          </Button>
        )}
      </div>
    </header>
  );
}
