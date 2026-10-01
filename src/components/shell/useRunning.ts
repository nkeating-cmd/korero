// Kōrero 1.42: everything running right now, derived live from the stores that
// own the work. Used by the Activity panel, the top bar badge and Today, so all
// three always agree.

import { useEffect, useState } from "react";
import { useMeetingJobs } from "../../stores/meetingJobsStore";
import { useNotes, noteTitle } from "../../stores/notesStore";
import { useRecorder } from "../../stores/recorderStore";
import { elapsedOf } from "../../stores/recorderProtocol";
import type { ActivityTarget } from "../../stores/activityStore";

export interface RunningItem {
  id: string;
  title: string;
  detail: string;
  /** 0–100, or null when the length is unknown. */
  progress: number | null;
  startedAt: number;
  target: ActivityTarget;
  kind: "recording" | "stopping" | "transcribe" | "notes" | "dictation" | "note-process";
}

/** Re-render once a second while anything is running (for elapsed clocks). */
export const useTick = (active: boolean): void => {
  const [, setN] = useState(0);
  useEffect(() => {
    if (!active) return;
    const t = window.setInterval(() => setN((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [active]);
};

const words = (s: string) => (s.trim() ? s.trim().split(/\s+/).length : 0);

export const useRunning = (): RunningItem[] => {
  const job = useMeetingJobs((s) => s.job);
  const task = useMeetingJobs((s) => s.task);
  const stopping = useMeetingJobs((s) => s.stopping);
  const recording = useRecorder((s) => s.recording);
  const paused = useRecorder((s) => s.paused);
  const elapsedBase = useRecorder((s) => s.elapsedBase);
  const runStartedAt = useRecorder((s) => s.runStartedAt);
  const recTitle = useRecorder((s) => s.title);
  const dictation = useNotes((s) => s.dictation);
  const dictationNoteId = useNotes((s) => s.dictationNoteId);
  const dictationStartedAt = useNotes((s) => s.dictationStartedAt);
  const processing = useNotes((s) => s.processing);
  const notes = useNotes((s) => s.notes);

  const items: RunningItem[] = [];

  if (recording) {
    const secs = Math.floor(elapsedOf({ elapsedBase, runStartedAt }));
    const mm = `${Math.floor(secs / 60)}:${(secs % 60).toString().padStart(2, "0")}`;
    items.push({
      id: "recording",
      kind: "recording",
      title: paused ? "Recording paused" : "Recording a meeting",
      detail: `${recTitle || "Meeting"} · ${mm}`,
      progress: null,
      startedAt: Date.now() - secs * 1000,
      target: { section: "meetings", meetingId: "live" },
    });
  }
  if (stopping) {
    items.push({
      id: "stopping",
      kind: "stopping",
      title: "Saving the meeting",
      detail: "Finishing the transcript; rebuilding any missed speech from the recording",
      progress: null,
      startedAt: stopping.startedAt,
      target: { section: "meetings" },
    });
  }
  if (task) {
    const p = task.progress;
    const label =
      task.kind === "import"
        ? "Transcribing an import"
        : task.kind === "recover"
          ? "Recovering a recording"
          : task.kind === "both"
            ? "Re-transcribing, then notes"
            : "Re-transcribing";
    items.push({
      id: "task",
      kind: "transcribe",
      title: label,
      detail:
        task.label.replace(/^(transcribing|importing|recovering)\s+/i, "") +
        (p ? ` · part ${p.window}${p.total ? ` of ${p.total}` : ""}` : ""),
      progress: p && p.total ? Math.min(100, Math.round((p.window / p.total) * 100)) : null,
      startedAt: task.startedAt,
      target: { section: "meetings", meetingId: task.meetingId ?? undefined, tab: "transcript" },
    });
  }
  if (job) {
    const n = words(job.live);
    items.push({
      id: "job",
      kind: "notes",
      title: job.kind === "refine" ? "Refining notes" : "Writing notes",
      detail: `${job.title ?? "Meeting"}${n ? ` · ${n.toLocaleString()} words so far` : ""}`,
      progress: null,
      startedAt: job.startedAt,
      target: { section: "meetings", meetingId: job.meetingId, tab: "notes" },
    });
  }
  if (dictation === "recording" || dictation === "finishing") {
    const note = notes.find((x) => x.id === dictationNoteId);
    items.push({
      id: "dictation",
      kind: "dictation",
      title: dictation === "finishing" ? "Transcribing a note dictation" : "Dictating into a note",
      detail: note ? noteTitle(note) : "Note",
      progress: null,
      startedAt: dictationStartedAt ?? Date.now(),
      target: { section: "notes", noteId: dictationNoteId ?? undefined },
    });
  }
  if (processing) {
    const note = notes.find((x) => x.id === processing.noteId);
    items.push({
      id: "note-process",
      kind: "note-process",
      title: "Processing a note",
      detail: note ? noteTitle(note) : "Note",
      progress: null,
      startedAt: processing.startedAt,
      target: { section: "notes", noteId: processing.noteId },
    });
  }

  useTick(items.length > 0);
  return items;
};

export const fmtSince = (startedAt: number): string => {
  const s = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  return `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;
};
