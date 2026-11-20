/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Check, CircleDashed, Copy, Loader2, Mail, RefreshCw, Stethoscope, TriangleAlert, X } from "lucide-react";
import { toast } from "sonner";
import { commands, type CheckResult, type Fix } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { useLocalAi } from "../../../stores/localAiStore";
import { useOllamaJob } from "../../../stores/ollamaJobStore";
import { firstProblem, fixOffer, headline, reportMailto } from "../../../lib/doctor";

/**
 * Kōrero 1.49 (B10): "Check local AI".
 *
 * Says in plain words whether local clean-up works and, if not, what to do
 * about the first problem: one button when Kōrero can fix it (starting
 * something runs at once; anything that downloads or changes a setting says
 * so on the button), or plain steps when it can't. The checks come from
 * Rust (`local_ai_check`); the report can be copied, or sent to Kyt in an
 * email the person reads first.
 */

const StatusIcon: React.FC<{ status: CheckResult["status"] }> = ({ status }) => {
  switch (status) {
    case "pass":
      return <Check size={15} className="text-[var(--kx-ok)] shrink-0 mt-[2px]" aria-label="Passed" role="img" />;
    case "warn":
      return <TriangleAlert size={15} className="text-[var(--kx-warn)] shrink-0 mt-[2px]" aria-label="Warning" role="img" />;
    case "fail":
      return <X size={15} className="text-[var(--kx-warn)] shrink-0 mt-[2px]" aria-label="Problem" role="img" />;
    default:
      return <CircleDashed size={15} className="text-[var(--kx-ink-2)] shrink-0 mt-[2px]" aria-label="Not checked" role="img" />;
  }
};

