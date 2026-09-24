// Kōrero (v1.30.2, reworked 2026-09-25): ALL long-running meeting work lives
// here, outside React, so it survives leaving the Meetings tab.
//
// HISTORY. v1.30.2 lifted post-processing (the notes) out of MeetingsSettings,
// because App.tsx unmounts the active section's component on every tab change
// and the notes were being generated into a dead `setState`. It left three
// things behind, and those are what "it doesn't run in the background" kept
// meaning after v1.30.2:
//
//   - STOP. Turning a stopped recording into a meeting happened inside the
//     view. Leaving the tab while it said "Processing…" meant the meeting was
//     never created; only the WAVs survived, in the Recordings list.
//   - TRANSCRIPTION (Re-transcribe, Transcribe + post-process, Import,
//     Recover). The result was written to disk "because the view is gone" by
//     checking the STARTING view's own mountedRef — so if you had come back in
//     the meantime, the new view saved its older copy over it. See
//     meetingsBridge.ts for that race and its fix.
//   - BUSY STATE. Spinners and progress for all of the above were component
//     state, so coming back mid-run showed idle buttons over running work, and
//     a second click queued a duplicate run.
//
// Now: one job slot for model work (`job`: notes, refine), one for
// transcription work (`task`), and a `stopping` flag for Stop. Every result is
// handed to `meetingsBridge`, which puts it in whichever Meetings view is open,
// or on disk if none is. The view only READS this store to draw its buttons.
//
// Scope stays "one at a time" (task OR job), as before: the transcription
// engine and the local model are both single, shared resources. Stop is never
// refused — it must always be possible to end a meeting.

import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { commands } from "@/bindings";
import {
  createMeetingsBridge,
  type DeliveryOutcome,
  type MeetingDoc,
} from "./meetingsBridge";

// ---------------------------------------------------------------------------
// The single route to meetings.json (see meetingsBridge.ts).
export const meetingsBridge = createMeetingsBridge({
  load: async () => {
    try {
      const r = await commands.meetingsStoreLoad();
      return r.status === "ok"
        ? { ok: true as const, data: r.data }
        : {
            ok: false as const,
            error: String(r.error ?? "could not read the meetings store"),
          };
    } catch (e) {
      return { ok: false as const, error: errText(e) };
    }
  },
  save: async (json) => {
    try {
      const r = await commands.meetingsStoreSave(json);
      return r.status === "ok"
        ? { ok: true as const }
        : { ok: false as const, error: String(r.error) };
    } catch (e) {
      return { ok: false as const, error: errText(e) };
    }
  },
});

// ---------------------------------------------------------------------------
// Types

export type MeetingJobKind = "post" | "both" | "refine";

export interface MeetingJob {
  meetingId: string;
  kind: MeetingJobKind;
  startedAt: number;
  /** Streamed tokens so far — the live preview, kept while the tab is closed. */
  live: string;
  status: "running";
}

export type MeetingTaskKind = "transcribe" | "both" | "import" | "recover";

export interface MeetingTask {
  kind: MeetingTaskKind;
  /** The meeting being re-transcribed; null for an import/recover (it has no
   *  meeting until the transcript exists). */
  meetingId: string | null;
  /** Human label for "Busy with …" when the work is not the meeting on screen. */
  label: string;
  /** The file being recovered (so its row in Recordings can show a spinner). */
  path?: string;
  startedAt: number;
  progress: { window: number; total: number | null } | null;
}

export interface TranscriptSegLike {
  source: string;
  text: string;
  start_ms?: number;
}

/** The transcript fields a (re-)transcription produces. */
export interface TranscriptPatch {
  you: string;
  others: string;
  transcript: TranscriptSegLike[];
}

interface MeetingJobsState {
  job: MeetingJob | null;
  task: MeetingTask | null;
  stopping: { startedAt: number } | null;

  /** Generate notes. Resolves false only if the run was REFUSED (busy). */
  start: (args: {
    meetingId: string;
    kind: "post" | "both";
    text: string;
    prompt: string;
    trimKey: string;
  }) => Promise<boolean>;

