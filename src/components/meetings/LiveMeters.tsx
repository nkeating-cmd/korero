/* eslint-disable i18next/no-literal-string */
import React, { memo, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { TriangleAlert } from "lucide-react";
import { fmtClock } from "./model";

/**
 * Kōrero 1.42: one card per side of the call — who is being heard, on which
 * device, and how long a side has been quiet. Level events arrive ~10× a
 * second, so they live in this memoised component only (v1.14.0 item 6).
 *
 * "Quiet" uses the same idea as the backend's speech gate: a frame counts as
 * heard when its level clears a small threshold.
 */

const HEARD = 0.012;
/** Seconds of silence from the call before the "different device?" hint. */
const QUIET_CALL_HINT_SECS = 90;
const BARS = 14;

interface SideState {
  levels: number[];
  lastHeard: number | null;
  written: number;
}

const empty = (): SideState => ({ levels: Array(BARS).fill(0), lastHeard: null, written: 0 });

export const LiveMeters: React.FC<{
  active: boolean;
  paused: boolean;
  devices: { mic: string; system: string } | null;
  startedAt: number;
  compact?: boolean;
}> = memo(({ active, paused, devices, startedAt, compact }) => {
  const [sides, setSides] = useState<{ you: SideState; others: SideState }>({
    you: empty(),
    others: empty(),
  });
  const [, tick] = useState(0);
  // "They're quiet": hide the no-call-audio hint until the call is heard again.
  const [quietOk, setQuietOk] = useState(false);
  const startedRef = useRef(startedAt);
  startedRef.current = startedAt;

  useEffect(() => {
    const un = listen<{ source: "you" | "others"; level: number; written: number }>(
      "meeting-level",
      (e) => {
        const { source, level, written } = e.payload;
        if (source === "others" && level >= HEARD) setQuietOk(false);
        setSides((prev) => {
          const s = prev[source] ?? empty();
          return {
            ...prev,
            [source]: {
              levels: [...s.levels.slice(1), Math.min(1, Math.sqrt(Math.max(0, level)) * 1.4)],
              lastHeard: level >= HEARD ? Date.now() : s.lastHeard,
              written,
            },
          };
        });
      },
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (!active) setSides({ you: empty(), others: empty() });
  }, [active]);

  useEffect(() => {
    if (!active) return;
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [active]);

  const card = (key: "you" | "others", label: string, device: string) => {
    const s = sides[key];
    const since = s.lastHeard ?? startedRef.current;
    const quietFor = Math.max(0, Math.floor((Date.now() - since) / 1000));
    const quiet = !paused && quietFor >= (key === "you" ? 30 : 20);
    const status = paused
      ? "Paused"
      : quiet
        ? `Quiet for ${fmtClock(quietFor)}`
        : compact
          ? "Hearing"
          : key === "you"
            ? "Hearing you"
            : "Hearing the call";
    const warn = quiet && quietFor >= (key === "you" ? 120 : 90);
    const colour = paused ? "var(--kx-ink-2)" : quiet ? "var(--kx-warn)" : "var(--kx-ok)";
    return (
      <div
        className="kx-card px-3.5 py-3"
        style={warn ? { borderColor: "var(--kx-warn-border)" } : undefined}
      >
        <div className="flex justify-between items-baseline gap-2">
          <span className="kx-heading">{label}</span>
          <span className="text-[12.5px]" style={{ color: colour }}>
            {status}
          </span>
        </div>
        <div className="kx-meta truncate">{device}</div>
        {!compact && (
          <div aria-hidden="true" className="flex items-end gap-[3px] h-6 mt-2.5">
            {s.levels.map((v, i) => (
              <span
                key={i}
                className="w-1 rounded-[1px]"
                style={{
                  height: `${Math.max(3, Math.round(v * 24))}px`,
                  background: v * 24 > 3 && !paused ? "var(--kx-ok)" : "var(--kx-control)",
                }}
              />
            ))}
          </div>
        )}
        <div className="kx-meta kx-mono mt-1">{fmtClock(Math.floor(s.written / 16000))} captured</div>
      </div>
    );
  };

  // C1: when the call has been silent for a while, say what that usually
  // means, while it can still be fixed.
  const othersSince = sides.others.lastHeard ?? startedRef.current;
  const othersQuiet = Math.max(0, Math.floor((Date.now() - othersSince) / 1000));
  const showQuietHint = active && !paused && !quietOk && othersQuiet >= QUIET_CALL_HINT_SECS;

  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-3">
        {card("you", "You", devices?.mic ?? "Microphone")}
        {card("others", "Others", devices?.system ?? "Call audio")}
      </div>
      {showQuietHint && (
        <div role="status" className="kx-banner kx-banner-warn">
          <TriangleAlert size={16} />
          <div className="flex-1 min-w-0">
            <p className="font-semibold">No sound from the call for {fmtClock(othersQuiet)}</p>
            <p className="mt-0.5">
              If people are talking, the call is probably playing through a different device.
              Kōrero is listening to {devices?.system ?? "your default speakers"}.
            </p>
          </div>
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm shrink-0" onClick={() => setQuietOk(true)}>
            They’re quiet
          </button>
        </div>
      )}
    </div>
  );
});
LiveMeters.displayName = "LiveMeters";
