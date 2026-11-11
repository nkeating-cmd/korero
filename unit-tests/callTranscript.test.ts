import { describe, expect, test } from "bun:test";
import {
  callRebuildBlocker,
  combine,
  notesInput,
  sourceChipLabel,
  type Meeting,
  type TranscriptSeg,
} from "../src/components/meetings/model";
import { afterStopPlan, notesModelReady } from "../src/components/meetings/autoNotes";
import { withName } from "../src/components/meetings/SpeakerNames";

// Kōrero 1.46: a call's transcript rebuilt from its recordings, callers named,
// and what a call does by itself after Stop. Every name and line here is fictional.

const seg = (source: string, text: string, start_ms: number, speaker?: number | null): TranscriptSeg => ({
  source,
  text,
  start_ms,
  speaker,
});

const call = (over: Partial<Meeting> = {}): Meeting => ({
  id: "c1",
  title: "Call",
  you: "",
  others: "",
  transcript: [
    seg("you", "kia ora koutou", 0),
    seg("others", "mōrena", 1_000, 0),
    seg("others", "shall we start", 2_000, 1),
    seg("others", "with the budget", 3_000, 1),
    seg("you", "yes", 4_000),
  ],
  processed: "",
  processPrompt: "",
  createdAt: 0,
  systemCaptured: true,
  micPath: "C:/recordings/call-mic.wav",
  systemPath: "C:/recordings/call-system.wav",
  youLabel: "You",
  othersLabel: "Others",
  imported: false,
  mode: "call",
  ...over,
});

describe("callers in the text the notes model reads", () => {
  test("each caller is its own turn, and one caller's consecutive lines join", () => {
    const m = call();
    expect(combine(m.you, m.others, m.youLabel, m.othersLabel, m.transcript)).toBe(
      ["You: kia ora koutou", "Caller 1: mōrena", "Caller 2: shall we start with the budget", "You: yes"].join("\n"),
    );
  });
  test("caller names reach the notes input", () => {
    const m = call({ callerNames: ["Mere", "Tama"] });
    const text = notesInput(m, m.transcript ?? []);
    expect(text).toContain("Mere: mōrena");
    expect(text).toContain("Tama: shall we start with the budget");
    expect(text).not.toContain("Caller");
  });
  test("a meeting saved before 1.46 reads exactly as it did", () => {
    const old = call({ transcript: [seg("you", "a", 0), seg("others", "b", 1_000), seg("others", "c", 2_000)] });
    expect(combine(old.you, old.others, old.youLabel, old.othersLabel, old.transcript)).toBe("You: a\nOthers: b c");
  });
});

describe("the header pill", () => {
  test("says how many callers were found, once there is more than one", () => {
    expect(sourceChipLabel(call({ callersFound: 3 }))).toBe("You + 3 callers");
    expect(sourceChipLabel(call({ callersFound: 1 }))).toBe("You + Others");
    expect(sourceChipLabel(call())).toBe("You + Others");
    expect(sourceChipLabel(call({ callersFound: 3, systemCaptured: false }))).toBe("You only");
  });
});

describe("callRebuildBlocker", () => {
  test("a recorded call can be rebuilt, from either side alone", () => {
    expect(callRebuildBlocker(call())).toBeNull();
    expect(callRebuildBlocker(call({ systemPath: null }))).toBeNull();
    expect(callRebuildBlocker(call({ micPath: null }))).toBeNull();
  });
  test("room meetings, imports, combined meetings and missing recordings are refused", () => {
    expect(callRebuildBlocker(call({ mode: "room" }))).not.toBeNull();
    expect(callRebuildBlocker(call({ imported: true }))).not.toBeNull();
    expect(callRebuildBlocker(call({ mergedFrom: ["a", "b"] }))).not.toBeNull();
    expect(callRebuildBlocker(call({ micPath: null, systemPath: null }))).not.toBeNull();
    expect(callRebuildBlocker(call({ micPath: "C:/x/in.m4a", systemPath: null }))).not.toBeNull();
  });
});

describe("what a call does after Stop", () => {
  const base = { micWav: true, systemWav: true, hasLiveText: true, autoNotes: true, modelReady: true };
  test("rebuilds, then writes notes", () => {
    expect(afterStopPlan(base)).toEqual({ rebuild: true, notes: true, notesFromLive: true });
  });
  test("no notes when switched off, or when no AI model is set up", () => {
    expect(afterStopPlan({ ...base, autoNotes: false }).notes).toBe(false);
    expect(afterStopPlan({ ...base, modelReady: false }).notes).toBe(false);
    expect(afterStopPlan({ ...base, modelReady: false }).rebuild).toBe(true);
  });
  test("no recording to rebuild from: notes still come from the live transcript", () => {
    expect(afterStopPlan({ ...base, micWav: false, systemWav: false })).toEqual({
      rebuild: false,
      notes: true,
      notesFromLive: true,
    });
  });
  test("nothing heard live and nothing to rebuild from: nothing to do", () => {
    expect(afterStopPlan({ ...base, micWav: false, systemWav: false, hasLiveText: false })).toEqual({
      rebuild: false,
      notes: false,
      notesFromLive: false,
    });
  });
  test("a silent live transcript never becomes the fallback for notes", () => {
    expect(afterStopPlan({ ...base, hasLiveText: false })).toEqual({ rebuild: true, notes: true, notesFromLive: false });
  });
});

describe("notesModelReady", () => {
  test("needs a chosen provider with a model", () => {
    expect(notesModelReady(null)).toBe(false);
    expect(notesModelReady({ post_process_provider_id: "", post_process_models: {} })).toBe(false);
    expect(notesModelReady({ post_process_provider_id: "ollama", post_process_models: { ollama: "  " } })).toBe(false);
    expect(notesModelReady({ post_process_provider_id: "ollama", post_process_models: { openai: "x" } })).toBe(false);
    expect(notesModelReady({ post_process_provider_id: "ollama", post_process_models: { ollama: "gemma4:12b" } })).toBe(true);
  });
});

describe("withName", () => {
  test("sets one name, pads to the count, trims, and a blank clears", () => {
    expect(withName(undefined, 3, 1, " Mere ")).toEqual(["", "Mere", ""]);
    expect(withName(["Aroha", "Mere"], 2, 1, "")).toEqual(["Aroha", ""]);
    expect(withName(["Aroha"], 1, 2, "Tama")).toEqual(["Aroha", "", "Tama"]);
  });
});
