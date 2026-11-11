import { describe, expect, test } from "bun:test";
import {
  callRebuildBlocker,
  combine,
  notesInput,
  sourceChipLabel,
  type Meeting,
  type TranscriptSeg,
} from "../src/components/meetings/model";
import { afterStopPlan, autoNotesOn, notesModelReady, parseChoice } from "../src/components/meetings/autoNotes";
import { combineCallers, restoreLivePatch } from "../src/components/meetings/model";
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

describe("notes are never sent to a cloud model on the strength of a default", () => {
  test("nobody has chosen: on for a model on this computer, off for a cloud one", () => {
    expect(autoNotesOn("unset", true)).toBe(true);
    expect(autoNotesOn("unset", false)).toBe(false);
  });
  test("a choice is a choice, wherever the model runs", () => {
    expect(autoNotesOn("on", false)).toBe(true);
    expect(autoNotesOn("on", true)).toBe(true);
    expect(autoNotesOn("off", true)).toBe(false);
    expect(autoNotesOn("off", false)).toBe(false);
  });
});

describe("a yes is remembered for the provider it was given for", () => {
  test("same provider: on; another provider: nobody has chosen", () => {
    expect(parseChoice("1:ollama", "ollama")).toBe("on");
    expect(parseChoice("1:ollama", "openai")).toBe("unset");
    expect(autoNotesOn(parseChoice("1:ollama", "openai"), false)).toBe(false);
  });
  test("no is no for every provider; nothing stored, or an older value, is unset", () => {
    expect(parseChoice("0", "openai")).toBe("off");
    expect(parseChoice(null, "ollama")).toBe("unset");
    expect(parseChoice("1", "ollama")).toBe("unset");
  });
});

describe("a call recorded before line timings is refused", () => {
  test("text with no lines, or lines that all start together", () => {
    expect(callRebuildBlocker(call({ transcript: [], you: "old text", others: "" }))).not.toBeNull();
    expect(callRebuildBlocker(call({ transcript: [seg("you", "a", 0), seg("others", "b", 0)] }))).not.toBeNull();
    expect(callRebuildBlocker(call({ transcript: [seg("you", "a", 0)] }))).toBeNull();
    expect(callRebuildBlocker(call({ transcript: [], you: "", others: "" }))).toBeNull();
  });
});

describe("combining two calls keeps their callers apart", () => {
  test("the second call's callers move up past the first's, names with them", () => {
    const a = { transcript: [seg("others", "a1", 0, 0), seg("others", "a2", 1, 1)], callersFound: 2, callerNames: ["Mere", ""] };
    const b = { transcript: [seg("you", "me", 0), seg("others", "b1", 1, 0)], callersFound: 1, callerNames: ["Tama"] };
    const out = combineCallers(a, b);
    expect(out.transcript?.map((s) => s.speaker ?? null)).toEqual([0, 1, null, 2]);
    expect(out.callersFound).toBe(3);
    expect(out.callerNames).toEqual(["Mere", "", "Tama"]);
  });
  test("meetings with no callers combine exactly as before", () => {
    const a = { transcript: [seg("you", "a", 0), seg("others", "b", 1)] };
    const b = { transcript: [seg("others", "c", 0)] };
    expect(combineCallers(a, b)).toEqual({ transcript: [...a.transcript, ...b.transcript] });
  });
  test("caller numbers are read from the lines when the count was never saved", () => {
    const a = { transcript: [seg("others", "a", 0, 1)] };
    const b = { transcript: [seg("others", "b", 0, 0)] };
    expect(combineCallers(a, b).transcript?.map((s) => s.speaker)).toEqual([1, 2]);
  });
});

describe("the live transcript can be put back", () => {
  test("nothing kept: nothing to restore", () => {
    expect(restoreLivePatch({})).toBeNull();
    expect(restoreLivePatch({ liveTranscript: [] })).toBeNull();
  });
  test("the kept lines return, both text fields with them, caller labels cleared", () => {
    const live = [seg("you", "kia ora", 0), seg("others", "hello", 1_000), seg("you", "next", 2_000)];
    expect(restoreLivePatch({ liveTranscript: live })).toEqual({
      transcript: live,
      you: "kia ora next",
      others: "hello",
      callersFound: undefined,
      callerNames: undefined,
      speakersFound: undefined,
      speakerNames: undefined,
    });
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
