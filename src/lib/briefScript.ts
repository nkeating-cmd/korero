// Kōrero 1.44: turning text into a script the voice engine can read whole.
//
// The voice engine renders a script in one generation and stops after about
// 330 words, without saying so. The Audio Brief page always drafted a short
// spoken script first; the Meetings "Audio brief" button sent the raw notes
// (Markdown, up to ~1,000 words) instead, so a fixed engine would have read
// part of them, headings and all. Both now go through `draftSpokenScript`.
//
// Pure apart from the injected `post` call, so it can be unit-tested.

/** Keep in step with `MAX_BRIEF_WORDS` in src-tauri/src/meeting.rs. */
export const MAX_BRIEF_WORDS = 330;

export const SHORTEN_PROMPT =
  "Shorten the spoken script below to under 250 words. Keep the most important points and the closing line; " +
  "keep it written for the ear (numbers as words, no headings, lists or symbols). Return ONLY the script text.";

export const scriptWords = (s: string): number => s.split(/\s+/).filter(Boolean).length;

/** The backend's message when no voice engine is set up. */
export const isEngineMissing = (msg: string): boolean => /voice engine not found/i.test(msg);

type Post = (text: string, prompt: string) => Promise<{ status: "ok"; data: string } | { status: "error"; error: string }>;

/**
 * Draft a spoken script from `text` with `prompt`; if the draft is too long
 * to render whole, ask once for a shorter one. Throws with a plain message if
 * it is still too long, so nothing is ever rendered cut off.
 */
export async function draftSpokenScript(text: string, prompt: string, post: Post): Promise<string> {
  const first = await post(text, prompt);
  if (first.status !== "ok") throw new Error(first.error);
  let script = first.data.trim();
  if (scriptWords(script) > MAX_BRIEF_WORDS) {
    const shorter = await post(script, SHORTEN_PROMPT);
    if (shorter.status !== "ok") throw new Error(shorter.error);
    script = shorter.data.trim();
  }
  const n = scriptWords(script);
  if (!script) throw new Error("The model returned an empty script.");
  if (n > MAX_BRIEF_WORDS) {
    throw new Error(
      `The script came out at ${n} words, and the voice engine stops after about ${MAX_BRIEF_WORDS}. ` +
        "Open Audio Brief to shorten it, then render.",
    );
  }
  return script;
}