  /** Revise existing notes with the reader's feedback (Undo in the toast).
   *  Resolves true when the refined notes landed. */
  refine: (args: {
    meetingId: string;
    previous: string;
    feedback: string;
  }) => Promise<boolean>;

  /** Re-transcribe a meeting's audio; with `thenNotes`, generate notes after. */
  transcribe: (args: {
    meetingId: string;
    title: string;
    micPath: string | null;
    systemPath: string | null;
    thenNotes?: {
      /** Pure: builds the model input from the NEW transcript (trim applied). */
      buildText: (t: TranscriptPatch) => string;
      prompt: string;
      trimKey: string;
    };
  }) => Promise<boolean>;

  /** Transcribe an audio file into a new meeting; optionally generate notes.
   *  `notesPrompt` null = transcript only. */
  importFile: (args: {
    path: string;
    label: string;
    /** Pure: builds the new meeting from the transcript text. */
    makeMeeting: (transcript: string) => MeetingDoc;
    notesPrompt: string | null;
  }) => Promise<boolean>;

  /** Transcribe a WAV from the Recordings list into a new meeting. */
  recover: (args: {
    path: string;
    label: string;
    makeMeeting: (transcript: string) => MeetingDoc;
  }) => Promise<boolean>;

  /** Stop the running meeting and turn it into a meeting entry. */
  stopMeeting: (args: {
    /** Pure: builds the new meeting from the stop result. */
    makeMeeting: (r: {
      you: string;
      others: string;
      segments: TranscriptSegLike[];
      mic_path: string | null;
      system_path: string | null;
    }) => MeetingDoc;
  }) => Promise<void>;
}

type SetState = (
  fn: (s: MeetingJobsState) => Partial<MeetingJobsState>,
) => void;

// ---------------------------------------------------------------------------
// Module-level listeners, registered once, so they keep working while the
// Meetings tab is closed.

let deltaListenerAttached = false;
let globalListenersAttached = false;

function attachDeltaListener(set: SetState) {
  if (deltaListenerAttached) return;
  deltaListenerAttached = true;
  // Reset on failure so the next run retries; the preview is cosmetic, the
  // run itself is unaffected (review finding, v1.30.2).
  listen<string>("meeting-postprocess-delta", (e) => {
    set((s) =>
      s.job ? { job: { ...s.job, live: s.job.live + e.payload } } : {},
    );
  }).catch((err) => {
    deltaListenerAttached = false;
    console.error("Could not attach the post-process delta listener:", err);
  });
}

function attachGlobalListeners(set: SetState) {
  if (globalListenersAttached) return;
  globalListenersAttached = true;
  // Chunked-transcription progress, for whichever task is running.
  listen<{ id: string; window: number; total: number | null }>(
    "meeting-transcribe-progress",
    (e) => {
      set((s) =>
        s.task
          ? {
              task: {
                ...s.task,
                progress: { window: e.payload.window, total: e.payload.total },
              },
            }
          : {},
      );
    },
  ).catch((err) =>
    console.error("Could not attach the transcription progress listener:", err),
  );
  // Capture-health warnings raised DURING a meeting (silent microphone, no
  // system audio). A toast rather than Meetings-page state: the point is to
  // reach the user wherever they are in the app, while they can still fix it.
  listen<string>("meeting-capture-warning", (e) => {
    toast.warning(e.payload, { duration: 30_000 });
  }).catch((err) =>
    console.error("Could not attach the capture-warning listener:", err),
  );
}

// ---------------------------------------------------------------------------
// Helpers

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Say where a result went when it did NOT land on screen. */
function reportLanding(
  outcome: DeliveryOutcome,
  msgs: { done: string; saved: string; what: string },
) {
  switch (outcome) {
    case "shown":
      return;
    case "view":
      toast.success(msgs.done);
      return;
    case "disk":
      toast.success(msgs.saved);
      return;
    case "missing":
      toast.message(
        `${msgs.what} finished, but that meeting has been deleted.`,
      );
      return;
    case "failed":
      toast.error(
        `${msgs.what} finished, but the meetings store could not be written. Nothing on disk was changed; please try again.`,
      );
      return;
  }
}

