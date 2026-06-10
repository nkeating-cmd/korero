/**
 * Korero (v1.3.0) -- OllamaPullButton
 *
 * Shows a connection-status indicator and a "Pull model" button for local
 * Ollama providers.  When clicked it streams Ollama's /api/pull endpoint via
 * the pull_ollama_model Rust command, displaying a live progress bar and
 * status text.
 *
 * Rendered inside PostProcessingSettingsApiComponent when
 * selectedProvider.is_local_provider is true.
 *
 * v1.3.0 enhancements
 * -------------------
 * - Connection probe: pings <baseUrl>/api/tags on mount / baseUrl change.
 *   Shows a green dot ("Ollama running + model storage path") or red dot
 *   ("Ollama not reachable at <url>") so the user can immediately see whether
 *   the local server is up.
 * - Model storage path is displayed when connected so the user knows where
 *   Ollama persists downloaded models on disk.
 */

import React, { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Download, Loader2, CheckCircle2, AlertCircle, Play } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { commands } from "@/bindings";
import { Button } from "../../ui/Button";

interface OllamaPullProgressPayload {
  status: string;
  digest?: string;
  total?: number;
  completed?: number;
}

type PullState = "idle" | "pulling" | "done" | "error";
type ConnStatus = "checking" | "ok" | "offline";

interface OllamaPullButtonProps {
  /** OpenAI-compat v1 base URL, e.g. "http://localhost:11434/v1" */
  baseUrl: string;
  /** Model tag selected in the model dropdown, e.g. "gemma3:4b" */
  modelName: string;
  /** Called after a successful pull so the model list can be refreshed */
  onModelPulled?: () => void;
}

/** Strip the /v1 suffix added by the OpenAI-compat path to obtain the native
 *  Ollama API base, e.g. "http://localhost:11434/v1" -> "http://localhost:11434". */
function ollamaApiBase(baseUrl: string): string {
  return baseUrl.replace(/\/v1\/?$/, "").replace(/\/$/, "");
}

