// Kōrero (v1.30.2): the meeting post-processing job, lifted OUT of React.
//
// WHY. App.tsx renders only the active section's component, so leaving the
// Meetings tab UNMOUNTS MeetingsSettings. Everything about a post-process run
// used to live inside that component:
//
//   - the awaited `commands.meetingPostProcess(...)` promise,
//   - the `meeting-postprocess-delta` listener feeding the live preview,
//   - the `busy` flag driving the spinner,
//   - and `patchMeeting`, which is a `setMeetings` on that component.
//
// So navigating away didn't stop the model — Rust kept generating, and the
// machine kept paying for it — but the answer came back to a dead `setState`
// and was dropped on the floor. Worse, the 500 ms debounced autosave cleared
// its own timer on unmount, so even a result that HAD landed a moment earlier
// was never written to meetings.json. From the outside this looks exactly like
// "navigating out of the tab cancels the job".
//
// The fix mirrors what audioBriefStore already does for Audio Brief: the job
// lives in a module-level singleton that outlives the view. It keeps running in
// the background, accumulates its streamed text here, and — this is the part
// that matters — guarantees the finished notes reach disk whether or not any
// component is alive to receive them.
//
// Scope: ONE job at a time, deliberately. The UI only ever offers one, and a
// single slot means "is something running?" has one true answer instead of a
// map the UI has to reconcile.

import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";

export type MeetingJobKind = "post" | "both";
export type MeetingJobStatus = "running" | "done" | "error";

/** The patch a finished run produces. Mirrors the three fields the old
 *  inline `patchMeeting` wrote, so adoption by the view is a straight spread. */
export interface MeetingJobResult {
  processed: string;
  processPrompt: string;
  processedTrimKey: string;
}

export interface MeetingJob {
  meetingId: string;
  kind: MeetingJobKind;
  startedAt: number;
  /** Streamed tokens so far — the live preview, rebuilt on return to the tab. */
  live: string;
  status: MeetingJobStatus;
  error?: string;
  /** Set on success, cleared once a mounted view has applied it. */
  result?: MeetingJobResult;
  /** True once a view has taken `result` into its own state. */
  consumed: boolean;
}

interface MeetingJobsState {
  job: MeetingJob | null;
  /**
   * Start a run. Resolves to false if one was already in flight and this start
   * was REFUSED — callers must surface that, or the user gets no notes and no
   * explanation. (Review finding: `runImport` fired and ignored the result, so
   * an import started during a meeting run silently produced nothing.)
   */
  start: (args: {
    meetingId: string;
    kind: MeetingJobKind;
    text: string;
    prompt: string;
    trimKey: string;
  }) => Promise<boolean>;
  /** A mounted view claims the finished result so it can patch its own state. */
  consume: (meetingId: string) => MeetingJobResult | null;
  /** Dismiss a finished/failed job so the banner goes away. */
  clear: () => void;
}

// ---------------------------------------------------------------------------
// Durable landing strip.
//
// If nobody claimed the result shortly after it arrived, the view is not
// mounted and there is no React state to patch — so write it into the meetings
// store ourselves, read-modify-write.
//
// The delay is longer than the view's 500 ms autosave debounce ON PURPOSE. When
// the view IS mounted it consumes the result, patches its own array and its
// debounce saves the whole document; doing our own write as well would race
// that save with a copy of the array we read BEFORE the patch, and could put
// the notes back over a newer edit. Waiting until after that window, AND
// claiming the job synchronously before the first await (see `start`), makes
// the two paths mutually exclusive: exactly one of them writes.
const ADOPTION_GRACE_MS = 1200;

/**
 * Read-modify-write a patch into the meetings store on disk.
 *
 * Used whenever the result of long-running work arrives with no mounted view to
 * receive it. Obeys the v1.29.0 R-02 rule without exception: only ever save a
 * store that was successfully READ, so a file momentarily locked by OneDrive or
 * antivirus can never be replaced with a truncated or empty document.
 */
export async function persistMeetingPatch(
  meetingId: string,
  patch: Record<string, unknown>,
): Promise<boolean> {
  try {
    const res = await commands.meetingsStoreLoad();
    if (res.status !== "ok" || !res.data.trim()) return false;
    const list = JSON.parse(res.data) as Array<Record<string, unknown>>;
    if (!Array.isArray(list)) return false;
    let found = false;
    const next = list.map((m) => {
      if (m && m.id === meetingId) {
        found = true;
        return { ...m, ...patch };
      }
      return m;
    });
    if (!found) return false; // meeting deleted while the work was running
    const saved = await commands.meetingsStoreSave(JSON.stringify(next));
    return saved.status === "ok";
  } catch (e) {
    console.error("Could not persist meeting patch:", e);
    return false;
  }
}

