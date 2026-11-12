import { describe, expect, test } from "bun:test";
import {
  applyMeetingPatch,
  isUnnamed,
  typedTitle,
  cleanTopic,
  datedTitle,
  isPlaceholderTitle,
  placeholderTitle,
  titleDate,
} from "../src/components/meetings/titles";

const at = (y: number, m: number, d: number, h = 9, min = 7) =>
  new Date(y, m - 1, d, h, min).getTime();

describe("title dates", () => {
  test("day, month, year", () => {
    expect(titleDate(at(2026, 11, 12))).toBe("12 Nov 2026");
    expect(titleDate(at(2026, 9, 5))).toBe("5 Sep 2026");
  });
  test("the placeholder carries the date and the time", () => {
    expect(placeholderTitle(at(2026, 11, 12, 18, 3))).toBe(
      "Meeting · 12 Nov 2026, 6:03 pm",
    );
    expect(placeholderTitle(at(2026, 1, 2, 0, 5))).toBe(
      "Meeting · 2 Jan 2026, 12:05 am",
    );
    expect(placeholderTitle(at(2026, 1, 2, 12, 0))).toBe(
      "Meeting · 2 Jan 2026, 12:00 pm",
    );
  });
  test("a topic gets the date after it", () => {
    expect(datedTitle("Budget forecast and hiring", at(2026, 11, 12))).toBe(
      "Budget forecast and hiring · 12 Nov 2026",
    );
  });
});

describe("which titles are placeholders", () => {
  test("the ones Kōrero made up, in every format it has used", () => {
    for (const t of [
      "",
      "   ",
      placeholderTitle(at(2026, 11, 12, 18, 3)),
      "Meeting 5 Nov 2026, 6:03 pm",
      "Meeting Nov 5, 2026, 6:03 PM",
      "Meeting 05/11/2026, 18:03",
      "Meeting · 5/11/2026, 6:03:22 pm",
      "Meeting 5 Sept 2026, 9:07 am",
      "Imported · call-2026-11-05.wav",
      "Recovered · meeting-20261105-you.wav",
    ]) {
      expect(isPlaceholderTitle(t)).toBe(true);
    }
    expect(isPlaceholderTitle(null)).toBe(true);
    expect(isPlaceholderTitle(undefined)).toBe(true);
  });
  test("anything a person typed, or a topic already given", () => {
    for (const t of [
      "Budget forecast and hiring · 12 Nov 2026",
      "Meeting with Mere",
      "Meeting with Mere at 3:30",
      "Meeting with Mere from 3:30",
      "Meeting March planning 10:30",
      "Meeting Marketing 10:30",
      "Meeting 12 Nov",
      "Weekly catch-up",
      "Imported",
      "Kia ora koutou",
    ]) {
      expect(isPlaceholderTitle(t)).toBe(false);
    }
  });
});

describe("tidying the model's answer", () => {
  test("strips quotes, labels, Markdown and a full stop", () => {
    expect(cleanTopic('"Budget forecast and hiring."')).toBe(
      "Budget forecast and hiring",
    );
    expect(cleanTopic("Title: Budget forecast and hiring")).toBe(
      "Budget forecast and hiring",
    );
    expect(
      cleanTopic("**Budget forecast and hiring**\n\nThis title reflects…"),
    ).toBe("Budget forecast and hiring");
    expect(cleanTopic("# Kaupapa Māori evaluation plan")).toBe(
      "Kaupapa Māori evaluation plan",
    );
  });
  test("drops a date the model added anyway", () => {
    expect(cleanTopic("Budget forecast · 12 Nov 2026")).toBe("Budget forecast");
    expect(cleanTopic("Budget forecast (12/11/2026)")).toBe("Budget forecast");
  });
  test("keeps it short", () => {
    const long = cleanTopic(
      "One two three four five six seven eight nine ten eleven twelve",
    );
    expect(long?.split(" ").length).toBe(9);
  });
  test("refuses what is not a topic", () => {
    for (const bad of [
      "",
      "  \n ",
      "ok",
      "I cannot determine a title from this text",
      "Sorry, there is not enough",
      "Meeting 5 Nov 2026, 6:03 pm",
    ]) {
      expect(cleanTopic(bad)).toBeNull();
    }
  });
});

