// Kōrero 1.44: the Meetings audio brief writes a spoken script first, and never
// sends the engine more than it can read whole. Run with `bun test unit-tests`.

import { describe, expect, test } from "bun:test";
import {
  MAX_BRIEF_WORDS,
  SHORTEN_PROMPT,
  draftSpokenScript,
  isEngineMissing,
  scriptWords,
} from "../src/lib/briefScript";

const words = (n: number) => Array.from({ length: n }, (_, i) => `w${i}`).join(" ");

/** A fake model: answers each call in turn and records what it was sent. */
function fakePost(answers: string[]) {
  const calls: { text: string; prompt: string }[] = [];
  const post = async (text: string, prompt: string) => {
    calls.push({ text, prompt });
    return { status: "ok" as const, data: answers[calls.length - 1] ?? "" };
  };
  return { post, calls };
}

describe("draftSpokenScript", () => {
  test("meetings-brief-drafts-first: the notes go to the model, not to the engine", async () => {
    const notes = "## Decisions\n- **Ship** 1.44 on 12/10";
    const { post, calls } = fakePost(["Here is the brief. We agreed to ship on the twelfth."]);
    const script = await draftSpokenScript(notes, "Rewrite for the ear.", post);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({ text: notes, prompt: "Rewrite for the ear." });
    expect(script).toBe("Here is the brief. We agreed to ship on the twelfth.");
  });

  test("a draft over the limit is shortened once", async () => {
    const { post, calls } = fakePost([words(MAX_BRIEF_WORDS + 40), words(240)]);
    const script = await draftSpokenScript("notes", "p", post);
    expect(calls).toHaveLength(2);
    expect(calls[1].prompt).toBe(SHORTEN_PROMPT);
    expect(scriptWords(script)).toBe(240);
  });

  test("brief-word-guard: still too long after shortening is refused, never rendered", async () => {
    const { post } = fakePost([words(500), words(MAX_BRIEF_WORDS + 1)]);
    expect(draftSpokenScript("notes", "p", post)).rejects.toThrow(/stops after about 330/);
  });

  test("a model error is passed on", async () => {
    const post = async () => ({ status: "error" as const, error: "No model is configured" });
    expect(draftSpokenScript("notes", "p", post)).rejects.toThrow("No model is configured");
  });
});

test("isEngineMissing matches the backend's message", () => {
  expect(isEngineMissing("On-device voice engine not found. Choose its folder in Settings → Models")).toBe(true);
  expect(isEngineMissing("TTS engine failed (exit code: 1).")).toBe(false);
});
