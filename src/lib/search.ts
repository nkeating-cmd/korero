// Kōrero 1.42: search for Ctrl K. Pure — no Tauri, no React — so it is unit
// tested on its own (unit-tests/search.test.ts).
//
// Macron-insensitive by design: "whanau" finds "whānau" and "Taupo" finds
// "Taupō", in both directions. Speech engines and people both drop macrons,
// and a search that misses a meeting because of one would be the te reo
// feature failing at the worst moment.

export type SearchKind = "action" | "meeting" | "note" | "dictation" | "setting";

export interface SearchDoc {
  id: string;
  kind: SearchKind;
  title: string;
  /** Longer text searched after the title (transcript, notes, body). */
  body?: string;
  /** Extra words that should find this item but are not shown. */
  keywords?: string;
  /** Short line shown under the title (date, length, status). */
  meta?: string;
  /** Milliseconds since the epoch; newer items win ties. */
  at?: number;
}

export interface Snippet {
  before: string;
  match: string;
  after: string;
}

export interface SearchHit {
  doc: SearchDoc;
  score: number;
  /** Number of places the whole query appears (title + body). */
  matches: number;
  snippet: Snippet | null;
}

const MARKS = /[̀-ͯ]/g;

/** Lower-case and strip diacritics (ā→a, ō→o, é→e). */
export const fold = (s: string): string =>
  s.normalize("NFD").replace(MARKS, "").toLowerCase();

/**
 * Fold `text` and keep, for every folded character, the index of the original
 * character it came from — so a match found in folded text can be cut out of
 * the original, macrons intact.
 */
export const foldWithMap = (text: string): { folded: string; map: number[] } => {
  let folded = "";
  const map: number[] = [];
  let i = 0;
  for (const ch of text) {
    const f = fold(ch);
    for (let k = 0; k < f.length; k++) {
      folded += f[k];
      map.push(i);
    }
    i += ch.length;
  }
  return { folded, map };
};

export const terms = (query: string): string[] =>
  fold(query)
    .split(/\s+/)
    .map((t) => t.trim())
    .filter(Boolean);

const countOccurrences = (hay: string, needle: string): number => {
  if (!needle) return 0;
  let n = 0;
  let at = hay.indexOf(needle);
  while (at !== -1) {
    n++;
    at = hay.indexOf(needle, at + needle.length);
  }
  return n;
};

/** A window of `text` around the first occurrence of `needle` (folded). */
export const snippetOf = (
  text: string,
  query: string,
  radius = 48,
): Snippet | null => {
  const q = fold(query).trim();
  if (!q || !text) return null;
  const { folded, map } = foldWithMap(text);
  const at = folded.indexOf(q);
  if (at === -1) return null;
  const start = map[at];
  const endFolded = at + q.length - 1;
  const lastOrig = map[endFolded];
  // Extend to the end of the original character (surrogate pairs etc).
  const cp = text.codePointAt(lastOrig) ?? 0;
  const end = lastOrig + (cp > 0xffff ? 2 : 1);
  let from = Math.max(0, start - radius);
  let to = Math.min(text.length, end + radius);
  // Cut on word boundaries where we can.
  if (from > 0) {
    const sp = text.indexOf(" ", from);
    if (sp !== -1 && sp < start) from = sp + 1;
  }
  if (to < text.length) {
    const sp = text.lastIndexOf(" ", to);
    if (sp > end) to = sp;
  }
  const clean = (s: string) => s.replace(/\s+/g, " ");
  return {
    before: (from > 0 ? "…" : "") + clean(text.slice(from, start)),
    match: text.slice(start, end),
    after: clean(text.slice(end, to)) + (to < text.length ? "…" : ""),
  };
};

const KIND_WEIGHT: Record<SearchKind, number> = {
  action: 6,
  meeting: 4,
  note: 3,
  setting: 2,
  dictation: 1,
};

/**
 * Score one document against the query. Every term must appear somewhere
 * (title, keywords or body); titles count most. Returns null for no match.
 */
export const scoreDoc = (doc: SearchDoc, query: string): SearchHit | null => {
  const ts = terms(query);
  if (ts.length === 0) return null;
  const title = fold(doc.title);
  const keywords = fold(doc.keywords ?? "");
  const body = fold(doc.body ?? "");
  let score = 0;
  for (const t of ts) {
    if (title.startsWith(t)) score += 40;
    else if (new RegExp(`(^|[^a-z0-9])${escapeRe(t)}`).test(title)) score += 30;
    else if (title.includes(t)) score += 20;
    else if (keywords.includes(t)) score += 14;
    else if (body.includes(t)) score += 6;
    else return null;
  }
  const whole = fold(query).trim();
  const matches = countOccurrences(title, whole) + countOccurrences(body, whole);
  score += Math.min(matches, 10);
  score += KIND_WEIGHT[doc.kind];
  if (doc.at) {
    const ageDays = (Date.now() - doc.at) / 86_400_000;
    score += Math.max(0, 6 - Math.log2(1 + Math.max(0, ageDays)));
  }
  const snippet =
    doc.body && !title.includes(whole) ? snippetOf(doc.body, query) : null;
  return { doc, score, matches, snippet };
};

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Search everything; best first, at most `perKind` results of each kind. */
export const search = (
  docs: SearchDoc[],
  query: string,
  perKind = 5,
): SearchHit[] => {
  if (!terms(query).length) return [];
  const hits = docs
    .map((d) => scoreDoc(d, query))
    .filter((h): h is SearchHit => h !== null)
    .sort((a, b) => b.score - a.score || (b.doc.at ?? 0) - (a.doc.at ?? 0));
  const taken: Partial<Record<SearchKind, number>> = {};
  return hits.filter((h) => {
    const n = taken[h.doc.kind] ?? 0;
    if (n >= perKind) return false;
    taken[h.doc.kind] = n + 1;
    return true;
  });
};

/** Group hits for display, in a fixed, predictable order. */
export const KIND_ORDER: SearchKind[] = [
  "action",
  "meeting",
  "note",
  "dictation",
  "setting",
];

export const groupHits = (
  hits: SearchHit[],
): { kind: SearchKind; hits: SearchHit[] }[] =>
  KIND_ORDER.map((kind) => ({
    kind,
    hits: hits.filter((h) => h.doc.kind === kind),
  })).filter((g) => g.hits.length > 0);
