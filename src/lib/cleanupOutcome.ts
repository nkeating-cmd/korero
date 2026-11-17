/**
 * Kōrero 1.49: what happened to a dictation's clean-up, in plain words.
 *
 * History stores a fixed reason word (`post_process_failure`, from Rust's
 * `CleanupFailReason`) when a requested clean-up did not land. Today and
 * History read it here, so both say the same thing and the wording is
 * unit-tested (unit-tests/cleanupOutcome.test.ts).
 */

import type { CleanupFailReason } from "../bindings";

export interface CleanupEntryLike {
  post_process_requested: boolean;
  post_processed_text: string | null;
  post_process_failure?: CleanupFailReason | null;
}

export type CleanupState = "none" | "cleaned" | "late" | "running" | "failed";

export interface CleanupOutcome {
  state: CleanupState;
  /** Short line for Today's meta row (no leading separator). */
  short: string;
  /** Longer line for History. */
  long: string;
  /** True when "Check local AI" is the right next step. */
  suggestCheck: boolean;
}

const REASON: Record<CleanupFailReason, string> = {
  not_set_up: "clean-up isn't set up",
  unreachable: "the AI model didn't answer",
  proxy_blocked: "a network proxy blocked the AI model",
  model_missing: "the AI model isn't on this computer",
  out_of_memory: "not enough free memory for the AI model",
  too_long: "too long for the AI model to read",
  timed_out: "the AI model took too long",
  key_rejected: "the AI provider refused the key",
  http_error: "the AI model returned an error",
  empty: "the AI model answered with nothing",
  still_running: "clean-up still running",
  came_late: "cleaned up later",
  other: "clean-up failed",
};

/** Reasons the user cannot fix from "Check local AI" (nothing to check). */
const NOT_A_SETUP_FAULT: ReadonlySet<CleanupFailReason> = new Set(["too_long", "empty"]);

const hasText = (t: string | null) => t !== null && t.trim().length > 0;

export function cleanupOutcome(e: CleanupEntryLike): CleanupOutcome {
  if (!e.post_process_requested) {
    return hasText(e.post_processed_text)
      ? { state: "cleaned", short: "cleaned up", long: "Cleaned up", suggestCheck: false }
      : { state: "none", short: "", long: "", suggestCheck: false };
  }
  const why = e.post_process_failure ?? null;
  if (hasText(e.post_processed_text)) {
    return why === "came_late"
      ? {
          state: "late",
          short: "cleaned up later (the AI model was slow to answer)",
          long: "Pasted as spoken, then cleaned up here once the AI model answered",
          suggestCheck: false,
        }
      : { state: "cleaned", short: "cleaned up", long: "Cleaned up", suggestCheck: false };
  }
  if (why === "still_running") {
    return {
      state: "running",
      short: "pasted as spoken; clean-up still running",
      long: "Pasted as spoken. The clean-up is still running and will appear here",
      suggestCheck: false,
    };
  }
  // Before 1.49 no reason was stored: keep the old words.
  const reason = why === null || why === "came_late" ? "clean-up failed" : REASON[why];
  return {
    state: "failed",
    short: `pasted as spoken: ${reason}`,
    long: `Pasted as spoken: ${reason}`,
    suggestCheck: why === null || !NOT_A_SETUP_FAULT.has(why),
  };
}
