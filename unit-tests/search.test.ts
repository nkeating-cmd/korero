// Kōrero 1.42: Ctrl K search. Run with `bun test unit-tests`.
import { describe, expect, test } from "bun:test";
import {
  fold,
  foldWithMap,
  groupHits,
  scoreDoc,
  search,
  snippetOf,
  type SearchDoc,
} from "../src/lib/search";

const doc = (over: Partial<SearchDoc>): SearchDoc => ({
  id: over.id ?? Math.random().toString(36),
  kind: over.kind ?? "meeting",
  title: over.title ?? "Untitled",
  body: over.body,
  keywords: over.keywords,
  at: over.at,
});

describe("fold", () => {
  test("strips macrons and lower-cases", () => {
    expect(fold("Whānau Tāmaki Makaurau Taupō")).toBe("whanau tamaki makaurau taupo");
  });
  test("leaves plain ASCII alone", () => {
    expect(fold("Board prep")).toBe("board prep");
  });
  test("maps folded characters back to the original", () => {
    const { folded, map } = foldWithMap("kōrero");
    expect(folded).toBe("korero");
    expect(map).toEqual([0, 1, 2, 3, 4, 5]);
  });
});

describe("macron-insensitive matching (the point of folding)", () => {
  const d = doc({ title: "Whānau day planning" });
  test("a query without macrons finds a title with them", () => {
    expect(scoreDoc(d, "whanau")).not.toBeNull();
  });
  test("a query with macrons finds a title without them", () => {
    expect(scoreDoc(doc({ title: "Whanau day" }), "whānau")).not.toBeNull();
  });
  test("no match returns null", () => {
    expect(scoreDoc(d, "budget")).toBeNull();
  });
});

describe("every term must appear", () => {
  const d = doc({ title: "Library fit-out", body: "the community room for whānau groups" });
  test("terms across title and body both match", () => {
    expect(scoreDoc(d, "library whanau")).not.toBeNull();
  });
  test("one missing term means no match", () => {
    expect(scoreDoc(d, "library budget")).toBeNull();
  });
  test("an empty query matches nothing", () => {
    expect(search([d], "   ")).toEqual([]);
  });
});

describe("ranking", () => {
  test("a title match beats a body match", () => {
    const titleHit = doc({ id: "t", title: "Whānau day planning" });
    const bodyHit = doc({ id: "b", title: "Library fit-out", body: "for whānau groups" });
    const hits = search([bodyHit, titleHit], "whanau");
    expect(hits[0].doc.id).toBe("t");
  });
  test("keywords find settings that do not use the word in their title", () => {
    const s = doc({ kind: "setting", title: "New Zealand English", keywords: "macrons te reo nz spelling" });
    expect(scoreDoc(s, "macrons")).not.toBeNull();
  });
  test("caps results per kind", () => {
    const many = Array.from({ length: 12 }, (_, i) =>
      doc({ id: `m${i}`, title: `Meeting ${i} whanau` }),
    );
    expect(search(many, "whanau", 5)).toHaveLength(5);
  });
  test("counts whole-query occurrences", () => {
    const d = doc({ title: "Hui", body: "whānau here, whānau there, whanau everywhere" });
    expect(scoreDoc(d, "whanau")?.matches).toBe(3);
  });
});

describe("snippets keep the original text", () => {
  test("cuts the match out of the original, macrons intact", () => {
    const s = snippetOf("We want the space to work for whānau groups, not just study.", "whanau");
    expect(s?.match).toBe("whānau");
    expect(s?.before.endsWith("for ")).toBe(true);
    expect(s?.after.startsWith(" groups")).toBe(true);
  });
  test("adds ellipses when the text is cut", () => {
    const long = `${"a ".repeat(80)}needle${" b".repeat(80)}`;
    const s = snippetOf(long, "needle", 20);
    expect(s?.before.startsWith("…")).toBe(true);
    expect(s?.after.endsWith("…")).toBe(true);
  });
  test("returns null when there is no match", () => {
    expect(snippetOf("nothing here", "whanau")).toBeNull();
  });
});

describe("grouping", () => {
  test("groups in a fixed order and drops empty kinds", () => {
    const hits = search(
      [
        doc({ id: "n", kind: "note", title: "whanau note" }),
        doc({ id: "a", kind: "action", title: "Add whanau to custom words" }),
        doc({ id: "m", kind: "meeting", title: "Whanau day" }),
      ],
      "whanau",
    );
    expect(groupHits(hits).map((g) => g.kind)).toEqual(["action", "meeting", "note"]);
  });
});
