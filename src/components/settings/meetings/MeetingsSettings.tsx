/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import {
  Circle,
  Square,
  Pause,
  Play,
  Loader2,
  Copy,
  Plus,
  Trash2,
  Check,
  Users,
  TriangleAlert,
  RotateCcw,
  FileAudio,
  RefreshCw,
  Download,
  Sparkles,
  Wand2,
  Pencil,
  Upload,
  X,
  Activity,
  GraduationCap,
  GitMerge,
  Cpu,
  Volume2,
  FolderOpen,
  Search,
  // Kōrero (v1.27.0): trim markers UI.
  Scissors,
} from "lucide-react";
import { toast } from "sonner";
import { confirmDestructive } from "../../ui/confirmToast";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { Markdown } from "../../ui/Markdown";
import { CaptureMeters } from "./CaptureMeters";
import { AddCorrectionInline } from "../../ui/Corrections";
import { Button } from "../../ui/Button";
import { Dropdown, type DropdownOption } from "../../ui/Dropdown";
import { commands, type ModelInfo } from "../../../bindings";
import { useSettings } from "../../../hooks/useSettings";

/**
 * Kōrero fork (v1.13.0): Meetings page.
 *
 * Record mic ("You") + system loopback ("Others"); audio is saved to disk first
 * (failsafe), then transcribed. Per meeting: rename (in the list or the header),
 * pick the transcription model, re-transcribe, post-process with a custom prompt
 * (rendered as markdown), or do both. You can also import a WAV file and have it
 * transcribed + post-processed. Recovery of on-disk recordings is at the bottom.
 */

// v1.17.0: one chronological transcript segment (matches the Rust TranscriptSeg).
// NOTE: this is a LOCAL interface, not a specta-generated binding — so widening
// the Rust struct needs a matching edit HERE and no bindings.ts stub patch.
interface TranscriptSeg {
  source: string; // "you" | "others"
  text: string;
  // Kōrero (v1.26.0): milliseconds from the start of the source file.
  // ABSOLUTE — never rebased to a trim window, so it can be compared directly
  // against a trim marker and used as an audio seek target.
  //
  // Optional in TypeScript ONLY because meetings recorded before v1.26.0 have
  // segments without it on disk. `normaliseMeetings` defaults those to 0, so
  // after load it is always a number. Do NOT write a window predicate against
  // a possibly-undefined value: `undefined >= n` and `undefined <= n` are BOTH
  // false in JS, so a bare comparison silently excludes every legacy segment.
  start_ms?: number;
}

/// The one place a segment's offset is read. Kept as a helper so the
/// legacy-default rule above cannot be forgotten at a call site.
const segMs = (s: TranscriptSeg): number =>
  typeof s.start_ms === "number" ? s.start_ms : 0;

interface Meeting {
  id: string;
  title: string;
  you: string;
  others: string;
  // v1.17.0: ORDERED, interleaved transcript. When present, this is the source
  // of truth for display / copy / export / post-processing, so both speakers
  // appear in the order they spoke. `you`/`others` are kept as the per-speaker
  // grouping the inline editor still uses. Empty for older meetings and
  // single-file imports — callers fall back to the two-block `you`/`others`.
  transcript?: TranscriptSeg[];
  processed: string;
  processPrompt: string;
  createdAt: number;
  systemCaptured: boolean;
  micPath: string | null;
  systemPath: string | null;
  // v1.14.5: editable speaker tags — rename "You"/"Others" to real names
  // (e.g. "Nic" / "Gerard"). Used in display, copy, export, and processing.
  youLabel: string;
  othersLabel: string;
  // v1.17.0: imported/recovered files have exactly ONE audio source, so the
  // "system audio not captured" warning doesn't apply to them.
  imported: boolean;
  // Kōrero (v1.27.0): TRIM MARKERS — a view window over `transcript`, in the
  // same file-absolute milliseconds as `TranscriptSeg.start_ms`. `undefined`
  // on either side means "unbounded that way"; both undefined = untrimmed.
  //
  // These are MARKERS, not an edit: nothing is ever removed from `transcript`
  // and the audio on disk is untouched, so a trim is always reversible. That
  // is the whole design — see `visibleSegs` for the single read-side filter,
  // and the warning attached to it about never persisting a filtered array.
  //
  // No store migration is needed: the meetings store is opaque JSON and
  // `normaliseMeetings` spreads `...m`, so absent keys simply stay absent.
  trimStartMs?: number;
  trimEndMs?: number;
  // Kōrero (v1.27.0): the trim window that was in force when `processed` was
  // generated, as `${trimStartMs ?? ""}:${trimEndMs ?? ""}`. `processed` is a
  // MATERIALISED string — once written, no read-side filter can retroactively
  // apply a trim to it — so the only honest thing to do when the window moves
  // is to tell the user the notes are stale. Absent on meetings whose notes
  // predate v1.27.0; those were necessarily generated untrimmed, so callers
  // default it to the untrimmed key rather than treating it as "unknown".
  processedTrimKey?: string;
}

interface RecordingFile {
  path: string;
  file_name: string;
  modified: number;
}

const STORE_KEY = "korero.meetings.v1";
const DEFAULT_PROMPT =
  "Summarise this meeting: key points, decisions, and action items (with owners).";

const newId = () =>
  (crypto as any)?.randomUUID?.() ?? `m_${Date.now()}_${Math.random()}`;

// Normalise older meetings that predate processed/processPrompt so `.trim()`
// on those fields is always safe.
const normaliseMeetings = (parsed: unknown): Meeting[] => {
  if (!Array.isArray(parsed)) return [];
  return (parsed as Meeting[]).map((m) => ({
    ...m,
    processed: m.processed ?? "",
    processPrompt: m.processPrompt ?? "",
    // v1.17.0: older meetings predate the ordered transcript — default to [].
    // Kōrero (v1.26.0): also default each ELEMENT's start_ms. This used to
    // default the container only, which is a different thing: a pre-v1.26.0
    // meeting would keep segments with `start_ms === undefined`, and because
    // `undefined >= n` and `undefined <= n` are both false, ANY trim window
    // would have excluded every segment of every legacy meeting — silently,
    // with no error and no empty-state.
    transcript: (Array.isArray(m.transcript) ? m.transcript : []).map((s) => ({
      ...s,
      start_ms: typeof s?.start_ms === "number" ? s.start_ms : 0,
    })),
    youLabel: m.youLabel?.trim() || "You",
    othersLabel: m.othersLabel?.trim() || "Others",
    // Older imported meetings predate the flag — infer from the title so
    // existing "Imported · x.m4a" entries stop warning too.
    imported:
      m.imported ??
      (m.title?.startsWith("Imported ·") || m.title?.startsWith("Recovered ·") || false),
  }));
};

// Legacy localStorage store (pre-v1.13.4) — read only for one-time migration.
const loadLegacyMeetings = (): Meeting[] => {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (raw) return normaliseMeetings(JSON.parse(raw));
  } catch {
    /* ignore corrupt store */
  }
  return [];
};

