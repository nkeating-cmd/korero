// Kōrero 1.49: what Today and History say about a dictation's clean-up.
// Run with `bun test unit-tests`.

import { describe, expect, test } from "bun:test";
import type { CleanupFailReason } from "../src/bindings";
import { cleanupOutcome } from "../src/lib/cleanupOutcome";

const entry = (
  requested: boolean,
  text: string | null,
  failure: CleanupFailReason | null = null,
) => ({ post_process_requested: requested, post_processed_text: text, post_process_failure: failure });

const ALL: CleanupFailReason[] = [
  "not_set_up",
  "unreachable",
  "proxy_blocked",
  "model_missing",
  "out_of_memory",
  "too_long",
  "timed_out",
  "key_rejected",
  "http_error",
  "empty",
  "still_running",
  "came_late",
  "other",
];

describe("cleanupOutcome", () => {
  test("not asked for: nothing to say", () => {
    expect(cleanupOutcome(entry(false, null)).state).toBe("none");
  });

  test("cleaned", () => {
    expect(cleanupOutcome(entry(true, "Tidy."))).toMatchObject({ state: "cleaned", suggestCheck: false });
  });

  test("came late says why, and is not a fault", () => {
    const o = cleanupOutcome(entry(true, "Tidy.", "came_late"));
    expect(o.state).toBe("late");
    // It says the model was slow, not why (a cloud provider can be slow too).
    expect(o.short).toContain("slow to answer");
    expect(o.suggestCheck).toBe(false);
  });

  test("still running is not a failure", () => {
    expect(cleanupOutcome(entry(true, null, "still_running"))).toMatchObject({
      state: "running",
      suggestCheck: false,
    });
  });

  test("an entry from before 1.49 keeps the old words", () => {
    const o = cleanupOutcome(entry(true, null));
    expect(o.short).toBe("pasted as spoken: clean-up failed");
    expect(o.suggestCheck).toBe(true);
  });

  test("whitespace-only text counts as no clean-up", () => {
    expect(cleanupOutcome(entry(true, " \n\t ", "timed_out")).state).toBe("failed");
  });

  test("every failure reason has its own words and none is blank", () => {
    const seen = new Set<string>();
    for (const r of ALL.filter((r) => r !== "still_running" && r !== "came_late" && r !== "other")) {
      const o = cleanupOutcome(entry(true, null, r));
      expect(o.state).toBe("failed");
      expect(o.short.startsWith("pasted as spoken: ")).toBe(true);
      expect(o.short.length).toBeGreaterThan("pasted as spoken: ".length + 5);
      seen.add(o.short);
    }
    expect(seen.size).toBe(10);
  });

  test("a proxy is named, so the doctor and the user look in the right place", () => {
    expect(cleanupOutcome(entry(true, null, "proxy_blocked")).short).toContain("proxy");
  });

  test("only faults the doctor can help with suggest it", () => {
    expect(cleanupOutcome(entry(true, null, "too_long")).suggestCheck).toBe(false);
    expect(cleanupOutcome(entry(true, null, "empty")).suggestCheck).toBe(false);
    expect(cleanupOutcome(entry(true, null, "model_missing")).suggestCheck).toBe(true);
    expect(cleanupOutcome(entry(true, null, "unreachable")).suggestCheck).toBe(true);
  });

  test("no wording uses American spelling or an exclamation", () => {
    for (const r of ALL) {
      const o = cleanupOutcome(entry(true, null, r));
      expect(o.long).not.toMatch(/color|organiz|!/);
    }
  });
});
