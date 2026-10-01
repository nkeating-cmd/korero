/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useMemo, useState } from "react";
import {
  Circle,
  NotebookPen,
  Upload,
  UsersRound,
  Mic,
  Copy,
  Check,
  ChevronRight,
  TriangleAlert,
  CircleAlert,
} from "lucide-react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { commands, type HistoryEntry } from "@/bindings";
import i18n from "../../i18n";
import { formatRelativeTime } from "../../utils/dateFormat";
import { useNav } from "../../stores/navStore";
import { useRecorder } from "../../stores/recorderStore";
import { useMeetingJobs } from "../../stores/meetingJobsStore";
import { elapsedOf } from "../../stores/recorderProtocol";
import { useNotes, noteTitle } from "../../stores/notesStore";
import { useActivity } from "../../stores/activityStore";
import { meetingsBridge } from "../../stores/meetingJobsStore";
import { useSettings } from "../../hooks/useSettings";
import { useRunning, fmtSince } from "../shell/useRunning";
import { goTo } from "../shell/goTo";
import {
  durationOf,
  fmtClock,
  fmtDuration,
  normaliseMeetings,
  titleOf,
  type Meeting,
} from "../meetings/model";
import { HealthPanel } from "./HealthPanel";
import { formatBinding } from "./formatBinding";

/**
 * Kōrero 1.42: Today. What you can start, what is running, what you did
 * lately, and whether the setup is healthy — so a switch like New Zealand
 * English can never again sit off for months unnoticed.
 */

const greeting = (): string => {
  const h = new Date().getHours();
  return h < 12 ? "Mōrena" : "Kia ora";
};

const relative = (ms: number) =>
  formatRelativeTime(String(Math.floor(ms / 1000)), i18n.language);

type RecentRow =
  | { kind: "meeting"; at: number; meeting: Meeting }
  | { kind: "note"; at: number; id: string; title: string; words: number }
  | { kind: "dictation"; at: number; entry: HistoryEntry };

