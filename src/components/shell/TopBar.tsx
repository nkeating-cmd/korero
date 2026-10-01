/* eslint-disable i18next/no-literal-string */
import React from "react";
import { Search, Activity } from "lucide-react";
import { useNav } from "../../stores/navStore";
import { useActivity } from "../../stores/activityStore";
import { useRecorder } from "../../stores/recorderStore";
import { elapsedOf } from "../../stores/recorderProtocol";
import { fmtClock } from "../meetings/model";
import { useRunning } from "./useRunning";

/**
 * Kōrero 1.42 top bar: the same on every page. Search (Ctrl K) on the left;
 * a live recording chip while a meeting records, wherever you are; Activity on
 * the right with a count of running work.
 */
export const TopBar: React.FC = () => {
  const setPaletteOpen = useNav((s) => s.setPaletteOpen);
  const activityOpen = useNav((s) => s.activityOpen);
  const setActivityOpen = useNav((s) => s.setActivityOpen);
  const unseen = useActivity((s) => s.unseen);
  const hasAttention = useActivity((s) => s.items.some((i) => i.status !== "done"));
  const recording = useRecorder((s) => s.recording);
  const paused = useRecorder((s) => s.paused);
  const elapsedBase = useRecorder((s) => s.elapsedBase);
  const runStartedAt = useRecorder((s) => s.runStartedAt);
  const running = useRunning();
  const count = running.length;

  return (
    <div className="h-[52px] shrink-0 flex items-center gap-3 px-6 border-b border-[var(--kx-hairline-soft)]">
      <button
        type="button"
        onClick={() => setPaletteOpen(true)}
        className="flex items-center gap-2.5 w-[440px] max-w-[52%] min-h-[36px] px-3 rounded-[9px] border border-[var(--kx-hairline)] bg-[var(--kx-surface)] text-[13px] text-[var(--kx-ink-2)] text-left hover:border-[var(--kx-control)] transition-colors"
      >
        <Search size={15} aria-hidden="true" />
        <span className="flex-1 truncate">Search meetings, notes and dictations</span>
        <span className="kx-kbd">Ctrl K</span>
      </button>

      <div className="flex-1" />

      {recording && (
        <button
          type="button"
          onClick={() => useNav.getState().openMeeting({ id: "live" })}
          className={`kx-chip ${paused ? "kx-chip-warn" : "kx-chip-alert"} kx-chip-button min-h-[30px] px-3 font-semibold`}
          title="Show the live meeting"
        >
          <span className={`kx-dot ${paused ? "kx-dot-warn" : "kx-dot-alert"}`} aria-hidden="true" />
          {paused ? "Paused" : "Recording"}{" "}
          <span className="kx-mono font-normal">
            {fmtClock(elapsedOf({ elapsedBase, runStartedAt }))}
          </span>
        </button>
      )}

      <button
        type="button"
        aria-pressed={activityOpen}
        aria-label={`Activity${count ? `, ${count} running` : ""}${unseen && hasAttention ? ", needs a look" : ""}`}
        onClick={() => setActivityOpen(!activityOpen)}
        className={`kx-btn kx-btn-sm min-h-[36px] ${
          activityOpen
            ? "border-[var(--kx-accent)] bg-[var(--kx-selected)] text-[var(--kx-accent-ink)]"
            : "kx-btn-secondary"
        }`}
      >
        <Activity size={16} aria-hidden="true" />
        Activity
        {count > 0 ? (
          <span className="kx-badge" aria-hidden="true">
            {count}
          </span>
        ) : unseen > 0 && hasAttention ? (
          <span className="kx-dot kx-dot-warn" aria-hidden="true" />
        ) : null}
      </button>
    </div>
  );
};
