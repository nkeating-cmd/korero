// Kōrero 1.42: the meeting model and its pure helpers, shared by the Meetings
// page, the recorder store, search, Today and Activity.
//
// Moved verbatim out of MeetingsSettings.tsx (where they were file-local), so
// every consumer reads a meeting the same way. The comments that explain the
// trim rules travel with the code: they are load-bearing.

import { placeholderTitle } from "./titles";

/** One chronological transcript segment (matches the Rust TranscriptSeg). */
export interface TranscriptSeg {
  source: string; // "you" | "others"
  text: string;
  // Milliseconds from the start of the source file. ABSOLUTE — never rebased to
  // a trim window. Optional only because meetings recorded before v1.26.0 have
  // segments without it on disk; `normaliseMeetings` defaults those to 0.
  start_ms?: number;
  /** Kōrero 1.45 (D-007): who said a microphone line, set by "Separate speakers"
   *  (0-based; shown as "Speaker n+1" or the name given). Absent or null before it
   *  runs and on every meeting saved before 1.45.
   *  1.46: a call-audio ("others") line carries its caller the same way, shown as
   *  "Caller n+1" or the name given. The two sides number their voices separately. */
  speaker?: number | null;
}

/** 1.45 (B3): "On a call" (mic + the call's audio) or "In the room" (mic only). */
export type MeetingMode = "call" | "room";

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
  /** 1.47: a person typed this title, so Kōrero never replaces it (see titles.ts). */
  titleByUser?: boolean;
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
  /** 1.45: how many people "Separate speakers" found. Absent until it has run. */
  speakersFound?: number;
  /** 1.45: names for Speaker 1..N, by index; blank or missing = "Speaker n". */
  speakerNames?: string[];
  /** 1.46: how many voices were found in the call's audio. Absent until the call's
   *  transcript has been rebuilt from its recordings. */
  callersFound?: number;
  /** 1.46: names for Caller 1..N, by index; blank or missing = "Caller n". */
  callerNames?: string[];
  /** 1.46: the transcript as it was heard live, kept when the automatic rebuild
   *  after Stop replaces it, so it can be put back if the rebuild is worse. */
  liveTranscript?: TranscriptSeg[];
  /** 1.45: the two meetings this one was combined from (set by Combine). */
  mergedFrom?: string[];
  /** 1.45 (B3): how it was recorded. Absent on older meetings, which were calls. */
  mode?: MeetingMode;
}

export interface RecordingFile {
  path: string;
  file_name: string;
  modified: number;
}

// 1.46: what you need after a meeting comes first. The actions and decisions used to
// sit at the bottom of a long summary; now they open the notes.
export const DEFAULT_PROMPT =
  "Write notes for this meeting in this order. 1. **Actions**: a list, one line each: who, what, and by when if a date was said. Write \"None agreed\" if there were none. 2. **Decisions**: a list, one line each. 3. **Summary**: three sentences at most. 4. **Key points**: short bullets, most important first. Use the speakers' names where the transcript gives them. Do not invent owners or dates.";

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
 * Kōrero 1.45: why "Separate speakers" cannot run on this meeting, or null if it can.
 * Mirrors `timing_problem` in diarise.rs, which refuses the same cases.
 */
export const separationBlocker = (m: Meeting): string | null => {
  if (!m.micPath || !/\.wav$/i.test(m.micPath))
    return "Needs a recorded microphone file (imports are not supported yet).";
  // A room meeting's transcript is rebuilt from its recording, so its lines need no timings.
  if (m.mode === "room") return null;
  if (m.mergedFrom && m.mergedFrom.length > 0)
    return "A combined meeting mixes two recordings, so its lines can't be matched to one.";
  const segs = m.transcript ?? [];
  const mic = segs.filter((s) => s.source === "you" && s.text.trim()).map((s) => s.start_ms ?? 0);
  if (mic.length === 0) return "There are no microphone lines to label.";
  if (mic.length >= 2 && mic.every((ms) => ms === mic[0]))
    return "Its lines have no timings (recorded before Kōrero 1.26). Re-transcribe it first.";
  for (let i = 1; i < segs.length; i++) {
    if ((segs[i].start_ms ?? 0) < (segs[i - 1].start_ms ?? 0))
      return "Its transcript is not in time order (a combined meeting, for example), so its lines can't be matched to one recording.";
  }
  return null;
};

/** 1.45: the second header pill, saying how the meeting was recorded. */
export const sourceChipLabel = (m: Meeting): string => {
  if (m.imported) return "Imported file";
  if (m.mode === "room")
    return m.speakersFound && m.speakersFound > 1 ? `${m.speakersFound} speakers` : "In the room";
  if (m.systemCaptured && m.callersFound && m.callersFound > 1)
    return `${m.youLabel} + ${m.callersFound} callers`;
  return m.systemCaptured ? `${m.youLabel} + ${m.othersLabel}` : `${m.youLabel} only`;
};

/**
 * Kōrero 1.46: why a call's transcript cannot be rebuilt from its recordings, or null
 * if it can. A room meeting has its own rebuild; an import has no separate sides; a
 * combined meeting mixes two recordings.
 */