/**
 * Prepend a brand-new meeting to the store on disk.
 *
 * The import path needs this: transcription of a 45-minute file takes minutes,
 * and if the view is gone when it finishes there is no React state to add the
 * meeting to. Without this the entire import — transcript included — evaporated.
 */
export async function persistNewMeeting(
  meeting: Record<string, unknown>,
): Promise<boolean> {
  try {
    const res = await commands.meetingsStoreLoad();
    if (res.status !== "ok") return false;
    const list = res.data.trim()
      ? (JSON.parse(res.data) as Array<Record<string, unknown>>)
      : [];
    if (!Array.isArray(list)) return false;
    if (list.some((m) => m && m.id === meeting.id)) return true; // already there
    const saved = await commands.meetingsStoreSave(
      JSON.stringify([meeting, ...list]),
    );
    return saved.status === "ok";
  } catch (e) {
    console.error("Could not persist imported meeting:", e);
    return false;
  }
}

// ---------------------------------------------------------------------------
// Delta listener, registered ONCE at module load rather than per-mount. This is
// what lets the live preview keep accumulating while the tab is closed, so
// coming back shows the text generated in the meantime instead of a blank box.
let deltaListenerAttached = false;

function attachDeltaListener(
  set: (fn: (s: MeetingJobsState) => Partial<MeetingJobsState>) => void,
) {
  if (deltaListenerAttached) return;
  deltaListenerAttached = true;
  // Review finding: the flag was set before the async registration resolved and
  // there was no catch, so a rejected `listen()` killed the live preview for the
  // whole session AND produced an unhandled rejection. Reset on failure so the
  // next run retries; the preview is cosmetic, the run itself is unaffected.
  listen<string>("meeting-postprocess-delta", (e) => {
    set((s) =>
      s.job && s.job.status === "running"
        ? { job: { ...s.job, live: s.job.live + e.payload } }
        : {},
    );
  }).catch((err) => {
    deltaListenerAttached = false;
    console.error("Could not attach the post-process delta listener:", err);
  });
}

export const useMeetingJobs = create<MeetingJobsState>((set, get) => ({
  job: null,

  start: async ({ meetingId, kind, text, prompt, trimKey }) => {
    // One slot, one job. The UI disables its controls while a job runs, so this
    // is a backstop — but without it a second start would overwrite the slot and
    // the first run's answer would arrive to a job that no longer exists,
    // silently. Refusing is the only outcome that cannot lose work.
    const existing = get().job;
    if (existing && existing.status === "running") {
      console.warn(
        `Refusing to start a second post-process run; ${existing.meetingId} is still generating.`,
      );
      return false;
    }
    attachDeltaListener(set);
    set({
      job: {
        meetingId,
        kind,
        startedAt: Date.now(),
        live: "",
        status: "running",
        consumed: false,
      },
    });

    try {
      const r = await commands.meetingPostProcess(text, prompt);
      if (r.status !== "ok") throw new Error(r.error);

      const result: MeetingJobResult = {
        processed: r.data,
        processPrompt: prompt,
        processedTrimKey: trimKey,
      };
      set((s) =>
        s.job && s.job.meetingId === meetingId
          ? { job: { ...s.job, status: "done", result, live: "" } }
          : {},
      );

      // Give a mounted view its chance, then guarantee the result reaches disk.
      //
      // Review finding: this was a check-then-act. It read `!consumed`, then
      // awaited a multi-megabyte store load, then wrote back the snapshot it had
      // read. If the view committed during that await — plausible on a long
      // transcript, where React is busy re-rendering the streamed markdown —
      // the write reverted every edit made since the last save. Claiming the
      // job SYNCHRONOUSLY before any await closes the window: after this `set`,
      // `consume()` returns null and the view leaves the disk write to us.
      window.setTimeout(() => {
        const j = get().job;
        if (!j || j.meetingId !== meetingId || j.consumed || !j.result) return;
        set({ job: { ...j, consumed: true } });
        void persistMeetingPatch(meetingId, { ...j.result }).then((ok) => {
          if (!ok) {
            console.error(
              "Post-process result could not be written to the meetings store.",
            );
          }
        });
      }, ADOPTION_GRACE_MS);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set((s) =>
        s.job && s.job.meetingId === meetingId
          ? { job: { ...s.job, status: "error", error: message, live: "" } }
          : {},
      );
    }
    return true;
  },

  consume: (meetingId) => {
    const j = get().job;
    if (!j || j.meetingId !== meetingId || j.status !== "done" || !j.result) {
      return null;
    }
    set({ job: { ...j, consumed: true } });
    return j.result;
  },

  clear: () => set({ job: null }),
}));
