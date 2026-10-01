// Kōrero 1.42: the meeting recorder, outside React.
//
// Recording state used to be component state inside MeetingsSettings, so only
// the Meetings page could start, see or stop a meeting. Now Today, Ctrl K, the
// top bar, the Live view and the pop-out companion window all drive the same
// recorder. Stop is still owned by `useMeetingJobs` (it must survive any view
// unmounting); this store builds the meeting it saves.
//
// The companion window is a separate webview with its own JavaScript. It never
// owns state: this store (in the main window) broadcasts a snapshot on every
// change, and the companion sends commands back as events. One owner, so the
// two windows can never disagree about whether a meeting is running.

import { create } from "zustand";
export type { LiveSegment, CaptureWarning, RecorderSnapshot } from "./recorderProtocol";
import { emit, listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useMeetingJobs } from "./meetingJobsStore";
import { logActivity } from "./activityStore";
import { useNav } from "./navStore";
import type { MeetingDoc } from "./meetingsBridge";
import {
  COMPANION_LABEL,
  RECORDER_COMMAND_EVENT,
  RECORDER_STATE_EVENT,
  elapsedOf,
  type RecorderCommand,
  type RecorderSnapshot,
} from "./recorderProtocol";
import {
  defaultMeetingTitle,
  newId,
  type Meeting,
} from "../components/meetings/model";

interface RecorderState extends Omit<RecorderSnapshot, "stopping"> {
  starting: boolean;
  /** An 8-second capture test is running. */
  testing: boolean;
  /** When the last capture test finished, and what it found (this session). */
  lastTest: { at: number; micOk: boolean; systemOk: boolean } | null;
  captureError: string | null;
  restore: () => Promise<void>;
  start: () => Promise<boolean>;
  stop: () => Promise<void>;
  togglePause: () => Promise<void>;
  flag: (label?: string) => void;
  setTitle: (title: string) => void;
  refreshDevices: () => Promise<void>;
  dismissWarning: (at: number) => void;
  clearCaptureError: () => void;
  openCompanion: () => Promise<void>;
  testCapture: () => Promise<void>;
}

const LIVE_KEEP = 400;

let listenersAttached = false;

