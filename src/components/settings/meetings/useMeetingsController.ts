// Kōrero 1.42: the Meetings page's state and actions, separated from its
// layout so the page could be rebuilt as a library + meeting view without
// touching a line of the logic that keeps meetings safe.
//
// This is the logic of the pre-1.42 MeetingsSettings.tsx (v1.13 → v1.41),
// moved, not rewritten. Two things left it:
//   - Recording (start, pause, the clock, live segments, the capture test)
//     moved to `useRecorder`, so every page can drive a meeting.
//   - Stop's meeting builder moved with it.
// What was added: the open tab, a search highlight, Ask after the meeting, and
// flagged moments in the notes input.

import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { confirmDestructive } from "../../ui/confirmToast";
import type { DropdownOption } from "../../ui/Dropdown";
import { commands, type ModelInfo } from "../../../bindings";
import { useSettings } from "../../../hooks/useSettings";
import {
  useMeetingJobs,
  meetingsBridge,
  type TranscriptPatch,
} from "../../../stores/meetingJobsStore";
import type { MeetingDoc } from "../../../stores/meetingsBridge";
import { logActivity } from "../../../stores/activityStore";
import type { MeetingTab } from "../../../stores/navStore";
import { useRecorder } from "../../../stores/recorderStore";
import {
  DEFAULT_PROMPT,
  baseName,
  combine,
  fmtTrimMs,
  hiddenCount,
  isTrimmed,
  newId,
  normaliseMeetings,
  notesAreStale,
  notesInput,
  parseTrimInput,
  titleOf,
  trimKeyOf,
  visibleSegs,
  type Meeting,
  type RecordingFile,
  type TranscriptSeg,
} from "../../meetings/model";

const LEGACY_MEETINGS_STORAGE = "korero.meetings.v1";

// Legacy localStorage store (pre-v1.13.4) — read only for one-time migration.
const loadLegacyMeetings = (): Meeting[] => {
  try {
    const raw = localStorage.getItem(LEGACY_MEETINGS_STORAGE);
    if (raw) return normaliseMeetings(JSON.parse(raw));
  } catch {
    /* ignore corrupt store */
  }
  return [];
};

export type MeetingsView =
  | { kind: "meeting" }
  | { kind: "live" }
  | { kind: "import" }
  | { kind: "recordings" };