export const callRebuildBlocker = (m: Meeting): string | null => {
  if (m.mode === "room") return "A room meeting is rebuilt by speaker instead.";
  if (m.imported) return "An imported file has one recording, not the two sides of a call.";
  if (m.mergedFrom && m.mergedFrom.length > 0)
    return "A combined meeting mixes two recordings, so its lines can't be matched to one.";
  const wav = (p: string | null) => !!p && /\.wav$/i.test(p);
  if (!wav(m.micPath) && !wav(m.systemPath)) return "The recordings for this meeting are no longer on this computer.";
  // The rebuild matches saved lines to the recordings by time (as `call_timing_problem`
  // in diarise.rs does), so lines without timings would all be kept beside the new ones.
  const segs = (m.transcript ?? []).filter((s) => s.text.trim());
  if (segs.length === 0 && (m.you.trim() || m.others.trim()))
    return "Its lines have no timings (recorded before Kōrero 1.26). Re-transcribe it first.";
  if (segs.length >= 2 && segs.every((s) => (s.start_ms ?? 0) === (segs[0].start_ms ?? 0)))
    return "Its lines have no timings (recorded before Kōrero 1.26). Re-transcribe it first.";
  return null;
};

/**
 * Kōrero 1.46: the fields that put a call's live transcript back in place of the
 * rebuilt one, or null when no live transcript was kept. Caller labels and names
 * belong to the rebuilt lines, so they go; the kept copy stays for another change of mind.
 */
export const restoreLivePatch = (
  m: Pick<Meeting, "liveTranscript">,
): Pick<
  Meeting,
  "transcript" | "you" | "others" | "callersFound" | "callerNames" | "speakersFound" | "speakerNames"
> | null => {
  const live = m.liveTranscript;
  if (!live || live.length === 0) return null;
  const join = (source: string) =>
    live
      .filter((s) => s.source === source)
      .map((s) => s.text)
      .join(" ");
  return {
    transcript: live,
    you: join("you"),
    others: join("others"),
    callersFound: undefined,
    callerNames: undefined,
    speakersFound: undefined,
    speakerNames: undefined,
  };
};

/**
 * Kōrero 1.46: the transcript and caller fields of two meetings combined. Each meeting
 * numbers its callers from 0, so the second meeting's callers are moved up past the
 * first's and the name lists are joined: "Caller 1" of one meeting never becomes the
 * same person as "Caller 1" of the other. Microphone speakers are left as they were
 * (a combined meeting already refuses "Separate speakers").
 */
export const combineCallers = (
  first: Pick<Meeting, "transcript" | "callersFound" | "callerNames">,
  second: Pick<Meeting, "transcript" | "callersFound" | "callerNames">,
): Pick<Meeting, "transcript" | "callersFound" | "callerNames"> => {
  const used = (m: Pick<Meeting, "transcript" | "callersFound">) =>
    Math.max(
      m.callersFound ?? 0,
      ...(m.transcript ?? []).map((s) => (s.source === "others" && s.speaker != null ? s.speaker + 1 : 0)),
    );
  const a = used(first);
  const b = used(second);
  const transcript = [
    ...(first.transcript ?? []),
    ...(second.transcript ?? []).map((s) =>
      s.source === "others" && s.speaker != null ? { ...s, speaker: s.speaker + a } : s,
    ),
  ];
  if (a + b === 0) return { transcript };
  const names = [
    ...Array.from({ length: a }, (_, i) => first.callerNames?.[i]?.trim() ?? ""),
    ...Array.from({ length: b }, (_, i) => second.callerNames?.[i]?.trim() ?? ""),
  ];
  return { transcript, callersFound: a + b, callerNames: names.some(Boolean) ? names : undefined };
};

/**
 * Kōrero 1.45: the label for one transcript line. A microphone line that "Separate
 * speakers" assigned shows its speaker ("Speaker 2", or the name given). 1.46: a
 * call-audio line whose caller is known shows that caller ("Caller 2", or the name
 * given). Every other line keeps the two channel labels. The transcript view and
 * `combine` (which feeds the notes) both use this, so what you read is what the notes
 * model reads.
 */
