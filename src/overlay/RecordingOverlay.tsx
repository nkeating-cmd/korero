import { emit, listen } from "@tauri-apps/api/event";
import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, Cpu, Loader2, X, Lock, ClipboardCheck, TriangleAlert } from "lucide-react";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";
import { ErrorBoundary } from "@/components/ErrorBoundary";

// Kōrero (v1.8.0): added "recording-latched" for double-tap latch mode.
// In this state the overlay shows the same bars/mic layout as "recording" but
// with an amber/orange colour scheme (see RecordingOverlay.css).
// Kōrero 1.42: three short notices (see overlay.rs `show_overlay_notice`).
// The pill is the one part of Kōrero you see while dictating, so problems
// show here, not only as a toast in a main window hidden in the tray.
type OverlayState =
  | "recording"
  | "recording-latched"
  | "transcribing"
  | "processing"
  | "notice-paste-copied"
  | "notice-no-mic"
  | "notice-mic-denied";

const isRecordingState = (s: OverlayState) => s === "recording" || s === "recording-latched";

const fmtClock = (sec: number) => {
  const s = Math.max(0, Math.floor(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${r}` : `${m}:${r}`;
};

/**
 * Kōrero recording overlay (the dictation pill).
 *
 * Kōrero 1.42 (C1 on the design canvas): the pill uses the app's palette and
 * says more about itself — an NZ badge while listening when New Zealand
 * English is on, a clock and a real Stop when hands-free, the prompt's name
 * while cleaning up, and short notices when a paste falls back to the
 * clipboard or there is no microphone (with a Fix button).
 *
 * Bar scaleY values (stored in `levels`), unchanged since v1.2.0:
 *   When not recording: 0.15 (CSS default, no JS involvement).
 *   When recording: RAF loop drives continuous updates. Each frame:
 *     - idleScaleY = 0.12 + 0.06 * (0.5 + 0.5 * sin(t*4 + i*0.9))  → 0.12–0.18
 *     - audioScaleY = f(smoothedLevelsRef[i])                          → 0.12–1.0
 *     - displayed   = max(idleScaleY, audioScaleY)
 *   This ensures bars always breathe gently even when the room is silent,
 *   and audio peaks ride on top of the idle wave rather than collapsing to zero.
 */
const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  // Kōrero (v1.2.0): `levels` now stores final scaleY values (0.12–1.0) rather
  // than raw audio levels. The RAF loop computes and writes these; the bar JSX
  // reads them directly without an additional formula.
  const [levels, setLevels] = useState<number[]>(Array(9).fill(0.15));
  const smoothedLevelsRef = useRef<number[]>(Array(16).fill(0));
  const animFrameRef = useRef<number | null>(null);
  const animStartRef = useRef<number | null>(null);
  const direction = getLanguageDirection(i18n.language);
  // Kōrero 1.42: what the pill says about itself.
  const [nz, setNz] = useState(false);
  const [promptName, setPromptName] = useState<string | null>(null);
  const startedAtRef = useRef<number | null>(null);
  const visibleRef = useRef(false);
  const stateRef = useRef<OverlayState>("recording");
  const [, setTick] = useState(0);

  // Settings are read when the pill appears, so the NZ badge is never stale:
  // a change in the main window shows next time.
  const readSettings = async () => {
    try {
      const res = await commands.getAppSettings();
      if (res.status !== "ok") return;
      const st = res.data;
      setNz((st.selected_language ?? "").toLowerCase() === "en-nz");
    } catch {
      /* keep what we had */
    }
  };

  // ── Event listeners ────────────────────────────────────────────────────────
  useEffect(() => {
    // Kōrero (v1.20.0): register listeners robustly. The previous version
    // returned the cleanup FROM the async setup(), so useEffect received a
    // Promise (never a function) and the unlisten handlers were never called.
    // Harmless at runtime (the overlay window lives for the app's lifetime),
    // but it stacked DUPLICATE listeners on every HMR reload during
    // `tauri dev`, double-driving state. Now: collect the unlisten fns and
    // tear them down synchronously, with a `cancelled` guard for the case
    // where the effect is torn down before registration completes.
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    void (async () => {
      const off = await Promise.all([
        listen("show-overlay", async (event) => {
          const next = event.payload as OverlayState;
          // A new dictation starts the clock; going hands-free keeps it.
          if (next === "recording" && (!visibleRef.current || !isRecordingState(stateRef.current))) {
            startedAtRef.current = Date.now();
          } else if (next === "recording-latched" && startedAtRef.current === null) {
            startedAtRef.current = Date.now();
          }
          stateRef.current = next;
          visibleRef.current = true;
          setState(next);
          setIsVisible(true);
          await Promise.all([syncLanguageFromSettings(), readSettings()]);
        }),
        // The prompt's name comes from Rust, which resolves per-app routing,
        // so the pill names the prompt actually being used.
        listen<string | null>("overlay-prompt", (event) => {
          setPromptName(event.payload?.trim() || null);
        }),
        listen("hide-overlay", () => {
          visibleRef.current = false;
          startedAtRef.current = null;
          setIsVisible(false);
        }),
        // mic-level handler no longer calls setLevels (v1.2.0): the RAF loop
        // owns all setLevels calls so the idle wave and audio levels merge in
        // one place. Here we only update the smoothed ref.
        listen<number[]>("mic-level", (event) => {
          const newLevels = event.payload as number[];
          const smoothed = smoothedLevelsRef.current.map((prev, i) => {
            const target = newLevels[i] || 0;
            return prev * 0.65 + target * 0.35;
          });
          smoothedLevelsRef.current = smoothed;
        }),
      ]);
      if (cancelled) {
        off.forEach((u) => u());
        return;
      }
      unlisteners.push(...off);
    })();
    return () => {
      cancelled = true;
      unlisteners.forEach((u) => u());
    };
  }, []);

  // The hands-free clock ticks once a second while it is showing.
  useEffect(() => {
    if (state !== "recording-latched" || !isVisible) return;
    const t = window.setInterval(() => setTick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [state, isVisible]);

  const fix = () => {
    void commands.showMainWindowCommand().catch(() => {});
    void emit("korero://open-section", "general").catch(() => {});
    setIsVisible(false);
  };

  // ── RAF idle animation loop ─────────────────────────────────────────────────
  // Active only while recording AND visible. Drives a gentle staggered sine
  // wave across all 9 bars as a minimum floor, ensuring bars are never flat.
  // Audio levels from smoothedLevelsRef ride on top via Math.max().
  useEffect(() => {
    // Kōrero 1.42: hands-free shows a clock instead of bars, so the loop
    // runs for "recording" only (it was also active for "recording-latched").
    if (state !== "recording" || !isVisible) {
      if (animFrameRef.current !== null) {
        cancelAnimationFrame(animFrameRef.current);
        animFrameRef.current = null;
      }
      // Reset smoothed levels so next recording session starts clean
      smoothedLevelsRef.current = Array(16).fill(0);
      return;
    }

    animStartRef.current = null;

    const tick = (now: number) => {
      if (animStartRef.current === null) animStartRef.current = now;
      const t = (now - animStartRef.current) / 1000; // elapsed seconds

      const smoothed = smoothedLevelsRef.current;
      const merged = Array.from({ length: 9 }, (_, i) => {
        // Idle wave: 0.12–0.18 range, staggered per bar (phase offset 0.9 rad)
        const idleScaleY = 0.12 + 0.06 * (0.5 + 0.5 * Math.sin(t * 4.0 + i * 0.9));
        // Audio path: same formula as before, maps 0–1 audio level → 0.12–1.0 scaleY
        const audioLevel = smoothed[i] || 0;
        // Kōrero (v1.2.0 fix): no constant floor offset — maps 0→0, 1→1 with
        // gamma 0.7 for perceptual loudness curve. The old formula had a built-in
        // floor of 0.20 ((4+0)/20), which exceeded the idle wave ceiling of 0.18
        // and suppressed the breathing animation entirely at silence.
        const audioScaleY = Math.min(1, Math.pow(audioLevel, 0.7));
        // Merge: audio peak always wins; idle floor keeps bars breathing at silence
        return Math.max(idleScaleY, audioScaleY);
      });

      setLevels(merged);
      animFrameRef.current = requestAnimationFrame(tick);
    };

    animFrameRef.current = requestAnimationFrame(tick);

    return () => {
      if (animFrameRef.current !== null) {
        cancelAnimationFrame(animFrameRef.current);
        animFrameRef.current = null;
      }
    };
  }, [state, isVisible]);

  // ── Render ──────────────────────────────────────────────────────────────────
  // Kōrero 1.42: three cells, as since v1.3.0 (left | middle | right), with
  // the right cell sized to its content for the states that carry a text
  // button (hands-free Stop, Fix). See RecordingOverlay.css for the geometry
  // contract; nothing here may make the pill wider than its 280px max.
  //
  //   recording          mic        | bars + NZ badge           | cancel
  //   recording-latched  lock       | "Locked on · 3:12"        | Stop, cancel
  //   transcribing       —          | icon + "Transcribing…"    | —
  //   processing         —          | icon + "Cleaning up · X"  | —
  //   notice-paste-…     clipboard  | "Couldn't paste · copied" | —
  //   notice-no-mic      warning    | "No microphone found"     | Fix
  //   notice-mic-denied  warning    | "Microphone is blocked"   | Fix
  const elapsed = startedAtRef.current ? (Date.now() - startedAtRef.current) / 1000 : 0;
  const notice = state.startsWith("notice-");
  const cleaningLabel = promptName
    ? `${t("overlay.cleaningUp")} · ${promptName.length > 24 ? `${promptName.slice(0, 23)}…` : promptName}`
    : t("overlay.processing");

  return (
    <div
      dir={direction}
      className={`korero-overlay ${isVisible ? "fade-in" : ""}`}
      data-state={state}
      role={notice ? "status" : undefined}
    >
      <div className="overlay-left">
        {state === "recording" && (
          <div className="mic-wrap">
            <span className="mic-pulse" aria-hidden="true" />
            <Mic className="state-icon mic-icon" size={15} strokeWidth={2.2} />
          </div>
        )}
        {state === "recording-latched" && (
          <Lock className="state-icon lock-icon" size={15} strokeWidth={2.2} aria-hidden="true" />
        )}
        {state === "notice-paste-copied" && (
          <ClipboardCheck className="state-icon notice-icon" size={15} strokeWidth={2} aria-hidden="true" />
        )}
        {(state === "notice-no-mic" || state === "notice-mic-denied") && (
          <TriangleAlert className="state-icon notice-icon" size={15} strokeWidth={2} aria-hidden="true" />
        )}
      </div>

      <div className="overlay-middle">
        {state === "recording" && (
          <>
            <div className="bars-container" aria-label="Recording audio levels">
              {levels.map((v, i) => (
                <div
                  key={i}
                  className="bar"
                  style={{
                    // `v` is a pre-computed scaleY (0.12–1.0) from the RAF loop.
                    transform: `scaleY(${v.toFixed(3)})`,
                    opacity: Math.max(0.4, Math.min(1, v * 1.6)),
                  }}
                />
              ))}
            </div>
            {nz && (
              <span className="nz-badge" title={t("overlay.nzBadge")} aria-label={t("overlay.nzBadge")}>
                NZ
              </span>
            )}
          </>
        )}
        {state === "recording-latched" && (
          <span className="pill-label">
            {t("overlay.lockedOn")} · <span className="pill-clock">{fmtClock(elapsed)}</span>
          </span>
        )}
        {(state === "transcribing" || state === "processing") && (
          <div className="overlay-shimmer-group">
            {state === "transcribing" ? (
              <Cpu className="state-icon transcribe-icon" size={15} strokeWidth={2.0} />
            ) : (
              <Loader2 className="state-icon spin-icon" size={15} strokeWidth={2.2} />
            )}
            <span className="shimmer-text">
              {state === "transcribing" ? t("overlay.transcribing") : cleaningLabel}
            </span>
          </div>
        )}
        {state === "notice-paste-copied" && <span className="pill-label notice-label">{t("overlay.pasteCopied")}</span>}
        {state === "notice-no-mic" && <span className="pill-label notice-label">{t("overlay.noMic")}</span>}
        {state === "notice-mic-denied" && <span className="pill-label notice-label">{t("overlay.micBlocked")}</span>}
      </div>

      <div className="overlay-right">
        {state === "recording-latched" && (
          <button
            type="button"
            className="pill-text-button"
            onClick={() => commands.finishActiveRecording()}
            aria-label={t("overlay.stopAndTranscribe")}
            title={t("overlay.stopAndTranscribe")}
          >
            {t("overlay.stop")}
          </button>
        )}
        {isRecordingState(state) && (
          <button
            type="button"
            className="cancel-button"
            onClick={() => commands.cancelOperation()}
            aria-label={t("overlay.cancel")}
            title={t("overlay.cancel")}
          >
            <X size={13} strokeWidth={2.6} />
          </button>
        )}
        {(state === "notice-no-mic" || state === "notice-mic-denied") && (
          <button type="button" className="pill-text-button pill-fix" onClick={fix} title={t("overlay.fixHint")}>
            {t("overlay.fix")}
          </button>
        )}
      </div>
    </div>
  );
};

// Kōrero (v1.7.0 B4 / v1.8.0): wrap in ErrorBoundary so a render crash in the overlay
// shows a minimal fallback rather than freezing the transparent overlay window.
// The overlay fallback is intentionally empty — an invisible frozen overlay is
// less disruptive to the user than an error card floating over their work.
function OverlayRoot() {
  return (
    <ErrorBoundary fallback={null}>
      <RecordingOverlay />
    </ErrorBoundary>
  );
}

export default OverlayRoot;