const fmtClock = (s: number) =>
  `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;

// Map a segment's source tag to the meeting's editable speaker label.
const labelFor = (source: string, youLabel: string, othersLabel: string) =>
  source === "you" ? youLabel : othersLabel;

// v1.17.0: build the transcript text. When an ORDERED segment list is present,
// render it in speaking order (`Label: line` per turn) so both speakers
// interleave. Otherwise fall back to the legacy two-block grouping. Used for
// post-processing input, copy, and export — all now chronological.
const combine = (
  you: string,
  others: string,
  youLabel = "You",
  othersLabel = "Others",
  transcript?: TranscriptSeg[],
) => {
  if (transcript && transcript.length > 0) {
    // v1.20.0: merge consecutive segments from the SAME speaker into one turn.
    // Live VAD emits short segments, so a single person's sentence was split
    // across many "You:" lines — which fragmented the transcript, hurt
    // readability, and gave the post-processing model dozens of tiny labelled
    // turns to mangle. One label per contiguous turn fixes copy, export, AND
    // the post-processing input in one place.
    const merged: { source: string; text: string }[] = [];
    for (const s of transcript) {
      const text = s.text.trim();
      if (!text) continue;
      const last = merged[merged.length - 1];
      if (last && last.source === s.source) {
        last.text += ` ${text}`;
      } else {
        merged.push({ source: s.source, text });
      }
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

// ---- trim window (Kōrero v1.27.0) ----------------------------------------
// A trim is a VIEW, not an edit. `transcript` always holds every segment ever
// transcribed; these helpers are the ONLY place the window is applied.
//
// READ-SIDE ONLY. Never feed the output of `visibleSegs` back into
// `setMeetings`, `patchMeeting` or the debounced `meetingsStoreSave` effect.
// The store round-trips whatever is in state, so persisting a filtered array
// would delete the hidden text from disk and make the trim irreversible —
// exactly the failure this marker design exists to avoid. Everything that
// WRITES a meeting must carry the full `m.transcript`.

/// The single window predicate. Shared by `visibleSegs` (which decides what
/// leaves the app) and the renderer (which decides what is dimmed), so the two
/// can never disagree about whether a given segment is in the window.
const segInWindow = (m: Meeting, s: TranscriptSeg): boolean =>
  (m.trimStartMs == null || segMs(s) >= m.trimStartMs) &&
  (m.trimEndMs == null || segMs(s) <= m.trimEndMs);

/// Every consumer that turns a transcript into text — copy, export, search,
/// post-processing — reads through here instead of touching `m.transcript`.
const visibleSegs = (m: Meeting): TranscriptSeg[] =>
  (m.transcript ?? []).filter((s) => segInWindow(m, s));

const isTrimmed = (m: Meeting): boolean =>
  m.trimStartMs != null || m.trimEndMs != null;

const hiddenCount = (m: Meeting): number =>
  (m.transcript?.length ?? 0) - visibleSegs(m).length;

/// Identity of the current window, used to detect notes that were generated
/// under a DIFFERENT window. Untrimmed is ":" (not "") so a legacy meeting can
/// be defaulted to it unambiguously.
const trimKeyOf = (m: {
  trimStartMs?: number;
  trimEndMs?: number;
}): string => `${m.trimStartMs ?? ""}:${m.trimEndMs ?? ""}`;

/// `processed` is materialised text, so it cannot be filtered after the fact —
/// it can only be invalidated. Notes with no recorded key predate v1.27.0 and
/// were therefore generated untrimmed.
const notesAreStale = (m: Meeting): boolean =>
  m.processed.trim() !== "" && (m.processedTrimKey ?? ":") !== trimKeyOf(m);

/// mm:ss.s. One decimal — enough to land on a segment boundary, without
/// implying millisecond precision the user cannot actually aim at.
const fmtTrimMs = (ms: number): string => {
  const total = Math.max(0, ms) / 1000;
  const mins = Math.floor(total / 60);
  const secs = total - mins * 60;
  return `${mins}:${secs.toFixed(1).padStart(4, "0")}`;
};

/// Parse "m:ss.s", "mm:ss" or plain seconds ("93.5"). Returns null for anything
/// else so the caller can show ONE inline message — coercing junk to 0 would
/// silently move the in-point to the start of the meeting.
const parseTrimInput = (raw: string): number | null => {
  const t = raw.trim();
  if (!t) return null;
  const mmss = /^(\d+):([0-5]?\d(?:\.\d+)?)$/.exec(t);
  if (mmss) return Math.round((Number(mmss[1]) * 60 + Number(mmss[2])) * 1000);
  if (/^\d+(?:\.\d+)?$/.test(t)) return Math.round(Number(t) * 1000);
  return null;
};

const baseName = (p: string) => p.replace(/\\/g, "/").split("/").pop() || p;

export const MeetingsSettings: React.FC = () => {
  const { settings, updateSetting } = useSettings();

  // v1.13.4: meetings live on disk (appdata/meetings/meetings.json); loaded
  // async on mount, with one-time migration from the legacy localStorage store.
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [storeReady, setStoreReady] = useState(false);
  // v1.29.0 (R-02): non-null means the store could not be read this session.
  // While it is set, `storeReady` stays false and NOTHING is written to disk,
  // so a transient read failure can never be converted into permanent erasure.
  const [storeLoadError, setStoreLoadError] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  // v1.19.0: pause state for a live meeting.
  const [paused, setPaused] = useState(false);
  const [recProcessing, setRecProcessing] = useState(false);
  // v1.19.0: chunked-transcription progress for imports / re-transcribes.
  const [transcribeProgress, setTranscribeProgress] = useState<{
    window: number;
    total: number | null;
  } | null>(null);
  // v1.13.3: set when the Rust capture worker reports a disk-write failure
  // mid-meeting (meeting-capture-error event) — e.g. disk full.
  const [captureError, setCaptureError] = useState<string | null>(null);
  // v1.13.5: device test state (meters themselves live in CaptureMeters,
  // v1.14.0 item 6 — their 10 Hz events no longer re-render this page).
  const [testing, setTesting] = useState(false);
  const [devices, setDevices] = useState<{ mic: string; system: string } | null>(
    null,
  );
  // Phase B (v1.14.0): live transcript streamed during recording + on-the-fly
  // questions about the meeting so far (Phase C).
  const [liveSegments, setLiveSegments] = useState<
    { source: string; text: string }[]
  >([]);
  const [liveQuestion, setLiveQuestion] = useState("");
  const [liveAsking, setLiveAsking] = useState(false);
  const [liveAnswer, setLiveAnswer] = useState("");
  // v1.17.0: streaming post-process preview — accumulates `meeting-postprocess-delta`
  // tokens so the notes render as they generate instead of after a long wait.
  const [liveProcessed, setLiveProcessed] = useState("");
  // v1.21.0: local audio-brief (Qwen3-TTS) state. briefBusy spans a multi-minute
  // GPU render; briefUrl is an asset:// URL for the produced MP3.
  const [briefBusy, setBriefBusy] = useState(false);
  const [briefUrl, setBriefUrl] = useState<string | null>(null);
  // v1.22.0: raw MP3 path (for Download + Show-in-folder on the audio brief).
  const [briefPath, setBriefPath] = useState<string | null>(null);
  // v1.22.0: edit/refine the processed notes + inline-edit a transcript segment.
  const [editingNotes, setEditingNotes] = useState(false);
  const [notesDraft, setNotesDraft] = useState("");
  const [feedback, setFeedback] = useState("");
  const [refining, setRefining] = useState(false);
  const [editingSegIdx, setEditingSegIdx] = useState<number | null>(null);
  // Kōrero (v1.27.0): drafts for the mm:ss.s in/out fields. Held separately
  // from the markers so a half-typed "1:" never becomes a live trim, and so an
  // invalid entry can be rejected with a message instead of being coerced.
  const [trimInDraft, setTrimInDraft] = useState("");
  const [trimOutDraft, setTrimOutDraft] = useState("");
  const [trimError, setTrimError] = useState<string | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [systemCaptured, setSystemCaptured] = useState<boolean | null>(null);
  const [busy, setBusy] = useState<null | "transcribe" | "post" | "both">(null);
  // v1.25.0 (UX batch, audit #5): elapsed-time feedback for long operations —
  // a spinner alone reads as "frozen" after ~30 s on a 2-minute transcription.
  const [busyElapsed, setBusyElapsed] = useState(0);
  useEffect(() => {
    if (!busy) {
      setBusyElapsed(0);
      return;
    }
    const started = Date.now();
    const t = window.setInterval(
      () => setBusyElapsed(Math.floor((Date.now() - started) / 1000)),
      1000,
    );
    return () => window.clearInterval(t);
  }, [busy]);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [customPrompt, setCustomPrompt] = useState(DEFAULT_PROMPT);
  // v1.19.0: meeting/import post-processing prompt picker selections
  // ("custom" = use the editable textarea text; otherwise a saved prompt id).
  const [meetingPromptId, setMeetingPromptId] = useState<string>("custom");
  const [importPromptId, setImportPromptId] = useState<string>("custom");
  const [providerLocal, setProviderLocal] = useState<boolean | null>(null);
  const [recordings, setRecordings] = useState<RecordingFile[] | null>(null);
  const [busyFile, setBusyFile] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // v1.22.0: last exported file path — surfaced as a copyable + "Show in folder"
  // row (replaces the old ephemeral, non-selectable "Exported to <path>" toast).
  const [exportedPath, setExportedPath] = useState<string | null>(null);
  // v1.25.0 (UX batch, audit #1): the row previously showed the LAST export's
  // path even after switching meetings — clear it whenever the active
  // meeting changes.
  useEffect(() => {
    setExportedPath(null);
  }, [activeId]);
  // Review fixes (v1.25.0 #2/#3): live refs for the delete-confirm callback,
  // which can fire seconds later — possibly after this panel unmounted or the
  // selection moved.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);
  const activeIdRef = useRef<string | null>(null);
  useEffect(() => {
    activeIdRef.current = activeId;
  }, [activeId]);
  // v1.24.0 (paths): storage folders — effective recording dir + export seed.
  const [dirs, setDirs] = useState<{
    recordingDir: string;
    recordingIsCustom: boolean;
    defaultRecordingDir: string;
    exportSeedDir: string;
  } | null>(null);
  const [dirBusy, setDirBusy] = useState(false);
  // v1.22.0: meetings search query — filters the list by title + transcript + notes.
  const [search, setSearch] = useState("");
  const [editingListId, setEditingListId] = useState<string | null>(null);
  // v1.14.5: which speaker tag is being renamed in the transcript view.
  const [editingLabel, setEditingLabel] = useState<null | "you" | "others">(
    null,
  );
  // v1.15.0: teach-a-correction form (prefilled from the text selection).
  const [teachWrong, setTeachWrong] = useState<string | null>(null);
  // v1.17.0: merge-with picker selection.
  const [mergeWithId, setMergeWithId] = useState<string>("");
  // Import workflow
  const [importPath, setImportPath] = useState<string | null>(null);
  const [importPrompt, setImportPrompt] = useState(DEFAULT_PROMPT);
  const [importBusy, setImportBusy] = useState(false);

  const timerRef = useRef<number | null>(null);
  const titleRef = useRef<HTMLInputElement>(null);
  // v1.14.2: when the page remounts during an in-progress meeting, the timer
  // resumes from here instead of 0 (set by the mount-time status restore).
  const restoredElapsedRef = useRef(0);
  const active = meetings.find((m) => m.id === activeId) ?? null;

  // v1.22.0: clear per-meeting edit state when the active meeting changes.
  useEffect(() => {
    setEditingNotes(false);
    setEditingSegIdx(null);
    setFeedback("");
  }, [activeId]);

  // Kōrero (v1.27.0): keep the in/out fields showing the CURRENT window. They
  // re-seed from the markers (not just on meeting change) because the segment
  // "Start here" / "End here" buttons also move the window — without this the
  // fields would keep showing a stale time after a one-click trim.
  useEffect(() => {
    setTrimInDraft(
      active?.trimStartMs != null ? fmtTrimMs(active.trimStartMs) : "",
    );
    setTrimOutDraft(
      active?.trimEndMs != null ? fmtTrimMs(active.trimEndMs) : "",
    );
    setTrimError(null);
  }, [activeId, active?.trimStartMs, active?.trimEndMs]);
  const currentModel = settings?.selected_model ?? "";

  // v1.20.0: the active POST-PROCESSING provider/model (distinct from the
  // transcription model above). Surfaced in the Meetings tab so it's clear which
  // model is generating the notes. Mirrors the NotesSettings lookup.
  const ppProvider = settings?.post_process_providers?.find(
    (p) => p.id === settings?.post_process_provider_id,
  );
  const ppModel =
    (settings?.post_process_models ?? {})[
      settings?.post_process_provider_id ?? ""
    ] ?? "";
  const ppLabel = ppModel
    ? `${ppProvider?.label ?? "Model"} · ${ppModel}`
    : ppProvider?.label
      ? `${ppProvider.label} · no model set`
      : "not configured";

  // v1.19.0: post-processing prompt picker — shares the saved-prompts store with
  // dictation/Notes. Selecting a saved prompt loads its text into the editable
  // textarea (still editable); "Custom" leaves whatever is there.
  const promptOptions: DropdownOption[] = [
    ...(settings?.post_process_prompts ?? []).map((p) => ({
      value: p.id,
      label: p.name,
    })),
    { value: "custom", label: "Custom prompt…" },
  ];
  const savedPromptText = (id: string): string =>
    settings?.post_process_prompts?.find((p) => p.id === id)?.prompt ?? "";
  // Persist the current textarea text as a brand-new saved prompt (memory note:
  // post_process_prompts is written via updateSetting, not an upstream command).
  const savePromptAsNew = async (text: string): Promise<string | null> => {
    const body = text.trim();
    if (!body) {
      toast.error("Nothing to save — the prompt is empty.");
      return null;
    }
    const name = window.prompt("Name this prompt:")?.trim();
    if (!name) return null;
    const id = `user_${Date.now()}`;
    const next = [
      ...(settings?.post_process_prompts ?? []),
      { id, name, prompt: body, alias: null },
    ];
    try {
      await updateSetting("post_process_prompts", next);
      toast.success(`Saved prompt "${name}".`);
      return id;
    } catch (e) {
      toast.error(`Could not save prompt: ${String(e)}`);
      return null;
    }
  };

  // v1.13.4: load from disk; migrate the legacy localStorage store once, and
  // only clear the legacy copy after a verified round-trip to disk.
  useEffect(() => {
    (async () => {
      let list: Meeting[] = [];
      // v1.29.0 (R-02). THE MOST DESTRUCTIVE BUG THIS FILE HAS HAD.
      //
      // meetingsStoreLoad returns { status: "error" } instead of throwing, so
      // a read failure never reached the catch below — `list` simply stayed
      // empty. A truncated file threw, hit the catch, fell through to a
      // localStorage store deleted back in v1.13.4, and also ended empty.
      // Either way setStoreReady(true) fired and the autosave effect wrote
      // `[]` over meetings.json 500 ms later. Every meeting, gone, silently,
      // permanently. A file momentarily locked by antivirus or OneDrive was
      // enough, and WAVs age out at 30 days so the store is often the only
      // copy left.
      //
      // The rule now: we only ever save a store we successfully READ.
      let loadFailure: string | null = null;
      try {
        const res = await commands.meetingsStoreLoad();
        if (res.status === "error") {
          loadFailure = String(res.error ?? "could not read the meetings store");
        } else if (res.data.trim()) {
          list = normaliseMeetings(JSON.parse(res.data));
        }
        // An empty string is a genuinely absent store (first run) — not a
        // failure. That case must still be allowed to save.
      } catch (e) {
        loadFailure = e instanceof Error ? e.message : String(e);
      }

      if (loadFailure) {
        console.error("Meetings store failed to load:", loadFailure);
        setStoreLoadError(loadFailure);
        setMeetings([]);
        setActiveId(null);
        // Deliberately NOT setStoreReady(true): leaving it false is what keeps
        // the autosave effect from committing this empty list to disk.
        return;
      }

      if (list.length === 0) {
        const legacy = loadLegacyMeetings();
        if (legacy.length > 0) {
          list = legacy;
          try {
            const saved = await commands.meetingsStoreSave(
              JSON.stringify(legacy),
            );
            if (saved.status === "ok") {
              const check = await commands.meetingsStoreLoad();
              if (check.status === "ok" && check.data.trim()) {
                localStorage.removeItem(STORE_KEY);
              }
            }
          } catch {
            /* keep the legacy copy until a save round-trips */
          }
        }
      }
      setMeetings(list);
      setActiveId(list[0]?.id ?? null);
      setStoreReady(true);
    })();
  }, []);

  // v1.13.4: debounced save to disk — replaces the per-change localStorage
  // stringify (≈5 MB quota silently dropped writes; main-thread jank on
  // large transcripts). storeReady gates it so the initial empty state can
  // never overwrite a populated store before the load completes.
  useEffect(() => {
    if (!storeReady) return;
    const t = window.setTimeout(() => {
      commands
        .meetingsStoreSave(JSON.stringify(meetings))
        .then((r) => {
          if (r.status === "error") {
            toast.error(`Couldn't save meetings: ${r.error}`);
          }
        })
        .catch(() => {});
    }, 500);
    return () => window.clearTimeout(t);
  }, [meetings, storeReady]);

  useEffect(() => {
    if (recording) {
      // v1.14.2: resume from the restored elapsed time after a remount;
      // restoredElapsedRef is 0 for a freshly started recording.
      setElapsed(restoredElapsedRef.current);
      restoredElapsedRef.current = 0;
    } else if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
    return () => {
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
    };
  }, [recording]);

  // v1.19.0: the elapsed clock runs only while recording AND not paused, so
  // the displayed time matches the backend's paused-excluded elapsed.
  useEffect(() => {
    if (recording && !paused) {
      timerRef.current = window.setInterval(() => setElapsed((e) => e + 1), 1000);
    } else if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
    return () => {
      if (timerRef.current !== null) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [recording, paused]);

  // v1.13.3: surface mid-meeting capture failures (disk full / unwritable).
  // The recording up to the failure point is preserved on disk.
  useEffect(() => {
    const un = listen<string>("meeting-capture-error", (e) => {
      setCaptureError(e.payload);
      toast.error(e.payload);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Phase B (v1.14.0): live transcript segments from the meeting consumer.
  useEffect(() => {
    const un = listen<{ source: string; text: string }>(
      "meeting-live-segment",
      (e) => {
        setLiveSegments((prev) => [...prev, e.payload]);
      },
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  // v1.17.0: stream post-processing tokens into the preview as they arrive.
  useEffect(() => {
    const unDelta = listen<string>("meeting-postprocess-delta", (e) => {
      setLiveProcessed((prev) => prev + e.payload);
    });
    return () => {
      unDelta.then((f) => f());
    };
  }, []);

  // v1.19.0: chunked-transcription progress (imports + re-transcribes).
  useEffect(() => {
    const un = listen<{ id: string; window: number; total: number | null }>(
      "meeting-transcribe-progress",
      (e) => {
        setTranscribeProgress({ window: e.payload.window, total: e.payload.total });
      },
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  // v1.14.2: restore the recording UI if a meeting is still running on the
  // backend (the page was unmounted mid-meeting). Without this, navigating
  // away and back showed an idle page over a live recording — which also
  // explains "says it's recording although it doesn't look like it" in
  // reverse: recording without looking like it.
  useEffect(() => {
    (async () => {
      try {
        const res = await commands.meetingRecordingStatus();
        if (res.status === "ok" && res.data) {
          restoredElapsedRef.current = res.data.elapsed_secs;
          setSystemCaptured(res.data.system_captured);
          setPaused(res.data.paused); // v1.19.0: restore paused state too
          setRecording(true);
          refreshDevices();
          toast.message(
            "A meeting is still being recorded — controls restored.",
          );
        }
      } catch {
        /* no restore — page starts idle as usual */
      }
    })();
  }, []);

  // v1.13.5: device names for the meter labels.
  const refreshDevices = async () => {
    try {
      const res = await commands.meetingCaptureDevices();
      if (res.status === "ok") setDevices(res.data);
    } catch {
      /* names are cosmetic */
    }
  };
  useEffect(() => {
    refreshDevices();
  }, []);

  const loadRecordings = async () => {
    try {
      const res = await commands.meetingListRecordings();
      setRecordings(res.status === "ok" ? res.data : []);
    } catch {
      setRecordings([]);
    }
  };

  // v1.13.6: delete a saved WAV from disk (recovery list).
  // v1.25.0 (UX batch): confirmed first — recordings are unrecoverable (audit #2).
  const deleteRecording = async (f: RecordingFile) => {
    confirmDestructive(
      `Delete ${f.file_name}?`,
      "The audio file is removed from disk permanently.",
      "Delete",
      async () => {
        try {
          const res = await commands.meetingDeleteRecording(f.path);
          if (res.status === "ok") {
            toast.success(`Deleted ${f.file_name}`);
            loadRecordings();
          } else {
            toast.error(res.error);
          }
        } catch (e) {
          toast.error(String(e));
        }
      },
    );
  };

  useEffect(() => {
    loadRecordings();
    commands
      .meetingProviderIsLocal()
      .then((r) => setProviderLocal(r.status === "ok" ? r.data : null))
      .catch(() => setProviderLocal(null));
    commands
      .getAvailableModels()
      .then((r) =>
        setModels(r.status === "ok" ? r.data.filter((m) => m.is_downloaded) : []),
      )
      .catch(() => setModels([]));
  }, []);

  useEffect(() => {
    setCustomPrompt(
      active?.processPrompt?.trim() ? active.processPrompt : DEFAULT_PROMPT,
    );
  }, [activeId]);

  const patchMeeting = (id: string, patch: Partial<Meeting>) =>
    setMeetings((prev) => prev.map((m) => (m.id === id ? { ...m, ...patch } : m)));

  // ---- trim markers (Kōrero v1.27.0) -------------------------------------
  // All three writers patch ONLY the marker fields. `transcript` is never
  // passed, so there is no path from the trim UI to a shortened transcript.

  /// Move one edge of the window. Rejects an inverted result rather than
  /// accepting a window that would hide the entire meeting.
  const setTrimPoint = (which: "start" | "end", ms: number) => {
    if (!active) return;
    const start = which === "start" ? ms : active.trimStartMs;
    const end = which === "end" ? ms : active.trimEndMs;
    if (start != null && end != null && end <= start) {
      toast.error(
        which === "start"
          ? "That start is at or after the current end point — set the end further down first."
          : "That end is at or before the current start point — set the start higher up first.",
      );
      return;
    }
    patchMeeting(
      active.id,
      which === "start" ? { trimStartMs: ms } : { trimEndMs: ms },
    );
    setTrimError(null);
  };

  /// One click back to the whole meeting. Nothing to restore — the segments
  /// never left `transcript`, only the markers did.
  const clearTrim = () => {
    if (!active) return;
    patchMeeting(active.id, { trimStartMs: undefined, trimEndMs: undefined });
    setTrimError(null);
    toast.success("Trim cleared — showing the whole meeting.");
  };

  /// Commit both mm:ss.s fields together. Validated as a PAIR, because the
  /// only invalid state (end at or before start) spans the two of them.
  const commitTrimInputs = () => {
    if (!active) return;
    const inRaw = trimInDraft.trim();
    const outRaw = trimOutDraft.trim();
    // Kōrero (v1.27.0): parsed with explicit early returns rather than a
    // ternary, so each value narrows to `number | undefined` — `undefined`
    // means "no bound", `null` from the parser means "not a timecode", and the
    // marker fields must never be able to hold the latter.
    let startMs: number | undefined;
    if (inRaw) {
      const parsed = parseTrimInput(inRaw);
      if (parsed == null) {
        setTrimError(
          "Start must be mm:ss.s (e.g. 1:05.5) or seconds (e.g. 65.5).",
        );
        return;
      }
      startMs = parsed;
    }
    let endMs: number | undefined;
    if (outRaw) {
      const parsed = parseTrimInput(outRaw);
      if (parsed == null) {
        setTrimError("End must be mm:ss.s (e.g. 4:30.0) or seconds (e.g. 270).");
        return;
      }
      endMs = parsed;
    }
    if (startMs != null && endMs != null && endMs <= startMs) {
      setTrimError(
        "The end must come after the start — that window would hide the whole meeting.",
      );
      return;
    }
    setTrimError(null);
    patchMeeting(active.id, { trimStartMs: startMs, trimEndMs: endMs });
  };

  const titleOf = (m: Meeting) =>
    m.title.trim() || `Meeting · ${new Date(m.createdAt).toLocaleString()}`;

  // v1.22.0: auto-generate a meeting title stamped with the date + time at
  // creation. The user can overwrite it in the title field or the list.
  const defaultMeetingTitle = (when: number) =>
    `Meeting ${new Date(when).toLocaleString(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    })}`;

  // v1.22.0: case-insensitive search across title + transcript + processed notes.
  const searchQ = search.trim().toLowerCase();
  // Kōrero (v1.27.0): search reads through `visibleSegs`, so a trimmed-away
  // aside cannot resurface a meeting the user has deliberately narrowed. Search
  // is a read path — the full transcript is still on the record, untouched.
  const filteredMeetings = searchQ
    ? meetings.filter((m) =>
        `${titleOf(m)} ${m.you} ${m.others} ${visibleSegs(m)
          .map((s) => s.text)
          .join(" ")} ${m.processed}`
          .toLowerCase()
          .includes(searchQ),
      )
    : meetings;

  // v1.17.0: merge two meetings into a NEW combined entry — non-destructive,
  // both originals are kept. Parts are joined in chronological order, so
  // "Part 1" + "Part 2" imports line up correctly.
  const mergeMeetings = (otherId: string) => {
    const a = active;
    const b = meetings.find((m) => m.id === otherId);
    if (!a || !b || a.id === b.id) return;
    const [first, second] = a.createdAt <= b.createdAt ? [a, b] : [b, a];
    const joinPart = (x: string, y: string) =>
      [x.trim(), y.trim()].filter(Boolean).join("\n\n— · —\n\n");
    const m: Meeting = {
      id: newId(),
      title: `${titleOf(first)} + ${titleOf(second)}`,
      you: joinPart(first.you, second.you),
      others: joinPart(first.others, second.others),
      // v1.17.0: concatenate the two ordered transcripts (first part then
      // second) so the merged entry keeps the interleaved conversation view.
      //
      // Kōrero (v1.27.0): note this is the FULL transcript of both parts, not
      // `visibleSegs` — merge is a WRITE path, and filtering here would burn
      // the hidden text of both originals into a new record permanently.
      transcript: [
        ...(first.transcript ?? []),
        ...(second.transcript ?? []),
      ],
      // Kōrero (v1.27.0): the merged entry is deliberately UNTRIMMED — the
      // marker fields are omitted. `start_ms` is file-absolute, and the two
      // parts come from two different files, so the second part's offsets
      // restart from 0 and overlap the first's. Carrying either window across
      // would hide arbitrary segments of the wrong part. Same reasoning for
      // `processedTrimKey`: the joined notes belong to no single window.
      youLabel: first.youLabel,
      othersLabel: first.othersLabel,
      imported: first.imported && second.imported,
      processed: joinPart(first.processed, second.processed),
      processPrompt: first.processPrompt || second.processPrompt,
      createdAt: Date.now(),
      systemCaptured: first.systemCaptured || second.systemCaptured,
      // Audio stays with the originals; the merged entry references the
      // first part's files so re-transcribe still does something sensible.
      micPath: first.micPath ?? second.micPath,
      systemPath: first.systemPath ?? second.systemPath,
    };
    setMeetings((prev) => [m, ...prev]);
    setActiveId(m.id);
    setMergeWithId("");
    toast.success(
      "Merged into a new meeting — both originals kept. Re-process to get one combined summary.",
    );
    // Kōrero (v1.27.0): say it out loud when a trim was dropped. Silently
    // losing a window would look like the merge had resurrected deleted text.
    if (isTrimmed(first) || isTrimmed(second)) {
      toast.message("Trim cleared on the merged meeting.", {
        description:
          "The two parts come from different audio files, so their timings are no longer comparable. The originals keep their own trims.",
      });
    }
  };

  // ---- live query (Phase C, v1.14.0) ---------------------------------------
  // Ask the configured post-processing model about the meeting SO FAR, using
  // the live segments already in hand. Reuses meeting_query (48k-char cap +
  // egress allowlist on the Rust side).
  const askLive = async () => {
    const q = liveQuestion.trim();
    if (!q || liveAsking) return;
    const transcript = liveSegments
      .map((s) => `${s.source === "you" ? "You" : "Others"}: ${s.text}`)
      .join("\n");
    if (!transcript) return;
    setLiveAsking(true);
    setLiveAnswer("");
    try {
      const res = await commands.meetingQuery(transcript, q);
      if (res.status === "ok") setLiveAnswer(res.data);
      else toast.error(res.error);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setLiveAsking(false);
    }
  };

  // ---- device test (v1.13.5) ----------------------------------------------
  // Runs the EXACT meeting capture path for 8 s against throwaway WAVs so mic
  // and system-audio capture can be verified without risking a real meeting.
  const runTest = async () => {
    if (recording || recProcessing || testing) return;
    setTesting(true);
    refreshDevices();
    toast.message(
      "Testing for 8 seconds — play any audio now so the system meter has something to capture.",
    );
    try {
      const res = await commands.meetingTestCapture(8);
      if (res.status === "ok") {
        const { mic_device, system_device, mic_samples, system_samples } =
          res.data;
        if (mic_samples > 0) {
          toast.success(`Microphone OK — ${mic_device}`);
        } else {
          toast.error(
            `No audio from the microphone (${mic_device}). Check it isn't muted or in use by another app.`,
          );
        }
        if (system_samples > 0) {
          toast.success(`System audio OK — ${system_device}`);
        } else {
          toast.error(
            `No system audio captured (${system_device}). Loopback records the DEFAULT output device — if your meeting audio plays through a different device (e.g. a headset), set that device as the Windows default output, then re-test with audio playing.`,
            { duration: 12000 },
          );
        }
      } else {
        toast.error(res.error);
      }
    } catch (e) {
      toast.error(String(e));
    } finally {
      setTesting(false);
    }
  };

  // ---- recording ---------------------------------------------------------
  const toggleRecord = async () => {
    if (recProcessing) return;
    if (recording) {
      setRecording(false);
      setPaused(false); // v1.19.0: clear pause state on stop
      setRecProcessing(true);
      try {
        const res = await commands.meetingStopCapture();
        if (res.status === "ok") {
          const { you, others, segments, mic_path, system_path } = res.data;
          if (!you.trim() && !others.trim() && !mic_path && !system_path) {
            toast.message("No audio captured.");
          } else {
            const m: Meeting = {
              id: newId(),
              title: defaultMeetingTitle(Date.now()),
              you,
              others,
              // v1.17.0: chronological, interleaved transcript from the backend.
              transcript: segments ?? [],
              processed: "",
              processPrompt: "",
              createdAt: Date.now(),
              systemCaptured: systemCaptured ?? false,
              micPath: mic_path,
              systemPath: system_path,
              youLabel: "You",
              othersLabel: "Others",
              imported: false,
            };
            setMeetings((prev) => [m, ...prev]);
            setActiveId(m.id);
            // v1.17.0: warm the local post-processing model now, so the first
            // "Generate notes" doesn't pay the cold model-load cost.
            if (you.trim() || others.trim()) {
              commands.meetingPrewarmPostProcess().catch(() => {});
            }
            if (!you.trim() && !others.trim()) {
              toast.message(
                "Audio saved, but transcription was empty — you can re-transcribe it.",
              );
            }
          }
          loadRecordings();
        } else {
          toast.error(`Meeting stop failed: ${res.error}`);
        }
      } catch (e) {
        toast.error(`Meeting stop failed: ${String(e)}`);
      } finally {
        setRecProcessing(false);
        setSystemCaptured(null);
      }
    } else {
      try {
        setCaptureError(null);
        setLiveSegments([]);
        setLiveAnswer("");
        refreshDevices();
        const res = await commands.meetingStartCapture();
        if (res.status === "ok") {
          setSystemCaptured(res.data);
          setPaused(false); // v1.19.0: fresh meeting starts un-paused
          setRecording(true);
          if (!res.data) {
            toast.message(
              "System audio couldn't be captured — recording your mic only.",
            );
          }
        } else {
          toast.error(res.error);
        }
      } catch (e) {
        toast.error(`Could not start the meeting: ${String(e)}`);
      }
    }
  };

  // v1.19.0: pause / resume the live meeting. Optimistic UI: flip immediately,
  // roll back on error. The mic indicator stays on while paused (the stream is
  // kept open) — surfaced in the tip below the button.
  const togglePause = async () => {
    if (!recording || recProcessing) return;
    const next = !paused;
    setPaused(next);
    try {
      const res = next
        ? await commands.meetingPause()
        : await commands.meetingResume();
      if (res.status !== "ok") {
        setPaused(!next);
        toast.error(res.error);
      }
    } catch (e) {
      setPaused(!next);
      toast.error(`Could not ${next ? "pause" : "resume"} the meeting: ${String(e)}`);
    }
  };

  // ---- model -------------------------------------------------------------
  const changeModel = async (id: string) => {
    if (!id || id === currentModel) return;
    try {
      const res = await commands.setActiveModel(id);
      if (res.status !== "ok") toast.error(`Couldn't switch model: ${res.error}`);
    } catch (e) {
      toast.error(`Couldn't switch model: ${String(e)}`);
    }
  };

  // ---- transcription / post-processing helpers ---------------------------
  // v1.17.0: re-transcribe. For recorded WAV pairs, use the merge command so
  // the rebuilt transcript stays chronological (interleaved). Non-WAV imports
  // (m4a/mp3/…) fall back to per-file transcription with no ordered segments.
  const doTranscribe = async (
    m: Meeting,
  ): Promise<{ you: string; others: string; transcript: TranscriptSeg[] }> => {
    const isWav = (p: string | null) => !!p && /\.wav$/i.test(p);
    if (isWav(m.micPath) || isWav(m.systemPath)) {
      const r = await commands.meetingTranscribeMerge(
        isWav(m.micPath) ? m.micPath : null,
        isWav(m.systemPath) ? m.systemPath : null,
      );
      if (r.status !== "ok") throw new Error(r.error);
      const transcript = r.data as TranscriptSeg[];
      const join = (src: string) =>
        transcript
          .filter((s) => s.source === src)
          .map((s) => s.text)
          .join(" ");
      const you = join("you");
      const others = join("others");
      patchMeeting(m.id, { you, others, transcript });
      return { you, others, transcript };
    }
    const tx = async (path: string | null) => {
      if (!path) return "";
      const r = await commands.meetingTranscribeFile(path);
      if (r.status === "ok") return r.data;
      throw new Error(r.error);
    };
    const you = await tx(m.micPath);
    const others = await tx(m.systemPath);
    patchMeeting(m.id, { you, others, transcript: [] });
    return { you, others, transcript: [] };
  };

  const doPostProcess = async (m: Meeting, text: string): Promise<void> => {
    setLiveProcessed(""); // reset the streaming preview for this run
    try {
      const r = await commands.meetingPostProcess(text, customPrompt.trim());
      if (r.status !== "ok") throw new Error(r.error);
      // Kōrero (v1.27.0): stamp the window these notes were generated under.
      // `m` is the meeting as it was when the run STARTED, which is the window
      // whose text was actually sent to the model — if the user moved the trim
      // mid-run, the notes really are stale and should say so.
      patchMeeting(m.id, {
        processed: r.data,
        processPrompt: customPrompt.trim(),
        processedTrimKey: trimKeyOf(m),
      });
    } finally {
      // The persisted `processed` now renders; drop the transient preview.
      setLiveProcessed("");
    }
  };

  const onReTranscribe = async () => {
    if (!active || busy) return;
    setBusy("transcribe");
    try {
      const { you, others } = await doTranscribe(active);
      if (!you.trim() && !others.trim()) toast.message("Still no speech found.");
    } catch (e) {
      toast.error(`Re-transcription failed: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const onPostProcess = async () => {
    if (!active || busy) return;
    // Kōrero (v1.27.0): `combine` is a pure formatter — it renders whatever
    // array it is handed — so the window has to be applied HERE, at the call
    // site, or the model would summarise segments the user has hidden.
    const text = combine(
      active.you,
      active.others,
      active.youLabel,
      active.othersLabel,
      visibleSegs(active),
    );
    if (!text.trim()) {
      // Kōrero (v1.27.0): distinguish "no transcript" from "the trim hid it
      // all" — otherwise a user who over-trimmed is told to transcribe again.
      toast.message(
        isTrimmed(active)
          ? "The current trim hides every segment — widen or clear it first."
          : "Nothing to post-process — transcribe first.",
      );
      return;
    }
    setBusy("post");
    try {
      await doPostProcess(active, text);
    } catch (e) {
      toast.error(`Post-processing failed: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const onBoth = async () => {
    if (!active || busy) return;
    setBusy("both");
    try {
      const { you, others, transcript } = await doTranscribe(active);
      // Kōrero (v1.27.0): the freshly transcribed segments are not on `active`
      // yet (that setState hasn't flushed), so the window is applied to a
      // throwaway meeting-shaped value carrying the SAME markers. `start_ms`
      // is file-absolute and the audio has not changed, so a marker set before
      // the re-transcribe still points at the same moment afterwards.
      const text = combine(
        you,
        others,
        active.youLabel,
        active.othersLabel,
        visibleSegs({ ...active, transcript }),
      );
      if (!text.trim()) {
        toast.message("No speech found to post-process.");
        return;
      }
      await doPostProcess(active, text);
    } catch (e) {
      toast.error(`Transcribe + post-process failed: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  };

  // ---- import audio file -------------------------------------------------
  const pickImportFile = async () => {
    try {
      const sel = await openFileDialog({
        multiple: false,
        // v1.16.1: compressed formats decode via rodio (symphonia) — m4a is
      // what phone/Teams recordings usually arrive as.
      filters: [
        {
          name: "Audio files",
          extensions: ["wav", "m4a", "mp3", "aac", "flac", "ogg", "caf", "aiff", "aif"],
        },
      ],
      });
      if (typeof sel === "string") {
        setImportPath(sel);
        setImportPrompt(DEFAULT_PROMPT);
      }
    } catch (e) {
      toast.error(`Could not open file picker: ${String(e)}`);
    }
  };

  const runImport = async (alsoProcess: boolean) => {
    if (!importPath || importBusy) return;
    setImportBusy(true);
    setTranscribeProgress(null); // v1.19.0: reset the progress bar for this run
    try {
      const r = await commands.meetingTranscribeFile(importPath);
      if (r.status !== "ok") {
        toast.error(`Transcription failed: ${r.error}`);
        return;
      }
      const transcript = r.data;
      let processed = "";
      let processPrompt = "";
      if (alsoProcess && transcript.trim()) {
        const pr = await commands.meetingPostProcess(transcript, importPrompt.trim());
        if (pr.status === "ok") {
          processed = pr.data;
          processPrompt = importPrompt.trim();
        } else {
          toast.error(`Post-processing failed: ${pr.error}`);
        }
      }
      const m: Meeting = {
        id: newId(),
        title: `Imported · ${baseName(importPath)}`,
        you: transcript,
        others: "",
        youLabel: "You",
        othersLabel: "Others",
        imported: true,
        processed,
        processPrompt,
        // Kōrero (v1.27.0): a brand-new import is untrimmed, so its notes were
        // generated under the untrimmed window. Stamping it here means the
        // stale banner appears the moment the user first trims THIS meeting.
        processedTrimKey: ":",
        createdAt: Date.now(),
        systemCaptured: false,
        micPath: importPath,
        systemPath: null,
      };
      setMeetings((prev) => [m, ...prev]);
      setActiveId(m.id);
      setImportPath(null);
      toast.success("Imported audio transcribed.");
    } catch (e) {
      toast.error(`Import failed: ${String(e)}`);
    } finally {
      setImportBusy(false);
      setTranscribeProgress(null); // v1.19.0: clear the bar when done
    }
  };

  // ---- copy / export / delete -------------------------------------------
  const copyActive = async () => {
    if (!active) return;
    // Kōrero (v1.27.0): copy the WINDOW, not the record. `processed` is
    // appended verbatim because it is materialised text — it cannot be
    // filtered here; the stale-notes banner above it carries that warning
    // instead, and copy is deliberately not blocked by it.
    const body = combine(
      active.you,
      active.others,
      active.youLabel,
      active.othersLabel,
      visibleSegs(active),
    );
    const text = active.processed.trim()
      ? `${body}\n\n--- Processed ---\n${active.processed.trim()}`
      : body;
    try {
      await writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error("Could not copy to clipboard.");
    }
  };

  // v1.21.0: render a spoken audio brief of the processed notes via the local
  // Qwen3-TTS engine (Rust shells out to audio_brief.py). GPU-bound and slow;
  // no client timeout — the engine must not be interrupted.
  const genAudioBrief = async () => {
    if (!active || briefBusy) return;
    const text = active.processed.trim();
    if (!text) {
      toast.message("Generate notes first.");
      return;
    }
    setBriefBusy(true);
    setBriefUrl(null);
    try {
      const r = await commands.meetingGenerateAudioBrief(text, null, null, null);
      if (r.status !== "ok") throw new Error(r.error);
      setBriefUrl(convertFileSrc(r.data, "asset"));
      setBriefPath(r.data);
      toast.success("Audio brief ready.");
    } catch (e) {
      toast.error(`Audio brief failed: ${String(e)}`);
    } finally {
      setBriefBusy(false);
    }
  };

  const copyProcessed = async () => {
    if (!active?.processed.trim()) return;
    try {
      await writeText(active.processed.trim());
      toast.success("Processed notes copied (markdown).");
    } catch {
      toast.error("Could not copy to clipboard.");
    }
  };

  // v1.22.0: save manual edits to the processed notes.
  const saveNotes = () => {
    if (!active) return;
    patchMeeting(active.id, { processed: notesDraft });
    setEditingNotes(false);
    toast.success("Notes updated.");
  };

  // v1.22.0: refine the processed notes with free-text feedback to the model
  // (constrained to revise, not rewrite or invent). Undo restores the previous.
  const refineNotes = async () => {
    if (!active || refining) return;
    const fb = feedback.trim();
    if (!fb) {
      toast.message("Tell the AI what to improve.");
      return;
    }
    const current = active.processed.trim();
    if (!current) {
      toast.message("Generate notes first.");
      return;
    }
    setRefining(true);
    const prev = active.processed;
    const id = active.id;
    try {
      const prompt =
        "You are revising EXISTING meeting notes based on the reader's feedback. " +
        "Apply the feedback faithfully, keep the same Markdown structure and headings where still appropriate, " +
        "do not invent facts or add content not supported by the notes, and output ONLY the revised notes " +
        'with no preamble or commentary. Feedback: "' +
        fb +
        '".';
      const r = await commands.meetingPostProcess(current, prompt);
      if (r.status !== "ok") throw new Error(r.error);
      patchMeeting(id, { processed: r.data });
      setFeedback("");
      toast.success("Notes refined.", {
        action: {
          label: "Undo",
          onClick: () => patchMeeting(id, { processed: prev }),
        },
      });
    } catch (e) {
      toast.error(`Refine failed: ${String(e)}`);
    } finally {
      setRefining(false);
    }
  };

  // v1.22.0: save an inline edit to a single transcript segment.
  const saveSegment = (idx: number, text: string) => {
    if (!active) return;
    const before = (active.transcript ?? [])[idx]?.text ?? "";
    const next = (active.transcript ?? []).map((s, i) =>
      i === idx ? { ...s, text } : s,
    );
    patchMeeting(active.id, { transcript: next });
    setEditingSegIdx(null);
    // v1.22.0 (N2): if the edit was a clean single-word fix, offer to TEACH it
    // as a correction so it's fixed everywhere and biases future transcriptions.
    suggestCorrectionFromEdit(before, text);
  };

  // Detect a clean one-word substitution (same word count, exactly one differing
  // token) and OFFER to teach it. Deliberately conservative — skips rewrites,
  // insertions and deletions so it never suggests noise, and only ever suggests
  // (the user confirms with one click). Reuses the taught-corrections store, so
  // an accepted suggestion both fixes the word everywhere and, on Whisper models,
  // biases future decoding toward the right spelling.
  const suggestCorrectionFromEdit = (beforeText: string, afterText: string) => {
    const a = beforeText.trim().split(/\s+/).filter(Boolean);
    const b = afterText.trim().split(/\s+/).filter(Boolean);
    if (a.length === 0 || a.length !== b.length) return;
    const changed: number[] = [];
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) changed.push(i);
    if (changed.length !== 1) return; // only a single-word change
    const strip = (w: string) =>
      w.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, ""); // keep macrons, drop punctuation
    const wrong = strip(a[changed[0]]);
    const right = strip(b[changed[0]]);
    if (wrong.length < 2 || !right) return;
    if (wrong.toLowerCase() === right.toLowerCase()) return;
    const existing = settings?.transcript_corrections ?? [];
    if (existing.some((c) => c.wrong.toLowerCase() === wrong.toLowerCase()))
      return;
    toast(`Teach "${wrong}" → "${right}"?`, {
      description: "Fixes it everywhere and sharpens future transcriptions.",
      action: {
        label: "Teach",
        onClick: () => {
          updateSetting("transcript_corrections", [
            ...existing,
            { wrong, right },
          ]);
          toast.success(`Teaching "${wrong}" → "${right}".`);
        },
      },
    });
  };

  const exportActive = async () => {
    if (!active) return;
    const parts = [
      `# ${titleOf(active)}`,
      "",
      // Kōrero (v1.27.0): export the WINDOW. An exported file leaves the app
      // and gets shared, so this is the consumer where honouring the trim
      // matters most.
      combine(
        active.you,
        active.others,
        active.youLabel,
        active.othersLabel,
        visibleSegs(active),
      ) || "(no transcript)",
    ];
    // Kōrero (v1.27.0): record the window in the exported file so a reader
    // knows they are holding an excerpt rather than the whole meeting.
    if (isTrimmed(active)) {
      parts.push(
        "",
        `_Trimmed excerpt: ${
          active.trimStartMs != null ? fmtTrimMs(active.trimStartMs) : "start"
        } – ${
          active.trimEndMs != null ? fmtTrimMs(active.trimEndMs) : "end"
        } · ${hiddenCount(active)} segment(s) hidden._`,
      );
    }
    if (active.processed.trim()) {
      parts.push("", "## Processed", "", active.processed.trim());
      // Notes are materialised text — flag it rather than pretend the trim
      // applied to them.
      if (notesAreStale(active)) {
        parts.push(
          "",
          "_These notes were generated before the current trim — re-process to update them._",
        );
      }
    }
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const base = (active.title.trim() || "meeting").replace(/[\\/:*?"<>|]/g, "_");
    // v1.24.0 (paths, decision D2): Save-As each time, seeded with the
    // remembered export folder (Documents by default). Cancelling the dialog
    // cancels the export; the backend remembers the chosen folder afterwards.
    try {
      let seed = "";
      try {
        const info = await commands.meetingDirsInfo();
        if (info.status === "ok") seed = JSON.parse(info.data).exportSeedDir ?? "";
      } catch {
        /* seeding is best-effort — the dialog still opens */
      }
      const chosen = await saveFileDialog({
        title: "Export meeting",
        defaultPath: seed ? `${seed}\\${base}-${stamp}.md` : `${base}-${stamp}.md`,
        filters: [
          { name: "Markdown", extensions: ["md"] },
          { name: "Plain text", extensions: ["txt"] },
        ],
      });
      if (!chosen) return; // user cancelled
      const res = await commands.meetingExportTranscriptTo(chosen, parts.join("\n"));
      if (res.status === "ok") {
        setExportedPath(res.data);
        toast.success("Exported.");
      } else toast.error(`Export failed: ${res.error}`);
    } catch (e) {
      toast.error(`Export failed: ${String(e)}`);
    }
  };

  // ---- v1.24.0 (paths): storage folder handlers -------------------------

  const loadDirs = async () => {
    try {
      const r = await commands.meetingDirsInfo();
      if (r.status === "ok") setDirs(JSON.parse(r.data));
    } catch {
      /* non-fatal — the Storage card just shows a loading state */
    }
  };

  useEffect(() => {
    void loadDirs();
  }, []);

  const changeRecordingDir = async () => {
    const picked = await openFileDialog({
      directory: true,
      title: "Choose a folder for meeting recordings",
    });
    if (!picked || Array.isArray(picked)) return;
    setDirBusy(true);
    try {
      const r = await commands.setMeetingRecordingDir(picked);
      if (r.status !== "ok") throw new Error(r.error);
      await loadDirs();
      await loadRecordings();
      toast.success("New recordings will be saved there.", {
        description:
          "Existing recordings stay where they are (still listed below) — use Move existing to relocate them.",
      });
    } catch (e) {
      toast.error(
        `Couldn't use that folder: ${e instanceof Error ? e.message : String(e)}`,
      );
    } finally {
      setDirBusy(false);
    }
  };

  const resetRecordingDir = async () => {
    setDirBusy(true);
    try {
      const r = await commands.setMeetingRecordingDir(null);
      if (r.status !== "ok") throw new Error(r.error);
      await loadDirs();
      await loadRecordings();
      toast.success("Recording folder reset to the default.");
    } catch (e) {
      toast.error(`Reset failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setDirBusy(false);
    }
  };

  const moveExistingRecordings = async () => {
    setDirBusy(true);
    try {
      const r = await commands.meetingMoveRecordings();
      if (r.status !== "ok") throw new Error(r.error);
      const rep = JSON.parse(r.data) as {
        moved: Record<string, string>;
        failed: number;
        errors: string[];
      };
      const map = rep.moved ?? {};
      const movedCount = Object.keys(map).length;
      if (movedCount > 0) {
        // R8: rewrite stored file references through the normal state->autosave
        // path (the store is frontend-owned — a disk-side rewrite would be
        // clobbered by the next in-memory save).
        const next = meetings.map((m) => ({
          ...m,
          micPath: m.micPath && map[m.micPath] ? map[m.micPath] : m.micPath,
          systemPath:
            m.systemPath && map[m.systemPath]
              ? map[m.systemPath]
              : m.systemPath,
        }));
        setMeetings(next);
        // Peer-review M2: the files have ALREADY moved on disk — persist the
        // rewritten paths immediately rather than waiting for the autosave
        // effect, so a crash in that window can't leave the store pointing at
        // the emptied default folder. The autosave writing again is harmless
        // (atomic temp+rename).
        try {
          await commands.meetingsStoreSave(JSON.stringify(next));
        } catch {
          /* autosave effect remains the fallback */
        }
      }
      await loadRecordings();
      if (rep.failed > 0) {
        toast.warning(`Moved ${movedCount}, ${rep.failed} skipped.`, {
          description: rep.errors.slice(0, 3).join(" · "),
        });
      } else {
        toast.success(
          movedCount > 0
            ? `Moved ${movedCount} recording(s) to the new folder.`
            : "Nothing to move — the default folder has no meeting recordings.",
        );
      }
    } catch (e) {
      toast.error(`Move failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setDirBusy(false);
    }
  };

  const deleteMeeting = (id: string) => {
    // v1.25.0 (UX batch): confirm first — this removes the transcript, notes
    // AND both WAVs in one click (audit #2, the sharpest data-loss edge).
    const gone = meetings.find((m) => m.id === id);
    confirmDestructive(
      `Delete "${gone ? titleOf(gone) : "this meeting"}"?`,
      "Transcript, notes and the meeting audio are removed permanently.",
      "Delete",
      () => {
        // Review fix (v1.25.0 #2): the confirm toast outlives this panel — if
        // the user switched sections before confirming, setMeetings would be a
        // silent no-op while the WAVs still got deleted (metadata orphaned,
        // pointing at missing audio). Refuse instead.
        if (!mountedRef.current) {
          toast.message("Meetings view was closed — open Meetings and delete again.");
          return;
        }
        // v1.13.6: free the disk too — once the metadata is gone the WAVs are
        // only reachable via the recovery list, which is rarely what's wanted.
        [gone?.micPath, gone?.systemPath].forEach((p) => {
          if (p) commands.meetingDeleteRecording(p).catch(() => {});
        });
        setMeetings((prev) => {
          const next = prev.filter((m) => m.id !== id);
          // Review fix (v1.25.0 #3): activeId may have changed during the
          // toast — read the live value, not the click-time closure.
          if (id === activeIdRef.current) setActiveId(next[0]?.id ?? null);
          return next;
        });
      },
    );
  };

  const transcribeRecording = async (file: RecordingFile) => {
    setBusyFile(file.path);
    try {
      const res = await commands.meetingTranscribeFile(file.path);
      if (res.status === "ok") {
        const isOthers = /others|system/i.test(file.file_name);
        const m: Meeting = {
          id: newId(),
          title: `Recovered · ${file.file_name}`,
          you: isOthers ? "" : res.data,
          others: isOthers ? res.data : "",
          youLabel: "You",
          othersLabel: "Others",
          imported: true,
          processed: "",
          processPrompt: "",
          createdAt: file.modified ? file.modified * 1000 : Date.now(),
          systemCaptured: isOthers,
          micPath: isOthers ? null : file.path,
          systemPath: isOthers ? file.path : null,
        };
        setMeetings((prev) => [m, ...prev]);
        setActiveId(m.id);
        toast.success("Recording transcribed and added to your meetings.");
      } else {
        toast.error(`Transcription failed: ${res.error}`);
      }
    } catch (e) {
      toast.error(`Transcription failed: ${String(e)}`);
    } finally {
      setBusyFile(null);
    }
  };

  const modelOptions: DropdownOption[] = (models ?? []).map((m) => ({
    value: m.id,
    label: m.name,
  }));
  const activeHasAudio = !!active && (!!active.micPath || !!active.systemPath);
  const activeHasTranscript =
    !!active && (!!active.you.trim() || !!active.others.trim());

  return (
    <div className="max-w-4xl w-full mx-auto space-y-4">
      <div className="px-1">
        <h1 className="text-lg font-semibold text-text">Meetings</h1>
        <p className="text-sm text-text-subtle mt-1">
          Record both sides of a call — your mic (You) and system audio (Others) —
          or import an audio file. Audio is saved to disk first, so a recording is
          never lost. Everything stays on your machine.
        </p>
      </div>

      {/* v1.29.0 (R-02): the store could not be read. This is deliberately
          NOT a toast — a toast disappears and the user carries on believing
          their meetings are gone. While this banner is up, storeReady is
          false and nothing is written to disk, so the file on disk is exactly
          as it was. Reloading is safe; saving is not. */}
      {storeLoadError && (
        <div
          role="alert"
          className="glass-card p-4 flex items-start gap-3 border border-pill-urgent/40"
        >
          <TriangleAlert size={18} className="text-pill-urgent shrink-0 mt-0.5" />
          <div className="flex-1 min-w-0">
            <p className="text-sm font-semibold text-text">
              Couldn&rsquo;t read your saved meetings
            </p>
            <p className="text-sm text-text-subtle mt-1">
              Your meetings are still on disk — nothing has been deleted, and
              Kōrero will not overwrite the file while this message is showing.
              This is usually a file briefly locked by antivirus or cloud sync.
              Close and reopen Kōrero to try again.
            </p>
            <p className="text-xs text-text-subtle/80 mt-2 font-mono break-all">
              {storeLoadError}
            </p>
          </div>
        </div>
      )}

      {/* v1.13.3: persistent warning if the capture worker reported a
          disk-write failure — the toast is transient, this is not. */}
      {captureError && (
        <div className="glass-card p-3 flex items-start gap-2 border border-pill-urgent/40">
          <TriangleAlert size={16} className="text-pill-urgent shrink-0 mt-0.5" />
          <p className="text-sm text-text flex-1">{captureError}</p>
          <button
            onClick={() => setCaptureError(null)}
            className="text-text-subtle hover:text-text"
            aria-label="Dismiss"
          >
            <X size={14} />
          </button>
        </div>
      )}

      {/* Record control + transcription model + import */}
      <div className="glass-card p-4 flex flex-wrap items-center gap-3">
        <Button
          variant={recording ? "danger" : "primary"}
          size="md"
          onClick={toggleRecord}
          disabled={recProcessing}
          className="flex items-center gap-2"
        >
          {recProcessing ? (
            <>
              <Loader2 size={16} className="animate-spin" /> Transcribing…
            </>
          ) : recording ? (
            <>
              <Square size={15} /> Stop · {fmtClock(elapsed)}
            </>
          ) : (
            <>
              <Circle size={15} /> Record meeting
            </>
          )}
        </Button>

        {/* v1.19.0: pause / resume — only while a meeting is live. */}
        {recording && !recProcessing && (
          <Button
            variant="secondary"
            size="md"
            onClick={togglePause}
            className="flex items-center gap-2"
            title={
              paused
                ? "Resume capturing — audio while paused is not recorded"
                : "Pause capturing — your mic stays acquired (indicator stays on) but no audio is recorded"
            }
          >
            {paused ? (
              <>
                <Play size={15} /> Resume
              </>
            ) : (
              <>
                <Pause size={15} /> Pause
              </>
            )}
          </Button>
        )}

        <Button
          variant="secondary"
          size="md"
          onClick={runTest}
          disabled={recording || recProcessing || testing}
          className="flex items-center gap-1.5"
          title="Check both capture devices work — same capture path as a real meeting; test recordings are discarded"
        >
          {testing ? (
            <>
              <Loader2 size={15} className="animate-spin" /> Testing…
            </>
          ) : (
            <>
              <Activity size={15} /> Test audio
            </>
          )}
        </Button>

        <Button
          variant="secondary"
          size="md"
          onClick={pickImportFile}
          disabled={recording || recProcessing}
          className="flex items-center gap-1.5"
          title="Import an audio file (WAV, M4A, MP3, AAC, ALAC, FLAC, OGG, CAF, AIFF) to transcribe and process"
        >
          <Upload size={15} /> Import audio
        </Button>

        <div className="flex items-center gap-2">
          <span className="text-xs text-text-subtle">Model</span>
          <Dropdown
            options={modelOptions}
            selectedValue={currentModel}
            onSelect={changeModel}
            disabled={!models || models.length === 0 || recording}
          />
        </div>

        {recording && !paused && (
          <span className="flex items-center gap-2 text-sm text-text-muted">
            <span className="inline-block w-2 h-2 rounded-full bg-pill-urgent animate-pulse" />
            {systemCaptured === false ? "Recording (mic only)" : "Recording you + others"}
          </span>
        )}
        {recording && paused && (
          <span className="flex items-center gap-2 text-sm font-medium text-pill-warning">
            <span className="inline-block w-2 h-2 rounded-full bg-pill-warning" />
            Paused — not recording (mic still on)
          </span>
        )}
        {!recording && !recProcessing && !importPath && (
          <span className="text-xs text-text-subtle">
            Tip: use a headset for a clean You/Others split — with speakers,
            your mic also hears the other side, so their words bleed into You.
          </span>
        )}
      </div>

      {/* v1.13.5 meters, isolated into a memoised child (v1.14.0 item 6). */}
      <CaptureMeters
        recording={recording}
        testing={testing}
        elapsed={elapsed}
        devices={devices}
      />

      {/* Phase B (v1.14.0): live transcript while the meeting records, plus
          Phase C — ask the post-processing model about the meeting so far. */}
      {(recording || recProcessing) && liveSegments.length > 0 && (
        <div className="surface-content p-4 space-y-2">
          <h3 className="text-xs font-semibold text-text-muted uppercase tracking-wider">
            Live transcript
          </h3>
          <div className="max-h-48 overflow-y-auto space-y-1 text-sm pr-1">
            {liveSegments.map((s, i) => (
              <p key={i}>
                <span
                  className={
                    s.source === "you" ? "text-aurora-cyan" : "text-text-muted"
                  }
                >
                  {s.source === "you" ? "You" : "Others"}:
                </span>{" "}
                <span className="text-text">{s.text}</span>
              </p>
            ))}
          </div>
          <div className="flex items-center gap-2 pt-1">
            <input
              value={liveQuestion}
              onChange={(e) => setLiveQuestion(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") askLive();
              }}
              placeholder="Ask about the meeting so far…"
              className="flex-1 bg-white/5 border border-white/10 rounded-md px-2 py-1.5 text-sm text-text placeholder:text-text-subtle focus:outline-none"
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={askLive}
              disabled={liveAsking || !liveQuestion.trim()}
              className="flex items-center gap-1.5 shrink-0"
            >
              {liveAsking ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <Sparkles size={14} />
              )}
              Ask
            </Button>
          </div>
          {liveAnswer && (
            <div className="surface-content md-body">
              <Markdown>{liveAnswer}</Markdown>
            </div>
          )}
        </div>
      )}

      {/* Import workflow card */}
      {importPath && (
        <div className="surface-content p-4 space-y-3">
          <div className="flex items-center justify-between gap-2">
            <span className="flex items-center gap-2 text-sm text-text min-w-0">
              <FileAudio size={16} className="text-aurora-cyan shrink-0" />
              <span className="truncate">{baseName(importPath)}</span>
            </span>
            <button
              type="button"
              onClick={() => setImportPath(null)}
              title="Cancel import"
              className="text-text-subtle hover:text-text"
            >
              <X size={16} />
            </button>
          </div>
          <p className="text-xs text-text-subtle">
            Transcribed with the <strong className="text-text">Model</strong> selected
            above. Edit the post-processing prompt, then choose an action.
          </p>
          {/* v1.19.0: saved-prompt picker + Save-as-new for imports. */}
          <div className="flex items-center gap-2">
            <span className="text-xs text-text-subtle shrink-0">Prompt</span>
            <Dropdown
              options={promptOptions}
              selectedValue={importPromptId}
              onSelect={(id) => {
                setImportPromptId(id);
                if (id !== "custom") setImportPrompt(savedPromptText(id));
              }}
              disabled={importBusy}
            />
            <button
              type="button"
              onClick={async () => {
                const id = await savePromptAsNew(importPrompt);
                if (id) setImportPromptId(id);
              }}
              disabled={importBusy}
              className="flex items-center gap-1 text-xs text-text-subtle hover:text-text disabled:opacity-50"
              title="Save the current prompt as a new reusable prompt"
            >
              <Plus size={13} /> Save as new
            </button>
          </div>
          <textarea
            value={importPrompt}
            onChange={(e) => {
              setImportPrompt(e.target.value);
              setImportPromptId("custom");
            }}
            rows={2}
            placeholder="Post-processing prompt"
            className="w-full resize-y bg-glass-surface-thin rounded-lg px-3 py-2 text-sm text-text placeholder:text-text-subtle focus:outline-none border border-glass-border"
          />
          <div className="flex flex-wrap gap-2">
            <Button
              variant="primary"
              size="md"
              onClick={() => runImport(true)}
              disabled={importBusy}
              className="flex items-center gap-1.5"
            >
              {importBusy ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <Wand2 size={14} />
              )}
              Transcribe + process
            </Button>
            <Button
              variant="secondary"
              size="md"
              onClick={() => runImport(false)}
              disabled={importBusy}
              className="flex items-center gap-1.5"
            >
              {importBusy ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <RotateCcw size={14} />
              )}
              Transcribe only
            </Button>
          </div>
          {/* v1.19.0: chunked-transcription progress. Determinate bar when the
              window total is known (WAV); indeterminate otherwise (compressed). */}
          {importBusy && transcribeProgress && (
            <div className="space-y-1">
              <div className="flex items-center justify-between text-xs text-text-subtle">
                <span>Transcribing audio…</span>
                <span>
                  {transcribeProgress.total
                    ? `part ${transcribeProgress.window} of ${transcribeProgress.total}`
                    : `part ${transcribeProgress.window}`}
                </span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-glass-surface-thin">
                {transcribeProgress.total ? (
                  <div
                    className="h-full rounded-full bg-aurora-cyan transition-all duration-300"
                    style={{
                      width: `${Math.min(
                        100,
                        Math.round(
                          (transcribeProgress.window / transcribeProgress.total) * 100,
                        ),
                      )}%`,
                    }}
                  />
                ) : (
                  <div className="h-full w-1/3 animate-pulse rounded-full bg-aurora-cyan" />
                )}
              </div>
            </div>
          )}
        </div>
      )}

      <div className="flex gap-4">
        {/* Meeting list */}
        <div className="w-56 shrink-0 flex flex-col gap-2">
          {/* v1.22.0: search across meetings (title + transcript + notes). */}
          <div className="relative">
            <Search
              size={13}
              className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-text-subtle"
            />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search meetings…"
              aria-label="Search meetings"
              className="w-full rounded-lg border border-glass-border bg-glass-surface-thin py-1.5 pl-7 pr-2 text-xs text-text placeholder:text-text-subtle focus:outline-none"
            />
          </div>
          <div className="glass-card flex max-h-[55vh] flex-col gap-1 overflow-y-auto p-2">
          {filteredMeetings.length === 0 ? (
            <div className="px-3 py-6 flex flex-col items-center gap-2 text-center">
              <Users size={20} className="text-text-subtle" />
              <p className="text-xs text-text-subtle">
                {meetings.length === 0
                  ? "No meetings yet. Press Record to capture one."
                  : `No meetings match "${search.trim()}".`}
              </p>
            </div>
          ) : (
            filteredMeetings.map((m) => {
              const isActive = m.id === active?.id;
              const isEditing = editingListId === m.id;
              return (
                <div
                  key={m.id}
                  onClick={() => !isEditing && setActiveId(m.id)}
                  className={`group rounded-lg px-3 py-2 cursor-pointer transition-colors ${
                    isActive ? "bg-glass-accent-strong" : "hover:bg-white/5"
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    {isEditing ? (
                      <input
                        autoFocus
                        value={m.title}
                        onChange={(e) => patchMeeting(m.id, { title: e.target.value })}
                        onClick={(e) => e.stopPropagation()}
                        onBlur={() => setEditingListId(null)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" || e.key === "Escape")
                            setEditingListId(null);
                        }}
                        placeholder="Meeting name"
                        className="flex-1 min-w-0 bg-transparent text-sm text-text border-b border-aurora-cyan focus:outline-none"
                      />
                    ) : (
                      <span className="text-sm text-text truncate">{titleOf(m)}</span>
                    )}
                    <div className="flex items-center gap-1 shrink-0">
                      <button
                        type="button"
                        title="Rename meeting"
                        onClick={(e) => {
                          e.stopPropagation();
                          setEditingListId(m.id);
                        }}
                        className="opacity-0 group-hover:opacity-100 text-text-subtle hover:text-aurora-cyan transition-opacity"
                      >
                        <Pencil size={13} />
                      </button>
                      <button
                        type="button"
                        title="Delete meeting"
                        onClick={(e) => {
                          e.stopPropagation();
                          deleteMeeting(m.id);
                        }}
                        className="opacity-0 group-hover:opacity-100 text-text-subtle hover:text-pill-urgent transition-opacity"
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                </div>
              );
            })
          )}
          </div>
        </div>

        {/* Detail */}
        <div className="flex-1 min-w-0 surface-content p-4">
          {!active ? (
            <div className="py-16 flex flex-col items-center gap-2 text-center">
              <Plus size={22} className="text-text-subtle" />
              <p className="text-sm text-text-muted">
                Record or import a meeting to see its transcript here.
              </p>
            </div>
          ) : (
            <div className="space-y-4">
              {/* Rename + copy/export */}
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-1.5 flex-1 min-w-0">
                  <button
                    type="button"
                    title="Rename this meeting"
                    onClick={() => titleRef.current?.focus()}
                    className="text-text-subtle hover:text-aurora-cyan transition-colors shrink-0"
                  >
                    <Pencil size={13} />
                  </button>
                  <input
                    ref={titleRef}
                    value={active.title}
                    onChange={(e) => patchMeeting(active.id, { title: e.target.value })}
                    placeholder={`Meeting · ${new Date(
                      active.createdAt,
                    ).toLocaleString()}`}
                    className="flex-1 min-w-0 bg-transparent text-sm font-medium text-text placeholder:text-text-subtle focus:outline-none border-b border-transparent hover:border-glass-border focus:border-aurora-cyan transition-colors"
                  />
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={exportActive}
                    className="flex items-center gap-1.5"
                    title="Export transcript + processed notes to a file"
                  >
                    <Download size={14} /> Export
                  </Button>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={copyActive}
                    className="flex items-center gap-1.5"
                  >
                    {copied ? <Check size={14} /> : <Copy size={14} />}
                    {copied ? "Copied" : "Copy"}
                  </Button>
                </div>
              </div>

              {/* v1.22.0: export result — a selectable path + "Show in folder" +
                  "Copy", so the file location is both copy/pasteable AND openable
                  (was an ephemeral, non-selectable "Exported to <path>" toast). */}
              {exportedPath && (
                <div className="flex items-center gap-2 rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-2 text-xs text-text-muted">
                  <Check size={13} className="shrink-0 text-pill-positive" />
                  <span className="shrink-0">Exported to</span>
                  <code
                    className="min-w-0 flex-1 select-all truncate font-mono text-text"
                    title={exportedPath}
                  >
                    {exportedPath}
                  </code>
                  <button
                    type="button"
                    onClick={() =>
                      revealItemInDir(exportedPath).catch(() =>
                        toast.error("Could not open the folder."),
                      )
                    }
                    className="inline-flex shrink-0 items-center gap-1 transition-colors hover:text-aurora-cyan"
                    title="Open the containing folder"
                  >
                    <FolderOpen size={13} /> Show in folder
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      writeText(exportedPath)
                        .then(() => toast.success("Path copied."))
                        .catch(() => toast.error("Could not copy the path."))
                    }
                    className="inline-flex shrink-0 items-center gap-1 transition-colors hover:text-aurora-cyan"
                    title="Copy the file path"
                  >
                    <Copy size={13} /> Copy
                  </button>
                </div>
              )}

              {/* v1.17.0: only meaningful for RECORDED meetings — an import
                  always has exactly one audio source, so warning about a
                  missing second one was noise. */}
              {!active.systemCaptured && !active.imported && (
                <p className="flex items-center gap-1.5 text-xs text-pill-warning">
                  <TriangleAlert size={13} /> System audio was not captured for this
                  meeting — only your mic was recorded.
                </p>
              )}

              {/* v1.15.0: teach the transcriber — select a mis-heard word in
                  the transcript below, then click Teach. */}
              {teachWrong === null ? (
                <button
                  type="button"
                  onClick={() =>
                    setTeachWrong(
                      (window.getSelection()?.toString() ?? "")
                        .trim()
                        .slice(0, 80),
                    )
                  }
                  className="self-start text-xs text-text-subtle hover:text-aurora-cyan transition-colors flex items-center gap-1.5"
                  title="Select a mis-transcribed word below first, then click to teach the correction"
                >
                  <GraduationCap size={13} /> Teach a correction
                </button>
              ) : (
                <AddCorrectionInline
                  initialWrong={teachWrong}
                  onDone={() => setTeachWrong(null)}
                />
              )}

              {/* Kōrero (v1.27.0): trim status bar. Shown ONLY when a window is
                  set, so an untrimmed meeting carries no extra chrome — the
                  entry point is the per-segment "Start here" / "End here"
                  buttons below. Everything here is reversible: the bar's own
                  headline button clears the window in one click. */}
              {isTrimmed(active) && (
                <div className="glass-card-thin space-y-2 border-l-2 border-aurora-cyan/50 px-3 py-2">
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <button
                      type="button"
                      onClick={clearTrim}
                      aria-label={`Clear trim, ${hiddenCount(active)} segments hidden`}
                      title="Clear the trim and show the whole meeting again"
                      className="inline-flex items-center gap-1.5 rounded-md bg-aurora-cyan/15 px-2.5 py-1 text-xs font-medium text-aurora-cyan transition-colors hover:bg-aurora-cyan/25"
                    >
                      <Scissors size={12} />
                      Trimmed · {hiddenCount(active)} hidden
                      <X size={12} />
                    </button>
                    <span className="font-mono text-xs text-text-muted">
                      {active.trimStartMs != null
                        ? fmtTrimMs(active.trimStartMs)
                        : "0:00.0"}{" "}
                      –{" "}
                      {/* An unset end is an em dash, not "0:00" — the window
                          runs to the end of the meeting. */}
                      {active.trimEndMs != null
                        ? fmtTrimMs(active.trimEndMs)
                        : "—"}
                    </span>
                  </div>
                  <div className="flex flex-wrap items-end gap-3">
                    <div className="flex flex-col gap-0.5">
                      <label
                        htmlFor="korero-trim-in"
                        className="text-[11px] uppercase tracking-wider text-text-subtle"
                      >
                        Start (mm:ss.s)
                      </label>
                      <input
                        id="korero-trim-in"
                        type="text"
                        inputMode="decimal"
                        value={trimInDraft}
                        placeholder="0:00.0"
                        onChange={(e) => setTrimInDraft(e.target.value)}
                        onBlur={commitTrimInputs}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") commitTrimInputs();
                        }}
                        aria-invalid={trimError !== null}
                        className="w-24 rounded border border-glass-border bg-glass-surface-thin px-2 py-1 font-mono text-xs text-text focus:outline-none"
                      />
                    </div>
                    <div className="flex flex-col gap-0.5">
                      <label
                        htmlFor="korero-trim-out"
                        className="text-[11px] uppercase tracking-wider text-text-subtle"
                      >
                        End (mm:ss.s)
                      </label>
                      <input
                        id="korero-trim-out"
                        type="text"
                        inputMode="decimal"
                        value={trimOutDraft}
                        placeholder="end"
                        onChange={(e) => setTrimOutDraft(e.target.value)}
                        onBlur={commitTrimInputs}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") commitTrimInputs();
                        }}
                        aria-invalid={trimError !== null}
                        className="w-24 rounded border border-glass-border bg-glass-surface-thin px-2 py-1 font-mono text-xs text-text focus:outline-none"
                      />
                    </div>
                  </div>
                  {trimError && (
                    <p role="alert" className="text-xs text-pill-urgent">
                      {trimError}
                    </p>
                  )}
                  <p className="text-xs text-text-subtle">
                    Trimming hides segments from copy, export, search and
                    post-processing. The audio file is not changed and this is
                    fully reversible.
                  </p>
                </div>
              )}

              {/* Transcript — v1.14.5: speaker tags are editable (pencil), so
                  "Others" can become "Gerard" etc. Labels persist with the
                  meeting and flow into copy/export/post-processing.
                  v1.22.0: the transcript scrolls within its OWN box (max-height
                  + overflow-y) so a long transcript no longer scrolls the whole
                  app; pr-2 keeps the text clear of the scrollbar. */}
              <div className="max-h-[46vh] space-y-4 overflow-y-auto overflow-x-hidden pr-2">
              {(
                [
                  {
                    key: "you" as const,
                    label: active.youLabel,
                    text: active.you,
                    colour: "text-aurora-cyan",
                    fallback: "You",
                  },
                  {
                    key: "others" as const,
                    label: active.othersLabel,
                    text: active.others,
                    colour: "text-aurora-purple",
                    fallback: "Others",
                  },
                ]
              ).map((row) => (
                <div key={row.key} className="space-y-1">
                  <div className="flex items-center gap-1.5">
                    {editingLabel === row.key ? (
                      <input
                        autoFocus
                        defaultValue={row.label}
                        onBlur={(e) => {
                          const v = e.target.value.trim() || row.fallback;
                          setMeetings((prev) =>
                            prev.map((m) =>
                              m.id === active.id
                                ? row.key === "you"
                                  ? { ...m, youLabel: v }
                                  : { ...m, othersLabel: v }
                                : m,
                            ),
                          );
                          setEditingLabel(null);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" || e.key === "Escape") {
                            (e.target as HTMLInputElement).blur();
                          }
                        }}
                        className={`bg-white/5 border border-white/10 rounded px-1.5 py-0.5 text-xs font-semibold uppercase tracking-wider focus:outline-none w-44 ${row.colour}`}
                      />
                    ) : (
                      <>
                        <h3
                          className={`text-xs font-semibold uppercase tracking-wider ${row.colour}`}
                        >
                          {row.label}
                        </h3>
                        <button
                          type="button"
                          title={`Rename this speaker (currently "${row.label}")`}
                          onClick={() => setEditingLabel(row.key)}
                          className="opacity-50 hover:opacity-100 text-text-subtle hover:text-text transition-opacity"
                        >
                          <Pencil size={11} />
                        </button>
                      </>
                    )}
                  </div>
                  {/* v1.17.0: when an ordered transcript exists it's rendered
                      interleaved below, so the per-speaker block body is
                      suppressed (the header + rename pencil stay). The two-block
                      body only shows for older meetings / single-file imports. */}
                  {!(active.transcript && active.transcript.length > 0) && (
                    <p className="transcript-body whitespace-pre-wrap">
                      {row.text.trim() || "—"}
                    </p>
                  )}
                </div>
              ))}

              {/* v1.17.0: chronological, interleaved transcript — both speakers
                  in the order they actually spoke. */}
              {active.transcript && active.transcript.length > 0 && (
                <div className="space-y-0.5">
                  {/* Kōrero (v1.27.0): map the UNFILTERED transcript. `i` is
                      the index `saveSegment` writes back to, so a `.filter()`
                      before this `.map()` would renumber the rows and make an
                      inline edit land on the wrong segment. Excluded rows are
                      marked with a class instead — they stay in the DOM,
                      readable and in the tab order, which is also the point:
                      you must be able to see what you have hidden. */}
                  {active.transcript.map((s, i) => {
                    if (!s.text.trim() && editingSegIdx !== i) return null;
                    const label =
                      s.source === "you" ? active.youLabel : active.othersLabel;
                    const excluded = !segInWindow(active, s);
                    return (
                      <div
                        key={i}
                        // Kōrero (v1.27.0): 3-column grid — speaker gutter,
                        // text, actions. The gutter gives the speaker a stable
                        // POSITION rather than only a hue (colour alone fails
                        // for colour-blind readers), and the fixed actions
                        // column means revealing the buttons no longer reflows
                        // the line under the pointer.
                        className={`group grid grid-cols-[7rem_minmax(0,1fr)_auto] items-start gap-x-2 rounded border-l-2 py-0.5 pl-2 text-sm leading-relaxed ${
                          excluded
                            ? "border-text-subtle/40 opacity-40"
                            : "border-transparent"
                        }`}
                        // Excluded rows are dimmed, NOT hidden — they are still
                        // exposed to assistive tech, because "what did I cut?"
                        // is the question the trim UI has to keep answerable.
                        aria-hidden="false"
                        title={
                          excluded
                            ? "Outside the current trim — hidden from copy, export, search and post-processing, but still on the record."
                            : undefined
                        }
                      >
                        <span
                          // v1.31.0: the gutter DEMOTES so the transcript can
                          // lead. It keeps its hue and its weight -- position
                          // and weight still carry the speaker without colour
                          // alone -- but it stops competing with the words on
                          // size. The transcript is the product; the label is
                          // metadata about it.
                          className={`truncate font-semibold transcript-meta ${
                            s.source === "you"
                              ? "text-aurora-cyan"
                              : "text-aurora-purple"
                          }`}
                        >
                          {label}:
                        </span>
                        <div className="min-w-0 transcript-body">
                          {editingSegIdx === i ? (
                            <input
                              autoFocus
                              defaultValue={s.text.trim()}
                              onBlur={(e) => saveSegment(i, e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter")
                                  saveSegment(
                                    i,
                                    (e.target as HTMLInputElement).value,
                                  );
                                else if (e.key === "Escape")
                                  setEditingSegIdx(null);
                              }}
                              className="w-full rounded border border-aurora-cyan/50 bg-glass-surface-thin px-1.5 py-0.5 text-sm text-text focus:outline-none"
                            />
                          ) : (
                            <span className="whitespace-pre-wrap break-words">
                              {s.text.trim()}
                            </span>
                          )}
                        </div>
                        {/* Kōrero (v1.27.0): revealed on hover AND on keyboard
                            focus within the row — hover alone would make these
                            controls unreachable without a mouse. */}
                        <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100">
                          <button
                            type="button"
                            onClick={() => setTrimPoint("start", segMs(s))}
                            aria-label={`Start the trim at this line (${fmtTrimMs(segMs(s))})`}
                            title={`Start here — hide everything before ${fmtTrimMs(segMs(s))}`}
                            className="rounded px-1 text-[11px] text-text-subtle transition-colors hover:text-aurora-cyan"
                          >
                            Start here
                          </button>
                          <button
                            type="button"
                            onClick={() => setTrimPoint("end", segMs(s))}
                            aria-label={`End the trim at this line (${fmtTrimMs(segMs(s))})`}
                            title={`End here — hide everything after ${fmtTrimMs(segMs(s))}`}
                            className="rounded px-1 text-[11px] text-text-subtle transition-colors hover:text-aurora-cyan"
                          >
                            End here
                          </button>
                          <button
                            type="button"
                            onClick={() => setEditingSegIdx(i)}
                            aria-label="Edit this line"
                            title="Edit this line"
                            className="rounded px-0.5 text-text-subtle transition-colors hover:text-aurora-cyan"
                          >
                            <Pencil size={11} />
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
              </div>

              {/* Actions */}
              <div className="space-y-2 pt-3 border-t border-glass-border">
                <div className="flex flex-wrap gap-2">
                  <Button
                    variant="secondary"
                    size="md"
                    onClick={onReTranscribe}
                    disabled={!!busy || !activeHasAudio}
                    className="flex items-center gap-1.5"
                    title="Re-run speech-to-text from the saved recording"
                  >
                    {busy === "transcribe" ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : (
                      <RotateCcw size={14} />
                    )}
                    Re-transcribe
                  </Button>
                  <Button
                    variant="secondary"
                    size="md"
                    onClick={onPostProcess}
                    disabled={!!busy || !activeHasTranscript}
                    className="flex items-center gap-1.5"
                    title="Run the prompt below over the current transcript"
                  >
                    {busy === "post" ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : (
                      <Sparkles size={14} />
                    )}
                    Post-process
                  </Button>
                  <Button
                    variant="primary"
                    size="md"
                    onClick={onBoth}
                    disabled={!!busy || !activeHasAudio}
                    className="flex items-center gap-1.5"
                    title="Re-transcribe, then run the prompt below"
                  >
                    {busy === "both" ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : (
                      <Wand2 size={14} />
                    )}
                    Transcribe + post-process
                  </Button>
                  {busy && (
                    <span className="self-center text-xs text-text-subtle tabular-nums">
                      {/* Review fix (v1.25.0 #5): aria-live only on the phase
                          word — a live region containing the ticking counter
                          would be re-announced every second. */}
                      <span aria-live="polite">
                        {busy === "post"
                          ? "Processing"
                          : busy === "transcribe"
                            ? "Transcribing"
                            : "Working"}
                      </span>
                      … {busyElapsed}s
                      {busy !== "post" && busyElapsed > 20
                        ? " — long recordings take a few minutes"
                        : ""}
                    </span>
                  )}
                </div>

                {/* v1.17.0: merge with another meeting (non-destructive). */}
                {meetings.length > 1 && (
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-text-subtle">Merge with</span>
                    <Dropdown
                      options={meetings
                        .filter((m) => m.id !== active.id)
                        .map((m) => ({ value: m.id, label: titleOf(m) }))}
                      selectedValue={mergeWithId}
                      onSelect={setMergeWithId}
                    />
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => mergeMeetings(mergeWithId)}
                      disabled={!mergeWithId}
                      className="flex items-center gap-1.5"
                      title="Combine both meetings' transcripts into a new entry, in chronological order — both originals are kept"
                    >
                      <GitMerge size={14} /> Merge
                    </Button>
                  </div>
                )}

                <label className="block text-xs text-text-subtle">
                  Post-processing prompt (this meeting)
                </label>
                {/* v1.19.0: saved-prompt picker + Save-as-new. */}
                <div className="flex items-center gap-2">
                  <Dropdown
                    options={promptOptions}
                    selectedValue={meetingPromptId}
                    onSelect={(id) => {
                      setMeetingPromptId(id);
                      if (id !== "custom") setCustomPrompt(savedPromptText(id));
                    }}
                  />
                  <button
                    type="button"
                    onClick={async () => {
                      const id = await savePromptAsNew(customPrompt);
                      if (id) setMeetingPromptId(id);
                    }}
                    className="flex items-center gap-1 text-xs text-text-subtle hover:text-text"
                    title="Save the current prompt as a new reusable prompt"
                  >
                    <Plus size={13} /> Save as new
                  </button>
                </div>
                <textarea
                  value={customPrompt}
                  onChange={(e) => {
                    setCustomPrompt(e.target.value);
                    setMeetingPromptId("custom");
                  }}
                  rows={2}
                  className="w-full resize-y bg-glass-surface-thin rounded-lg px-3 py-2 text-sm text-text placeholder:text-text-subtle focus:outline-none border border-glass-border"
                />

                {providerLocal === false ? (
                  <p className="flex items-start gap-1.5 text-xs text-pill-warning">
                    <TriangleAlert size={13} className="mt-0.5 shrink-0" /> Cloud
                    model <span className="font-medium">{ppLabel}</span> —
                    post-processing sends this transcript off your machine. Use a
                    local model (Ollama) for fully-local processing.
                  </p>
                ) : (
                  <p className="flex items-center gap-1.5 text-xs text-text-subtle">
                    <Cpu size={13} className="shrink-0" /> Post-processing model:{" "}
                    <span className="font-medium text-text">{ppLabel}</span>
                    {providerLocal ? " (local)" : ""}
                  </p>
                )}

                {/* v1.17.0: live streaming preview while the model generates. */}
                {(busy === "post" || busy === "both") && liveProcessed.trim() && (
                  <div className="space-y-1 pt-1">
                    <h3 className="flex items-center gap-1.5 text-xs font-semibold text-pill-positive uppercase tracking-wider">
                      <Loader2 size={12} className="animate-spin" /> Generating
                      notes…
                    </h3>
                    <div className="surface-content md-body">
                      <Markdown>{liveProcessed}</Markdown>
                    </div>
                  </div>
                )}

                {active.processed.trim() && (
                  <div className="space-y-1 pt-1">
                    <div className="flex items-center justify-between gap-2">
                      <h3 className="flex items-baseline gap-2 text-xs font-semibold text-pill-positive uppercase tracking-wider">
                        Processed notes
                        <span className="font-normal normal-case tracking-normal text-text-subtle">
                          via {ppLabel}
                        </span>
                      </h3>
                      <div className="flex items-center gap-2">
                        {/* v1.21.0: render a spoken audio brief of the notes via
                            the local Qwen3-TTS engine. GPU-bound + slow (minutes);
                            the button shows a long rendering state. */}
                        {/* Kōrero (v1.27.0): blocked while the notes are stale.
                            Copy and export only cost a paste, but an audio
                            brief is a multi-minute GPU render — spending that
                            on text that predates the current trim is the one
                            case worth stopping outright. */}
                        <button
                          type="button"
                          onClick={genAudioBrief}
                          disabled={briefBusy || notesAreStale(active)}
                          title={
                            notesAreStale(active)
                              ? "These notes were generated before the current trim — re-process to update them before rendering audio."
                              : "Generate a spoken audio brief (local Qwen3-TTS, on-device)"
                          }
                          className="flex items-center gap-1 text-text-subtle hover:text-aurora-cyan transition-colors disabled:opacity-50"
                        >
                          {briefBusy ? (
                            <Loader2 size={13} className="animate-spin" />
                          ) : (
                            <Volume2 size={13} />
                          )}
                          <span className="text-xs">
                            {briefBusy ? "Rendering…" : "Audio brief"}
                          </span>
                        </button>
                        <button
                          type="button"
                          onClick={copyProcessed}
                          title="Copy processed notes (markdown)"
                          className="text-text-subtle hover:text-aurora-cyan transition-colors"
                        >
                          <Copy size={13} />
                        </button>
                        {!editingNotes && (
                          <button
                            type="button"
                            onClick={() => {
                              setNotesDraft(active.processed);
                              setEditingNotes(true);
                            }}
                            title="Edit the processed notes"
                            className="text-text-subtle hover:text-aurora-cyan transition-colors"
                          >
                            <Pencil size={13} />
                          </button>
                        )}
                      </div>
                    </div>
                    {/* Kōrero (v1.27.0): `processed` is a materialised string —
                        there is no read-side filter that can retroactively
                        apply a trim to it, so the only honest option is to say
                        the notes are out of date. Deliberately a WARNING, not a
                        block: copy and export still work, because a stale
                        summary is often still the thing the user wants. */}
                    {notesAreStale(active) && (
                      <p
                        role="status"
                        className="flex items-start gap-1.5 rounded-md border border-pill-warning/40 bg-pill-warning/10 px-2.5 py-1.5 text-xs text-pill-warning"
                      >
                        <TriangleAlert size={13} className="mt-px shrink-0" />
                        These notes were generated before the current trim —
                        re-process to update them.
                      </p>
                    )}
                    {editingNotes ? (
                      <div className="space-y-2">
                        <textarea
                          value={notesDraft}
                          onChange={(e) => setNotesDraft(e.target.value)}
                          rows={12}
                          aria-label="Edit processed notes"
                          className="w-full resize-y rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-2 font-mono text-sm leading-relaxed text-text focus:outline-none"
                        />
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            onClick={saveNotes}
                            className="inline-flex items-center rounded-md bg-aurora-cyan/15 px-2.5 py-1 text-xs font-medium text-aurora-cyan transition-colors hover:bg-aurora-cyan/25"
                          >
                            Save
                          </button>
                          <button
                            type="button"
                            onClick={() => setEditingNotes(false)}
                            className="inline-flex items-center rounded-md px-2.5 py-1 text-xs text-text-muted transition-colors hover:text-text"
                          >
                            Cancel
                          </button>
                        </div>
                      </div>
                    ) : (
                      <div className="surface-content md-body">
                        <Markdown>{active.processed.trim()}</Markdown>
                      </div>
                    )}
                    {/* v1.22.0: give the AI feedback to improve the notes. */}
                    {!editingNotes && (
                      <div className="flex items-center gap-2 pt-1">
                        <input
                          value={feedback}
                          onChange={(e) => setFeedback(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") refineNotes();
                          }}
                          placeholder="Tell the AI how to improve these notes…"
                          aria-label="Feedback to improve the notes"
                          disabled={refining}
                          className="min-w-0 flex-1 rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-1.5 text-xs text-text placeholder:text-text-subtle focus:outline-none disabled:opacity-50"
                        />
                        <button
                          type="button"
                          onClick={refineNotes}
                          disabled={refining || !feedback.trim()}
                          className="inline-flex shrink-0 items-center gap-1 rounded-lg border border-glass-border bg-glass-surface-thin px-2.5 py-1.5 text-xs text-text-muted transition-colors hover:text-aurora-cyan disabled:opacity-50"
                        >
                          {refining ? (
                            <Loader2 size={13} className="animate-spin" />
                          ) : (
                            <Sparkles size={13} />
                          )}
                          {refining ? "Refining…" : "Refine"}
                        </button>
                      </div>
                    )}
                    {briefBusy && (
                      <p className="flex items-center gap-1.5 text-xs text-text-subtle">
                        <Loader2 size={12} className="animate-spin" /> Rendering
                        audio on-device — GPU-bound, can take a few minutes for a
                        long note. Leave it running.
                      </p>
                    )}
                    {briefUrl && !briefBusy && (
                      <div className="mt-1 space-y-1.5">
                        <audio controls src={briefUrl} className="w-full" />
                        <div className="flex items-center gap-3 text-xs text-text-muted">
                          <a
                            href={briefUrl}
                            download="audio-brief.mp3"
                            className="inline-flex items-center gap-1 transition-colors hover:text-aurora-cyan"
                          >
                            <Download size={13} /> Download
                          </a>
                          {briefPath && (
                            <button
                              type="button"
                              onClick={() =>
                                revealItemInDir(briefPath).catch(() =>
                                  toast.error("Could not open the folder."),
                                )
                              }
                              className="inline-flex items-center gap-1 transition-colors hover:text-aurora-cyan"
                              title="Open the folder where the audio is saved"
                            >
                              <FolderOpen size={13} /> Show in folder
                            </button>
                          )}
                        </div>
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Recovery: recordings on disk */}
      <div className="space-y-2">
        <div className="flex items-center justify-between px-1">
          <h2 className="text-xs font-semibold text-text-muted uppercase tracking-wider">
            Recordings on disk · kept 30 days
          </h2>
          <button
            type="button"
            onClick={loadRecordings}
            title="Refresh"
            className="text-text-subtle hover:text-aurora-cyan transition-colors"
          >
            <RefreshCw size={14} />
          </button>
        </div>
        <div className="glass-card p-1.5">
          {recordings === null ? (
            <div className="px-4 py-4 text-sm text-text-subtle text-center">Loading…</div>
          ) : recordings.length === 0 ? (
            <div className="px-4 py-4 text-xs text-text-subtle text-center">
              No saved recordings. They appear here after you record a meeting, and
              can be transcribed even if the app closed unexpectedly.
            </div>
          ) : (
            <div className="divide-y divide-glass-border">
              {recordings.map((f) => (
                <div key={f.path} className="flex items-center gap-3 px-4 py-2.5">
                  <FileAudio size={16} className="text-text-subtle shrink-0" />
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-text truncate">{f.file_name}</p>
                    <p className="text-xs text-text-subtle">
                      {f.modified ? new Date(f.modified * 1000).toLocaleString() : ""}
                    </p>
                  </div>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => transcribeRecording(f)}
                    disabled={busyFile === f.path}
                    className="flex items-center gap-1.5 shrink-0"
                  >
                    {busyFile === f.path ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : (
                      <RotateCcw size={14} />
                    )}
                    Transcribe
                  </Button>
                  <button
                    type="button"
                    title="Delete this recording from disk"
                    onClick={() => deleteRecording(f)}
                    disabled={busyFile === f.path}
                    className="text-text-subtle hover:text-pill-urgent transition-colors shrink-0"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* v1.24.0 (paths): storage — where recordings are written */}
      <div className="space-y-2">
        <h2 className="px-1 text-xs font-semibold text-text-muted uppercase tracking-wider">
          Storage
        </h2>
        <div className="surface-content p-4 space-y-3">
          {dirs === null ? (
            <p className="text-sm text-text-subtle">Loading…</p>
          ) : (
            <>
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <p className="text-sm text-text">Recording folder</p>
                  <p
                    className="mt-0.5 select-all break-all text-xs text-text-muted"
                    title={dirs.recordingDir}
                  >
                    {dirs.recordingDir}
                    {!dirs.recordingIsCustom && (
                      <span className="text-text-subtle"> (default)</span>
                    )}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <button
                    type="button"
                    onClick={() =>
                      revealItemInDir(dirs.recordingDir).catch(() =>
                        toast.error("Could not open the folder."),
                      )
                    }
                    title="Show this folder in Explorer"
                    className="text-text-subtle transition-colors hover:text-aurora-cyan"
                  >
                    <FolderOpen size={15} />
                  </button>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={changeRecordingDir}
                    disabled={dirBusy}
                  >
                    Change…
                  </Button>
                  {dirs.recordingIsCustom && (
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={resetRecordingDir}
                      disabled={dirBusy}
                    >
                      Reset
                    </Button>
                  )}
                </div>
              </div>
              {dirs.recordingIsCustom && (
                <div className="flex items-center justify-between gap-3 border-t border-glass-border pt-3">
                  <p className="text-xs text-text-muted">
                    Recordings made before the change are still in the default
                    folder. Move them here so everything lives in one place.
                  </p>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={moveExistingRecordings}
                    disabled={dirBusy}
                    className="flex shrink-0 items-center gap-1.5"
                  >
                    {dirBusy ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : null}
                    Move existing
                  </Button>
                </div>
              )}
              <p className="text-xs text-text-subtle">
                Tip: recordings are large — a local drive works best. Cloud-synced
                folders (OneDrive/Dropbox) can churn while a meeting records.
                Exports ask where to save each time and remember your last folder
                ({dirs.exportSeedDir}).
              </p>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
