// Kōrero 1.47: meeting titles. One format everywhere:
//
//   <topic> · <day month year>          "Budget forecast and hiring · 12 Nov 2026"
//
// A meeting nobody has named yet carries a placeholder ("Meeting · 12 Nov 2026,
// 6:03 pm", "Imported · call.wav"). Once it has notes, the notes model is asked
// for the topic and the placeholder is replaced. A title a person typed is never
// replaced. Pure and free of imports: the meetings bridge and the unit tests use it.

// Written out rather than left to the system's locale data, so every machine and
// every version of the web view gives the same title.
const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/** Day, month, year, the New Zealand way: "12 Nov 2026". */
export const titleDate = (when: number): string => {
  const d = new Date(when);
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
};

const titleTime = (when: number): string => {
  const d = new Date(when);
  const h = d.getHours();
  return `${h % 12 === 0 ? 12 : h % 12}:${String(d.getMinutes()).padStart(2, "0")} ${h < 12 ? "am" : "pm"}`;
};

/** The name a recording carries until it is given a real one. */
export const placeholderTitle = (when: number): string =>
  `Meeting · ${titleDate(when)}, ${titleTime(when)}`;

// "Meeting" followed only by numbers, an English month and am/pm, in any order: the 1.47
// placeholder and the ones earlier versions saved ("Meeting 5 Nov 2026, 6:03 pm",
// "Meeting Nov 5, 2026, 6:03 PM", "Meeting · 5/11/2026, 18:03:00").
const MONTH =
  "(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\\.?";
const DATED = new RegExp(
  `^Meeting(?: ·)?(?:[ \u00a0\u202f,./:\\-]+(?:\\d{1,4}|${MONTH}|[ap]\\.?m\\.?))+$`,
  "i",
);
const HAS_TIME = /\d{1,2}[:.]\d{2}/;
const FROM_FILE = /^(?:Imported|Recovered) · \S/;

/** True when nobody has named this meeting: the title is empty or one Kōrero made up. */
export const isPlaceholderTitle = (
  title: string | null | undefined,
): boolean => {
  const t = (title ?? "").trim();
  if (!t) return true;
  if (FROM_FILE.test(t)) return true;
  return DATED.test(t) && HAS_TIME.test(t);
};

const MAX_WORDS = 9;
const MAX_CHARS = 72;

/**
 * Tidy what the model answered into a topic, or null when it is not usable. Models
 * add quotes, a "Title:" label, Markdown, a full stop or a second line of
 * explanation; none of that belongs in a title.
 */
export const cleanTopic = (raw: string): string | null => {
  const line = raw
    // A thinking model's working is not its answer.
    .replace(/<think>[\s\S]*?(?:<\/think>|$)/gi, "")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .find((l) => l.length > 0);
  if (!line) return null;
  let t = line
    .replace(/^[#>*\-\s]+/, "")
    .replace(/^(?:title|topic|meeting title)\s*[:\-–]\s*/i, "")
    .replace(/[*_`]+/g, "")
    .replace(/^["'“”‘’«»]+|["'“”‘’«»]+$/g, "")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[.。:;,\-–·]+$/, "")
    .trim();
  // A model that was told "no date" sometimes adds one anyway; ours goes on the end.
  t = t
    .replace(
      /\s*[·\-–(,]\s*\d{1,2}(?:\s+[A-Za-z]{3,9}|[/.]\d{1,2})[\s/.]\d{2,4}\)?$/,
      "",
    )
    .trim();
  if (t.length < 3 || t.startsWith("<")) return null;
  // "Nothing to name" in the ways models say it, and names that say nothing.
  if (
    /^(?:none|n\/a|no (?:clear )?(?:topic|title)|untitled|unknown|unclear|insufficient|not enough)\b/i.test(
      t,
    ) ||
    /^(?:meeting|notes|meeting notes|summary|discussion)$/i.test(t)
  )
    return null;
  const words = t.split(" ");
  if (words.length > MAX_WORDS) t = words.slice(0, MAX_WORDS).join(" ");
  if (t.length > MAX_CHARS)
    t = t
      .slice(0, MAX_CHARS)
      .replace(/\s+\S*$/, "")
      .trim();
  // An apology or a refusal is not a topic.
  if (
    /^(?:i (?:am|cannot|can't|could not)|sorry|as an ai|here (?:is|are)|sure)\b/i.test(
      t,
    )
  )
    return null;
  if (isPlaceholderTitle(t)) return null;
  return t;
};

/** The finished title for a topic: "<topic> · 12 Nov 2026". */
export const datedTitle = (topic: string, when: number): string =>
  `${topic} · ${titleDate(when)}`;

/** What the notes model is asked. The reply goes through `cleanTopic`. */
export const TITLE_PROMPT =
  "Name this meeting by its main topic. Reply with the topic only: 3 to 7 words, sentence case, " +
  "New Zealand spelling, macrons kept on Māori words. No date, no time, no quotation marks, no " +
  'full stop, no label such as "Title:", and not the words "meeting" or "notes". Use a ' +
  "person's name only if the meeting was about that person. If the text says too little to name " +
  "a topic, reply with the single word NONE.";

/** How much of the notes (or transcript) the title is drawn from. */
export const TITLE_INPUT_CHARS = 6000;

type Doc = Record<string, unknown>;

/**
 * True when nobody has named this meeting. A person who types in the title box sets
 * `titleByUser`, so a name that happens to look like a placeholder ("Meeting 2:30 pm")
 * is still theirs; clearing the box hands the naming back.
 */
export const isUnnamed = (m: {
  title?: unknown;
  titleByUser?: unknown;
}): boolean =>
  m.titleByUser !== true &&
  isPlaceholderTitle(typeof m.title === "string" ? m.title : "");

/** What the title box writes: the text, and whether a person has now named the meeting. */
export const typedTitle = (
  text: string,
): { title: string; titleByUser: boolean } => ({
  title: text,
  titleByUser: text.trim().length > 0,
});

/**
 * Apply a patch to a meeting. A `titleSuggestion` in the patch becomes the title only
 * if nobody has named the meeting by this moment (`isUnnamed`), so a name typed while
 * the model was thinking wins. The suggestion itself is never stored.
 */
export const applyMeetingPatch = <T extends object>(doc: T, patch: Doc): T => {
  if (!("titleSuggestion" in patch)) return { ...doc, ...patch };
  const { titleSuggestion, ...rest } = patch;
  const next = { ...doc, ...rest } as Doc;
  const topic =
    typeof titleSuggestion === "string" ? cleanTopic(titleSuggestion) : null;
  if (topic && isUnnamed(next)) {
    const when =
      typeof next.createdAt === "number" ? next.createdAt : Date.now();
    next.title = datedTitle(topic, when);
  }
  return next as T;
};
