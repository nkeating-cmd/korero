/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import {
  Circle,
  Pause,
  Play,
  Square,
  Flag,
  ExternalLink,
  Sparkles,
  Loader2,
  TriangleAlert,
  X,
  Check,
  Activity,
} from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { Markdown } from "../ui/Markdown";
import { useRecorder } from "../../stores/recorderStore";
import { elapsedOf } from "../../stores/recorderProtocol";
import { useMeetingJobs } from "../../stores/meetingJobsStore";
import { LiveMeters } from "./LiveMeters";
import { fmtClock } from "./model";

/**
 * Kōrero 1.42: the meeting while it records (A3 on the design canvas).
 * Everything here reads the recorder store, so leaving and coming back — or
 * driving the meeting from the pop-out companion — keeps it in step.
 */

const useClock = (active: boolean) => {
  const [, set] = useState(0);
  useEffect(() => {
    if (!active) return;
    const t = window.setInterval(() => set((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [active]);
};

export const LiveMeetingView: React.FC = () => {
  const recording = useRecorder((s) => s.recording);
  const paused = useRecorder((s) => s.paused);
  const starting = useRecorder((s) => s.starting);
  const testing = useRecorder((s) => s.testing);
  const elapsedBase = useRecorder((s) => s.elapsedBase);
  const runStartedAt = useRecorder((s) => s.runStartedAt);
  const title = useRecorder((s) => s.title);
  const live = useRecorder((s) => s.live);
  const flags = useRecorder((s) => s.flags);
  const warnings = useRecorder((s) => s.warnings);
  const devices = useRecorder((s) => s.devices);
  const systemCaptured = useRecorder((s) => s.systemCaptured);
  const captureError = useRecorder((s) => s.captureError);
  const stopping = useMeetingJobs((s) => s.stopping);
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<{ q: string; a: string } | null>(null);
  const [asking, setAsking] = useState(false);
  const transcriptRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  useClock(recording);

  const elapsed = elapsedOf({ elapsedBase, runStartedAt });
  const startedAt = Date.now() - elapsed * 1000;

  // F flags a moment, when you are not typing in a field.
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing = el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
      if (!typing && !e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        useRecorder.getState().flag();
        toast.success("Moment flagged.", { duration: 1500 });
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [recording]);

  // Follow the transcript as it grows, unless you have scrolled up to read.
  useEffect(() => {
    const el = transcriptRef.current;
    if (el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, [live.length]);

  const askNow = async () => {
    const q = question.trim();
    if (!q || asking) return;
    const transcript = live.map((s) => `${s.source === "you" ? "You" : "Others"}: ${s.text}`).join("\n");
    if (!transcript.trim()) {
      toast.message("Nothing has been transcribed yet.");
      return;
    }
    setAsking(true);
    try {
      const res = await commands.meetingQuery(transcript, q);
      if (res.status === "ok") setAnswer({ q, a: res.data });
      else toast.error(res.error);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setAsking(false);
    }
  };

  // ---- Saving, after Stop -------------------------------------------------
  if (!recording && stopping) {
    return (
      <div className="kx-page" style={{ maxWidth: 760 }}>
        <p className="kx-overline">Just now</p>
        <h1 className="kx-title mt-1">Saving the meeting</h1>
        <p className="kx-meta mt-1 mb-6">
          The audio is already on disk. You can leave this page; the meeting appears in your
          library when it is ready.
        </p>
        <ol className="flex flex-col gap-4" aria-label="Progress">
          <li className="flex gap-3.5 items-start">
            <span className="kx-step kx-step-done" aria-label="Done">
              <Check size={14} />
            </span>
            <div>
              <div className="kx-heading">Audio saved</div>
              <div className="kx-meta">Your mic and the call, as separate recordings</div>
            </div>
          </li>
          <li className="flex gap-3.5 items-start">
            <span className="kx-step kx-step-running" aria-label="Working">
              <Loader2 size={14} className="animate-spin" />
            </span>
            <div className="flex-1">
              <div className="kx-heading">Finishing the transcript</div>
              <div className="kx-meta">
                Anything live transcription missed is rebuilt from the recording. Long meetings take a minute or two.
              </div>
              <div className="kx-progress kx-progress-indeterminate mt-2.5 max-w-[420px]">
                <span />
              </div>
            </div>
          </li>
        </ol>
      </div>
    );
  }

  // ---- Not recording ------------------------------------------------------
  if (!recording) {
    return (
      <div className="kx-page" style={{ maxWidth: 760 }}>
        <h1 className="kx-title">Record a meeting</h1>
        <p className="kx-meta mt-1 mb-5">
          Kōrero records your microphone (You) and the call's audio (Others) as separate files,
          saved to disk as it goes, so a crash never loses a meeting. Everything stays on this
          computer.
        </p>
        <div className="flex flex-wrap gap-2.5">
          <button
            type="button"
            className="kx-btn kx-btn-primary"
            disabled={starting || testing}
            onClick={() => void useRecorder.getState().start()}
          >
            {starting ? <Loader2 size={15} className="animate-spin" /> : <Circle size={13} fill="currentColor" />}
            Start recording
          </button>
          <button
            type="button"
            className="kx-btn kx-btn-secondary"
            disabled={starting || testing}
            onClick={() => void useRecorder.getState().testCapture()}
          >
            {testing ? <Loader2 size={15} className="animate-spin" /> : <Activity size={15} />}
            {testing ? "Testing…" : "Test audio first"}
          </button>
        </div>
        {testing && (
          <div className="mt-5">
            <LiveMeters active={testing} paused={false} devices={devices} startedAt={Date.now()} />
            <p className="kx-meta mt-2">Play a video or song now — the Others card should move with it.</p>
          </div>
        )}
        <p className="kx-banner kx-banner-info mt-6">
          Use a headset for a clean You/Others split. With speakers, your mic also hears the other
          side, so their words bleed into You.
        </p>
      </div>
    );
  }

  // ---- Recording ----------------------------------------------------------
  return (
    <div className="kx-page kx-page-wide kx-live" style={{ maxWidth: 1180 }}>
      <div className="kx-live-main flex flex-col gap-4 min-w-0">
        <div className="flex items-center gap-2.5 min-w-0">
          <span className={`kx-chip ${paused ? "kx-chip-warn" : "kx-chip-alert"} font-semibold min-h-[28px] shrink-0`}>
            <span className={`kx-dot ${paused ? "kx-dot-warn" : "kx-dot-alert"}`} aria-hidden="true" />
            {paused ? "Paused" : "Recording"} <span className="kx-mono font-normal">{fmtClock(elapsed)}</span>
          </span>
          <label htmlFor="live-title" className="kx-sr-only">
            Meeting title
          </label>
          <input
            id="live-title"
            value={title}
            onChange={(e) => useRecorder.getState().setTitle(e.target.value)}
            placeholder="Meeting title"
            className="flex-1 min-w-0 bg-transparent kx-title border-b border-transparent hover:border-[var(--kx-hairline)] focus:border-[var(--kx-accent-ink)]"
          />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" className="kx-btn kx-btn-secondary" onClick={() => void useRecorder.getState().togglePause()}>
            {paused ? <Play size={14} /> : <Pause size={14} />}
            {paused ? "Resume" : "Pause"}
          </button>
          <button
            type="button"
            className="kx-btn kx-btn-secondary"
            disabled={paused}
            aria-keyshortcuts="F"
            onClick={() => {
              useRecorder.getState().flag();
              toast.success("Moment flagged.", { duration: 1500 });
            }}
            title="Flag this moment (F)"
          >
            <Flag size={14} /> Flag moment
          </button>
          <button type="button" className="kx-btn kx-btn-danger" onClick={() => void useRecorder.getState().stop()}>
            <Square size={11} fill="currentColor" /> Stop
          </button>
          <span className="kx-meta ms-1">
            <span className="kx-kbd">F</span> flags a moment
          </span>
        </div>

        <LiveMeters active={recording} paused={paused} devices={devices} startedAt={startedAt} />

        {systemCaptured === false && (
          <p className="kx-banner kx-banner-warn">
            <TriangleAlert size={16} />
            <span>
              <strong>Recording your mic only.</strong> The call's audio could not be captured, so
              Others will be empty.
            </span>
          </p>
        )}
        {captureError && (
          <div role="alert" className="kx-banner kx-banner-bad">
            <TriangleAlert size={16} />
            <span className="flex-1">{captureError}</span>
            <button type="button" aria-label="Dismiss" className="kx-ink-2 hover:text-white" onClick={() => useRecorder.getState().clearCaptureError()}>
              <X size={14} />
            </button>
          </div>
        )}
        {warnings.map((w) => (
          <div key={w.at} role="alert" className="kx-banner kx-banner-warn">
            <TriangleAlert size={16} />
            <span className="flex-1">{w.text}</span>
            <button type="button" aria-label="Dismiss" className="kx-ink-2 hover:text-white" onClick={() => useRecorder.getState().dismissWarning(w.at)}>
              <X size={14} />
            </button>
          </div>
        ))}

        <section aria-labelledby="live-tx" className="kx-card flex flex-col min-h-[260px]">
          <div className="kx-card-header">
            <h2 id="live-tx" className="kx-overline">
              Live transcript
            </h2>
            <span className="kx-meta">Saved to disk as you go</span>
          </div>
          <div
            ref={transcriptRef}
            onScroll={(e) => {
              const el = e.currentTarget;
              stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
            }}
            className="px-4 py-3 overflow-y-auto max-h-[46vh] select-text"
            aria-live="off"
          >
            {live.length === 0 ? (
              <p className="kx-meta py-6 text-center">Listening… the first lines appear after a few seconds of speech.</p>
            ) : (
              live.map((s, i) => {
                const flagged = flags.some((f) => Math.abs(f.atSec - s.atSec) < 6);
                return (
                  <p key={i} className="kx-live-line">
                    <span className={`kx-live-who ${s.source === "you" ? "kx-accent-ink" : "text-[var(--kx-ink-soft)]"}`}>
                      {s.source === "you" ? "You" : "Others"}
                    </span>
                    <span className="kx-read">
                      {s.text}
                      {flagged && <Flag size={12} className="inline ml-1.5 text-[var(--kx-warn)]" aria-label="Flagged" />}
                    </span>
                  </p>
                );
              })
            )}
          </div>
        </section>
      </div>

      <aside className="kx-live-aside flex flex-col gap-4">
        <section aria-labelledby="live-ask" className="kx-card p-4">
          <h2 id="live-ask" className="kx-heading mb-2">
            Ask about the meeting so far
          </h2>
          <div className="flex gap-2">
            <label htmlFor="live-q" className="kx-sr-only">
              Question
            </label>
            <input
              id="live-q"
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void askNow();
              }}
              placeholder="What have we agreed so far?"
              className="kx-input"
            />
            <button type="button" className="kx-btn kx-btn-secondary kx-btn-icon shrink-0" aria-label="Ask" disabled={asking || !question.trim()} onClick={() => void askNow()}>
              {asking ? <Loader2 size={15} className="animate-spin" /> : <Sparkles size={15} />}
            </button>
          </div>
          {answer && (
            <div className="mt-3 md-body text-[13.5px]">
              <Markdown>{answer.a}</Markdown>
            </div>
          )}
        </section>

        <section aria-labelledby="live-flags" className="kx-card p-4">
          <h2 id="live-flags" className="kx-heading mb-1">
            Flagged
          </h2>
          {flags.length === 0 ? (
            <p className="kx-meta">Press F, or Flag moment, when something matters. The notes give flagged moments their own lines.</p>
          ) : (
            <ul className="flex flex-col">
              {flags.map((f) => (
                <li key={f.atSec} className="flex gap-2.5 py-1.5 items-baseline">
                  <span className="kx-mono kx-meta">{fmtClock(f.atSec)}</span>
                  <span className="text-[13px] text-[var(--kx-ink-soft)] truncate">
                    {nearestText(live, f.atSec) || "Flagged moment"}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>

        <button type="button" className="kx-btn kx-btn-secondary w-full" onClick={() => void useRecorder.getState().openCompanion()}>
          <ExternalLink size={15} /> Pop out the recorder
        </button>
        <p className="kx-meta -mt-2 px-0.5">A small window that stays on top of your call.</p>
      </aside>
    </div>
  );
};

const nearestText = (live: { text: string; atSec: number }[], atSec: number): string => {
  let best = "";
  let gap = Infinity;
  for (const s of live) {
    const g = Math.abs(s.atSec - atSec);
    if (g < gap) {
      gap = g;
      best = s.text;
    }
  }
  return best;
};