export const TodayPage: React.FC = () => {
  const { settings } = useSettings();
  const recording = useRecorder((s) => s.recording);
  const starting = useRecorder((s) => s.starting);
  const elapsedBase = useRecorder((s) => s.elapsedBase);
  const runStartedAt = useRecorder((s) => s.runStartedAt);
  const running = useRunning();
  const outcomes = useActivity((s) => s.items);
  const notes = useNotes((s) => s.notes);
  const [meetings, setMeetings] = useState<Meeting[] | null>(null);
  const [dictations, setDictations] = useState<HistoryEntry[] | null>(null);
  const [copiedId, setCopiedId] = useState<number | null>(null);

  // Re-read when anything finishes, so a just-saved meeting shows up here.
  const outcomeCount = outcomes.length;
  useEffect(() => {
    let cancelled = false;
    void meetingsBridge.load().then((r) => {
      if (cancelled) return;
      try {
        setMeetings(r.ok && r.data.trim() ? normaliseMeetings(JSON.parse(r.data)) : []);
      } catch {
        setMeetings([]);
      }
    });
    commands
      .getHistoryEntries(null, 8)
      .then((r) => {
        if (!cancelled) setDictations(r.status === "ok" ? r.data.entries : []);
      })
      .catch(() => !cancelled && setDictations([]));
    return () => {
      cancelled = true;
    };
  }, [outcomeCount]);

  const recent: RecentRow[] = useMemo(() => {
    const rows: RecentRow[] = [];
    for (const m of meetings ?? []) rows.push({ kind: "meeting", at: m.createdAt, meeting: m });
    for (const n of notes) {
      if (!n.content.trim()) continue;
      rows.push({
        kind: "note",
        at: n.updatedAt,
        id: n.id,
        title: noteTitle(n),
        words: n.content.trim().split(/\s+/).length,
      });
    }
    for (const e of dictations ?? []) {
      if (!(e.post_processed_text || e.transcription_text).trim()) continue;
      const ms = e.timestamp < 1e12 ? e.timestamp * 1000 : e.timestamp;
      rows.push({ kind: "dictation", at: ms, entry: e });
    }
    return rows.sort((a, b) => b.at - a.at).slice(0, 6);
  }, [meetings, notes, dictations]);

  const recentOutcomes = outcomes
    .filter((o) => Date.now() - o.at < 3 * 3600_000)
    .slice(0, 3);

  const dictateKey = formatBinding(settings?.bindings?.transcribe?.current_binding);

  const startMeeting = async () => {
    if (recording) {
      useNav.getState().openMeeting({ id: "live" });
      return;
    }
    const ok = await useRecorder.getState().start();
    if (ok) useNav.getState().openMeeting({ id: "live" });
  };

  const newNote = () => {
    const id = useNotes.getState().addNote();
    useNav.getState().openNote(id);
  };

  const importAudio = async () => {
    if (useRecorder.getState().recording || useMeetingJobs.getState().stopping) {
      toast.message("Finish the meeting first: importing uses the same speech engine.");
      return;
    }
    try {
      const sel = await openFileDialog({
        multiple: false,
        filters: [
          {
            name: "Audio files",
            extensions: ["wav", "m4a", "mp3", "aac", "flac", "ogg", "caf", "aiff", "aif"],
          },
        ],
      });
      if (typeof sel === "string") useNav.getState().openMeeting({ id: `import:${sel}` });
    } catch (e) {
      toast.error(`Could not open the file picker: ${String(e)}`);
    }
  };

  const copyDictation = async (e: HistoryEntry) => {
    try {
      await writeText(e.post_processed_text || e.transcription_text);
      setCopiedId(e.id);
      window.setTimeout(() => setCopiedId((c) => (c === e.id ? null : c)), 1500);
    } catch {
      toast.error("Could not copy to clipboard.");
    }
  };

  const today = new Date().toLocaleDateString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
  });

  return (
    <div className="kx-page kx-today">
      <div className="kx-today-main flex flex-col gap-5 min-w-0">
        <header>
          <p className="kx-overline">{today}</p>
          <h1 className="kx-display mt-1.5">{greeting()}</h1>
          <p className="kx-meta mt-1.5 text-[14px]">
            {dictateKey ? (
              <>
                Press <span className="kx-kbd">{dictateKey}</span> in any app to dictate.
              </>
            ) : (
              "Set a dictation shortcut to dictate in any app."
            )}{" "}
            Your audio never leaves this computer.
          </p>
        </header>

        <div className="grid gap-3 kx-today-actions">
          <button
            type="button"
            onClick={startMeeting}
            disabled={starting}
            className="kx-action kx-action-primary"
          >
            <span className="kx-accent-ink">
              {recording ? <UsersRound size={20} /> : <Circle size={18} fill="currentColor" />}
            </span>
            <span className="kx-heading text-[14.5px]">
              {recording ? "Meeting in progress" : "Record a meeting"}
            </span>
            <span className="text-[12.5px] text-[var(--kx-selected-ink)]">
              {recording
                ? `${fmtClock(elapsedOf({ elapsedBase, runStartedAt }))} · show it`
                : "Your mic and the call's audio, kept apart"}
            </span>
          </button>
          <button type="button" onClick={newNote} className="kx-action">
            <span className="kx-accent-ink">
              <NotebookPen size={20} />
            </span>
            <span className="kx-heading text-[14.5px]">New note</span>
            <span className="kx-meta">Long-form dictation, in the app</span>
          </button>
          <button type="button" onClick={importAudio} className="kx-action">
            <span className="kx-accent-ink">
              <Upload size={20} />
            </span>
            <span className="kx-heading text-[14.5px]">Import audio</span>
            <span className="kx-meta">A recording from your phone or Teams</span>
          </button>
        </div>

        {(running.length > 0 || recentOutcomes.length > 0) && (
          <section aria-labelledby="today-bg" className="kx-card">
            <div className="kx-card-header">
              <h2 id="today-bg" className="kx-heading">
                {running.length > 0 ? "Working in the background" : "Just finished"}
              </h2>
              <span className="kx-meta">Carries on if you leave this page</span>
            </div>
            <div className="kx-divide">
              {running.map((r) => (
                <div key={r.id} className="flex items-center gap-3.5 px-4 py-3">
                  <div className="flex-1 min-w-0">
                    <div className="flex justify-between gap-3">
                      <span className="kx-heading">{r.title}</span>
                      <span className="kx-meta kx-mono">
                        {r.progress !== null ? `${r.progress}%` : fmtSince(r.startedAt)}
                      </span>
                    </div>
                    <div className="kx-meta truncate">{r.detail}</div>
                    {r.kind !== "recording" && (
                      <div className={`kx-progress mt-2 ${r.progress === null ? "kx-progress-indeterminate" : ""}`}>
                        <span style={r.progress !== null ? { width: `${r.progress}%` } : undefined} />
                      </div>
                    )}
                  </div>
                  <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm shrink-0" onClick={() => goTo(r.target)}>
                    Open
                  </button>
                </div>
              ))}
              {recentOutcomes.map((o) => (
                <div key={o.id} className="flex items-center gap-3.5 px-4 py-3">
                  <span className="shrink-0">
                    {o.status === "done" ? (
                      <Check size={17} className="text-[var(--kx-ok)]" aria-label="Done" />
                    ) : o.status === "attention" ? (
                      <TriangleAlert size={17} className="text-[var(--kx-warn)]" aria-label="Needs a look" />
                    ) : (
                      <CircleAlert size={17} className="text-[var(--kx-alert)]" aria-label="Failed" />
                    )}
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="kx-heading">{o.title}</div>
                    <div className="kx-meta truncate">
                      {[o.detail, relative(o.at)].filter(Boolean).join(" · ")}
                    </div>
                  </div>
                  {o.target && (
                    <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm shrink-0" onClick={() => goTo(o.target)}>
                      View
                    </button>
                  )}
                </div>
              ))}
            </div>
          </section>
        )}

        <section aria-labelledby="today-recent" className="kx-card">
          <div className="kx-card-header">
            <h2 id="today-recent" className="kx-heading">
              Recent
            </h2>
            <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => useNav.getState().setPaletteOpen(true)}>
              Search everything
            </button>
          </div>
          {meetings === null || dictations === null ? (
            <p className="px-4 py-6 kx-meta text-center">Loading…</p>
          ) : recent.length === 0 ? (
            <div className="px-4 py-8 flex flex-col items-center gap-1.5 text-center">
              <Mic size={22} className="kx-ink-2" />
              <p className="text-[13.5px] text-[var(--kx-ink-soft)]">Nothing yet</p>
              <p className="kx-meta">Dictate, record a meeting or start a note, and it will show up here.</p>
            </div>
          ) : (
            <div className="kx-divide">
              {recent.map((r) =>
                r.kind === "meeting" ? (
                  <button
                    key={`m-${r.meeting.id}`}
                    type="button"
                    className="kx-row"
                    onClick={() => useNav.getState().openMeeting({ id: r.meeting.id })}
                  >
                    <UsersRound size={17} className="kx-ink-2 shrink-0" />
                    <span className="flex-1 min-w-0">
                      <span className="block truncate text-[13.5px]">{titleOf(r.meeting)}</span>
                      <span className="kx-meta">
                        {[
                          "Meeting",
                          relative(r.at),
                          durationOf(r.meeting) ? fmtDuration(durationOf(r.meeting) as number) : null,
                          r.meeting.processed.trim() ? "notes ready" : "transcript only",
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </span>
                    <ChevronRight size={16} className="kx-ink-2 shrink-0" />
                  </button>
                ) : r.kind === "note" ? (
                  <button key={`n-${r.id}`} type="button" className="kx-row" onClick={() => useNav.getState().openNote(r.id)}>
                    <NotebookPen size={17} className="kx-ink-2 shrink-0" />
                    <span className="flex-1 min-w-0">
                      <span className="block truncate text-[13.5px]">{r.title}</span>
                      <span className="kx-meta">
                        Note · {relative(r.at)} · {r.words.toLocaleString()} words
                      </span>
                    </span>
                    <ChevronRight size={16} className="kx-ink-2 shrink-0" />
                  </button>
                ) : (
                  <div key={`d-${r.entry.id}`} className="flex items-center gap-3.5 px-4 py-[11px]">
                    <Mic size={17} className="kx-ink-2 shrink-0" />
                    <span className="flex-1 min-w-0">
                      <span className="block truncate text-[13.5px]">
                        {r.entry.post_processed_text || r.entry.transcription_text}
                      </span>
                      <span className="kx-meta">
                        Dictation · {relative(r.at)}
                        {r.entry.post_processed_text ? " · cleaned up" : ""}
                        {r.entry.post_process_requested && !r.entry.post_processed_text && (
                          <span className="text-[var(--kx-warn)]"> · clean-up failed, pasted as spoken</span>
                        )}
                      </span>
                    </span>
                    <button
                      type="button"
                      aria-label="Copy dictation"
                      title="Copy"
                      onClick={() => copyDictation(r.entry)}
                      className="kx-btn kx-btn-secondary kx-btn-icon kx-btn-sm shrink-0"
                    >
                      {copiedId === r.entry.id ? <Check size={14} /> : <Copy size={14} />}
                    </button>
                  </div>
                ),
              )}
            </div>
          )}
        </section>
      </div>

      <aside className="kx-today-aside flex flex-col gap-4">
        <HealthPanel meetings={meetings} />
        <section aria-labelledby="today-keys" className="kx-card p-4">
          <h2 id="today-keys" className="kx-heading mb-2">
            Your shortcuts
          </h2>
          {(
            [
              ["transcribe", "Dictate"],
              ["transcribe_alt", "Dictate, one hand"],
              ...(settings?.post_process_enabled
                ? ([["transcribe_with_post_process", "Dictate + clean up"]] as const)
                : []),
            ] as const
          ).map(([id, label]) => {
            const key = formatBinding(settings?.bindings?.[id]?.current_binding);
            if (!key) return null;
            return (
              <div key={id} className="flex justify-between items-center py-1.5">
                <span className="text-[13px] text-[var(--kx-ink-soft)]">{label}</span>
                <span className="kx-kbd">{key}</span>
              </div>
            );
          })}
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm mt-1.5 -ml-2.5" onClick={() => useNav.getState().go("general")}>
            Change shortcuts
          </button>
        </section>
      </aside>
    </div>
  );
};
