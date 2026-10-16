import { describe, expect, test } from "bun:test";
import {
  combine,
  separationBlocker,
  speakerLabel,
  type Meeting,
  type TranscriptSeg,
} from "../src/components/meetings/model";

// Kōrero 1.45 (D-007): speaker labels, and when "Separate speakers" is refused.

const seg = (source: string, text: string, start_ms: number, speaker?: number | null): TranscriptSeg => ({
  source,
  text,
  start_ms,
  speaker,
});

const meeting = (over: Partial<Meeting> = {}): Meeting => ({
  id: "m1",
  title: "Room",
  you: "",
  others: "",
  transcript: [seg("you", "kia ora", 0), seg("others", "hello", 1000), seg("you", "next", 2000)],
  processed: "",
  processPrompt: "",
  createdAt: 0,
  systemCaptured: false,
  micPath: "C:/recordings/room-mic.wav",
  systemPath: null,
  youLabel: "You",
  othersLabel: "Others",
  imported: false,
  ...over,
});

describe("combine without speakers is unchanged", () => {
  test("alternating lines keep one turn per line", () => {
    const t = [seg("you", "a", 0), seg("others", "b", 1), seg("you", "c", 2)];
    expect(combine("", "", "You", "Others", t)).toBe("You: a\nOthers: b\nYou: c");
  });

  test("two channels renamed to the same label still alternate (review L2)", () => {
    const t = [seg("you", "a", 0), seg("others", "b", 1)];
    expect(combine("", "", "Team", "Team", t)).toBe("Team: a\nTeam: b");
  });

  test("consecutive lines from one channel merge", () => {
    const t = [seg("you", "a", 0), seg("you", "b", 1)];
    expect(combine("", "", "You", "Others", t)).toBe("You: a b");
  });
});

describe("combine with speakers", () => {
  test("separated microphone lines become separate turns, names win over numbers", () => {
    const t = [seg("you", "a", 0, 0), seg("you", "b", 1, 1), seg("you", "c", 2, 1), seg("others", "d", 3, null)];
    expect(combine("", "", "You", "Others", t, ["Aroha"])).toBe("Aroha: a\nSpeaker 2: b c\nOthers: d");
  });

  test("a speaker on a call-audio line is ignored", () => {
    expect(speakerLabel({ youLabel: "You", othersLabel: "Others" }, { source: "others", speaker: 2 })).toBe("Others");
  });
});

describe("separationBlocker", () => {
  test("a recorded meeting with timed microphone lines can be separated", () => {
    expect(separationBlocker(meeting())).toBeNull();
  });

  test("imports and missing recordings are refused", () => {
    expect(separationBlocker(meeting({ micPath: null }))).not.toBeNull();
    expect(separationBlocker(meeting({ micPath: "C:/imports/phone.m4a" }))).not.toBeNull();
  });

  test("combined meetings are refused, marked or not (review M2)", () => {
    expect(separationBlocker(meeting({ mergedFrom: ["a", "b"] }))).not.toBeNull();
    const concatenated = [seg("you", "a", 0), seg("you", "b", 9000), seg("you", "c", 1000)];
    expect(separationBlocker(meeting({ transcript: concatenated }))).not.toBeNull();
  });

  test("meetings without line timings are refused (review M1)", () => {
    const untimed = [seg("you", "a", 0), seg("you", "b", 0), seg("you", "c", 0)];
    expect(separationBlocker(meeting({ transcript: untimed }))).toContain("Re-transcribe");
  });

  test("no microphone lines is refused", () => {
    expect(separationBlocker(meeting({ transcript: [seg("others", "hi", 0)] }))).not.toBeNull();
  });
});