describe("offering a title to a meeting", () => {
  const when = at(2026, 11, 12, 18, 3);
  const unnamed = {
    id: "a",
    title: placeholderTitle(when),
    createdAt: when,
    processed: "",
  };

  test("an unnamed meeting takes it, with its own date", () => {
    const next = applyMeetingPatch(unnamed, {
      titleSuggestion: '"Budget forecast and hiring."',
    });
    expect(next.title).toBe("Budget forecast and hiring · 12 Nov 2026");
    expect("titleSuggestion" in next).toBe(false);
  });
  test("a meeting someone named keeps its name", () => {
    const named = { ...unnamed, title: "Mere and Tim, quarterly" };
    const next = applyMeetingPatch(named, {
      titleSuggestion: "Budget forecast",
    });
    expect(next.title).toBe("Mere and Tim, quarterly");
    expect("titleSuggestion" in next).toBe(false);
  });
  test("an already named-by-topic meeting is not renamed when notes are rewritten", () => {
    const first = applyMeetingPatch(unnamed, {
      titleSuggestion: "Budget forecast",
    });
    const second = applyMeetingPatch(first, {
      titleSuggestion: "Something else entirely",
    });
    expect(second.title).toBe("Budget forecast · 12 Nov 2026");
  });
  test("NONE, an apology or rubbish leaves the placeholder", () => {
    for (const s of ["NONE", "none.", "I cannot name this", "", 42, null]) {
      expect(applyMeetingPatch(unnamed, { titleSuggestion: s }).title).toBe(
        unnamed.title,
      );
    }
  });
  test("other fields in the same patch still land", () => {
    const next = applyMeetingPatch(unnamed, {
      processed: "notes",
      titleSuggestion: "Budget forecast",
    });
    expect(next.processed).toBe("notes");
  });
  test("a patch without a suggestion is a plain merge", () => {
    expect(applyMeetingPatch(unnamed, { title: "Typed" }).title).toBe("Typed");
  });
  test("a name a person typed is theirs even when it looks like a placeholder", () => {
    for (const typed of [
      "Meeting 2:30 pm",
      "Meeting 12.11.2026 10.30",
      "Imported · from my phone",
    ]) {
      const named = { ...unnamed, ...typedTitle(typed) };
      expect(isUnnamed(named)).toBe(false);
      expect(
        applyMeetingPatch(named, { titleSuggestion: "Budget forecast" }).title,
      ).toBe(typed);
    }
  });
  test("clearing the title box hands the naming back", () => {
    const cleared = { ...unnamed, ...typedTitle("Mine"), ...typedTitle("  ") };
    expect(isUnnamed(cleared)).toBe(true);
    expect(
      applyMeetingPatch(cleared, { titleSuggestion: "Budget forecast" }).title,
    ).toBe("Budget forecast · 12 Nov 2026");
  });
});

describe("answers that are not a topic", () => {
  test("a thinking model's working is dropped, its answer kept", () => {
    expect(
      cleanTopic(
        "<think>They discussed money.</think>\nBudget forecast and hiring",
      ),
    ).toBe("Budget forecast and hiring");
    expect(cleanTopic("<think>never finished")).toBeNull();
  });
  test("ways of saying there is nothing to name", () => {
    for (const s of [
      "NONE",
      "None.",
      "NONE - too little text",
      "No topic",
      "N/A",
      "Insufficient information",
      "Untitled",
      "# Meeting notes",
      "Notes",
      "<answer>",
    ]) {
      expect(cleanTopic(s)).toBeNull();
    }
  });
  test("real topics that start with similar words survive", () => {
    expect(cleanTopic("Notes app redesign review")).toBe(
      "Notes app redesign review",
    );
    expect(cleanTopic("Q3 2026 budget")).toBe("Q3 2026 budget");
    expect(cleanTopic("Wānanga planning and kai")).toBe(
      "Wānanga planning and kai",
    );
    expect(cleanTopic("Year-end close: what's left")).toBe(
      "Year-end close: what's left",
    );
  });
  test("an old placeholder with a narrow space before pm is recognised", () => {
    expect(isPlaceholderTitle("Meeting 5 Nov 2026, 6:03\u202fpm")).toBe(true);
  });
});
