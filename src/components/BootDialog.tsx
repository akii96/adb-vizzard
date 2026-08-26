import { Eye, EyeOff, FolderOpen, Loader2, ShieldAlert } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { Button, Checkbox, Input, Modal } from "@/components/ui/primitives";
import * as ipc from "@/lib/ipc";
import { useStore } from "@/store/session";
import type { CredSource } from "@/types";

/** How long a revealed token stays visible before collapsing back to masked. */
const REVEAL_TIMEOUT_MS = 15_000;

const SOURCE_CAPTIONS: Record<CredSource, string | null> = {
  environment: "from DATABRICKS_HOST / DATABRICKS_TOKEN",
  dot_env: "from .env in the working directory",
  remembered: "remembered on this PC",
  none: null,
};

export function BootDialog() {
  const { connection, offline, settings, boot, connect, setOffline, toast } = useStore();

  const [host, setHost] = useState("");
  const [token, setToken] = useState("");
  /**
   * True while the token field still shows the backend's masked preview rather
   * than something the user typed. Connecting in this state sends no token at
   * all, so a remembered secret never round-trips through the webview.
   */
  const [tokenIsPreview, setTokenIsPreview] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const revealTimer = useRef<number | null>(null);
  const connectRef = useRef<HTMLButtonElement>(null);

  const open = !connection && !offline;

  // Seed the form from whatever the backend resolved at boot.
  useEffect(() => {
    if (!open || !boot || !settings) return;
    setHost(boot.host || settings.host || "");
    setRemember(settings.remember);
    if (boot.hasToken && boot.masked) {
      setToken(boot.masked);
      setTokenIsPreview(true);
    }
  }, [open, boot?.host, boot?.hasToken, boot?.masked, settings?.host, settings?.remember]);

  // When creds were pre-filled, the common case is a single click, so put focus
  // on Connect rather than on a field the user does not need to touch.
  useEffect(() => {
    if (open && boot?.hasToken) connectRef.current?.focus();
  }, [open, boot?.hasToken]);

  useEffect(() => {
    return () => {
      if (revealTimer.current) window.clearTimeout(revealTimer.current);
    };
  }, []);

  const hostWarning = useMemo(() => {
    const trimmed = host.trim();
    if (!trimmed) return false;
    const withoutScheme = trimmed.replace(/^https?:\/\//i, "");
    const hostname = withoutScheme.split("/")[0]?.split(":")[0] ?? "";
    return hostname.length > 0 && !hostname.endsWith(".azuredatabricks.net");
  }, [host]);

  async function toggleReveal() {
    if (revealed) {
      setRevealed(false);
      if (revealTimer.current) window.clearTimeout(revealTimer.current);
      return;
    }

    // A preview is only a mask, so ask the backend for the real value.
    if (tokenIsPreview) {
      try {
        const plaintext = await ipc.revealToken();
        setToken(plaintext);
        setTokenIsPreview(false);
      } catch (err) {
        toast("error", ipc.normalizeError(err).message);
        return;
      }
    }

    setRevealed(true);
    revealTimer.current = window.setTimeout(() => setRevealed(false), REVEAL_TIMEOUT_MS);
  }

  async function onSubmit() {
    setBusy(true);
    setError(null);
    try {
      await connect(host, tokenIsPreview ? null : token, remember);
    } catch (err) {
      setError(ipc.normalizeError(err).message);
    } finally {
      setBusy(false);
    }
  }

  const caption = boot ? SOURCE_CAPTIONS[boot.source] : null;
  const canSubmit = host.trim().length > 0 && (tokenIsPreview || token.trim().length > 0);

  return (
    <Modal open={open} mandatory labelledBy="boot-title" className="max-w-lg">
      <div className="mb-5 text-center">
        <h1 id="boot-title" className="text-lg font-semibold text-fg">
          Connect to Databricks
        </h1>
        <p className="mt-1 text-xs text-muted">
          ADB Vizzard reads MLflow benchmark runs from your workspace.
        </p>
      </div>

      <form
        className="space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit && !busy) void onSubmit();
        }}
      >
        <div>
          <label htmlFor="boot-host" className="mb-1.5 block text-xs font-medium text-muted">
            Host
          </label>
          <Input
            id="boot-host"
            value={host}
            onChange={(event) => setHost(event.target.value)}
            placeholder="https://adb-1234567890.1.azuredatabricks.net"
            spellCheck={false}
            autoComplete="off"
            data-autofocus={boot?.hasToken ? undefined : true}
          />
          {hostWarning && (
            <p className="mt-1.5 flex items-start gap-1.5 text-xs text-warn">
              <ShieldAlert className="mt-px h-3.5 w-3.5 shrink-0" />
              <span>
                This is not an <code>*.azuredatabricks.net</code> host. Double-check it before
                sending your token there.
              </span>
            </p>
          )}
        </div>

        <div>
          <label htmlFor="boot-token" className="mb-1.5 block text-xs font-medium text-muted">
            Token
          </label>
          <div className="relative">
            <Input
              id="boot-token"
              type={revealed ? "text" : "password"}
              value={token}
              onChange={(event) => {
                setToken(event.target.value);
                setTokenIsPreview(false);
              }}
              onFocus={() => {
                // Clear the mask on focus so typing replaces it cleanly rather
                // than appending to a string of bullets.
                if (tokenIsPreview) {
                  setToken("");
                  setTokenIsPreview(false);
                }
              }}
              placeholder="dapi…"
              spellCheck={false}
              autoComplete="off"
              className="pr-10 font-mono text-xs"
            />
            <button
              type="button"
              onClick={() => void toggleReveal()}
              aria-label={revealed ? "Hide token" : "Show token"}
              title={revealed ? "Hide token" : "Show token"}
              className="absolute right-1 top-1 grid h-7 w-8 place-items-center rounded-md text-muted transition-colors hover:bg-raised hover:text-fg"
            >
              {revealed ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
            </button>
          </div>
          {caption && <p className="mt-1.5 text-xs text-muted">{caption}</p>}
        </div>

        <div className="rounded-lg border border-border bg-raised/50 p-3">
          <Checkbox
            checked={remember}
            onChange={setRemember}
            label="Remember me on this PC"
            description={
              settings?.sync_root_warning ? (
                <span className="text-warn">
                  Your settings folder looks like it is inside a synced folder, so only the host
                  will be remembered.
                </span>
              ) : (
                "Saves your host and token in plain text on this machine. Clear it any time from Settings."
              )
            }
          />
        </div>

        {error && (
          <div className="rounded-lg border border-bad/30 bg-bad/10 px-3 py-2 text-xs text-bad">
            {error}
          </div>
        )}

        <Button
          ref={connectRef}
          type="submit"
          variant="primary"
          disabled={!canSubmit || busy}
          className="w-full"
        >
          {busy && <Loader2 className="h-4 w-4 animate-spin" />}
          {busy ? "Connecting…" : "Connect"}
        </Button>

        <div className="border-t border-border pt-3 text-center">
          <button
            type="button"
            onClick={() => setOffline(true)}
            className="inline-flex items-center gap-1.5 text-xs text-muted transition-colors hover:text-accent"
          >
            <FolderOpen className="h-3.5 w-3.5" />
            Skip — work from a local pull folder
          </button>
          <p className="mt-1 text-[11px] text-muted/80">
            Reads an existing <code>exp_pull_*</code> directory. No credentials needed.
          </p>
        </div>
      </form>
    </Modal>
  );
}