const NOTES_MSGS = {
  done: "Meeting notes are ready.",
  saved: "Meeting notes are ready and saved — open Meetings to see them.",
  what: "Post-processing",
};
const TRANSCRIPT_MSGS = {
  done: "Transcription finished.",
  saved: "Transcription finished and was saved.",
  what: "Transcription",
};

/** Re-transcribe recorded WAVs chronologically; non-WAV imports per file. */
async function transcribeAudio(
  micPath: string | null,
  systemPath: string | null,
): Promise<TranscriptPatch> {
  const isWav = (p: string | null) => !!p && /\.wav$/i.test(p);
  if (isWav(micPath) || isWav(systemPath)) {
    const r = await commands.meetingTranscribeMerge(
      isWav(micPath) ? micPath : null,
      isWav(systemPath) ? systemPath : null,
    );
    if (r.status !== "ok") throw new Error(r.error);
    const transcript = r.data as TranscriptSegLike[];
    const join = (src: string) =>
      transcript
        .filter((s) => s.source === src)
        .map((s) => s.text)
        .join(" ");
    return { you: join("you"), others: join("others"), transcript };
  }
  const tx = async (path: string | null) => {
    if (!path) return "";
    const r = await commands.meetingTranscribeFile(path);
    if (r.status === "ok") return r.data;
    throw new Error(r.error);
  };
  const you = await tx(micPath);
  const others = await tx(systemPath);
  return { you, others, transcript: [] };
}

// ---------------------------------------------------------------------------

