/* eslint-disable i18next/no-literal-string */
import React from "react";
import { Circle, Upload, Search, FileAudio, Loader2, UsersRound } from "lucide-react";
import { useRecorder } from "../../stores/recorderStore";
import { elapsedOf } from "../../stores/recorderProtocol";
import type { MeetingsController } from "../settings/meetings/useMeetingsController";
import { durationOf, fmtClock, fmtDuration, titleOf, type Meeting } from "./model";

/**
 * Kōrero 1.42: the meeting library — Record and Import at the top, search,
 * meetings grouped by when they happened, and the on-disk recordings at the
 * bottom. The meeting in progress is pinned first while it records.
 */

const dayKey = (ms: number) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
};

const groupOf = (ms: number): string => {
  const now = new Date();
  const today = dayKey(now.getTime());
  const yesterday = dayKey(now.getTime() - 86_400_000);
  const k = dayKey(ms);
  if (k === today) return "Today";
  if (k === yesterday) return "Yesterday";
  if (now.getTime() - ms < 7 * 86_400_000) return "This week";
  return "Earlier";
};

export const MeetingsLibrary: React.FC<{ c: MeetingsController; wide?: boolean }> = ({ c, wide }) => {
  const recording = useRecorder((s) => s.recording);
  const paused = useRecorder((s) => s.paused);
  const starting = useRecorder((s) => s.starting);
  const recTitle = useRecorder((s) => s.title);
  const elapsedBase = useRecorder((s) => s.elapsedBase);
  const runStartedAt = useRecorder((s) => s.runStartedAt);

  const status = (m: Meeting): { text: string; tone: "accent" | "warn" | "plain" } => {
    if (c.job && c.job.meetingId === m.id) {
      return { text: c.job.kind === "refine" ? "Refining notes" : "Writing notes", tone: "accent" };
    }
    if (c.task && c.task.meetingId === m.id) return { text: "Transcribing", tone: "accent" };
    if ((m.captureWarnings ?? []).length > 0 && !m.wrapUpDismissed) {
      return { text: "check the capture", tone: "warn" };
    }
    if (m.processed.trim()) return { text: "notes ready", tone: "plain" };
    return { text: "transcript only", tone: "plain" };
  };

  const groups: { label: string; items: Meeting[] }[] = [];
  for (const m of c.filteredMeetings) {
    const label = groupOf(m.createdAt);
    const g = groups.find((x) => x.label === label);
    if (g) g.items.push(m);
    else groups.push({ label, items: [m] });
  }

  const startRecording = async () => {
    if (recording) {
      c.setView({ kind: "live" });
      return;
    }
    const ok = await useRecorder.getState().start();
    if (ok) c.setView({ kind: "live" });
  };

  return (
    <section
      aria-label="Meeting library"
      className="kx-library"
      style={wide ? { width: "100%", borderRight: "none", background: "transparent" } : undefined}
    >
      <div className="px-4 pt-5 pb-3 flex flex-col gap-3">
        <h1 className="kx-title">Meetings</h1>
        <div className="flex gap-2">
          <button
            type="button"
            className="kx-btn kx-btn-primary flex-1"
            onClick={() => void startRecording()}
            disabled={starting || !!c.stopping}
          >
            {starting ? <Loader2 size={14} className="animate-spin" /> : <Circle size={11} fill="currentColor" />}
            {recording ? "Show live" : "Record"}
          </button>
          <button
            type="button"
            className="kx-btn kx-btn-secondary"
            onClick={() => void c.pickImportFile()}
            disabled={c.importBusy || recording || !!c.stopping}
            title={recording || c.stopping ? "Finish the meeting first" : undefined}
          >
            <Upload size={14} /> Import
          </button>
        </div>
        <div className="relative">
          <label htmlFor="kx-msearch" className="kx-sr-only">
            Search meetings
          </label>
          <Search size={14} className="absolute left-3 top-[11px] kx-ink-2 pointer-events-none" aria-hidden="true" />
          <input
            id="kx-msearch"
            type="search"
            value={c.search}
            onChange={(e) => c.setSearch(e.target.value)}
            placeholder="Titles, transcripts, notes"
            className="kx-input pl-8"
          />
        </div>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-2.5 pb-3">
        {(recording || c.stopping) && (
          <>
            <div className="kx-overline px-2 pt-1 pb-1.5">Now</div>
            <button
              type="button"
              aria-current={c.view.kind === "live" ? "true" : undefined}
              onClick={() => c.setView({ kind: "live" })}
              className="kx-list-item"
            >
              <span className="flex items-center gap-2 text-[13.5px] font-semibold">
                <span className={`kx-dot ${paused ? "kx-dot-warn" : "kx-dot-alert"}`} aria-hidden="true" />
                <span className="truncate">{recording ? recTitle || "Meeting" : "Saving the meeting"}</span>
              </span>
              <span className="kx-meta kx-mono">
                {recording
                  ? `${paused ? "Paused" : "Recording"} · ${fmtClock(elapsedOf({ elapsedBase, runStartedAt }))}`
                  : "Finishing the transcript…"}
              </span>
            </button>
          </>
        )}

        {c.importPath && (
          <>
            <div className="kx-overline px-2 pt-3 pb-1.5">Import</div>
            <button
              type="button"
              aria-current={c.view.kind === "import" ? "true" : undefined}
              onClick={() => c.setView({ kind: "import" })}
              className="kx-list-item"
            >
              <span className="flex items-center gap-2 text-[13.5px]">
                <FileAudio size={14} className="kx-accent-ink shrink-0" />
                <span className="truncate">{c.importPath.replace(/\\/g, "/").split("/").pop()}</span>
              </span>
              <span className="kx-meta">{c.importBusy ? "Transcribing…" : "Ready to transcribe"}</span>
            </button>
          </>
        )}

        {c.storeLoadError ? null : c.meetings.length === 0 && !recording ? (
          <div className="px-3 py-8 text-center flex flex-col items-center gap-2">
            <UsersRound size={22} className="kx-ink-2" />
            <p className="kx-meta">No meetings yet. Press Record, or import a recording.</p>
          </div>
        ) : c.filteredMeetings.length === 0 && c.search.trim() ? (
          <p className="kx-meta px-3 py-6 text-center">No meetings match “{c.search.trim()}”.</p>
        ) : (
          groups.map((g) => (
            <div key={g.label}>
              <div className="kx-overline px-2 pt-3 pb-1.5">{g.label}</div>
              <div className="flex flex-col gap-0.5">
                {g.items.map((m) => {
                  const st = status(m);
                  const d = durationOf(m);
                  const isActive = c.view.kind === "meeting" && m.id === c.activeId;
                  return (
                    <button
                      key={m.id}
                      type="button"
                      aria-current={isActive ? "true" : undefined}
                      onClick={() => c.selectMeeting(m.id)}
                      className="kx-list-item"
                    >
                      <span className="block truncate text-[13.5px]">{titleOf(m)}</span>
                      <span className="kx-meta flex items-center gap-1.5 min-w-0">
                        <span className="shrink-0">
                          {new Date(m.createdAt).toLocaleString(undefined, {
                            ...(g.label === "Today" || g.label === "Yesterday"
                              ? { hour: "2-digit", minute: "2-digit" }
                              : { day: "numeric", month: "short" }),
                          })}
                          {d ? ` · ${fmtDuration(d)}` : m.imported ? " · imported" : ""}
                        </span>
                        <span aria-hidden="true">·</span>
                        <span
                          className="truncate"
                          style={{
                            color:
                              st.tone === "accent"
                                ? "var(--kx-accent-ink)"
                                : st.tone === "warn"
                                  ? "var(--kx-warn)"
                                  : undefined,
                          }}
                        >
                          {st.text}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          ))
        )}
      </div>

      <div className="px-4 py-3 border-t border-[var(--kx-hairline-soft)] flex items-center justify-between gap-2">
        <span className="kx-meta">
          {c.meetings.length} meeting{c.meetings.length === 1 ? "" : "s"}
        </span>
        <button
          type="button"
          className="kx-btn kx-btn-quiet kx-btn-sm"
          aria-current={c.view.kind === "recordings" ? "true" : undefined}
          onClick={() => {
            void c.loadRecordings();
            c.setView({ kind: "recordings" });
          }}
        >
          Recordings & storage
          {c.recordings && c.recordings.length > 0 ? ` (${c.recordings.length})` : ""}
        </button>
      </div>
    </section>
  );
};
