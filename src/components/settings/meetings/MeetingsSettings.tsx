/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import {
  Circle,
  Square,
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
} from "lucide-react";
import { toast } from "sonner";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
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
interface TranscriptSeg {
  source: string; // "you" | "others"
  text: string;
}

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
    transcript: Array.isArray(m.transcript) ? m.transcript : [],
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
    return transcript
      .filter((s) => s.text.trim())
      .map(
        (s) =>
          `${labelFor(s.source, youLabel, othersLabel)}: ${s.text.trim()}`,
      )
      .join("\n");
  }
  return [
    you.trim() ? `${youLabel}:\n${you.trim()}` : "",
    others.trim() ? `${othersLabel}:\n${others.trim()}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
};

const baseName = (p: string) => p.replace(/\\/g, "/").split("/").pop() || p;

export const MeetingsSettings: React.FC = () => {
  const { settings } = useSettings();

  // v1.13.4: meetings live on disk (appdata/meetings/meetings.json); loaded
  // async on mount, with one-time migration from the legacy localStorage store.
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [storeReady, setStoreReady] = useState(false);
  const [recording, setRecording] = useState(false);
  const [recProcessing, setRecProcessing] = useState(false);
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
  const [elapsed, setElapsed] = useState(0);
  const [systemCaptured, setSystemCaptured] = useState<boolean | null>(null);
  const [busy, setBusy] = useState<null | "transcribe" | "post" | "both">(null);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [customPrompt, setCustomPrompt] = useState(DEFAULT_PROMPT);
  const [providerLocal, setProviderLocal] = useState<boolean | null>(null);
  const [recordings, setRecordings] = useState<RecordingFile[] | null>(null);
  const [busyFile, setBusyFile] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
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
  const currentModel = settings?.selected_model ?? "";

  // v1.13.4: load from disk; migrate the legacy localStorage store once, and
  // only clear the legacy copy after a verified round-trip to disk.
  useEffect(() => {
    (async () => {
      let list: Meeting[] = [];
      try {
        const res = await commands.meetingsStoreLoad();
        if (res.status === "ok" && res.data.trim()) {
          list = normaliseMeetings(JSON.parse(res.data));
        }
      } catch {
        /* fall through to legacy */
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
      timerRef.current = window.setInterval(() => setElapsed((e) => e + 1), 1000);
    } else if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
    return () => {
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
    };
  }, [recording]);

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
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
  const deleteRecording = async (f: RecordingFile) => {
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

  const titleOf = (m: Meeting) =>
    m.title.trim() || `Meeting · ${new Date(m.createdAt).toLocaleString()}`;

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
      transcript: [
        ...(first.transcript ?? []),
        ...(second.transcript ?? []),
      ],
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
              title: "",
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
      patchMeeting(m.id, { processed: r.data, processPrompt: customPrompt.trim() });
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
    const text = combine(
      active.you,
      active.others,
      active.youLabel,
      active.othersLabel,
      active.transcript,
    );
    if (!text.trim()) {
      toast.message("Nothing to post-process — transcribe first.");
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
      const text = combine(
        you,
        others,
        active.youLabel,
        active.othersLabel,
        transcript,
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
          extensions: ["wav", "m4a", "mp3", "aac", "flac", "ogg"],
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
    }
  };

  // ---- copy / export / delete -------------------------------------------
  const copyActive = async () => {
    if (!active) return;
    const text = active.processed.trim()
      ? `${combine(active.you, active.others, active.youLabel, active.othersLabel, active.transcript)}\n\n--- Processed ---\n${active.processed.trim()}`
      : combine(active.you, active.others, active.youLabel, active.othersLabel, active.transcript);
    try {
      await writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error("Could not copy to clipboard.");
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

  const exportActive = async () => {
    if (!active) return;
    const parts = [
      `# ${titleOf(active)}`,
      "",
      combine(
        active.you,
        active.others,
        active.youLabel,
        active.othersLabel,
        active.transcript,
      ) || "(no transcript)",
    ];
    if (active.processed.trim()) {
      parts.push("", "## Processed", "", active.processed.trim());
    }
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const base = active.title.trim() || "meeting";
    try {
      const res = await commands.meetingExportTranscript(`${base}-${stamp}`, parts.join("\n"));
      if (res.status === "ok") toast.success(`Exported to ${res.data}`);
      else toast.error(`Export failed: ${res.error}`);
    } catch (e) {
      toast.error(`Export failed: ${String(e)}`);
    }
  };

  const deleteMeeting = (id: string) => {
    // v1.13.6: free the disk too — once the metadata is gone the WAVs are
    // only reachable via the recovery list, which is rarely what's wanted.
    const gone = meetings.find((m) => m.id === id);
    [gone?.micPath, gone?.systemPath].forEach((p) => {
      if (p) commands.meetingDeleteRecording(p).catch(() => {});
    });
    setMeetings((prev) => {
      const next = prev.filter((m) => m.id !== id);
      if (id === activeId) setActiveId(next[0]?.id ?? null);
      return next;
    });
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

      {/* v1.13.3: persistent warning if the capture worker reported a
          disk-write failure — the toast is transient, this is not. */}
      {captureError && (
        <div className="glass-card p-3 flex items-start gap-2 border border-red-500/40">
          <TriangleAlert size={16} className="text-red-400 shrink-0 mt-0.5" />
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
          title="Import an audio file (WAV, M4A, MP3, FLAC, OGG) to transcribe and process"
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

        {recording && (
          <span className="flex items-center gap-2 text-sm text-text-muted">
            <span className="inline-block w-2 h-2 rounded-full bg-pill-urgent animate-pulse" />
            {systemCaptured === false ? "Recording (mic only)" : "Recording you + others"}
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
        <div className="glass-card p-4 space-y-2">
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
            <div className="glass-card-thin md-body">
              <Markdown>{liveAnswer}</Markdown>
            </div>
          )}
        </div>
      )}

      {/* Import workflow card */}
      {importPath && (
        <div className="glass-card p-4 space-y-3">
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
          <textarea
            value={importPrompt}
            onChange={(e) => setImportPrompt(e.target.value)}
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
        </div>
      )}

      <div className="flex gap-4">
        {/* Meeting list */}
        <div className="w-56 shrink-0 glass-card p-2 flex flex-col gap-1 max-h-[55vh] overflow-y-auto">
          {meetings.length === 0 ? (
            <div className="px-3 py-6 flex flex-col items-center gap-2 text-center">
              <Users size={20} className="text-text-subtle" />
              <p className="text-xs text-text-subtle">
                No meetings yet. Press Record to capture one.
              </p>
            </div>
          ) : (
            meetings.map((m) => {
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

        {/* Detail */}
        <div className="flex-1 min-w-0 glass-card p-4">
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

              {/* Transcript — v1.14.5: speaker tags are editable (pencil), so
                  "Others" can become "Gerard" etc. Labels persist with the
                  meeting and flow into copy/export/post-processing. */}
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
                    <p className="text-sm text-text-muted whitespace-pre-wrap leading-relaxed">
                      {row.text.trim() || "—"}
                    </p>
                  )}
                </div>
              ))}

              {/* v1.17.0: chronological, interleaved transcript — both speakers
                  in the order they actually spoke. */}
              {active.transcript && active.transcript.length > 0 && (
                <div className="space-y-1.5">
                  {active.transcript
                    .filter((s) => s.text.trim())
                    .map((s, i) => (
                      <p
                        key={i}
                        className="text-sm text-text-muted leading-relaxed"
                      >
                        <span
                          className={`font-semibold ${
                            s.source === "you"
                              ? "text-aurora-cyan"
                              : "text-aurora-purple"
                          }`}
                        >
                          {s.source === "you"
                            ? active.youLabel
                            : active.othersLabel}
                          :
                        </span>{" "}
                        {s.text.trim()}
                      </p>
                    ))}
                </div>
              )}

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
                <textarea
                  value={customPrompt}
                  onChange={(e) => setCustomPrompt(e.target.value)}
                  rows={2}
                  className="w-full resize-y bg-glass-surface-thin rounded-lg px-3 py-2 text-sm text-text placeholder:text-text-subtle focus:outline-none border border-glass-border"
                />

                {providerLocal === false ? (
                  <p className="flex items-start gap-1.5 text-xs text-pill-warning">
                    <TriangleAlert size={13} className="mt-0.5 shrink-0" /> Your Post
                    Process provider is a cloud service — post-processing sends this
                    transcript to it. Use a local model (Ollama) for fully-local
                    processing.
                  </p>
                ) : (
                  <p className="text-xs text-text-subtle">
                    Post-processing uses your Post Process provider/model (Ollama +
                    Gemma = fully-local).
                  </p>
                )}

                {/* v1.17.0: live streaming preview while the model generates. */}
                {(busy === "post" || busy === "both") && liveProcessed.trim() && (
                  <div className="space-y-1 pt-1">
                    <h3 className="flex items-center gap-1.5 text-xs font-semibold text-pill-positive uppercase tracking-wider">
                      <Loader2 size={12} className="animate-spin" /> Generating
                      notes…
                    </h3>
                    <div className="glass-card-thin md-body">
                      <Markdown>{liveProcessed}</Markdown>
                    </div>
                  </div>
                )}

                {active.processed.trim() && (
                  <div className="space-y-1 pt-1">
                    <div className="flex items-center justify-between gap-2">
                      <h3 className="text-xs font-semibold text-pill-positive uppercase tracking-wider">
                        Processed notes
                      </h3>
                      <button
                        type="button"
                        onClick={copyProcessed}
                        title="Copy processed notes (markdown)"
                        className="text-text-subtle hover:text-aurora-cyan transition-colors"
                      >
                        <Copy size={13} />
                      </button>
                    </div>
                    <div className="glass-card-thin md-body">
                      <Markdown>{active.processed.trim()}</Markdown>
                    </div>
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
    </div>
  );
};