export const useMeetingsController = () => {
  const { settings, updateSetting } = useSettings();

  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [view, setView] = useState<MeetingsView>({ kind: "meeting" });
  const [tab, setTab] = useState<MeetingTab>("transcript");
  const [highlight, setHighlight] = useState<string>("");
  const [storeReady, setStoreReady] = useState(false);
  // v1.29.0 (R-02): non-null means the store could not be read this session.
  // While set, `storeReady` stays false and NOTHING is written to disk.
  const [storeLoadError, setStoreLoadError] = useState<string | null>(null);

  const job = useMeetingJobs((s) => s.job);
  const task = useMeetingJobs((s) => s.task);
  const stopping = useMeetingJobs((s) => s.stopping);
  const transcribeProgress = task?.progress ?? null;

  const [briefBusy, setBriefBusy] = useState(false);
  const [briefUrl, setBriefUrl] = useState<string | null>(null);
  const [briefPath, setBriefPath] = useState<string | null>(null);
  const [briefFor, setBriefFor] = useState<string | null>(null);
  const [editingNotes, setEditingNotes] = useState(false);
  const [notesDraft, setNotesDraft] = useState("");
  const [feedback, setFeedback] = useState("");
  const refining = job?.kind === "refine";
  const [editingSegIdx, setEditingSegIdx] = useState<number | null>(null);
  const [trimInDraft, setTrimInDraft] = useState("");
  const [trimOutDraft, setTrimOutDraft] = useState("");
  const [trimError, setTrimError] = useState<string | null>(null);
  // Ask (after the meeting).
  const [askQuestion, setAskQuestion] = useState("");
  const [askAnswer, setAskAnswer] = useState<{ q: string; a: string } | null>(null);
  const [asking, setAsking] = useState(false);

  const importBusy = task?.kind === "import";
  const jobRunningHere = job && job.meetingId === activeId ? job : null;
  // Keyed off ANY running work: the engine and the local model are shared,
  // and the store runs one thing at a time.
  const busy: null | "transcribe" | "post" | "both" | "other" =
    task?.kind === "transcribe"
      ? "transcribe"
      : task?.kind === "both"
        ? "both"
        : task
          ? "other"
          : job
            ? job.kind === "both"
              ? "both"
              : "post"
            : null;
  const busyMeetingId = task ? task.meetingId : (job?.meetingId ?? null);
  const liveProcessed = jobRunningHere ? jobRunningHere.live : "";
  const elsewhereJobTitle =
    busy && busyMeetingId !== activeId
      ? busyMeetingId
        ? (meetings.find((m) => m.id === busyMeetingId)?.title ?? "another meeting")
        : (task?.label ?? "another job")
      : null;
  const [busyElapsed, setBusyElapsed] = useState(0);
  const busyStartedAt = task?.startedAt ?? job?.startedAt ?? null;
  useEffect(() => {
    if (!busy) {
      setBusyElapsed(0);
      return;
    }
    const started = busyStartedAt ?? Date.now();
    setBusyElapsed(Math.floor((Date.now() - started) / 1000));
    const t = window.setInterval(
      () => setBusyElapsed(Math.floor((Date.now() - started) / 1000)),
      1000,
    );
    return () => window.clearInterval(t);
  }, [busy, busyStartedAt]);

  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [customPrompt, setCustomPrompt] = useState(DEFAULT_PROMPT);
  const [meetingPromptId, setMeetingPromptId] = useState<string>("custom");
  const [importPromptId, setImportPromptId] = useState<string>("custom");
  const [providerLocal, setProviderLocal] = useState<boolean | null>(null);
  const [recordings, setRecordings] = useState<RecordingFile[] | null>(null);
  const busyFile = task?.kind === "recover" ? (task.path ?? null) : null;
  const [copied, setCopied] = useState(false);
  const [exportedPath, setExportedPath] = useState<string | null>(null);
  useEffect(() => {
    setExportedPath(null);
  }, [activeId]);
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
  const [dirs, setDirs] = useState<{
    recordingDir: string;
    recordingIsCustom: boolean;
    defaultRecordingDir: string;
    exportSeedDir: string;
  } | null>(null);
  const [dirBusy, setDirBusy] = useState(false);
  const [search, setSearch] = useState("");
  const [editingListId, setEditingListId] = useState<string | null>(null);
  const [editingLabel, setEditingLabel] = useState<null | "you" | "others">(null);
  const [teachWrong, setTeachWrong] = useState<string | null>(null);
  const [mergeWithId, setMergeWithId] = useState<string>("");
  const [importPath, setImportPath] = useState<string | null>(null);
  const [importPrompt, setImportPrompt] = useState(DEFAULT_PROMPT);

  const active = meetings.find((m) => m.id === activeId) ?? null;

  // Clear per-meeting edit state when the active meeting changes.
  useEffect(() => {
    setEditingNotes(false);
    setEditingSegIdx(null);
    setFeedback("");
    setAskAnswer(null);
    setAskQuestion("");
  }, [activeId]);

  // Keep the in/out fields showing the CURRENT window.
  useEffect(() => {
    setTrimInDraft(active?.trimStartMs != null ? fmtTrimMs(active.trimStartMs) : "");
    setTrimOutDraft(active?.trimEndMs != null ? fmtTrimMs(active.trimEndMs) : "");
    setTrimError(null);
  }, [activeId, active?.trimStartMs, active?.trimEndMs]);
  const currentModel = settings?.selected_model ?? "";

  const ppProvider = settings?.post_process_providers?.find(
    (p) => p.id === settings?.post_process_provider_id,
  );
  const ppModel =
    (settings?.post_process_models ?? {})[settings?.post_process_provider_id ?? ""] ?? "";
  const ppLabel = ppModel
    ? `${ppProvider?.label ?? "Model"} · ${ppModel}`
    : ppProvider?.label
      ? `${ppProvider.label} · no model set`
      : "not configured";

  const promptOptions: DropdownOption[] = [
    ...(settings?.post_process_prompts ?? []).map((p) => ({ value: p.id, label: p.name })),
    { value: "custom", label: "Custom prompt…" },
  ];
  const savedPromptText = (id: string): string =>
    settings?.post_process_prompts?.find((p) => p.id === id)?.prompt ?? "";
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

  // This view's handle on `meetingsBridge` (see meetingsBridge.ts).
  const meetingsRef = useRef<Meeting[]>([]);
  useEffect(() => {
    meetingsRef.current = meetings;
  }, [meetings]);
  const storeReadyRef = useRef(false);
  const lastSavedRef = useRef<string>("");

  useEffect(() => {
    // Attach BEFORE loading, so work finishing mid-load waits for this view.
    const token = meetingsBridge.attach({
      applyPatch: (id, patch) => {
        if (!meetingsRef.current.some((m) => m.id === id)) return "missing";
        const apply = (list: Meeting[]) =>
          list.map((m) => (m.id === id ? { ...m, ...(patch as Partial<Meeting>) } : m));
        meetingsRef.current = apply(meetingsRef.current);
        setMeetings(apply);
        return activeIdRef.current === id ? "shown" : "view";
      },
      addMeeting: (doc, select) => {
        const [m] = normaliseMeetings([doc]);
        if (!meetingsRef.current.some((x) => x.id === m.id)) {
          meetingsRef.current = [m, ...meetingsRef.current];
          setMeetings((prev) => (prev.some((x) => x.id === m.id) ? prev : [m, ...prev]));
        }
        if (select) {
          activeIdRef.current = m.id;
          setActiveId(m.id);
          setView({ kind: "meeting" });
          setTab(m.captureWarnings?.length ? "transcript" : "notes");
        }
        return activeIdRef.current === m.id ? "shown" : "view";
      },
    });
    (async () => {
      let list: Meeting[] = [];
      // v1.29.0 (R-02): we only ever save a store we successfully READ.
      let loadFailure: string | null = null;
      try {
        const res = await meetingsBridge.load();
        if (!res.ok) loadFailure = res.error || "could not read the meetings store";
        else if (res.data.trim()) list = normaliseMeetings(JSON.parse(res.data));
      } catch (e) {
        loadFailure = e instanceof Error ? e.message : String(e);
      }
      if (loadFailure) {
        console.error("Meetings store failed to load:", loadFailure);
        setStoreLoadError(loadFailure);
        setMeetings([]);
        setActiveId(null);
        meetingsBridge.detach(token);
        return;
      }
      if (list.length === 0) {
        const legacy = loadLegacyMeetings();
        if (legacy.length > 0) {
          list = legacy;
          try {
            const saved = await meetingsBridge.save(JSON.stringify(legacy));
            if (saved.ok) {
              const check = await meetingsBridge.load();
              if (check.ok && check.data.trim()) localStorage.removeItem(LEGACY_MEETINGS_STORAGE);
            }
          } catch {
            /* keep the legacy copy until a save round-trips */
          }
        }
      }
      meetingsRef.current = list;
      // A focus request (search, Today) may already have chosen a meeting.
      const initial = activeIdRef.current && list.some((m) => m.id === activeIdRef.current)
        ? activeIdRef.current
        : (list[0]?.id ?? null);
      activeIdRef.current = initial;
      setMeetings(list);
      setActiveId(initial);
      storeReadyRef.current = true;
      setStoreReady(true);
      meetingsBridge.markReady(token);
    })();
    return () => {
      let finalSave: string | undefined;
      if (storeReadyRef.current) {
        const payload = JSON.stringify(meetingsRef.current);
        if (payload !== lastSavedRef.current) finalSave = payload;
      }
      storeReadyRef.current = false;
      meetingsBridge.detach(token, finalSave);
    };
  }, []);

  // Debounced save, through the bridge's single FIFO queue.
  useEffect(() => {
    if (!storeReady) return;
    const payload = JSON.stringify(meetings);
    const t = window.setTimeout(() => {
      void meetingsBridge.save(payload).then((r) => {
        if (!r.ok) toast.error(`Couldn't save meetings: ${r.error}`);
        else lastSavedRef.current = payload;
      });
    }, 500);
    return () => window.clearTimeout(t);
  }, [meetings, storeReady]);

  const loadRecordings = async () => {
    try {
      const res = await commands.meetingListRecordings();
      setRecordings(res.status === "ok" ? res.data : []);
    } catch {
      setRecordings([]);
    }
  };

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
          } else toast.error(res.error);
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
      .then((r) => setModels(r.status === "ok" ? r.data.filter((m) => m.is_downloaded) : []))
      .catch(() => setModels([]));
  }, []);

  // Reload the Recordings list when a meeting finishes saving.
  const wasStopping = useRef(false);
  useEffect(() => {
    if (wasStopping.current && !stopping) loadRecordings();
    wasStopping.current = !!stopping;
  }, [stopping]);

  useEffect(() => {
    setCustomPrompt(active?.processPrompt?.trim() ? active.processPrompt : DEFAULT_PROMPT);
  }, [activeId]);

  const patchMeeting = (id: string, patch: Partial<Meeting>) =>
    setMeetings((prev) => prev.map((m) => (m.id === id ? { ...m, ...patch } : m)));

  // ---- trim markers (v1.27.0): writers patch ONLY the marker fields --------
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
    patchMeeting(active.id, which === "start" ? { trimStartMs: ms } : { trimEndMs: ms });
    setTrimError(null);
  };

  const clearTrim = () => {
    if (!active) return;
    patchMeeting(active.id, { trimStartMs: undefined, trimEndMs: undefined });
    setTrimError(null);
    toast.success("Trim cleared — showing the whole meeting.");
  };

  const commitTrimInputs = () => {
    if (!active) return;
    const inRaw = trimInDraft.trim();
    const outRaw = trimOutDraft.trim();
    let startMs: number | undefined;
    if (inRaw) {
      const parsed = parseTrimInput(inRaw);
      if (parsed == null) {
        setTrimError("Start must be mm:ss.s (e.g. 1:05.5) or seconds (e.g. 65.5).");
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
      setTrimError("The end must come after the start — that window would hide the whole meeting.");
      return;
    }
    setTrimError(null);
    patchMeeting(active.id, { trimStartMs: startMs, trimEndMs: endMs });
  };

  // Search in the library: title + transcript (honouring the trim) + notes.
  const searchQ = search.trim().toLowerCase();
  const filteredMeetings = searchQ
    ? meetings.filter((m) =>
        `${titleOf(m)} ${m.you} ${m.others} ${visibleSegs(m)
          .map((s) => s.text)
          .join(" ")} ${m.processed}`
          .normalize("NFD")
          .replace(/[̀-ͯ]/g, "")
          .toLowerCase()
          .includes(searchQ.normalize("NFD").replace(/[̀-ͯ]/g, "")),
      )
    : meetings;

  // v1.17.0: merge two meetings into a NEW combined entry — non-destructive.
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
      // The FULL transcript of both parts: merge is a write path.
      transcript: [...(first.transcript ?? []), ...(second.transcript ?? [])],
      // Deliberately UNTRIMMED: the two parts' offsets are not comparable.
      youLabel: first.youLabel,
      othersLabel: first.othersLabel,
      imported: first.imported && second.imported,
      processed: joinPart(first.processed, second.processed),
      processPrompt: first.processPrompt || second.processPrompt,
      createdAt: Date.now(),
      systemCaptured: first.systemCaptured || second.systemCaptured,
      micPath: first.micPath ?? second.micPath,
      systemPath: first.systemPath ?? second.systemPath,
    };
    setMeetings((prev) => [m, ...prev]);
    setActiveId(m.id);
    setMergeWithId("");
    toast.success(
      "Merged into a new meeting — both originals kept. Re-process to get one combined summary.",
    );
    if (isTrimmed(first) || isTrimmed(second)) {
      toast.message("Trim cleared on the merged meeting.", {
        description:
          "The two parts come from different audio files, so their timings are no longer comparable. The originals keep their own trims.",
      });
    }
  };

  // ---- Ask (after the meeting, 1.42) ---------------------------------------
  const ask = async () => {
    const q = askQuestion.trim();
    if (!active || !q || asking) return;
    const transcript = combine(
      active.you,
      active.others,
      active.youLabel,
      active.othersLabel,
      visibleSegs(active),
    );
    if (!transcript.trim()) {
      toast.message("There is no transcript to ask about yet.");
      return;
    }
    setAsking(true);
    try {
      const res = await commands.meetingQuery(transcript, q);
      if (res.status === "ok") setAskAnswer({ q, a: res.data });
      else toast.error(res.error);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setAsking(false);
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

  // ---- transcription / notes: every action hands its work to the store -----
  const onReTranscribe = async () => {
    if (!active || busy) return;
    await useMeetingJobs.getState().transcribe({
      meetingId: active.id,
      title: titleOf(active),
      micPath: active.micPath,
      systemPath: active.systemPath,
    });
  };

  const onPostProcess = async () => {
    if (!active || busy) return;
    // The trim is applied HERE, and flagged moments are added for the model.
    const text = notesInput(active, visibleSegs(active));
    if (!text.trim()) {
      toast.message(
        isTrimmed(active)
          ? "The current trim hides every segment — widen or clear it first."
          : "Nothing to post-process — transcribe first.",
      );
      return;
    }
    setTab("notes");
    await useMeetingJobs.getState().start({
      meetingId: active.id,
      title: titleOf(active),
      kind: "post",
      text,
      prompt: customPrompt.trim(),
      trimKey: trimKeyOf(active),
    });
  };

  const onBoth = async () => {
    if (!active || busy) return;
    const m = active;
    setTab("notes");
    await useMeetingJobs.getState().transcribe({
      meetingId: m.id,
      title: titleOf(m),
      micPath: m.micPath,
      systemPath: m.systemPath,
      thenNotes: {
        buildText: (t: TranscriptPatch) => {
          const next = { ...m, you: t.you, others: t.others, transcript: t.transcript as TranscriptSeg[] };
          return notesInput(next, visibleSegs(next));
        },
        prompt: customPrompt.trim(),
        trimKey: trimKeyOf(m),
      },
    });
  };

  // ---- import ------------------------------------------------------------
  const pickImportFile = async () => {
    const blocked = importBlocked();
    if (blocked) {
      toast.message(blocked);
      return;
    }
    try {
      const sel = await openFileDialog({
        multiple: false,
        filters: [
          {
            name: "Audio files",
            extensions: ["wav", "m4a", "mp3", "aac", "flac", "ogg", "caf", "aiff", "aif"],
          },
        ],
      });
      if (typeof sel === "string") startImport(sel);
    } catch (e) {
      toast.error(`Could not open file picker: ${String(e)}`);
    }
  };

  /** Why an import can't start right now, or null. */
  const importBlocked = (): string | null => {
    if (useRecorder.getState().recording || useMeetingJobs.getState().stopping) {
      return "Finish the meeting first: importing uses the same speech engine.";
    }
    if (useMeetingJobs.getState().task?.kind === "import") {
      return "An import is already running. Start the next one when it finishes.";
    }
    return null;
  };

  const startImport = (path: string) => {
    const blocked = importBlocked();
    if (blocked) {
      toast.message(blocked);
      return;
    }
    setImportPath(path);
    setImportPrompt(DEFAULT_PROMPT);
    setImportPromptId("custom");
    setView({ kind: "import" });
  };

  const runImport = async (alsoProcess: boolean) => {
    if (!importPath || importBusy) return;
    const path = importPath;
    const ok = await useMeetingJobs.getState().importFile({
      path,
      label: baseName(path),
      notesPrompt: alsoProcess ? importPrompt.trim() : null,
      makeMeeting: (transcript) => {
        const m: Meeting = {
          id: newId(),
          title: `Imported · ${baseName(path)}`,
          you: transcript,
          others: "",
          youLabel: "You",
          othersLabel: "Others",
          imported: true,
          processed: "",
          processPrompt: "",
          processedTrimKey: ":",
          createdAt: Date.now(),
          systemCaptured: false,
          micPath: path,
          systemPath: null,
        };
        return m as unknown as MeetingDoc;
      },
    });
    if (ok && mountedRef.current) setImportPath(null);
  };

  // ---- copy / export / brief / notes ------------------------------------
  const copyActive = async () => {
    if (!active) return;
    const body = combine(active.you, active.others, active.youLabel, active.othersLabel, visibleSegs(active));
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

  const genAudioBrief = async () => {
    if (!active || briefBusy) return;
    const text = active.processed.trim();
    if (!text) {
      toast.message("Generate notes first.");
      return;
    }
    const forId = active.id;
    const forTitle = titleOf(active);
    setBriefBusy(true);
    setBriefUrl(null);
    setBriefFor(forId);
    try {
      const r = await commands.meetingGenerateAudioBrief(text, null, null, null);
      if (r.status !== "ok") throw new Error(r.error);
      setBriefUrl(convertFileSrc(r.data, "asset"));
      setBriefPath(r.data);
      toast.success("Audio brief ready.");
      logActivity({
        status: "done",
        title: "Audio brief ready",
        detail: forTitle,
        target: { section: "meetings", meetingId: forId, tab: "notes" },
      });
    } catch (e) {
      toast.error(`Audio brief failed: ${String(e)}`);
      logActivity({ status: "failed", title: "Audio brief failed", detail: String(e) });
    } finally {
      setBriefBusy(false);
    }
  };

  const copyProcessed = async () => {
    if (!active?.processed.trim()) return;
    try {
      await writeText(active.processed.trim());
      toast.success("Notes copied (markdown).");
    } catch {
      toast.error("Could not copy to clipboard.");
    }
  };

  const saveNotes = () => {
    if (!active) return;
    patchMeeting(active.id, { processed: notesDraft });
    setEditingNotes(false);
    toast.success("Notes updated.");
  };

  const refineNotes = async () => {
    if (!active || busy) return;
    const fb = feedback.trim();
    if (!fb) {
      toast.message("Tell the AI what to improve.");
      return;
    }
    if (!active.processed.trim()) {
      toast.message("Generate notes first.");
      return;
    }
    const ok = await useMeetingJobs.getState().refine({
      meetingId: active.id,
      title: titleOf(active),
      previous: active.processed,
      feedback: fb,
    });
    if (ok && mountedRef.current) setFeedback("");
  };

  const saveSegment = (idx: number, text: string) => {
    if (!active) return;
    const before = (active.transcript ?? [])[idx]?.text ?? "";
    const next = (active.transcript ?? []).map((s, i) => (i === idx ? { ...s, text } : s));
    patchMeeting(active.id, { transcript: next });
    setEditingSegIdx(null);
    suggestCorrectionFromEdit(before, text);
  };

  // Offer to TEACH a clean one-word substitution (conservative by design).
  const suggestCorrectionFromEdit = (beforeText: string, afterText: string) => {
    const a = beforeText.trim().split(/\s+/).filter(Boolean);
    const b = afterText.trim().split(/\s+/).filter(Boolean);
    if (a.length === 0 || a.length !== b.length) return;
    const changed: number[] = [];
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) changed.push(i);
    if (changed.length !== 1) return;
    const strip = (w: string) => w.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
    const wrong = strip(a[changed[0]]);
    const right = strip(b[changed[0]]);
    if (wrong.length < 2 || !right) return;
    if (wrong.toLowerCase() === right.toLowerCase()) return;
    const existing = settings?.transcript_corrections ?? [];
    if (existing.some((c) => c.wrong.toLowerCase() === wrong.toLowerCase())) return;
    toast(`Teach "${wrong}" → "${right}"?`, {
      description: "Fixes it everywhere and sharpens future transcriptions.",
      action: {
        label: "Teach",
        onClick: () => {
          updateSetting("transcript_corrections", [...existing, { wrong, right }]);
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
      combine(active.you, active.others, active.youLabel, active.othersLabel, visibleSegs(active)) ||
        "(no transcript)",
    ];
    if (isTrimmed(active)) {
      parts.push(
        "",
        `_Trimmed excerpt: ${
          active.trimStartMs != null ? fmtTrimMs(active.trimStartMs) : "start"
        } – ${active.trimEndMs != null ? fmtTrimMs(active.trimEndMs) : "end"} · ${hiddenCount(
          active,
        )} segment(s) hidden._`,
      );
    }
    if (active.processed.trim()) {
      parts.push("", "## Processed", "", active.processed.trim());
      if (notesAreStale(active)) {
        parts.push("", "_These notes were generated before the current trim — re-process to update them._");
      }
    }
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const base = (active.title.trim() || "meeting").replace(/[\\/:*?"<>|]/g, "_");
    try {
      let seed = "";
      try {
        const info = await commands.meetingDirsInfo();
        if (info.status === "ok") seed = JSON.parse(info.data).exportSeedDir ?? "";
      } catch {
        /* seeding is best-effort */
      }
      const chosen = await saveFileDialog({
        title: "Export meeting",
        defaultPath: seed ? `${seed}\\${base}-${stamp}.md` : `${base}-${stamp}.md`,
        filters: [
          { name: "Markdown", extensions: ["md"] },
          { name: "Plain text", extensions: ["txt"] },
        ],
      });
      if (!chosen) return;
      const res = await commands.meetingExportTranscriptTo(chosen, parts.join("\n"));
      if (res.status === "ok") {
        setExportedPath(res.data);
        toast.success("Exported.");
      } else toast.error(`Export failed: ${res.error}`);
    } catch (e) {
      toast.error(`Export failed: ${String(e)}`);
    }
  };

  // ---- storage folders (v1.24.0) ------------------------------------------
  const loadDirs = async () => {
    try {
      const r = await commands.meetingDirsInfo();
      if (r.status === "ok") setDirs(JSON.parse(r.data));
    } catch {
      /* non-fatal */
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
          "Existing recordings stay where they are — use Move existing to relocate them.",
      });
    } catch (e) {
      toast.error(`Couldn't use that folder: ${e instanceof Error ? e.message : String(e)}`);
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
    // The move rewrites every meeting's file paths, so it needs a store that
    // was READ (R-02): otherwise the files would move and the references
    // could not follow, or an empty list could be written over the file.
    if (!storeReadyRef.current) {
      toast.error("Your meetings haven't loaded, so their recordings can't be moved yet. Reopen Kōrero and try again.");
      return;
    }
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
        // Rewrite stored file references and persist at once: the files have
        // ALREADY moved on disk.
        // From the LATEST list (a meeting may have landed during a long move),
        // never from the list as it was when the button was pressed.
        const rewrite = (list: Meeting[]) =>
          list.map((m) => ({
            ...m,
            micPath: m.micPath && map[m.micPath] ? map[m.micPath] : m.micPath,
            systemPath: m.systemPath && map[m.systemPath] ? map[m.systemPath] : m.systemPath,
          }));
        const next = rewrite(meetingsRef.current);
        meetingsRef.current = next;
        setMeetings(rewrite);
        if (storeReadyRef.current) {
          try {
            await meetingsBridge.save(JSON.stringify(next));
          } catch {
            /* autosave remains the fallback */
          }
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
    const gone = meetings.find((m) => m.id === id);
    confirmDestructive(
      `Delete "${gone ? titleOf(gone) : "this meeting"}"?`,
      "Transcript, notes and the meeting audio are removed permanently.",
      "Delete",
      () => {
        if (!mountedRef.current) {
          toast.message("Meetings was closed — open Meetings and delete again.");
          return;
        }
        [gone?.micPath, gone?.systemPath].forEach((p) => {
          if (p) commands.meetingDeleteRecording(p).catch(() => {});
        });
        setMeetings((prev) => {
          const next = prev.filter((m) => m.id !== id);
          if (id === activeIdRef.current) setActiveId(next[0]?.id ?? null);
          return next;
        });
      },
    );
  };

  const transcribeRecording = async (file: RecordingFile) => {
    await useMeetingJobs.getState().recover({
      path: file.path,
      label: file.file_name,
      makeMeeting: (transcript) => {
        const isOthers = /others|system/i.test(file.file_name);
        const m: Meeting = {
          id: newId(),
          title: `Recovered · ${file.file_name}`,
          you: isOthers ? "" : transcript,
          others: isOthers ? transcript : "",
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
        return m as unknown as MeetingDoc;
      },
    });
  };

  const modelOptions: DropdownOption[] = (models ?? []).map((m) => ({
    value: m.id,
    label: m.name,
  }));
  const activeHasAudio = !!active && (!!active.micPath || !!active.systemPath);
  const activeHasTranscript = !!active && (!!active.you.trim() || !!active.others.trim());

  const selectMeeting = (id: string) => {
    activeIdRef.current = id;
    setActiveId(id);
    setView({ kind: "meeting" });
    setHighlight("");
  };

  /** Open a meeting from elsewhere (search, Today, Activity). Safe before the store loads. */
  const focusMeeting = (id: string, focusTab?: MeetingTab, query?: string) => {
    if (storeReadyRef.current && !meetingsRef.current.some((m) => m.id === id)) {
      toast.message("That meeting is no longer in your library.");
      setView({ kind: "meeting" });
      return;
    }
    activeIdRef.current = id;
    setActiveId(id);
    setView({ kind: "meeting" });
    setTab(focusTab ?? (query ? "transcript" : "notes"));
    setHighlight(query ?? "");
  };

  return {
    // data
    meetings,
    setMeetings,
    filteredMeetings,
    active,
    activeId,
    setActiveId,
    selectMeeting,
    focusMeeting,
    view,
    setView,
    tab,
    setTab,
    highlight,
    setHighlight,
    storeReady,
    storeLoadError,
    // jobs
    job,
    task,
    stopping,
    busy,
    busyElapsed,
    busyMeetingId,
    elsewhereJobTitle,
    liveProcessed,
    jobRunningHere,
    transcribeProgress,
    importBusy,
    refining,
    busyFile,
    // models and prompts
    models,
    modelOptions,
    currentModel,
    changeModel,
    customPrompt,
    setCustomPrompt,
    meetingPromptId,
    setMeetingPromptId,
    importPromptId,
    setImportPromptId,
    promptOptions,
    savedPromptText,
    savePromptAsNew,
    providerLocal,
    ppLabel,
    // edits
    patchMeeting,
    editingListId,
    setEditingListId,
    editingLabel,
    setEditingLabel,
    editingSegIdx,
    setEditingSegIdx,
    saveSegment,
    teachWrong,
    setTeachWrong,
    // trim
    trimInDraft,
    setTrimInDraft,
    trimOutDraft,
    setTrimOutDraft,
    trimError,
    setTrimPoint,
    clearTrim,
    commitTrimInputs,
    // search
    search,
    setSearch,
    // actions
    onReTranscribe,
    onPostProcess,
    onBoth,
    mergeWithId,
    setMergeWithId,
    mergeMeetings,
    copyActive,
    copied,
    exportActive,
    exportedPath,
    deleteMeeting,
    activeHasAudio,
    activeHasTranscript,
    // notes
    editingNotes,
    setEditingNotes,
    notesDraft,
    setNotesDraft,
    saveNotes,
    feedback,
    setFeedback,
    refineNotes,
    copyProcessed,
    briefBusy,
    briefUrl: briefFor === activeId ? briefUrl : null,
    briefPath: briefFor === activeId ? briefPath : null,
    genAudioBrief,
    // ask
    askQuestion,
    setAskQuestion,
    askAnswer,
    asking,
    ask,
    // import
    importPath,
    setImportPath,
    importPrompt,
    setImportPrompt,
    pickImportFile,
    startImport,
    runImport,
    // recordings + storage
    recordings,
    loadRecordings,
    deleteRecording,
    transcribeRecording,
    dirs,
    dirBusy,
    changeRecordingDir,
    resetRecordingDir,
    moveExistingRecordings,
  };
};

export type MeetingsController = ReturnType<typeof useMeetingsController>;
