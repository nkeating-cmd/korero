/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, ChevronDown, Cpu, Download, HardDrive, Loader2, MemoryStick, MonitorSmartphone, X } from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { CATALOGUE, type CatalogueEntry } from "../../../lib/modelCatalogue";
import { fitFor, gpuSummary, recommend, sortByFit, type Fit, type ProfileLike } from "../../../lib/modelFit";
import { etaSeconds, ok, useOllamaJob, type Step } from "../../../stores/ollamaJobStore";
import { isLocalInUse, isLoopbackUrl } from "../../shell/inUseModel";

/**
 * Kōrero 1.43: "Run AI clean-up on this computer".
 *
 * Reads this computer's graphics memory, memory and free disk, offers ONE
 * model that fits, and one button that downloads it, tests it and switches
 * clean-up to it. Everything else is one click further, labelled with how
 * well it fits. The download is a job in `ollamaJobStore`, so it carries on
 * if you leave the page.
 */

const FIT_CHIP: Record<Fit, { cls: string; label: string }> = {
  great: { cls: "kx-chip-ok", label: "Fits well" },
  tight: { cls: "kx-chip-accent", label: "Fits" },
  cpu: { cls: "kx-chip-warn", label: "Slower" },
  no: { cls: "", label: "Won't fit" },
};

const STEP_TEXT: Record<Step, string> = {
  idle: "",
  starting: "Starting Ollama…",
  pulling: "Downloading…",
  testing: "Testing the model…",
  switching: "Switching clean-up to it…",
  done: "Ready",
  error: "Didn't finish",
  cancelled: "Download cancelled",
};

const BUSY: Step[] = ["starting", "pulling", "testing", "switching"];

const gb = (mb: number) => `${mb >= 10 * 1024 ? Math.round(mb / 1024) : (mb / 1024).toFixed(1)} GB`;
const bytesGb = (b: number) => `${(b / 1e9).toFixed(1)} GB`;
const mins = (s: number | null) => (s === null ? "" : s < 60 ? "under a minute left" : `about ${Math.round(s / 60)} min left`);