export const OllamaPullButton: React.FC<OllamaPullButtonProps> = ({
  baseUrl,
  modelName,
  onModelPulled,
}) => {
  const [pullState, setPullState] = useState<PullState>("idle");
  const [statusText, setStatusText] = useState("");
  const [progress, setProgress] = useState<number | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [connStatus, setConnStatus] = useState<ConnStatus>("checking");
  // v1.17.0: Ollama doctor — is it installed, are we mid-fix, fix feedback,
  // and a nonce to force a re-probe after starting it.
  const [installed, setInstalled] = useState<boolean | null>(null);
  const [fixing, setFixing] = useState(false);
  const [fixMsg, setFixMsg] = useState<string | null>(null);
  const [probeNonce, setProbeNonce] = useState(0);
  const unlistenRef = useRef<(() => void) | null>(null);

  // -------------------------------------------------------------------------
  // Connection probe -- runs on mount and whenever baseUrl changes.
  // Delegates to the check_ollama_connection Rust command (reqwest, 4 s
  // timeout) rather than a frontend fetch().  This bypasses the WebView2
  // CSP which blocks http://localhost:* origins not listed in connect-src.
  // -------------------------------------------------------------------------
  useEffect(() => {
    if (!baseUrl.trim()) {
      setConnStatus("offline");
      return;
    }
    let cancelled = false;
    setConnStatus("checking");

    commands.checkOllamaConnection(baseUrl)
      .then((reachable) => {
        if (!cancelled) {
          setConnStatus(reachable ? "ok" : "offline");
          if (reachable) setFixMsg(null);
          // v1.17.0: when offline, find out whether it's a "not installed"
          // or a "not running" problem so the fix button can be the right one.
          if (!reachable) {
            commands
              .ollamaStatus(baseUrl)
              .then((s) => {
                if (!cancelled) setInstalled(s.installed);
              })
              .catch(() => {
                if (!cancelled) setInstalled(null);
              });
          }
        }
      })
      .catch(() => {
        if (!cancelled) setConnStatus("offline");
      });

    return () => { cancelled = true; };
  }, [baseUrl, probeNonce]);

  // ---- v1.17.0: fix actions ------------------------------------------------
  const startOllama = async () => {
    if (fixing) return;
    setFixing(true);
    setFixMsg(null);
    try {
      const res = await commands.ollamaStart(baseUrl);
      if (res.status === "ok" && res.data) {
        setFixMsg(null);
      } else if (res.status === "ok") {
        setFixMsg(
          "Ollama launched but isn't answering yet — it should go green in a few seconds.",
        );
      } else {
        setFixMsg(res.error);
        if (res.error.toLowerCase().includes("installed")) setInstalled(false);
      }
    } catch (e) {
      setFixMsg(String(e));
    } finally {
      setFixing(false);
      setProbeNonce((n) => n + 1);
    }
  };

  const installOllama = async () => {
    setFixMsg(null);
    try {
      const res = await commands.ollamaInstall();
      if (res.status === "ok") {
        setFixMsg(
          "Installer opened in a console window — when it finishes, come back and click Start Ollama.",
        );
      } else {
        // No winget on this machine — fall back to the website.
        await openUrl("https://ollama.com").catch(() => {});
        setFixMsg(res.error);
      }
    } catch {
      await openUrl("https://ollama.com").catch(() => {});
    }
  };

  // -------------------------------------------------------------------------
  // Event listener cleanup on unmount.
  // -------------------------------------------------------------------------
  useEffect(() => {
    return () => {
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    };
  }, []);

  // Reset pull state whenever the selected model changes so the button stays
  // relevant (e.g. user switches from gemma3:4b to llama3.1:8b).
  useEffect(() => {
    setPullState("idle");
    setStatusText("");
    setProgress(null);
    setErrorMsg(null);
  }, [modelName]);

  const handlePull = useCallback(async () => {
    if (!modelName.trim() || !baseUrl.trim()) return;

    setPullState("pulling");
    setStatusText("Connecting to Ollama...");
    setProgress(null);
    setErrorMsg(null);

    // Subscribe to progress events before issuing the command so we don't
    // miss early events (manifest fetch completes almost instantly).
    const unlisten = await listen<OllamaPullProgressPayload>(
      "ollama-pull-progress",
      (event) => {
        const p = event.payload;
        setStatusText(p.status || "");
        if (p.total && p.total > 0 && p.completed !== undefined) {
          setProgress(Math.round((p.completed / p.total) * 100));
        }
      },
    );
    unlistenRef.current = unlisten;

    try {
      await commands.pullOllamaModel(baseUrl, modelName);
      setPullState("done");
      setStatusText("Model ready");
      setProgress(100);
      // After a successful pull, re-probe so the badge refreshes to "ok".
      setConnStatus("ok");
      onModelPulled?.();
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      setPullState("error");
      setErrorMsg(msg);
      setStatusText("");
    } finally {
      unlisten();
      unlistenRef.current = null;
    }
  }, [baseUrl, modelName, onModelPulled]);

  const handleReset = useCallback(() => {
    setPullState("idle");
    setStatusText("");
    setProgress(null);
    setErrorMsg(null);
  }, []);

  // Don't render if no model is selected -- nothing to pull.
  if (!modelName.trim()) return null;

  return (
    <div className="flex flex-col gap-2 pt-1">

      {/* ------------------------------------------------------------------ */}
      {/* Connection status + model storage path                              */}
      {/* ------------------------------------------------------------------ */}
      <div className="flex items-center gap-2 text-xs min-h-[1.25rem]">
        {connStatus === "checking" && (
          <span className="text-mid-gray/40 italic">Checking Ollama...</span>
        )}

        {connStatus === "ok" && (
          <>
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-400 flex-shrink-0" />
            <span className="text-mid-gray/70">
              Ollama running
              {" · "}
              models in{" "}
              <span className="font-mono text-mid-gray/90 select-all">
                %USERPROFILE%\.ollama\models\
              </span>
            </span>
          </>
        )}

        {connStatus === "offline" && (
          <>
            <span className="h-1.5 w-1.5 rounded-full bg-red-400 flex-shrink-0" />
            <span className="text-red-400/80">
              {installed === false
                ? "Ollama isn't installed on this machine"
                : "Ollama isn't running"}
              {" — "}
              <span className="font-mono">{ollamaApiBase(baseUrl)}</span>
            </span>
          </>
        )}
      </div>

      {/* v1.17.0: fix-it row — the red line is now actionable. */}
      {connStatus === "offline" && (
        <div className="flex flex-wrap items-center gap-2">
          {installed === false ? (
            <>
              <Button onClick={installOllama} variant="secondary" size="sm">
                <Download className="h-3.5 w-3.5 mr-1.5" />
                Install Ollama
              </Button>
              <button
                onClick={() => { void openUrl("https://ollama.com"); }}
                className="text-xs text-mid-gray/60 hover:text-mid-gray/90 underline-offset-2 hover:underline transition-colors"
              >
                or download from ollama.com
              </button>
            </>
          ) : (
            <Button
              onClick={startOllama}
              variant="secondary"
              size="sm"
              disabled={fixing}
            >
              {fixing ? (
                <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
              ) : (
                <Play className="h-3.5 w-3.5 mr-1.5" />
              )}
              {fixing ? "Starting Ollama..." : "Start Ollama"}
            </Button>
          )}
          {fixMsg && (
            <span className="text-xs text-mid-gray/70">{fixMsg}</span>
          )}
        </div>
      )}

      {/* ------------------------------------------------------------------ */}
      {/* Pull button row + inline status text                                */}
      {/* ------------------------------------------------------------------ */}
      <div className="flex items-center gap-3">
        {pullState === "idle" && (
          <Button
            onClick={handlePull}
            variant="secondary"
            size="sm"
            disabled={connStatus !== "ok"}
          >
            <Download className="h-3.5 w-3.5 mr-1.5" />
            Pull {modelName}
          </Button>
        )}

        {pullState === "pulling" && (
          <Button variant="secondary" size="sm" disabled>
            <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
            Pulling...
          </Button>
        )}

        {pullState === "done" && (
          <>
            <div className="flex items-center gap-1.5 text-emerald-400 text-xs font-medium">
              <CheckCircle2 className="h-3.5 w-3.5 flex-shrink-0" />
              <span>Ready</span>
            </div>
            <button
              onClick={handleReset}
              className="text-xs text-mid-gray/60 hover:text-mid-gray/90 underline-offset-2 hover:underline transition-colors"
            >
              Pull again
            </button>
          </>
        )}

        {pullState === "error" && (
          <>
            <div className="flex items-center gap-1.5 text-red-400 text-xs font-medium">
              <AlertCircle className="h-3.5 w-3.5 flex-shrink-0" />
              <span className="truncate max-w-[260px]">
                {errorMsg || "Pull failed"}
              </span>
            </div>
            <button
              onClick={handleReset}
              className="text-xs text-mid-gray/60 hover:text-mid-gray/90 underline-offset-2 hover:underline transition-colors flex-shrink-0"
            >
              Retry
            </button>
          </>
        )}

        {pullState === "pulling" && statusText && (
          <span className="text-xs text-mid-gray/60 truncate">
            {statusText}
            {progress !== null && ` -- ${progress}%`}
          </span>
        )}
      </div>

      {/* Progress bar -- only during pull when we have a concrete percentage */}
      {pullState === "pulling" && progress !== null && (
        <div className="h-1 bg-mid-gray/20 rounded-full overflow-hidden">
          <div
            className="h-full bg-cyan-400/70 rounded-full transition-all duration-300"
            style={{ width: `${progress}%` }}
          />
        </div>
      )}
    </div>
  );
};
