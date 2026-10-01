// Kōrero 1.42: the meeting model and its pure helpers, shared by the Meetings
// page, the recorder store, search, Today and Activity.
//
// Moved verbatim out of MeetingsSettings.tsx (where they were file-local), so
// every consumer reads a meeting the same way. The comments that explain the
// trim rules travel with the code: they are load-bearing.

/** One chronological transcript segment (matches the Rust TranscriptSeg). */
export interface TranscriptSeg {
  source: string; // "you" | "others"
  text: string;
  // Milliseconds from the start of the source file. ABSOLUTE — never rebased to
  // a trim window. Optional only because meetings recorded before v1.26.0 have
  // segments without it on disk; `normaliseMeetings` defaults those to 0.
  start_ms?: number;
}

/** A moment the user flagged while recording (1.42). */
export interface MeetingFlag {
  /** Seconds into the recording (pause time excluded, so it lines up with the
   *  audio files and with `start_ms / 1000`). */
  atSec: number;
  label: string;
}

export interface Meeting {
  id: string;
  title: string;
  you: string;
  others: string;
  transcript?: TranscriptSeg[];
  processed: string;
  processPrompt: string;
  createdAt: number;
  systemCaptured: boolean;
  micPath: string | null;
  systemPath: string | null;
  youLabel: string;
  othersLabel: string;
  imported: boolean;
  trimStartMs?: number;
  trimEndMs?: number;
  processedTrimKey?: string;
  // 1.42 additions. Absent on older meetings; every reader defaults them.
  flags?: MeetingFlag[];
  /** Plain-English capture problems reported at Stop (silent mic, no call
   *  audio, a side rebuilt from the recording). */
  captureWarnings?: string[];
  /** Length of the recording in seconds, when known. */
  durationSec?: number;
  /** The after-Stop wrap-up card was closed by the user. */
  wrapUpDismissed?: boolean;
}

export interface RecordingFile {
  path: string;
  file_name: string;
  modified: number;
}

export const DEFAULT_PROMPT =
  "Summarise this meeting: key points, decisions, and action items (with owners).";

export const newId = (): string =>
  (crypto as unknown as { randomUUID?: () => string })?.randomUUID?.() ??
  `m_${Date.now()}_${Math.random()}`;

/** The one place a segment's offset is read. */
export const segMs = (s: TranscriptSeg): number =>
  typeof s.start_ms === "number" ? s.start_ms : 0;

// Normalise older meetings so `.trim()` on optional fields is always safe.
export const normaliseMeetings = (parsed: unknown): Meeting[] => {
  if (!Array.isArray(parsed)) return [];
  return (parsed as Meeting[]).map((m) => ({
    ...m,
    processed: m.processed ?? "",
    processPrompt: m.processPrompt ?? "",
    // Default each ELEMENT's start_ms, not just the container: `undefined >= n`
    // and `undefined <= n` are both false, so a legacy segment would fall out
    // of every trim window silently.
    transcript: (Array.isArray(m.transcript) ? m.transcript : []).map((s) => ({
      ...s,
      start_ms: typeof s?.start_ms === "number" ? s.start_ms : 0,
    })),
    youLabel: m.youLabel?.trim() || "You",
    othersLabel: m.othersLabel?.trim() || "Others",
    imported:
      m.imported ??
      (m.title?.startsWith("Imported ·") ||
        m.title?.startsWith("Recovered ·") ||
        false),
    flags: Array.isArray(m.flags) ? m.flags : [],
    captureWarnings: Array.isArray(m.captureWarnings) ? m.captureWarnings : [],
  }));
};

export const fmtClock = (s: number): string => {
  const total = Math.max(0, Math.floor(s));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = (total % 60).toString().padStart(2, "0");
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${sec}` : `${m}:${sec}`;
};

/** "1 h 14 min" / "32 min" / "45 s". */
export const fmtDuration = (secs: number): string => {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s} s`;
  const h = Math.floor(s / 3600);
  const m = Math.round((s % 3600) / 60);
  if (h === 0) return `${m} min`;
  return `${h} h ${m.toString().padStart(2, "0")} min`;
};

export const labelFor = (source: string, youLabel: string, othersLabel: string) =>
  source === "you" ? youLabel : othersLabel;

/**
 * The transcript as text, in speaking order (`Label: line` per turn). Consecutive
 * segments from the same speaker merge into one turn. Falls back to the legacy
 * two-block grouping when there is no ordered transcript.
 */
