/* eslint-disable i18next/no-literal-string */
import React, { useState } from "react";
import { toast } from "sonner";
import { Plus, Trash2, Crosshair, Loader2 } from "lucide-react";
import { commands } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { Dropdown } from "@/components/ui/Dropdown";

/**
 * Kōrero (v1.22.0, P2): per-app prompt routing.
 *
 * When you dictate-and-clean-up, the post-processing prompt is normally the one
 * globally selected above. Here you can add rules so that, when a particular app
 * is in focus, a different prompt is used automatically — e.g. focus Slack and
 * the cleanup uses your chat prompt; focus Mail and it uses the email prompt.
 *
 * Rules are stored as "title_substring=prompt_id" strings in
 * settings.post_process_app_routes (matched case-insensitively against the
 * foreground window title). An empty list means routing is off, so this never
 * changes behaviour until you add a rule. State lives in settings, so it
 * survives tab switches; writes go through the dedicated command.
 */
export const PostProcessAppRouting: React.FC = () => {
  const { getSetting, refreshSettings } = useSettings();

  const routes: string[] = (getSetting("post_process_app_routes") as string[]) || [];
  const prompts: { id: string; name: string }[] =
    (getSetting("post_process_prompts") as { id: string; name: string }[]) || [];

  const [matchText, setMatchText] = useState("");
  const [promptId, setPromptId] = useState("");
  const [busy, setBusy] = useState(false);
  const [detecting, setDetecting] = useState(false);

  const promptName = (id: string) =>
    prompts.find((p) => p.id === id)?.name ?? id;

  const save = async (next: string[]) => {
    setBusy(true);
    try {
      const r = await commands.setPostProcessAppRoutes(next);
      if (r.status !== "ok") throw new Error(r.error);
      await refreshSettings();
    } catch (e) {
      toast.error(`Could not save routing: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const addRoute = async () => {
    const m = matchText.trim();
    const pid = promptId || prompts[0]?.id || "";
    if (!m) {
      toast.message("Type or detect an app keyword first.");
      return;
    }
    if (m.includes("=")) {
      toast.message("The app keyword can't contain an '=' sign.");
      return;
    }
    if (!pid) {
      toast.message("Pick a prompt.");
      return;
    }
    await save([...routes, `${m}=${pid}`]);
    setMatchText("");
  };

  const removeRoute = async (idx: number) => {
    await save(routes.filter((_, i) => i !== idx));
  };

  // Detect the focused app's window title (reusing the existing spike command).
  // 3-second delay so you can click here, then focus the target app.
  const detectApp = () => {
    setDetecting(true);
    toast.message("Focus the target app now — detecting in 3 seconds…");
    setTimeout(async () => {
      try {
        const r = await commands.getActiveWindowTitle();
        if (r.status === "ok") {
          // Suggest the last " - X" segment (usually the app name), else the
          // whole title — the user can trim it to a stable keyword.
          const title = r.data;
          const parts = title.split(/\s[-–—|]\s/);
          setMatchText((parts[parts.length - 1] || title).trim());
          toast.success(`Detected: ${title}`);
        } else {
          toast.error(`Couldn't detect the app: ${r.error}`);
        }
      } catch (e) {
        toast.error(`Couldn't detect the app: ${String(e)}`);
      } finally {
        setDetecting(false);
      }
    }, 3000);
  };

  const promptOptions = prompts.map((p) => ({ value: p.id, label: p.name }));
  const inputClass =
    "min-w-0 flex-1 rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-1.5 text-sm text-text placeholder:text-text-subtle focus:outline-none disabled:opacity-50";

  return (
    <div className="space-y-3 px-4 py-3">
      <p className="text-sm leading-relaxed text-text-muted">
        Use a different cleanup prompt automatically depending on which app is in
        focus when you dictate. Matched on the window title — a short keyword like{" "}
        <span className="font-medium text-text">Slack</span>,{" "}
        <span className="font-medium text-text">Gmail</span> or{" "}
        <span className="font-medium text-text">Word</span> works best. No rules =
        routing off (your selected prompt is always used).
      </p>

      {/* Existing rules */}
      {routes.length > 0 && (
        <div className="space-y-2">
          {routes.map((entry, idx) => {
            const eq = entry.indexOf("=");
            const m = eq >= 0 ? entry.slice(0, eq) : entry;
            const pid = eq >= 0 ? entry.slice(eq + 1) : "";
            return (
              <div
                key={idx}
                className="flex items-center gap-2 rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-2 text-sm"
              >
                <span className="font-medium text-text">{m}</span>
                <span className="text-text-subtle">→</span>
                <span className="text-text-muted">{promptName(pid)}</span>
                <button
                  type="button"
                  onClick={() => removeRoute(idx)}
                  disabled={busy}
                  title="Remove this rule"
                  className="ml-auto text-text-subtle transition-colors hover:text-pill-warning disabled:opacity-50"
                >
                  <Trash2 size={14} />
                </button>
              </div>
            );
          })}
        </div>
      )}

      {/* Add a rule */}
      <div className="flex flex-wrap items-center gap-2">
        <input
          value={matchText}
          onChange={(e) => setMatchText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") addRoute();
          }}
          placeholder="App keyword (e.g. Slack)"
          aria-label="App keyword"
          disabled={busy}
          className={inputClass}
        />
        <button
          type="button"
          onClick={detectApp}
          disabled={detecting || busy}
          title="Detect the focused app (3-second delay)"
          className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-glass-border bg-glass-surface-thin px-2.5 py-1.5 text-xs text-text-muted transition-colors hover:text-aurora-cyan disabled:opacity-50"
        >
          {detecting ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <Crosshair size={14} />
          )}
          {detecting ? "Detecting…" : "Detect"}
        </button>
        <div className="shrink-0">
          <Dropdown
            options={promptOptions}
            selectedValue={promptId || prompts[0]?.id || ""}
            onSelect={setPromptId}
          />
        </div>
        <button
          type="button"
          onClick={addRoute}
          disabled={busy || !matchText.trim()}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-aurora-cyan/40 bg-aurora-cyan/15 px-2.5 py-1.5 text-xs font-medium text-aurora-cyan transition-colors hover:bg-aurora-cyan/25 disabled:opacity-50"
        >
          <Plus size={14} /> Add rule
        </button>
      </div>
    </div>
  );
};
