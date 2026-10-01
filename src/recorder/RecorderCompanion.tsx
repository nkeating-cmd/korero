/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Circle,
  Pause,
  Play,
  Square,
  Flag,
  Pin,
  PinOff,
  TriangleAlert,
  X,
  Loader2,
  ExternalLink,
  Check,
} from "lucide-react";
import {
  RECORDER_COMMAND_EVENT,
  RECORDER_STATE_EVENT,
  elapsedOf,
  type RecorderCommand,
  type RecorderSnapshot,
} from "../stores/recorderProtocol";
import { LiveMeters } from "../components/meetings/LiveMeters";
import { fmtClock } from "../components/meetings/model";

/**
 * Kōrero 1.42: the pop-out recorder (C1). Small, always on top, and built for
 * a glance during a call: the clock, both sides' meters, Pause / Flag / Stop,
 * and the last few lines. It mirrors the main window and sends commands
 * back; the recording itself never depends on this window being open.
 */

const send = (cmd: RecorderCommand) => {
  void emit(RECORDER_COMMAND_EVENT, cmd).catch(() => {});
};

const useTick = (on: boolean) => {
  const [, setN] = useState(0);
  useEffect(() => {
    if (!on) return;
    const t = window.setInterval(() => setN((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [on]);
};

export const RecorderCompanion: React.FC = () => {
  const [snap, setSnap] = useState<RecorderSnapshot | null>(null);
  const [waited, setWaited] = useState(false);
  const [pinned, setPinned] = useState(true);
  const [titleDraft, setTitleDraft] = useState<string | null>(null);
  const [startPending, setStartPending] = useState(false);
  const [flashFlag, setFlashFlag] = useState(false);
  const linesRef = useRef<HTMLDivElement>(null);
  const wasStopping = useRef(false);
  const [justSaved, setJustSaved] = useState(false);

  useEffect(() => {
    const un = listen<RecorderSnapshot>(RECORDER_STATE_EVENT, (e) => {
      setSnap(e.payload);
      if (e.payload.recording) setStartPending(false);
    });
    // Ask the main window for the current state; it answers with a broadcast.
    send({ action: "hello" });
    const t = window.setTimeout(() => setWaited(true), 2500);
    return () => {
      window.clearTimeout(t);
      un.then((f) => f());
    };
  }, []);

  // "Saved" shows once the main window has finished with the meeting.
  useEffect(() => {
    if (!snap) return;
    if (snap.stopping) wasStopping.current = true;
    else if (wasStopping.current && !snap.recording) {
      wasStopping.current = false;
      setJustSaved(true);
    }
    if (snap.recording) setJustSaved(false);
  }, [snap]);

  const recording = !!snap?.recording;
  const paused = !!snap?.paused;
  useTick(recording && !paused);

  // F flags a moment while you are not typing.
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing = el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA");
      if (!typing && !e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        flag();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [recording]);

  // Follow the newest line.
  const liveCount = snap?.live.length ?? 0;
  const lastText = liveCount > 0 ? (snap?.live[liveCount - 1]?.text ?? "") : "";
  useEffect(() => {
    const el = linesRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [liveCount, lastText]);

  const flag = () => {
    send({ action: "flag" });
    setFlashFlag(true);
    window.setTimeout(() => setFlashFlag(false), 900);
  };

  const togglePin = async () => {
    const next = !pinned;
    try {
      await getCurrentWindow().setAlwaysOnTop(next);
      setPinned(next);
    } catch {
      /* the window keeps its current behaviour */
    }
  };

  const close = () => {
    void getCurrentWindow()
      .close()
      .catch(() => {});
  };

  const elapsed = snap ? elapsedOf(snap) : 0;
  const startedAt = Date.now() - elapsed * 1000;

  const header = (
    <div className="flex items-center gap-2 px-4 pt-3.5">
      {recording ? (
        <span className={`kx-dot ${paused ? "kx-dot-warn" : "kx-dot-alert"}`} aria-hidden="true" />
      ) : (
        <span className="kx-dot" aria-hidden="true" style={{ background: "var(--kx-control)" }} />
      )}
      <span className="kx-overline flex-1">
        {recording ? (paused ? "Paused" : "Recording") : snap?.stopping ? "Saving" : "Kōrero recorder"}
      </span>
      <button
        type="button"
        className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm"
        aria-pressed={pinned}
        aria-label={pinned ? "Stop keeping this window on top" : "Keep this window on top"}
        title={pinned ? "On top of other windows" : "Not on top"}
        onClick={() => void togglePin()}
      >
        {pinned ? <Pin size={15} /> : <PinOff size={15} />}
      </button>
    </div>
  );

  // ---- Waiting for the main window -----------------------------------------
  if (!snap) {
    return (
      <div className="h-full flex flex-col bg-[var(--kx-ground)] text-[var(--kx-ink)]">
        {header}
        <div className="flex-1 flex flex-col items-center justify-center gap-3 px-6 text-center">
          {waited ? (
            <>
              <p className="kx-meta">Kōrero isn’t answering. It may still be starting up.</p>
              <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => send({ action: "hello" })}>
                Try again
              </button>
            </>
          ) : (
            <Loader2 size={18} className="animate-spin kx-ink-2" aria-label="Connecting" />
          )}
        </div>
      </div>
    );
  }

  // ---- Not recording: saving, saved, or idle --------------------------------
  if (!recording) {
    return (
      <div className="h-full flex flex-col bg-[var(--kx-ground)] text-[var(--kx-ink)]">
        {header}
        <div className="flex-1 flex flex-col justify-center gap-3 px-5">
          {snap.stopping ? (
            <>
              <h1 className="kx-title">Saving the meeting</h1>
              <p className="kx-meta">
                The audio is on disk. Kōrero is finishing the transcript; this window can close.
              </p>
              <div className="kx-progress kx-progress-indeterminate">
                <span />
              </div>
            </>
          ) : justSaved ? (
            <>
              <span className="kx-step kx-step-done" aria-hidden="true">
                <Check size={14} />
              </span>
              <h1 className="kx-title">Meeting saved</h1>
              <p className="kx-meta">The transcript is ready in Kōrero, and the notes follow.</p>
            </>
          ) : (
            <>
              <h1 className="kx-title">No meeting recording</h1>
              <p className="kx-meta">Start one here, or from Kōrero.</p>
            </>
          )}
          {!snap.stopping && (
            <div className="flex flex-wrap gap-2 mt-2">
              <button
                type="button"
                className="kx-btn kx-btn-primary"
                disabled={startPending}
                onClick={() => {
                  setStartPending(true);
                  send({ action: "start" });
                  window.setTimeout(() => setStartPending(false), 8000);
                }}
              >
                {startPending ? <Loader2 size={14} className="animate-spin" /> : <Circle size={11} fill="currentColor" />}
                {justSaved ? "Record another" : "Record a meeting"}
              </button>
              <button type="button" className="kx-btn kx-btn-secondary" onClick={() => send({ action: "open" })}>
                <ExternalLink size={14} /> Open in Kōrero
              </button>
            </div>
          )}
        </div>
        <div className="px-4 py-3 flex justify-end">
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={close}>
            Close
          </button>
        </div>
      </div>
    );
  }

  // ---- Recording -----------------------------------------------------------
  const lines = snap.live.slice(-8);
  return (
    <div className="h-full flex flex-col bg-[var(--kx-ground)] text-[var(--kx-ink)]">
      {header}
      <div className="px-4 pt-1.5 flex flex-col gap-3 shrink-0">
        <div className="flex items-baseline justify-between gap-3">
          <span
            className="kx-mono text-[34px] leading-none"
            style={{ color: paused ? "var(--kx-ink-2)" : "var(--kx-ink)" }}
            aria-label={`Recorded ${fmtClock(elapsed)}`}
          >
            {fmtClock(elapsed)}
          </span>
          {snap.flags.length > 0 && (
            <span className="kx-chip kx-chip-warn">
              <Flag size={12} /> {snap.flags.length}
            </span>
          )}
        </div>
        <label htmlFor="kx-rec-title" className="kx-sr-only">
          Meeting title
        </label>
        <input
          id="kx-rec-title"
          value={titleDraft ?? snap.title}
          onFocus={() => setTitleDraft(snap.title)}
          onChange={(e) => setTitleDraft(e.target.value)}
          onBlur={() => {
            if (titleDraft !== null && titleDraft.trim() && titleDraft !== snap.title) {
              send({ action: "title", title: titleDraft.trim() });
            }
            setTitleDraft(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === "Escape") (e.target as HTMLInputElement).blur();
          }}
          className="kx-input min-h-[34px] text-[14px]"
        />

        <div className="grid grid-cols-3 gap-2">
          <button
            type="button"
            className="kx-btn kx-btn-secondary"
            onClick={() => send({ action: paused ? "resume" : "pause" })}
          >
            {paused ? <Play size={14} /> : <Pause size={14} />}
            {paused ? "Resume" : "Pause"}
          </button>
          <button
            type="button"
            className="kx-btn kx-btn-warn"
            onClick={flag}
            aria-keyshortcuts="F"
            title="Flag this moment (F)"
          >
            {flashFlag ? <Check size={14} /> : <Flag size={14} />}
            {flashFlag ? "Flagged" : "Flag"}
          </button>
          <button type="button" className="kx-btn kx-btn-danger" onClick={() => send({ action: "stop" })}>
            <Square size={12} fill="currentColor" /> Stop
          </button>
        </div>

        <LiveMeters active={recording} paused={paused} devices={snap.devices} startedAt={startedAt} compact />

        {snap.systemCaptured === false && (
          <p className="kx-banner kx-banner-warn">
            <TriangleAlert size={15} />
            <span>The call’s audio isn’t being captured — only your mic.</span>
          </p>
        )}
        {snap.warnings.slice(-2).map((w) => (
          <div key={w.at} className="kx-banner kx-banner-warn">
            <TriangleAlert size={15} />
            <span className="flex-1">{w.text}</span>
            <button
              type="button"
              aria-label="Dismiss"
              className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm"
              onClick={() => send({ action: "dismiss-warning", at: w.at })}
            >
              <X size={13} />
            </button>
          </div>
        ))}
      </div>

      <div className="px-4 pt-3 pb-1 kx-overline">Just said</div>
      <div ref={linesRef} className="flex-1 min-h-0 overflow-y-auto px-4 pb-2" aria-live="off">
        {lines.length === 0 ? (
          <p className="kx-meta">Lines appear here a few seconds after they are spoken.</p>
        ) : (
          lines.map((l, i) => (
            <p key={`${l.atSec}-${i}`} className="text-[13.5px] leading-snug mb-1.5 text-[var(--kx-ink-read)]">
              <span
                className="font-semibold mr-1.5"
                style={{ color: l.source === "you" ? "var(--kx-accent-ink)" : "var(--kx-ink-soft)" }}
              >
                {l.source === "you" ? "You" : "Others"}
              </span>
              {l.text}
            </p>
          ))
        )}
      </div>

      <div className="px-4 py-2.5 border-t border-[var(--kx-hairline-soft)] flex items-center justify-between">
        <span className="kx-meta">
          <span className="kx-kbd">F</span> flags a moment
        </span>
        <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => send({ action: "open" })}>
          <ExternalLink size={13} /> Open in Kōrero
        </button>
      </div>
    </div>
  );
};
