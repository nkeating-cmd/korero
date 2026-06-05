/* eslint-disable i18next/no-literal-string */
import React, { memo, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

/**
 * Kōrero fork (v1.14.0, item 6): live capture meters, isolated in a memoised
 * child component. Level events arrive ~10×/second while recording; keeping
 * that state HERE means they re-render this panel only — previously every
 * event re-rendered the entire Meetings page.
 */

interface LevelState {
  level: number;
  written: number;
}

interface Props {
  recording: boolean;
  testing: boolean;
  elapsed: number;
  devices: { mic: string; system: string } | null;
}

const fmtClock = (s: number) =>
  `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;

export const CaptureMeters: React.FC<Props> = memo(
  ({ recording, testing, elapsed, devices }) => {
    const [levels, setLevels] = useState<
      Partial<Record<"you" | "others", LevelState>>
    >({});

    useEffect(() => {
      const un = listen<{
        source: "you" | "others";
        level: number;
        written: number;
      }>("meeting-level", (e) => {
        setLevels((prev) => ({
          ...prev,
          [e.payload.source]: {
            level: e.payload.level,
            written: e.payload.written,
          },
        }));
      });
      return () => {
        un.then((f) => f());
      };
    }, []);

    // Fresh meters whenever a session (recording or test) starts.
    useEffect(() => {
      if (recording || testing) setLevels({});
    }, [recording, testing]);

    if (!recording && !testing) return null;

    return (
      <div className="glass-card p-4 space-y-3">
        {(
          [
            { key: "you", label: "You", device: devices?.mic ?? "Microphone" },
            {
              key: "others",
              label: "Others",
              device: devices?.system ?? "System audio",
            },
          ] as const
        ).map((row) => {
          const lv = levels[row.key];
          const written = lv?.written ?? 0;
          const pct = Math.min(
            100,
            Math.round(Math.sqrt(Math.min(1, lv?.level ?? 0)) * 100),
          );
          const stalled = recording && elapsed > 5 && written === 0;
          return (
            <div key={row.key}>
              <div className="flex items-center justify-between text-xs text-text-subtle mb-1">
                <span className="truncate">
                  <span className="text-text font-medium">{row.label}</span> ·{" "}
                  {row.device}
                </span>
                <span className="shrink-0">
                  {fmtClock(Math.floor(written / 16000))} captured
                </span>
              </div>
              <div className="h-2 rounded-full bg-white/10 overflow-hidden">
                <div
                  className="h-full rounded-full transition-[width] duration-150"
                  style={{
                    width: `${pct}%`,
                    background: "var(--color-aurora-cyan)",
                  }}
                />
              </div>
              {stalled && (
                <p className="text-xs mt-1 text-amber-400">
                  {row.key === "you"
                    ? "No microphone audio captured yet — check the mic isn't muted or held by another app."
                    : `No system audio captured yet — make sure the meeting audio is playing through "${row.device}" (loopback records the default output device only).`}
                </p>
              )}
            </div>
          );
        })}
        {testing && (
          <p className="text-xs text-text-subtle">
            Play any audio now (a video or song) — the Others meter should move
            with it.
          </p>
        )}
      </div>
    );
  },
);
CaptureMeters.displayName = "CaptureMeters";