export const combine = (
  you: string,
  others: string,
  youLabel = "You",
  othersLabel = "Others",
  transcript?: TranscriptSeg[],
): string => {
  if (transcript && transcript.length > 0) {
    const merged: { source: string; text: string }[] = [];
    for (const s of transcript) {
      const text = s.text.trim();
      if (!text) continue;
      const last = merged[merged.length - 1];
      if (last && last.source === s.source) last.text += ` ${text}`;
      else merged.push({ source: s.source, text });
    }
    return merged
      .map((m) => `${labelFor(m.source, youLabel, othersLabel)}: ${m.text}`)
      .join("\n");
  }
  return [
    you.trim() ? `${youLabel}:\n${you.trim()}` : "",
    others.trim() ? `${othersLabel}:\n${others.trim()}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
};

// ---- trim window ---------------------------------------------------------
// A trim is a VIEW, not an edit. READ-SIDE ONLY: never persist the output of
// `visibleSegs`, or the hidden text would be deleted from disk.

export const segInWindow = (m: Meeting, s: TranscriptSeg): boolean =>
  (m.trimStartMs == null || segMs(s) >= m.trimStartMs) &&
  (m.trimEndMs == null || segMs(s) <= m.trimEndMs);

export const visibleSegs = (m: Meeting): TranscriptSeg[] =>
  (m.transcript ?? []).filter((s) => segInWindow(m, s));

export const isTrimmed = (m: Meeting): boolean =>
  m.trimStartMs != null || m.trimEndMs != null;

export const hiddenCount = (m: Meeting): number =>
  (m.transcript?.length ?? 0) - visibleSegs(m).length;

export const trimKeyOf = (m: { trimStartMs?: number; trimEndMs?: number }) =>
  `${m.trimStartMs ?? ""}:${m.trimEndMs ?? ""}`;

export const notesAreStale = (m: Meeting): boolean =>
  m.processed.trim() !== "" && (m.processedTrimKey ?? ":") !== trimKeyOf(m);

export const fmtTrimMs = (ms: number): string => {
  const total = Math.max(0, ms) / 1000;
  const mins = Math.floor(total / 60);
  const secs = total - mins * 60;
  return `${mins}:${secs.toFixed(1).padStart(4, "0")}`;
};

export const parseTrimInput = (raw: string): number | null => {
  const t = raw.trim();
  if (!t) return null;
  const mmss = /^(\d+):([0-5]?\d(?:\.\d+)?)$/.exec(t);
  if (mmss) return Math.round((Number(mmss[1]) * 60 + Number(mmss[2])) * 1000);
  if (/^\d+(?:\.\d+)?$/.test(t)) return Math.round(Number(t) * 1000);
  return null;
};

export const baseName = (p: string): string =>
  p.replace(/\\/g, "/").split("/").pop() || p;

export const titleOf = (m: Pick<Meeting, "title" | "createdAt">): string =>
  m.title.trim() || `Meeting · ${new Date(m.createdAt).toLocaleString()}`;

export const defaultMeetingTitle = (when: number): string =>
  `Meeting ${new Date(when).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  })}`;

/** Recording length from the last transcript segment, when no duration was
 *  stored (meetings recorded before 1.42). Null when it cannot be told. */
export const durationOf = (m: Meeting): number | null => {
  if (typeof m.durationSec === "number" && m.durationSec > 0) return m.durationSec;
  const segs = m.transcript ?? [];
  if (segs.length === 0) return null;
  const last = Math.max(...segs.map(segMs));
  return last > 0 ? Math.round(last / 1000) : null;
};

export const hasTranscript = (m: Meeting): boolean =>
  !!m.you.trim() || !!m.others.trim() || (m.transcript ?? []).some((s) => s.text.trim());

export const wordCount = (text: string): number =>
  text.trim() ? text.trim().split(/\s+/).length : 0;

/** Words heard from each side, from the timed segments when there are any. */
export const sideWords = (m: Meeting): { you: number; others: number } => {
  const segs = m.transcript ?? [];
  if (segs.length === 0) return { you: wordCount(m.you), others: wordCount(m.others) };
  let you = 0;
  let others = 0;
  for (const s of segs) {
    if (s.source === "you") you += wordCount(s.text);
    else others += wordCount(s.text);
  }
  return { you, others };
};

/**
 * The text sent to the notes model: the visible transcript, plus the moments
 * the user flagged, each anchored to the nearest line so the model can give
 * them their own entries. Pure: callers pass the segments (with any trim
 * already applied).
 */
export const notesInput = (m: Meeting, segs: TranscriptSeg[]): string => {
  const body = combine(m.you, m.others, m.youLabel, m.othersLabel, segs);
  const flags = (m.flags ?? []).filter((f) => Number.isFinite(f.atSec));
  if (!body.trim() || flags.length === 0) return body;
  const lines = flags
    .slice()
    .sort((a, b) => a.atSec - b.atSec)
    .map((f) => {
      const near = nearestSegment(segs, f.atSec * 1000);
      const quote = near ? near.text.trim().slice(0, 140) : "";
      const label = f.label.trim();
      return `- ${fmtClock(f.atSec)}${label ? ` (${label})` : ""}${quote ? `: "${quote}"` : ""}`;
    });
  return `${body}\n\n[The person who recorded this flagged these moments as important. Give each one its own line in the notes.]\n${lines.join("\n")}`;
};

export const nearestSegment = (
  segs: TranscriptSeg[],
  ms: number,
): TranscriptSeg | null => {
  let best: TranscriptSeg | null = null;
  let bestGap = Infinity;
  for (const s of segs) {
    if (!s.text.trim()) continue;
    const gap = Math.abs(segMs(s) - ms);
    if (gap < bestGap) {
      best = s;
      bestGap = gap;
    }
  }
  return best;
};