export const LocalAiDoctor: React.FC = () => {
  const { settings, updateSetting } = useSettings();
  const {
    report,
    checking,
    checkError,
    check,
    addRuntime,
    step,
    doneBytes,
    totalBytes,
    cancelRuntime,
    runtime,
    refreshRuntime,
    followInstall,
    repairRuntime,
    stopUsingRuntime,
    removeRuntime,
  } = useLocalAi();
  const modelJob = useOllamaJob();
  const [fixing, setFixing] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const prevJobStep = useRef(modelJob.step);
  const prevRuntimeStep = useRef(step);
  const fixRef = useRef<HTMLButtonElement>(null);
  const baseUrl =
    settings?.post_process_providers?.find((p) => p.id === "ollama")?.base_url ?? "http://localhost:11434/v1";

  useEffect(() => {
    if (!report && !checking) void check(false);
    // A download started before this page loaded (a reload, another window)
    // is shown and followed rather than reported as an error.
    void refreshRuntime().then((s) => {
      if (s?.installing) void followInstall();
    });
  }, []);
  // A model download from the card above ends in a full check: "set up"
  // means checked working, not just downloaded. Only when it has just
  // finished: a finished job stays "done", and every visit would otherwise
  // load the model again (review).
  useEffect(() => {
    if (modelJob.step === "done" && prevJobStep.current !== "done") {
      void check(true);
      void refreshRuntime(); // the models now take more space
    }
    prevJobStep.current = modelJob.step;
  }, [modelJob.step]);
  // Kōrero's local AI added from the setup card: check again, or the page
  // keeps showing the problem it had before (UAT, 18 Nov 2026).
  useEffect(() => {
    if (step === "done" && prevRuntimeStep.current !== "done") void check(false);
    prevRuntimeStep.current = step;
  }, [step]);

  const problem = report ? firstProblem(report.checks) : null;
  const offer = problem?.fix ? fixOffer(problem.fix) : null;
  const head = report ? headline(report.verdict) : null;
  const downloading = step === "downloading" || step === "starting";
  const pct = totalBytes > 0 ? Math.min(100, Math.round((doneBytes / totalBytes) * 100)) : 0;

  const runFix = async (f: Fix) => {
    setFixing(true);
    try {
      switch (f.kind) {
        case "start_runtime":
        case "start_ollama": {
          const r = await commands.ollamaStart(baseUrl);
          if (r.status === "error") toast.error("It didn't start", { description: r.error });
          else if (!r.data) toast.error("It started but isn't answering yet. Try the check again in a moment.");
          break;
        }
        case "turn_on_cleanup":
          await updateSetting("post_process_enabled", true);
          break;
        case "install_runtime":
        case "update_runtime":
          if (!(await addRuntime())) {
            const s = useLocalAi.getState();
            if (s.step === "error" && s.error) toast.error("Kōrero's local AI didn't install", { description: s.error });
          }
          break;
        case "use_runtime": {
          const r = await commands.localAiRuntimeUse();
          if (r.status === "error") toast.error("Couldn't switch to Kōrero's local AI", { description: r.error });
          await useSettingsStore.getState().refreshSettings();
          break;
        }
        case "download_model":
          if (!useOllamaJob.getState().start(f.model)) toast.message("A download is already running.");
          return; // the full check runs when the download job finishes
        default:
          break;
      }
      await check(false);
    } finally {
      setFixing(false);
      setTimeout(() => fixRef.current?.focus(), 0);
    }
  };

  const manage = async (run: () => Promise<boolean>, failed: string) => {
    setFixing(true);
    try {
      if (!(await run())) {
        const e = useLocalAi.getState().error;
        if (e) toast.error(failed, { description: e });
      }
      await check(false);
    } finally {
      setFixing(false);
      setConfirmRemove(false);
    }
  };
  const hasRuntime = !!runtime?.supported && (runtime.installed.length > 0 || runtime.update_needed);
  const diskText = runtime
    ? runtime.disk_mb >= 1024
      ? `${(runtime.disk_mb / 1024).toFixed(1)} GB`
      : `${runtime.disk_mb} MB`
    : "";

  const copyReport = async () => {
    if (!report) return;
    try {
      await writeText(report.report_text);
      toast.success("Report copied. It holds no dictation, names or keys.");
    } catch {
      toast.error("Couldn't copy the report.");
    }
  };

  return (
    <div className="kx-card p-5 flex flex-col gap-4" aria-busy={checking}>
      <div className="flex items-start gap-3">
        <Stethoscope size={18} className="text-[var(--kx-ink-2)] mt-[2px]" aria-hidden="true" />
        <div className="flex-1 min-w-0">
          <h3 className="text-[15px] font-semibold text-[var(--kx-ink)]">Check local AI</h3>
          <p className="kx-meta mt-1" aria-live="polite">
            {checking
              ? "Checking…"
              : checkError
                ? checkError
                : head
                  ? head.text
                  : "Find out whether clean-up on this computer works, and what to do if it doesn't."}
          </p>
        </div>
        <div className="flex gap-2 shrink-0">
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" disabled={checking} onClick={() => void check(false)}>
            <RefreshCw size={14} aria-hidden="true" /> Check again
          </button>
          <button
            type="button"
            className="kx-btn kx-btn-secondary kx-btn-sm"
            disabled={checking}
            onClick={() => void check(true)}
            title="Also tests a real clean-up of an invented sentence. Up to 3 minutes if the model has to load first."
          >
            {checking ? <Loader2 size={14} className="animate-spin" aria-hidden="true" /> : null} Full check
          </button>
        </div>
      </div>

      {problem && (
        <div
          className={`rounded-[10px] border p-4 flex flex-col gap-2 ${
            problem.status === "fail" ? "border-[var(--kx-warn)]" : "border-[var(--kx-hairline)]"
          }`}
        >
          <div className="text-[14px] font-medium text-[var(--kx-ink)]">{problem.title}</div>
          <p className="text-[13px] text-[var(--kx-ink-read)]">{problem.detail}</p>
          {offer?.help && <p className="kx-meta">{offer.help}</p>}
          {downloading && (problem.fix?.kind === "install_runtime" || problem.fix?.kind === "update_runtime") && (
            <>
              <div
                className="kx-progress"
                role="progressbar"
                aria-label="Downloading Kōrero's local AI"
                aria-valuenow={pct}
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <span style={{ width: `${pct}%` }} />
              </div>
              <p className="kx-meta">
                {step === "starting"
                  ? "Starting it…"
                  : totalBytes > 0
                    ? `${Math.round(doneBytes / 1048576)} of ${Math.round(totalBytes / 1048576)} MB · every file is checked`
                    : "Connecting…"}
              </p>
            </>
          )}
          {offer && offer.safety !== "guide" && problem.fix && (
            <div className="flex gap-2">
              <button
                ref={fixRef}
                type="button"
                className="kx-btn kx-btn-primary kx-btn-sm"
                disabled={fixing || checking || downloading}
                onClick={() => void runFix(problem.fix as Fix)}
              >
                {fixing ? <Loader2 size={14} className="animate-spin" aria-hidden="true" /> : null} {offer.button}
              </button>
              {step === "downloading" && (
                <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => void cancelRuntime()}>
                  Cancel
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {report && (
        <ul className="flex flex-col" aria-label="What was checked">
          {report.checks.map((c) => (
            <li key={c.id} className="flex items-start gap-2.5 py-1.5 border-t border-[var(--kx-hairline-soft)] first:border-t-0">
              <StatusIcon status={c.status} />
              <div className="flex-1 min-w-0">
                <span className="text-[13px] text-[var(--kx-ink-read)]">{c.title}</span>
                <span className="kx-meta"> · {c.detail}</span>
              </div>
            </li>
          ))}
        </ul>
      )}

      {hasRuntime && runtime && (
        <div className="flex flex-wrap gap-2 items-center border-t border-[var(--kx-hairline-soft)] pt-3" aria-label="Kōrero's local AI">
          <span className="kx-meta flex-1 min-w-[12rem]">
            Kōrero's local AI (Ollama {runtime.version}) · {diskText} with its models ·{" "}
            {runtime.in_use ? "clean-up uses it" : "not in use"}
            {runtime.update_needed ? " · an update is waiting to download" : ""}
          </span>
          {!confirmRemove ? (
            <>
              <button
                type="button"
                className="kx-btn kx-btn-quiet kx-btn-sm"
                disabled={fixing || downloading}
                title="Checks every file again and replaces any that is damaged"
                onClick={() => void manage(repairRuntime, "The repair didn't finish")}
              >
                Repair
              </button>
              {runtime.in_use && (
                <button
                  type="button"
                  className="kx-btn kx-btn-quiet kx-btn-sm"
                  disabled={fixing || downloading}
                  title="Puts clean-up back the way it was before; keeps the files"
                  onClick={() =>
                    void manage(async () => {
                      const ok = await stopUsingRuntime();
                      if (ok) {
                        // Say where clean-up goes now: it may be a cloud provider again.
                        const s = useSettingsStore.getState().settings;
                        const p = s?.post_process_providers?.find((x) => x.id === s?.post_process_provider_id);
                        toast.success(
                          s?.post_process_enabled
                            ? `Clean-up is back to ${p?.label ?? "the provider you used before"}.`
                            : "Kōrero's local AI is stopped, and clean-up is off as it was before.",
                        );
                      }
                      return ok;
                    }, "Couldn't switch back")
                  }
                >
                  Stop using it
                </button>
              )}
              <button
                type="button"
                className="kx-btn kx-btn-quiet kx-btn-sm"
                disabled={fixing || downloading}
                onClick={() => setConfirmRemove(true)}
              >
                Remove…
              </button>
            </>
          ) : (
            <>
              <span className="text-[13px] text-[var(--kx-ink-read)]">
                Remove Kōrero's local AI and its models? This frees {diskText}.
              </span>
              <button
                type="button"
                className="kx-btn kx-btn-primary kx-btn-sm"
                disabled={fixing}
                onClick={() => void manage(removeRuntime, "Couldn't remove it all")}
              >
                Remove
              </button>
              <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => setConfirmRemove(false)}>
                Keep it
              </button>
            </>
          )}
        </div>
      )}

      {report && (
        <div className="flex flex-wrap gap-2 items-center">
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm -ml-2.5" onClick={() => void copyReport()}>
            <Copy size={14} aria-hidden="true" /> Copy report
          </button>
          <button
            type="button"
            className="kx-btn kx-btn-quiet kx-btn-sm"
            onClick={() => void openUrl(reportMailto(report)).catch(() => toast.error("Couldn't open your email program."))}
          >
            <Mail size={14} aria-hidden="true" /> Send report to Kyt
          </button>
          <span className="kx-meta text-[11.5px]">Opens an email for you to read and send. It holds no dictation, names or keys.</span>
        </div>
      )}
    </div>
  );
};
