import { Trash2 } from "lucide-react";
import { useState } from "react";

import { Badge, Button, Input, Modal, Switch } from "@/components/ui/primitives";
import * as ipc from "@/lib/ipc";
import { formatBytes } from "@/lib/utils";
import { useStore } from "@/store/session";
import type { CacheStats } from "@/types";

export function SettingsDialog({
  open,
  onClose,
  cache,
  onCacheChange,
}: {
  open: boolean;
  onClose: () => void;
  cache: CacheStats | null;
  onCacheChange: (stats: CacheStats) => void;
}) {
  const { settings, applySettings, toast, refresh } = useStore();

  const [fieldsDraft, setFieldsDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  if (!settings) return null;

  const fieldsValue = fieldsDraft ?? settings.compare_fields.join(", ");

  async function patch(update: Parameters<typeof ipc.saveSettings>[0]) {
    setBusy(true);
    try {
      applySettings(await ipc.saveSettings(update));
    } catch (error) {
      toast("error", ipc.normalizeError(error).message);
    } finally {
      setBusy(false);
    }
  }

  async function toggleCompareFields(enabled: boolean) {
    await patch({ compare_fields_enabled: enabled });
    await refresh();
  }

  async function commitCompareFields() {
    const fields = fieldsValue
      .split(",")
      .map((field) => field.trim())
      .filter(Boolean);
    setFieldsDraft(null);
    await patch({ compare_fields: fields });
    await refresh();
  }

  async function onForget() {
    try {
      applySettings(await ipc.forgetCredentials());
      toast("success", "Saved credentials cleared.");
      onClose();
      // The session is gone, so the boot dialog needs to come back.
      await useStore.getState().disconnect();
    } catch (error) {
      toast("error", ipc.normalizeError(error).message);
    }
  }

  async function onClearCache() {
    try {
      onCacheChange(await ipc.clearCache());
      toast("success", "Cache cleared.");
    } catch (error) {
      toast("error", ipc.normalizeError(error).message);
    }
  }

  return (
    <Modal open={open} onClose={onClose} labelledBy="settings-title" className="max-w-lg">
      <h2 id="settings-title" className="mb-4 text-base font-semibold">
        Settings
      </h2>

      <div className="space-y-5">
        <section>
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
            Credentials
          </h3>
          <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-raised/50 px-3 py-2.5">
            <div className="min-w-0">
              <p className="text-sm">
                {settings.has_stored_token ? "Token saved on this PC" : "No token saved"}
              </p>
              <p className="mt-0.5 truncate text-xs text-muted" title={settings.settings_path}>
                {settings.has_stored_token
                  ? `${settings.masked_token} in ${settings.settings_path}`
                  : "Enable “Remember me” when connecting to save it."}
              </p>
            </div>
            <Button
              variant="danger"
              size="sm"
              disabled={!settings.has_stored_token}
              onClick={() => void onForget()}
            >
              Forget
            </Button>
          </div>
          {settings.sync_root_warning && (
            <p className="mt-2 text-xs text-warn">
              Your config folder looks like it is inside a synced folder, so tokens are never
              written there.
            </p>
          )}
        </section>

        <section>
          <div className="mb-2 flex items-center gap-2">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">
              Comparison fields
            </h3>
            <Switch
              checked={settings.compare_fields_enabled}
              onChange={(enabled) => void toggleCompareFields(enabled)}
              disabled={busy}
              label="Show comparison fields"
            />
          </div>
          <Input
            value={fieldsValue}
            onChange={(event) => setFieldsDraft(event.target.value)}
            onBlur={() => void commitCompareFields()}
            placeholder="tensor_parallel_size, env:VLLM_ROCM_USE_AITER"
            spellCheck={false}
            disabled={!settings.compare_fields_enabled}
            className="font-mono text-xs"
          />
          <p className="mt-1.5 text-xs leading-relaxed text-muted">
            {settings.compare_fields_enabled ? (
              <>
                Comma-separated. Use a flag name without <code>--</code>, or{" "}
                <code>env:NAME</code> for a docker env var. Missing values export as{" "}
                <code>image-default</code>, matching <code>adb-summarize</code>.
              </>
            ) : (
              <>
                Off, so the table and exports show only the cases and the metrics. Switch it on
                to add a column per field, showing what each run was served with.
              </>
            )}
          </p>
        </section>

        <section>
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
            Session data
          </h3>
          <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-raised/50 px-3 py-2.5">
            <div>
              <p className="text-sm">
                {cache ? `${cache.entries} artifacts` : "—"}{" "}
                <span className="text-muted">
                  {cache ? `· ${formatBytes(cache.bytes)} of ${formatBytes(cache.cap_bytes)}` : ""}
                </span>
              </p>
              <p className="mt-0.5 text-xs leading-relaxed text-muted">
                Kept in memory so reopening a run is fast. Nothing is written to disk, and it is
                discarded when you close the app.
              </p>
            </div>
            <Button variant="secondary" size="sm" onClick={() => void onClearCache()}>
              <Trash2 className="h-3.5 w-3.5" />
              Clear
            </Button>
          </div>
        </section>

        <section>
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
            Concurrency
          </h3>
          <div className="grid grid-cols-2 gap-3">
            <label className="block">
              <span className="mb-1 block text-xs text-muted">Control plane</span>
              <Input
                type="number"
                min={1}
                max={32}
                value={settings.control_concurrency}
                disabled={busy}
                onChange={(event) =>
                  void patch({ control_concurrency: Number(event.target.value) })
                }
              />
            </label>
            <label className="block">
              <span className="mb-1 block text-xs text-muted">Blob downloads</span>
              <Input
                type="number"
                min={1}
                max={64}
                value={settings.blob_concurrency}
                disabled={busy}
                onChange={(event) => void patch({ blob_concurrency: Number(event.target.value) })}
              />
            </label>
          </div>
          <p className="mt-1.5 text-xs text-muted">
            Lower these if you are on a slow VPN and seeing timeouts.
          </p>
        </section>

        <section>
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">
              Recent runs
            </h3>
            <Badge>{settings.recent_runs.length}</Badge>
          </div>
          <Button
            variant="secondary"
            size="sm"
            disabled={settings.recent_runs.length === 0}
            onClick={() => void patch({ clear_recents: true })}
          >
            Clear history
          </Button>
        </section>
      </div>

      <div className="mt-6 flex justify-end">
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      </div>
    </Modal>
  );
}
