import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, Cpu, Loader2, X } from "lucide-react";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";
import { ErrorBoundary } from "@/components/ErrorBoundary";

// Kōrero (v1.8.0): added "recording-latched" for double-tap latch mode.
// In this state the overlay shows the same bars/mic layout as "recording" but
// with an amber/orange colour scheme (see RecordingOverlay.css).
type OverlayState = "recording" | "recording-latched" | "transcribing" | "processing";

/**
 * Korero recording overlay v2 — polished, state-distinct.
 *
 * State icons:
 *   recording          -> Mic with pulsing aurora ring + drop-shadow glow
 *   recording-latched  -> Mic with pulsing amber ring (double-tap latch mode, v1.8.0)
 *   transcribing       -> Cpu with matrix-green neon strobe (cyberpunk, v1.5.0)
 *   processing         -> Loader2 with magenta neon spin (cyberpunk, v1.5.0)
 *
 * State animations:
 *   recording          -> aurora-gradient waveform bars (live mic levels)
 *   recording-latched  -> amber/orange waveform bars; tap once to stop (v1.8.0)
 *   transcribing       -> shimmer text (system font, neon cyan glow + shimmer-fade, v1.9.0)
 *   processing         -> shimmer text + spinner
 *
 * Bar scaleY values (stored in `levels`):
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

  // ── Event listeners ────────────────────────────────────────────────────────
  useEffect(() => {
    const setup = async () => {
      const unlistenShow = await listen("show-overlay", async (event) => {
        await syncLanguageFromSettings();
        setState(event.payload as OverlayState);
        setIsVisible(true);
      });
      const unlistenHide = await listen("hide-overlay", () => {
        setIsVisible(false);
      });
      const unlistenLevel = await listen<number[]>("mic-level", (event) => {
        const newLevels = event.payload as number[];
        // Kōrero (v1.2.0): mic-level handler no longer calls setLevels.
        // The RAF loop owns all setLevels calls so the idle wave and audio
        // levels are merged in one place, avoiding racing setLevels calls
        // from two sources. We only update the smoothed ref here.
        const smoothed = smoothedLevelsRef.current.map((prev, i) => {
          const target = newLevels[i] || 0;
          return prev * 0.65 + target * 0.35;
        });
        smoothedLevelsRef.current = smoothed;
      });
      return () => {
        unlistenShow();
        unlistenHide();
        unlistenLevel();
      };
    };
    setup();
  }, []);

  // ── RAF idle animation loop ─────────────────────────────────────────────────
  // Active only while recording AND visible. Drives a gentle staggered sine
  // wave across all 9 bars as a minimum floor, ensuring bars are never flat.
  // Audio levels from smoothedLevelsRef ride on top via Math.max().
  useEffect(() => {
    // Kōrero (v1.8.0): RAF loop also active for "recording-latched" so bars
    // continue to animate while latch mode is engaged.
    if ((state !== "recording" && state !== "recording-latched") || !isVisible) {
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
  // Kōrero (v1.3.0): overlay layout reworked.
  //
  // Recording:
  //   left=mic icon  |  middle=waveform bars  |  right=cancel button
  //
  // Transcribing / Processing:
  //   left=empty  |  middle=icon + shimmer text  |  right=empty
  //
  // Previously the state icon always lived in overlay-left. For non-recording
  // states this created a visual off-centre because the icon in the 22px left
  // cell drew the eye left while the shimmer text centred only within the 1fr
  // middle column. Moving the icon into overlay-middle for these states lets
  // both icon and text centre together as a single visual unit (overlay-shimmer-
  // group flex row). overlay-left is empty for these states, keeping the grid
  // symmetric (22px | 1fr | 22px) with all content centred in the 1fr.
  return (
    <div
      dir={direction}
      className={`korero-overlay ${isVisible ? "fade-in" : ""}`}
      data-state={state}
    >
      {/* Left cell: mic icon during recording and recording-latched; empty otherwise. */}
      <div className="overlay-left">
        {(state === "recording" || state === "recording-latched") && (
          <div className="mic-wrap">
            <span className="mic-pulse" aria-hidden="true" />
            <Mic className="state-icon mic-icon" size={15} strokeWidth={2.2} />
          </div>
        )}
      </div>

      {/* Middle cell: bars when recording/recording-latched; icon + shimmer text otherwise.
          Non-recording: icon and text share overlay-shimmer-group so they
          centre as a unit within the 1fr column. */}
      <div className="overlay-middle">
        {(state === "recording" || state === "recording-latched") && (
          <div className="bars-container" aria-label="Recording audio levels">
            {levels.map((v, i) => (
              <div
                key={i}
                className="bar"
                style={{
                  // Kōrero (v1.2.0): `v` is now a pre-computed scaleY value (0.12–1.0)
                  // from the RAF loop — no additional mapping needed here.
                  // Opacity: 0.4 at idle (~0.12–0.18), rises to 1.0 at full audio.
                  transform: `scaleY(${v.toFixed(3)})`,
                  opacity: Math.max(0.4, Math.min(1, v * 1.6)),
                }}
              />
            ))}
          </div>
        )}
        {(state === "transcribing" || state === "processing") && (
          <div className="overlay-shimmer-group">
            {state === "transcribing" ? (
              <Cpu
                className="state-icon transcribe-icon"
                size={15}
                strokeWidth={2.0}
              />
            ) : (
              <Loader2
                className="state-icon spin-icon"
                size={15}
                strokeWidth={2.2}
              />
            )}
            <span className="shimmer-text">
              {state === "transcribing"
                ? t("overlay.transcribing")
                : t("overlay.processing")}
            </span>
          </div>
        )}
      </div>

      {/* Right cell: cancel button during recording and recording-latched; empty otherwise. */}
      <div className="overlay-right">
        {(state === "recording" || state === "recording-latched") && (
          <button
            type="button"
            className="cancel-button"
            onClick={() => commands.cancelOperation()}
            aria-label="Cancel recording"
          >
            <X size={13} strokeWidth={2.6} />
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