export const speakerLabel = (
  m: { youLabel: string; othersLabel: string; speakerNames?: string[]; callerNames?: string[] },
  s: { source: string; speaker?: number | null },
): string => {
  if (s.speaker == null) return labelFor(s.source, m.youLabel, m.othersLabel);
  return s.source === "you"
    ? m.speakerNames?.[s.speaker]?.trim() || `Speaker ${s.speaker + 1}`
    : m.callerNames?.[s.speaker]?.trim() || `Caller ${s.speaker + 1}`;
};

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
  speakerNames?: string[],
  callerNames?: string[],
): string => {
  if (transcript && transcript.length > 0) {
    // 1.45: lines are grouped by their label, so separated speakers become separate
    // turns ("Speaker 1: …", "Speaker 2: …") in the text the notes model reads.
    const merged: { key: string; seg: TranscriptSeg; text: string }[] = [];
    for (const s of transcript) {
      const text = s.text.trim();
      if (!text) continue;
      const key = `${s.source}|${s.speaker != null ? s.speaker : ""}`;
      const last = merged[merged.length - 1];
      if (last && last.key === key) last.text += ` ${text}`;
      else merged.push({ key, seg: s, text });
    }
    return merged
      .map((m) => `${speakerLabel({ youLabel, othersLabel, speakerNames, callerNames }, m.seg)}: ${m.text}`)
      .join("\n");
  }
  return [
    you.trim() ? `${youLabel}:\n${you.trim()}` : "",
    others.trim() ? `${othersLabel}:\n${others.trim()}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
};

// ---- reading view (1.46) ---------------------------------------------------

/** One speaker turn for reading: consecutive lines by one speaker, as a paragraph. */
export interface Turn {
  label: string;
  source: string;
  startMs: number;
  /** The lines of this turn, each with its index in the meeting's transcript. */
  lines: { seg: TranscriptSeg; index: number }[];
}

/** A turn is broken (same speaker, new paragraph with its own time) after this
 *  long a gap, or once it is this long, so a monologue still has landmarks. */
const TURN_GAP_MS = 45_000;
const TURN_MAX_CHARS = 700;

/**
 * Kōrero 1.46: the transcript grouped for reading. Consecutive lines with the same
 * label become one turn; empty lines are skipped (unless `keep` says otherwise, for
 * a line being edited). Grouping is a VIEW: indexes point back into the transcript,
 * so editing and trimming still act on single lines.
 */
export const groupTurns = (
  m: { youLabel: string; othersLabel: string; speakerNames?: string[]; callerNames?: string[] },
  transcript: TranscriptSeg[],
  keep: (index: number) => boolean = () => false,
): Turn[] => {
  const turns: Turn[] = [];
  let chars = 0;
  transcript.forEach((seg, index) => {
    if (!seg.text.trim() && !keep(index)) return;
    const label = speakerLabel(m, seg);
    const last = turns[turns.length - 1];
    const prev = last?.lines[last.lines.length - 1]?.seg;
    const same =
      !!last &&
      last.label === label &&
      last.source === seg.source &&
      !!prev &&
      segMs(seg) - segMs(prev) < TURN_GAP_MS &&
      chars < TURN_MAX_CHARS;
    if (same && last) {
      last.lines.push({ seg, index });
      chars += seg.text.length;
    } else {
      turns.push({ label, source: seg.source, startMs: segMs(seg), lines: [{ seg, index }] });
      chars = seg.text.length;
    }
  });
  return turns;
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
  m.title.trim() || placeholderTitle(m.createdAt);

/** 1.47: the name a recording carries until it is given one (see titles.ts). */
export const defaultMeetingTitle = (when: number): string => placeholderTitle(when);

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
 * Kōrero 1.44: the first line of the flags block. The backend
 * (`src-tauri/src/notes_parts.rs`, `FLAGS_INTRO`) recognises the block by
 * this exact line, and a Rust test checks the two copies match.
 */
export const FLAGS_INTRO =
  "[The person who recorded this flagged these moments as important. Give each one its own line in the notes.]";

/**
 * The text sent to the notes model: the moments the user flagged, each
 * anchored to the nearest line so the model can give them their own entries,
 * then a blank line, then the visible transcript. Pure: callers pass the
 * segments (with any trim already applied).
 *
 * 1.44: the flags come FIRST. Before, they came after the transcript, so on a
 * meeting longer than the notes limit they were the first thing cut. A long
 * meeting is now read in parts and every part sees the flags; the block must
 * not contain a blank line, because the backend ends it at the first one.
 */
export const notesInput = (m: Meeting, segs: TranscriptSeg[]): string => {
  const body = combine(m.you, m.others, m.youLabel, m.othersLabel, segs, m.speakerNames, m.callerNames);
  const flags = (m.flags ?? []).filter((f) => Number.isFinite(f.atSec));
  if (!body.trim() || flags.length === 0) return body;
  const oneLine = (s: string) => s.replace(/\s+/g, " ").trim();
  const lines = flags
    .slice()
    .sort((a, b) => a.atSec - b.atSec)
    .map((f) => {
      const near = nearestSegment(segs, f.atSec * 1000);
      const quote = near ? oneLine(near.text).slice(0, 140) : "";
      const label = oneLine(f.label);
      return `- ${fmtClock(f.atSec)}${label ? ` (${label})` : ""}${quote ? `: "${quote}"` : ""}`;
    });
  return `${FLAGS_INTRO}\n${lines.join("\n")}\n\n${body}`;
};

/**
 * Kōrero 1.44: true when saved notes end with the line 1.41–1.43 added to
 * notes written from only the first 48,000 characters of a long meeting.
 * Those notes can now be rewritten from the whole meeting.
 */
export const notesCoverPart = (processed: string): boolean =>
  /\*These notes cover only the first \d+% of the transcript/.test(processed);

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
