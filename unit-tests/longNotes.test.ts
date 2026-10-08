// Kōrero 1.44: long meetings are read in parts, and the flags come first.
// Run with `bun test unit-tests`. All meeting content here is fictional.

import { describe, expect, mock, test } from "bun:test";
import { FLAGS_INTRO, notesCoverPart, notesInput, type Meeting, type TranscriptSeg } from "../src/components/meetings/model";

// The jobs store imports Tauri; only its pure label helper is under test.
mock.module("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
mock.module("@/bindings", () => ({ commands: {} }));
const { phaseLabel } = await import("../src/stores/meetingJobsStore");

function meeting(segs: TranscriptSeg[], flags: Meeting["flags"] = []): Meeting {
  return {
    id: "m1",
    title: "Fictional planning meeting",
    you: "",
    others: "",
    transcript: segs,
    processed: "",
    processPrompt: "",
    createdAt: 0,
    systemCaptured: true,
    micPath: null,
    systemPath: null,
    youLabel: "You",
    othersLabel: "Others",
    imported: false,
    flags,
  };
}

/** About 80,000 characters of alternating turns, one every 10 seconds. */
function longSegs(): TranscriptSeg[] {
  return Array.from({ length: 400 }, (_, i) => ({
    source: i % 2 === 0 ? "you" : "others",
    text: `Turn ${i}: ` + "we talked about the timeline and the budget. ".repeat(4),
    start_ms: i * 10_000,
  }));
}

describe("notesInput (1.44)", () => {
  test("notes-flags-first: a flag at the last minute is inside the first 48,000 characters", () => {
    const segs = longSegs();
    const lastSec = (segs[segs.length - 1].start_ms ?? 0) / 1000;
    const text = notesInput(meeting(segs, [{ atSec: lastSec, label: "final decision" }]), segs);
    expect(text.length).toBeGreaterThan(48_000);
    expect(text.startsWith(FLAGS_INTRO)).toBe(true);
    expect(text.slice(0, 48_000)).toContain("(final decision)");
  });

  test("the flags block has no blank line, so the backend ends it at the first one", () => {
    const segs: TranscriptSeg[] = [
      { source: "you", text: "First line.\n\nSecond paragraph of the same turn.", start_ms: 0 },
      { source: "others", text: "Reply.", start_ms: 5_000 },
    ];
    const text = notesInput(meeting(segs, [{ atSec: 0, label: "two\n\nlines" }]), segs);
    const [block, ...rest] = text.split("\n\n");
    expect(block.startsWith(FLAGS_INTRO)).toBe(true);
    expect(block.split("\n")).toHaveLength(2); // the intro and one flag
    expect(rest.join("\n\n").startsWith("You: First line.")).toBe(true);
  });

  test("no flags: the transcript only, as before", () => {
    const segs = longSegs().slice(0, 3);
    const text = notesInput(meeting(segs), segs);
    expect(text.startsWith("You: Turn 0")).toBe(true);
    expect(text).not.toContain(FLAGS_INTRO);
  });
});

describe("notes written before 1.44 from part of a meeting", () => {
  test("are recognised by the line 1.41 to 1.43 added", () => {
    const old =
      "## Decisions\n- Ship it\n\n---\n\n*These notes cover only the first 78% of the transcript: the meeting is longer than the 48,000-character limit for notes. To cover a different part, set the trim markers to that part and generate the notes again.*";
    expect(notesCoverPart(old)).toBe(true);
    expect(notesCoverPart("## Decisions\n- Ship it")).toBe(false);
  });
});

describe("phaseLabel", () => {
  test("names the part being read, then the combine step", () => {
    expect(phaseLabel({ stage: "part", part: 1, of: 2 })).toBe("Long meeting: reading part 1 of 2…");
    expect(phaseLabel({ stage: "combine", part: 2, of: 2 })).toBe("Writing the notes from all 2 parts…");
  });
});