export const useRecorder = create<RecorderState>((set, get) => {
  const now = () => elapsedOf(get());

  const attach = () => {
    if (listenersAttached) return;
    listenersAttached = true;
    listen<{ source: string; text: string }>("meeting-live-segment", (e) => {
      if (!get().recording) return;
      set((s) => ({
        live: [...s.live, { ...e.payload, atSec: now() }].slice(-LIVE_KEEP),
      }));
    }).catch(() => (listenersAttached = false));
    listen<string>("meeting-capture-error", (e) => {
      set({ captureError: e.payload });
      toast.error(e.payload);
    }).catch(() => {});
    listen<string>("meeting-capture-warning", (e) => {
      set((s) => ({
        warnings: [...s.warnings, { at: Date.now(), text: e.payload }],
      }));
    }).catch(() => {});
    // Commands from the companion window.
    listen<RecorderCommand>(RECORDER_COMMAND_EVENT, (e) => {
      const st = get();
      switch (e.payload.action) {
        case "pause":
          if (!st.paused) void st.togglePause();
          break;
        case "resume":
          if (st.paused) void st.togglePause();
          break;
        case "stop":
          void st.stop();
          break;
        case "flag":
          st.flag(e.payload.label);
          break;
        case "title":
          st.setTitle(e.payload.title);
          break;
        case "dismiss-warning":
          st.dismissWarning(e.payload.at);
          break;
        case "start":
          if (!st.recording && !st.starting) void st.start();
          break;
        case "open":
          void commands.showMainWindowCommand().catch(() => {});
          if (st.recording || useMeetingJobs.getState().stopping) {
            useNav.getState().openMeeting({ id: "live" });
          } else {
            useNav.getState().go("meetings");
          }
          break;
        case "hello":
          broadcast();
          break;
      }
    }).catch(() => {});
  };

  const broadcast = () => {
    const s = get();
    const snap: RecorderSnapshot = {
      recording: s.recording,
      paused: s.paused,
      stopping: useMeetingJobs.getState().stopping !== null,
      elapsedBase: s.elapsedBase,
      runStartedAt: s.runStartedAt,
      systemCaptured: s.systemCaptured,
      title: s.title,
      live: s.live.slice(-12),
      flags: s.flags,
      warnings: s.warnings,
      devices: s.devices,
    };
    void emit(RECORDER_STATE_EVENT, snap).catch(() => {});
  };

  // Broadcast after every change (deferred so a burst of sets sends once).
  let pending = false;
  const scheduleBroadcast = () => {
    if (pending) return;
    pending = true;
    queueMicrotask(() => {
      pending = false;
      broadcast();
    });
  };

  queueMicrotask(() => {
    attach();
    useRecorder.subscribe(scheduleBroadcast);
    useMeetingJobs.subscribe((s, prev) => {
      if ((s.stopping === null) !== (prev.stopping === null)) scheduleBroadcast();
    });
  });

  return {
    recording: false,
    paused: false,
    starting: false,
    testing: false,
    lastTest: null,
    elapsedBase: 0,
    runStartedAt: null,
    systemCaptured: null,
    title: "",
    live: [],
    flags: [],
    warnings: [],
    devices: null,
    captureError: null,

    restore: async () => {
      attach();
      try {
        const res = await commands.meetingRecordingStatus();
        if (res.status === "ok" && res.data) {
          const d = res.data;
          set({
            recording: true,
            paused: d.paused,
            elapsedBase: d.elapsed_secs,
            runStartedAt: d.paused ? null : Date.now(),
            systemCaptured: d.system_captured,
            title: get().title || defaultMeetingTitle(Date.now() - d.elapsed_secs * 1000),
          });
          void get().refreshDevices();
        }
      } catch {
        /* nothing running, or the backend is not ready: start idle */
      }
    },

    start: async () => {
      const st = get();
      if (st.recording || st.starting || useMeetingJobs.getState().stopping) {
        return false;
      }
      attach();
      set({ starting: true, captureError: null });
      void get().refreshDevices();
      try {
        const res = await commands.meetingStartCapture();
        if (res.status !== "ok") {
          toast.error(res.error);
          return false;
        }
        set({
          recording: true,
          paused: false,
          elapsedBase: 0,
          runStartedAt: Date.now(),
          systemCaptured: res.data,
          title: defaultMeetingTitle(Date.now()),
          live: [],
          flags: [],
          warnings: [],
        });
        if (!res.data) {
          toast.message("System audio couldn't be captured — recording your mic only.");
        }
        return true;
      } catch (e) {
        toast.error(`Could not start the meeting: ${String(e)}`);
        return false;
      } finally {
        set({ starting: false });
      }
    },

    stop: async () => {
      const st = get();
      if (!st.recording) return;
      const durationSec = Math.round(now());
      const captured = st.systemCaptured ?? false;
      const flags = st.flags;
      const title = st.title.trim() || defaultMeetingTitle(Date.now());
      const liveWarnings = st.warnings.map((w) => w.text);
      const before = {
        paused: st.paused,
        elapsedBase: st.elapsedBase,
        runStartedAt: st.runStartedAt,
        systemCaptured: st.systemCaptured,
      };
      set({ recording: false, paused: false, runStartedAt: null, systemCaptured: null });
      const outcome = await useMeetingJobs.getState().stopMeeting({
        makeMeeting: ({ you, others, segments, mic_path, system_path, warnings }) => {
          const m: Meeting = {
            id: newId(),
            title,
            you,
            others,
            transcript: segments ?? [],
            processed: "",
            processPrompt: "",
            createdAt: Date.now(),
            systemCaptured: captured,
            micPath: mic_path,
            systemPath: system_path,
            youLabel: "You",
            othersLabel: "Others",
            imported: false,
            flags,
            durationSec,
            // Stop's own verdict first, then anything raised mid-meeting that
            // Stop did not repeat.
            captureWarnings: [
              ...warnings,
              ...liveWarnings.filter((w) => !warnings.includes(w)),
            ],
          };
          return m as unknown as MeetingDoc;
        },
      });
      if (outcome === "stop-failed") {
        // The backend may still be recording. Keep the title, flags and live
        // lines (they exist nowhere else) and re-read the real state.
        set({ elapsedBase: elapsedOf(before) });
        await get().restore();
        if (!get().recording) set({ ...before, recording: false, paused: false, runStartedAt: null });
        return;
      }
      set({ elapsedBase: 0, live: [], flags: [], warnings: [] });
    },

    togglePause: async () => {
      const st = get();
      if (!st.recording) return;
      const next = !st.paused;
      // Optimistic: freeze or restart the clock now, roll back on failure.
      const before = { paused: st.paused, elapsedBase: st.elapsedBase, runStartedAt: st.runStartedAt };
      set(
        next
          ? { paused: true, elapsedBase: now(), runStartedAt: null }
          : { paused: false, runStartedAt: Date.now() },
      );
      try {
        const res = next ? await commands.meetingPause() : await commands.meetingResume();
        if (res.status !== "ok") {
          set(before);
          toast.error(res.error);
        }
      } catch (e) {
        set(before);
        toast.error(`Could not ${next ? "pause" : "resume"} the meeting: ${String(e)}`);
      }
    },

    flag: (label = "") => {
      const st = get();
      if (!st.recording) return;
      const atSec = Math.round(now());
      // Two presses within three seconds are one moment.
      if (st.flags.some((f) => Math.abs(f.atSec - atSec) < 3)) return;
      set({ flags: [...st.flags, { atSec, label }] });
    },

    setTitle: (title) => set({ title }),

    refreshDevices: async () => {
      try {
        const res = await commands.meetingCaptureDevices();
        if (res.status === "ok") set({ devices: res.data });
      } catch {
        /* names are cosmetic */
      }
    },

    dismissWarning: (at) =>
      set((s) => ({ warnings: s.warnings.filter((w) => w.at !== at) })),

    clearCaptureError: () => set({ captureError: null }),

    // Runs the EXACT meeting capture path for 8 s against throwaway WAVs, so
    // mic and call audio can be checked without risking a real meeting.
    testCapture: async () => {
      const st = get();
      if (st.recording || st.testing || useMeetingJobs.getState().stopping) return;
      set({ testing: true });
      void get().refreshDevices();
      toast.message(
        "Testing for 8 seconds — play any audio now so the call-audio meter has something to capture.",
      );
      try {
        const res = await commands.meetingTestCapture(8);
        if (res.status !== "ok") {
          toast.error(res.error);
          return;
        }
        const { mic_device, system_device, mic_samples, system_samples } = res.data;
        const micOk = mic_samples > 0;
        const systemOk = system_samples > 0;
        set({ lastTest: { at: Date.now(), micOk, systemOk } });
        if (micOk) toast.success(`Microphone OK — ${mic_device}`);
        else
          toast.error(
            `No audio from the microphone (${mic_device}). Check it isn't muted or in use by another app.`,
          );
        if (systemOk) toast.success(`Call audio OK — ${system_device}`);
        else
          toast.error(
            `No call audio captured (${system_device}). Kōrero records the DEFAULT output device — if your call plays through a different device (a headset, say), make that the Windows default output, then test again with audio playing.`,
            { duration: 12000 },
          );
      } catch (e) {
        toast.error(String(e));
      } finally {
        set({ testing: false });
      }
    },

    openCompanion: async () => {
      try {
        const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
        const existing = await WebviewWindow.getByLabel(COMPANION_LABEL);
        if (existing) {
          await existing.show();
          await existing.setFocus();
          return;
        }
        const win = new WebviewWindow(COMPANION_LABEL, {
          url: "src/recorder/index.html",
          title: "Kōrero recorder",
          width: 380,
          height: 600,
          minWidth: 340,
          minHeight: 440,
          resizable: true,
          alwaysOnTop: true,
          focus: true,
        });
        win.once("tauri://error", (e) => {
          toast.error(`Could not open the recorder window: ${String(e.payload)}`);
          logActivity({
            status: "failed",
            title: "Recorder window did not open",
            detail: String(e.payload),
          });
        });
      } catch (e) {
        toast.error(`Could not open the recorder window: ${String(e)}`);
      }
    },
  };
});
