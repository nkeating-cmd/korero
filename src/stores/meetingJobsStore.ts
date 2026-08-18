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
  /** Start a run. Returns immediately; progress lands in the store. */
  start: (args: {
    meetingId: string;
    kind: MeetingJobKind;
    text: string;
    prompt: string;
    trimKey: string;
  }) => Promise<void>;
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
// the notes back over a newer edit. Waiting until after that window means the
// two paths are mutually exclusive: exactly one of them writes.
const ADOPTION_GRACE_MS = 1200;

async function persistUnclaimedResult(
  meetingId: string,
  result: MeetingJobResult,
): Promise<void> {
  try {
    const res = await commands.meetingsStoreLoad();
    // Same rule the view enforces (v1.29.0, R-02): only ever save a store we
    // successfully READ. A read failure here must not write anything, or a
    // momentarily locked file becomes an empty meetings.json.
    if (res.status !== "ok" || !res.data.trim()) return;

    const list = JSON.parse(res.data) as Array<Record<string, unknown>>;
    if (!Array.isArray(list)) return;

    let found = false;
    const next = list.map((m) => {
      if (m && m.id === meetingId) {
        found = true;
        return { ...m, ...result };
      }
      return m;
    });
    if (!found) return; // meeting deleted while the model was working

    const saved = await commands.meetingsStoreSave(JSON.stringify(next));
    if (saved.status !== "ok") {
      console.error("Post-process result could not be saved:", saved.error);
    }
  } catch (e) {
    console.error("Post-process result could not be saved:", e);
  }
}

// ---------------------------------------------------------------------------
// Delta listener, registered ONCE at module load rather than per-mount. This is
// what lets the live preview keep accumulating while the tab is closed, so
// coming back shows the text generated in the meantime instead of a blank box.
let deltaListenerAttached = false;

function attachDeltaListener(set: (fn: (s: MeetingJobsState) => Partial<MeetingJobsState>) => void) {
  if (deltaListenerAttached) return;
  deltaListenerAttached = true;
  void listen<string>("meeting-postprocess-delta", (e) => {
    set((s) =>
      s.job && s.job.status === "running"
        ? { job: { ...s.job, live: s.job.live + e.payload } }
        : {},
    );
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
      return;
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
      window.setTimeout(() => {
        const j = get().job;
        if (j && j.meetingId === meetingId && !j.consumed && j.result) {
          void persistUnclaimedResult(meetingId, j.result);
        }
      }, ADOPTION_GRACE_MS);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set((s) =>
        s.job && s.job.meetingId === meetingId
          ? { job: { ...s.job, status: "error", error: message, live: "" } }
          : {},
      );
    }
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