export const LocalModelSetup: React.FC = () => {
  const { settings, postProcessModelOptions, fetchPostProcessModels } = useSettings();
  const job = useOllamaJob();
  const [profile, setProfile] = useState<ProfileLike | null>(null);
  const [profileFailed, setProfileFailed] = useState(false);
  const [ollama, setOllama] = useState<{ installed: boolean; running: boolean } | null>(null);
  const [showAll, setShowAll] = useState(false);
  const [fixing, setFixing] = useState(false);
  // Keep keyboard focus somewhere sensible as buttons appear and vanish (A11Y-01).
  const primaryRef = useRef<HTMLButtonElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const dismissRef = useRef<HTMLButtonElement>(null);
  const focusLost = () => {
    const a = document.activeElement as HTMLButtonElement | null;
    return !a || a === document.body || a.disabled;
  };
  const focusLater = (r: React.RefObject<HTMLButtonElement | null>) => setTimeout(() => r.current?.focus(), 0);

  const ollamaProvider = settings?.post_process_providers?.find((p) => p.id === "ollama");
  const baseUrl = ollamaProvider?.base_url ?? "http://localhost:11434/v1";
  const local = isLoopbackUrl(baseUrl);
  const installed = postProcessModelOptions["ollama"] ?? [];
  const busy = BUSY.includes(job.step);

  useEffect(() => {
    commands
      .getMachineProfile()
      .then((r) => setProfile(ok(r)))
      .catch(() => setProfileFailed(true));
  }, []);

  const probe = () =>
    commands
      .ollamaStatus(baseUrl)
      .then((s) => {
        setOllama({ installed: s.installed, running: s.running });
        if (s.running) void fetchPostProcessModels("ollama").catch(() => {});
      })
      .catch(() => setOllama(null));
  useEffect(() => {
    void probe();
  }, [baseUrl, job.step === "done"]);

  useEffect(() => {
    if (job.step === "pulling" && focusLost()) focusLater(cancelRef);
    if ((job.step === "done" || job.step === "error" || job.step === "cancelled") && focusLost()) focusLater(dismissRef);
  }, [job.step]);

  const rec = useMemo(() => (profile ? recommend(CATALOGUE, profile) : null), [profile]);
  const others = useMemo(
    () => (profile ? sortByFit(CATALOGUE, profile).filter((e) => e.tag !== rec?.tag) : []),
    [profile, rec],
  );

  if (profileFailed || !profile) return null;

  const startOllama = async () => {
    setFixing(true);
    try {
      ok(await commands.ollamaStart(baseUrl));
    } catch (e) {
      toast.error("Ollama didn't start", { description: String(e instanceof Error ? e.message : e) });
    } finally {
      setFixing(false);
      void probe();
      focusLater(primaryRef);
    }
  };
  const installOllama = async () => {
    try {
      ok(await commands.ollamaInstall());
      toast.message("The Ollama installer opened in a console window. When it finishes, press Check again.");
    } catch {
      await openUrl("https://ollama.com/download").catch(() => {});
    }
  };
  const go = (tag: string) => {
    if (!useOllamaJob.getState().start(tag)) toast.message("A download is already running.");
  };

  const action = (e: CatalogueEntry, primary: boolean) => {
    const inUse = isLocalInUse(settings, e.tag);
    const have = installed.includes(e.tag);
    // Already downloaded: free disk space no longer matters (refuter DEF-06).
    const f = fitFor(e, have ? { ...profile, free_disk_mb: null } : profile);
    const disabled = f.fit === "no" || busy || inUse || !local || !ollama?.installed;
    const cls = primary ? "kx-btn kx-btn-primary shrink-0" : "kx-btn kx-btn-secondary kx-btn-sm shrink-0 w-[112px] justify-center";
    return (
      <button type="button" ref={primary ? primaryRef : undefined} className={cls} disabled={disabled} onClick={() => go(e.tag)}>
        {inUse ? (
          <>
            <Check size={14} aria-hidden="true" /> In use
          </>
        ) : have ? (
          "Use"
        ) : (
          <>
            <Download size={14} aria-hidden="true" /> {primary ? "Download and use" : "Download"}
          </>
        )}
      </button>
    );
  };

  const eta = etaSeconds(job.progress);
  const pct = job.progress.totalBytes > 0 ? Math.min(100, Math.round((job.progress.doneBytes / job.progress.totalBytes) * 100)) : 0;
  const share = job.result?.gpu_share;

  return (
    <div className="kx-card p-5 flex flex-col gap-4">
      <div>
        <h3 className="text-[15px] font-semibold text-[var(--kx-ink)]">Run AI clean-up on this computer</h3>
        <p className="kx-meta mt-1">
          {local
            ? "Kōrero picks a model that fits your hardware. It runs on this computer: nothing you say leaves it."
            : `Ollama is set to ${baseUrl}, which isn't this computer. Change the address under "Use a different provider or model" first.`}
        </p>
      </div>

      <div className="flex flex-wrap gap-x-5 gap-y-1.5 text-[12.5px] text-[var(--kx-ink-soft)]">
        <span className="inline-flex items-center gap-1.5">
          <MonitorSmartphone size={14} className="text-[var(--kx-ink-2)]" aria-hidden="true" />
          {gpuSummary(profile)}
        </span>
        <span className="inline-flex items-center gap-1.5">
          <MemoryStick size={14} className="text-[var(--kx-ink-2)]" aria-hidden="true" />
          {gb(profile.ram_mb)} memory
        </span>
        {profile.free_disk_mb !== null && (
          <span className="inline-flex items-center gap-1.5">
            <HardDrive size={14} className="text-[var(--kx-ink-2)]" aria-hidden="true" />
            {Math.round(profile.free_disk_mb / 1024)} GB free
          </span>
        )}
      </div>

      {ollama && !ollama.installed && (
        <div className="kx-banner kx-banner-info flex items-center gap-3">
          <span className="flex-1 text-[13px]">
            Local models run in Ollama, a free app. Install it once (about 1 GB), then come back here.
          </span>
          <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => void installOllama()}>
            Install Ollama
          </button>
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => void probe()}>
            Check again
          </button>
        </div>
      )}
      {ollama?.installed && !ollama.running && !busy && (
        <div className="kx-banner kx-banner-warn flex items-center gap-3">
          <span className="flex-1 text-[13px]">Ollama is installed but not running.</span>
          <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" disabled={fixing} onClick={() => void startOllama()}>
            {fixing ? <Loader2 size={14} className="animate-spin" aria-hidden="true" /> : null} Start Ollama
          </button>
        </div>
      )}

      {rec ? (
        <div className="rounded-[10px] border border-[var(--kx-accent)] bg-[var(--kx-selected)]/40 p-4">
          <div className="kx-overline !text-[var(--kx-accent-ink)]">Recommended for this computer</div>
          <div className="flex items-center gap-3 mt-2">
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="kx-mono text-[15px] text-[var(--kx-ink)]">{rec.tag}</span>
                <span className={`kx-chip ${FIT_CHIP[fitFor(rec, profile).fit].cls}`}>{FIT_CHIP[fitFor(rec, profile).fit].label}</span>
              </div>
              <p className="kx-meta mt-1">
                {rec.blurb} {gb(rec.downloadMb)} download.
              </p>
            </div>
            {action(rec, true)}
          </div>
        </div>
      ) : (
        <div className="kx-banner kx-banner-info text-[13px]">
          This computer doesn't have enough memory for a local model that would run well. You can still use a cloud
          provider under "Use a different provider or model"; it sends text off this computer.
        </div>
      )}

      {job.step !== "idle" && job.model && (
        <div className="rounded-[10px] border border-[var(--kx-hairline)] p-4 flex flex-col gap-2">
          <div className="flex items-center gap-2">
            {busy ? (
              <Loader2 size={14} className="animate-spin text-[var(--kx-accent-ink)]" aria-hidden="true" />
            ) : job.step === "done" ? (
              <Check size={14} className="text-[var(--kx-ok)]" aria-hidden="true" />
            ) : (
              <X size={14} className="text-[var(--kx-warn)]" aria-hidden="true" />
            )}
            <span className="flex-1 text-[13px] text-[var(--kx-ink-read)]">
              <span className="kx-mono">{job.model}</span> · {STEP_TEXT[job.step]}
            </span>
            {job.step === "pulling" && (
              <button ref={cancelRef} type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => job.cancel()}>
                Cancel
              </button>
            )}
            {!busy && (
              <button
                ref={dismissRef}
                type="button"
                className="kx-btn kx-btn-quiet kx-btn-sm"
                onClick={() => {
                  job.reset();
                  focusLater(primaryRef);
                }}
                aria-label="Dismiss"
              >
                <X size={14} aria-hidden="true" />
              </button>
            )}
          </div>
          {job.step === "pulling" && (
            <>
              <div
                className="kx-progress"
                role="progressbar"
                aria-label={`Downloading ${job.model}`}
                aria-valuenow={pct}
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <span style={{ width: `${pct}%` }} />
              </div>
              <p className="kx-meta">
                {job.progress.totalBytes > 0
                  ? `${bytesGb(job.progress.doneBytes)} of ${bytesGb(job.progress.totalBytes)}${eta !== null ? ` · ${mins(eta)}` : ""} · you can keep working`
                  : "Connecting to Ollama…"}
              </p>
            </>
          )}
          {/* Step changes and outcomes are announced; percentages are not (A11Y-02). */}
          <div aria-live="polite" className="kx-sr-only">
            {`${job.model}: ${STEP_TEXT[job.step]}.${job.step === "error" && job.error ? ` ${job.error}` : ""}`}
          </div>
          {job.step === "done" && job.result && (
            <p className="kx-meta">
              Cleaned a test sentence in {job.result.seconds.toFixed(1)} s
              {share === null || share === undefined
                ? "."
                : share >= 0.9
                  ? ", fully on your graphics card."
                  : `, with about ${Math.round((1 - share) * 100)}% running on the processor.`}
              {job.note ? ` ${job.note}` : ""}
            </p>
          )}
          {job.step === "error" && job.error && (
            <p className="kx-meta text-[var(--kx-warn-ink)] break-words">{job.error}</p>
          )}
        </div>
      )}

      <div>
        <button
          type="button"
          className="kx-btn kx-btn-quiet kx-btn-sm -ml-2.5"
          aria-expanded={showAll}
          onClick={() => setShowAll((v) => !v)}
        >
          <Cpu size={14} aria-hidden="true" /> {showAll ? "Hide other models" : `Other models (${others.length})`}
          <ChevronDown size={14} className={showAll ? "rotate-180" : ""} aria-hidden="true" />
        </button>
        {showAll && (
          <ul className="mt-1">
            {others.map((e) => {
              const f = fitFor(e, profile);
              return (
                <li key={e.tag} className="flex items-center gap-3 py-2.5 border-t border-[var(--kx-hairline-soft)] first:border-t-0">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="kx-mono text-[13px] text-[var(--kx-ink-read)]">{e.tag}</span>
                      <span className={`kx-chip ${FIT_CHIP[f.fit].cls}`}>{FIT_CHIP[f.fit].label}</span>
                      {isLocalInUse(settings, e.tag) && <span className="kx-chip kx-chip-accent">In use</span>}
                    </div>
                    <p className="kx-meta mt-0.5">
                      {e.blurb}
                      {f.fit !== "great" && <span className="text-[var(--kx-ink-2)]"> · {f.reason}</span>}
                    </p>
                  </div>
                  <span className="kx-meta shrink-0 w-[60px] text-right">{gb(e.downloadMb)}</span>
                  {action(e, false)}
                </li>
              );
            })}
          </ul>
        )}
        <p className="kx-meta mt-2 text-[11.5px]">
          Suggestions are provisional: they haven't yet been tested on Kōrero's clean-up prompts.
        </p>
      </div>
    </div>
  );
};