export const useMeetingJobs = create<MeetingJobsState>((set, get) => {
  attachGlobalListeners(set);

  const isBusy = () => !!get().task || !!get().job;

  const refuse = () => {
    const s = get();
    const what = s.task?.label ?? (s.job ? "generating notes" : "another job");
    toast.message(
      `Busy ${what} — one job at a time. Try again when it finishes.`,
    );
    return false;
  };

  /** Claim the task slot. Returns the claim time (its identity) or null. */
  const claimTask = (t: Omit<MeetingTask, "startedAt" | "progress">) => {
    if (isBusy()) return null;
    const startedAt = Date.now();
    set(() => ({ task: { ...t, startedAt, progress: null } }));
    return startedAt;
  };
  const releaseTask = (startedAt: number) =>
    set((s) =>
      s.task && s.task.startedAt === startedAt ? { task: null } : {},
    );

  /**
   * Run one model job in the single job slot and deliver its patch. Claims the
   * slot SYNCHRONOUSLY (before the first await), so callers can hand over from
   * a task without the buttons flickering idle in between.
   */
  const runJob = async (args: {
    meetingId: string;
    kind: MeetingJobKind;
    text: string;
    prompt: string;
    patchFor: (result: string) => Record<string, unknown>;
    /** Called with where the result landed; default reports it as notes. */
    onLanded?: (outcome: DeliveryOutcome) => void;
    failLabel: string;
  }): Promise<boolean> => {
    if (isBusy()) return false;
    attachDeltaListener(set);
    const startedAt = Date.now();
    set(() => ({
      job: {
        meetingId: args.meetingId,
        kind: args.kind,
        startedAt,
        live: "",
        status: "running" as const,
      },
    }));
    try {
      const r = await commands.meetingPostProcess(args.text, args.prompt);
      if (r.status !== "ok") throw new Error(r.error);
      const outcome = await meetingsBridge.deliverPatch(
        args.meetingId,
        args.patchFor(r.data),
      );
      (args.onLanded ?? ((o) => reportLanding(o, NOTES_MSGS)))(outcome);
    } catch (e) {
      toast.error(`${args.failLabel} failed: ${errText(e)}`);
    } finally {
      set((s) => (s.job && s.job.startedAt === startedAt ? { job: null } : {}));
    }
    return true;
  };

  const notesJob = (a: {
    meetingId: string;
    kind: "post" | "both";
    text: string;
    prompt: string;
    trimKey: string;
  }) =>
    runJob({
      meetingId: a.meetingId,
      kind: a.kind,
      text: a.text,
      prompt: a.prompt,
      failLabel: "Post-processing",
      patchFor: (processed) => ({
        processed,
        processPrompt: a.prompt,
        processedTrimKey: a.trimKey,
      }),
    });

  return {
    job: null,
    task: null,
    stopping: null,

    start: async (a) => {
      if (isBusy()) return refuse();
      return notesJob(a);
    },

    refine: async ({ meetingId, previous, feedback }) => {
      if (isBusy()) return refuse();
      const prompt =
        "You are revising EXISTING meeting notes based on the reader's feedback. " +
        "Apply the feedback faithfully, keep the same Markdown structure and headings where still appropriate, " +
        "do not invent facts or add content not supported by the notes, and output ONLY the revised notes " +
        'with no preamble or commentary. Feedback: "' +
        feedback +
        '".';
      let landed = false;
      await runJob({
        meetingId,
        kind: "refine",
        text: previous.trim(),
        prompt,
        failLabel: "Refine",
        patchFor: (processed) => ({ processed }),
        onLanded: (outcome) => {
          if (outcome === "missing" || outcome === "failed") {
            reportLanding(outcome, {
              ...NOTES_MSGS,
              what: "Refining the notes",
            });
            return;
          }
          landed = true;
          toast.success(
            outcome === "disk" ? "Notes refined and saved." : "Notes refined.",
            {
              action: {
                label: "Undo",
                // Through the bridge like everything else, so Undo works whether
                // or not the Meetings view is open when it is clicked.
                onClick: () => {
                  void meetingsBridge
                    .deliverPatch(meetingId, { processed: previous })
                    .then((o) => {
                      if (o === "failed" || o === "missing") {
                        toast.error(
                          "Could not undo — the notes could not be saved.",
                        );
                      }
                    });
                },
              },
            },
          );
        },
      });
      return landed;
    },

    transcribe: async ({
      meetingId,
      title,
      micPath,
      systemPath,
      thenNotes,
    }) => {
      const claim = claimTask({
        kind: thenNotes ? "both" : "transcribe",
        meetingId,
        label: `transcribing “${title}”`,
      });
      if (claim === null) return refuse();
      try {
        const patch = await transcribeAudio(micPath, systemPath);
        const outcome = await meetingsBridge.deliverPatch(meetingId, {
          ...patch,
        });
        if (outcome === "missing" || outcome === "failed") {
          reportLanding(outcome, TRANSCRIPT_MSGS);
          return false;
        }
        const empty = !patch.you.trim() && !patch.others.trim();
        if (!thenNotes) {
          if (empty) toast.message("Still no speech found.");
          else reportLanding(outcome, TRANSCRIPT_MSGS);
          return true;
        }
        const text = thenNotes.buildText(patch);
        if (!text.trim()) {
          toast.message("No speech found to post-process.");
          return true;
        }
        // Hand over to the notes job: release the task first (runJob refuses
        // while any task is held); runJob claims its slot synchronously.
        releaseTask(claim);
        void notesJob({
          meetingId,
          kind: "both",
          text,
          prompt: thenNotes.prompt,
          trimKey: thenNotes.trimKey,
        });
        return true;
      } catch (e) {
        toast.error(
          `${thenNotes ? "Transcribe + post-process" : "Re-transcription"} failed: ${errText(e)}`,
        );
        return false;
      } finally {
        releaseTask(claim);
      }
    },

    importFile: async ({ path, label, makeMeeting, notesPrompt }) => {
      const claim = claimTask({
        kind: "import",
        meetingId: null,
        label: `importing ${label}`,
      });
      if (claim === null) return refuse();
      try {
        const r = await commands.meetingTranscribeFile(path);
        if (r.status !== "ok") {
          toast.error(`Transcription failed: ${r.error}`);
          return false;
        }
        // Saved the moment a transcript exists, BEFORE any notes (v1.30.2):
        // the worst case is "transcript but no notes yet", never "nothing".
        const m = makeMeeting(r.data);
        const outcome = await meetingsBridge.deliverNew(m, true);
        if (outcome === "failed") {
          toast.error(
            "Transcription finished but the meetings store could not be written. The audio file is untouched; try importing again.",
          );
          return false;
        }
        toast.success(
          outcome === "disk"
            ? "Imported audio transcribed — open Meetings to see it."
            : "Imported audio transcribed.",
        );
        if (notesPrompt !== null && r.data.trim()) {
          releaseTask(claim);
          void notesJob({
            meetingId: m.id,
            kind: "post",
            text: r.data,
            prompt: notesPrompt,
            trimKey: ":",
          });
        }
        return true;
      } catch (e) {
        toast.error(`Import failed: ${errText(e)}`);
        return false;
      } finally {
        releaseTask(claim);
      }
    },

    recover: async ({ path, label, makeMeeting }) => {
      const claim = claimTask({
        kind: "recover",
        meetingId: null,
        label: `recovering ${label}`,
        path,
      });
      if (claim === null) return refuse();
      try {
        const r = await commands.meetingTranscribeFile(path);
        if (r.status !== "ok") {
          toast.error(`Transcription failed: ${r.error}`);
          return false;
        }
        const outcome = await meetingsBridge.deliverNew(
          makeMeeting(r.data),
          true,
        );
        if (outcome === "failed") {
          toast.error(
            "Transcription finished but the meetings store could not be written. The recording is untouched; try again.",
          );
          return false;
        }
        toast.success(
          outcome === "disk"
            ? "Recording transcribed and added to your meetings — open Meetings to see it."
            : "Recording transcribed and added to your meetings.",
        );
        return true;
      } catch (e) {
        toast.error(`Transcription failed: ${errText(e)}`);
        return false;
      } finally {
        releaseTask(claim);
      }
    },

    stopMeeting: async ({ makeMeeting }) => {
      if (get().stopping) return;
      set(() => ({ stopping: { startedAt: Date.now() } }));
      try {
        const res = await commands.meetingStopCapture();
        if (res.status !== "ok") {
          toast.error(`Meeting stop failed: ${res.error}`);
          return;
        }
        const { you, others, segments, mic_path, system_path } = res.data;
        const warnings = res.data.warnings ?? [];
        // Say WHY first (silent mic, no system audio, a side rebuilt from the
        // recording). These used to surface only as an empty transcript.
        for (const w of warnings) toast.warning(w, { duration: 30_000 });
        if (!you.trim() && !others.trim() && !mic_path && !system_path) {
          toast.message("No audio captured.");
          return;
        }
        const m = makeMeeting({
          you,
          others,
          segments: segments as TranscriptSegLike[],
          mic_path,
          system_path,
        });
        const outcome = await meetingsBridge.deliverNew(m, true);
        if (outcome === "failed") {
          toast.error(
            "The meeting was recorded, but the meetings store could not be written. The audio is safe — find it under Recordings and transcribe it from there.",
          );
          return;
        }
        if (outcome === "disk") {
          toast.success("Meeting saved — open Meetings to see it.");
        }
        if (you.trim() || others.trim()) {
          // v1.17.0: warm the local notes model so the first "Generate notes"
          // doesn't pay the cold model-load cost.
          commands.meetingPrewarmPostProcess().catch(() => {});
        } else if (warnings.length === 0) {
          toast.message(
            "Audio saved, but transcription was empty — you can re-transcribe it.",
          );
        }
      } catch (e) {
        toast.error(`Meeting stop failed: ${errText(e)}`);
      } finally {
        set(() => ({ stopping: null }));
      }
    },
  };
});
