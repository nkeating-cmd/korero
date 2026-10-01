// Kōrero 1.42: the contract between the recorder (main window) and the
// companion window. Types and constants only — importing this file must not
// start anything, because the companion imports it too.

import type { MeetingFlag } from "../components/meetings/model";

export interface LiveSegment {
  source: string;
  text: string;
  /** Seconds into the recording when the line arrived. */
  atSec: number;
}

export interface CaptureWarning {
  at: number;
  text: string;
}

/** What the companion window needs to draw itself. */
export interface RecorderSnapshot {
  recording: boolean;
  paused: boolean;
  stopping: boolean;
  elapsedBase: number;
  runStartedAt: number | null;
  systemCaptured: boolean | null;
  title: string;
  live: LiveSegment[];
  flags: MeetingFlag[];
  warnings: CaptureWarning[];
  devices: { mic: string; system: string } | null;
}

export type RecorderCommand =
  | { action: "pause" }
  | { action: "resume" }
  | { action: "stop" }
  | { action: "flag"; label?: string }
  | { action: "title"; title: string }
  | { action: "dismiss-warning"; at: number }
  /** Start a new meeting from the companion. */
  | { action: "start" }
  /** Bring the main window forward on the meeting (live, or the newest). */
  | { action: "open" }
  | { action: "hello" };

export const RECORDER_STATE_EVENT = "korero://recorder-state";
export const RECORDER_COMMAND_EVENT = "korero://recorder-command";
export const COMPANION_LABEL = "recorder";

/** Seconds recorded so far, pauses excluded. */
export const elapsedOf = (s: {
  elapsedBase: number;
  runStartedAt: number | null;
}): number =>
  s.elapsedBase +
  (s.runStartedAt !== null ? Math.max(0, (Date.now() - s.runStartedAt) / 1000) : 0);

