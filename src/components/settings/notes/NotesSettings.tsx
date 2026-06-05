/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import {
  Mic,
  Square,
  Loader2,
  Copy,
  Plus,
  Trash2,
  Check,
  Wand2,
  GraduationCap,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Button } from "../../ui/Button";
import { Dropdown, type DropdownOption } from "../../ui/Dropdown";
import {
  AddCorrectionInline,
  mineCorrectionSuggestions,
  useCorrections,
  type Correction,
} from "../../ui/Corrections";
import { commands, type ModelInfo } from "../../../bindings";
import { useSettings } from "../../../hooks/useSettings";

/**
 * Kōrero fork (v1.12.0): Notes page.
 *
 * A dedicated dictation canvas. Press Dictate, ramble, press again to stop —
 * the text is transcribed (optionally cleaned up by the post-processing prompt)
 * and inserted at the cursor, rather than pasted into another app. Copy the
 * note out when finished. Notes persist in the webview store so they survive
 * restarts; nothing is sent anywhere.
 */

interface Note {
  id: string;
  title: string;
  content: string;
  updatedAt: number;
}

const STORE_KEY = "korero.notes.v1";
const MODE_KEY = "korero.notes.postprocess";
// v1.14.3: per-page processing choices persist across restarts.
const PROMPT_ID_KEY = "korero.notes.promptId";
const PROMPT_TEXT_KEY = "korero.notes.customPrompt";
const PP_MODEL_KEY = "korero.notes.ppModel";

const newId = () =>
  (crypto as any)?.randomUUID?.() ?? `n_${Date.now()}_${Math.random()}`;

const blankNote = (): Note => ({
  id: newId(),
  title: "",
  content: "",
  updatedAt: Date.now(),
});

const loadNotes = (): Note[] => {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.length) return parsed as Note[];
    }
  } catch {
    /* ignore corrupt store */
  }
  return [blankNote()];
};

const titleOf = (n: Note) => {
  if (n.title.trim()) return n.title.trim();
  const firstLine = n.content.split("\n").find((l) => l.trim());
  return firstLine ? firstLine.trim().slice(0, 40) : "Untitled note";
};

const fmtTime = (s: number) => {
  const m = Math.floor(s / 60);
  const sec = s % 60;
  return `${m}:${sec.toString().padStart(2, "0")}`;
};

export const NotesSettings: React.FC = () => {
  const { settings } = useSettings();
  const ppEnabled = settings?.post_process_enabled ?? false;

  const [notes, setNotes] = useState<Note[]>(() => loadNotes());
  const [activeId, setActiveId] = useState<string>(() => loadNotes()[0].id);
  const [postProcess, setPostProcess] = useState<boolean>(
    () => localStorage.getItem(MODE_KEY) === "1",
  );
  const [recording, setRecording] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [justCopied, setJustCopied] = useState(false);
  // v1.14.3: whole-note processing — transcription model picker, per-run
  // prompt + AI-model choice, busy state, and a one-step undo snapshot.
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [promptId, setPromptId] = useState<string>(
    () => localStorage.getItem(PROMPT_ID_KEY) ?? "",
  );
  const [customPrompt, setCustomPrompt] = useState<string>(
    () => localStorage.getItem(PROMPT_TEXT_KEY) ?? "",
  );
  const [ppModel, setPpModel] = useState<string>(
    () => localStorage.getItem(PP_MODEL_KEY) ?? "",
  );
  const [processingNote, setProcessingNote] = useState(false);
  // v1.15.1: visible elapsed seconds while the model rewrites the note —
  // whole-note processing time scales with note length and model speed, so
  // show the cost instead of an anonymous spinner.
  const [processElapsed, setProcessElapsed] = useState(0);
  // v1.15.0: corrections memory — teach form + suggestions mined from the
  // most recent whole-note clean-up.
  const corrections = useCorrections();
  const [teachWrong, setTeachWrong] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<Correction[]>([]);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const timerRef = useRef<number | null>(null);
  // Snapshot for Undo after a whole-note rewrite — keyed by note id so a
  // note switch between process and undo can't clobber the wrong note.
  const lastSnapshotRef = useRef<{ id: string; content: string } | null>(null);

  // Post-processing must be enabled globally for clean-up mode to do anything.
  const cleanupActive = postProcess && ppEnabled;
  const activeNote = notes.find((n) => n.id === activeId) ?? notes[0];

  // Persist notes + mode. v1.14.4: debounced — previously the whole notes
  // array was stringified on EVERY keystroke (main-thread jank as notes grow).
  useEffect(() => {
    const t = window.setTimeout(() => {
      try {
        localStorage.setItem(STORE_KEY, JSON.stringify(notes));
      } catch {
        /* storage full / unavailable — keep working in memory */
      }
    }, 500);
    return () => window.clearTimeout(t);
  }, [notes]);

  useEffect(() => {
    localStorage.setItem(MODE_KEY, postProcess ? "1" : "0");
  }, [postProcess]);

  // v1.14.3: persist processing choices.
  useEffect(() => {
    localStorage.setItem(PROMPT_ID_KEY, promptId);
  }, [promptId]);
  useEffect(() => {
    localStorage.setItem(PROMPT_TEXT_KEY, customPrompt);
  }, [customPrompt]);
  useEffect(() => {
    localStorage.setItem(PP_MODEL_KEY, ppModel);
  }, [ppModel]);

  // v1.14.3: downloaded transcription models for the picker (same pattern as
  // Meetings — selecting one sets the app-wide active model).
  useEffect(() => {
    commands
      .getAvailableModels()
      .then((res) => {
        if (res.status === "ok") {
          setModels(res.data.filter((m) => m.is_downloaded));
        } else {
          setModels([]);
        }
      })
      .catch(() => setModels([]));
  }, []);

  // Default the prompt picker to the globally selected post-process prompt
  // once settings arrive (only when nothing was persisted).
  useEffect(() => {
    if (!promptId && settings?.post_process_selected_prompt_id) {
      setPromptId(settings.post_process_selected_prompt_id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings?.post_process_selected_prompt_id]);

  // Recording timer.
  useEffect(() => {
    if (recording) {
      setElapsed(0);
      timerRef.current = window.setInterval(
        () => setElapsed((e) => e + 1),
        1000,
      );
    } else if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
    return () => {
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
    };
  }, [recording]);

  const patchActive = (patch: Partial<Note>) => {
    setNotes((prev) =>
      prev.map((n) =>
        n.id === activeNote.id ? { ...n, ...patch, updatedAt: Date.now() } : n,
      ),
    );
  };

  // v1.14.3: returns the note's NEW content so callers can chain whole-note
  // processing without waiting on React state.
  const insertText = (text: string): string => {
    const t = text.trim();
    const content = activeNote.content;
    if (!t) return content;
    const ta = textareaRef.current;
    if (ta && document.activeElement === ta) {
      const s = ta.selectionStart ?? content.length;
      const e = ta.selectionEnd ?? content.length;
      const before = content.slice(0, s);
      const after = content.slice(e);
      const sep = before && !/\s$/.test(before) ? " " : "";
      const inserted = sep + t;
      const next = before + inserted + after;
      const caret = before.length + inserted.length;
      patchActive({ content: next });
      requestAnimationFrame(() => {
        ta.focus();
        ta.setSelectionRange(caret, caret);
      });
      return next;
    }
    const sep = content && !/\s$/.test(content) ? " " : "";
    const next = content + sep + t;
    patchActive({ content: next });
    return next;
  };

  // ---- whole-note processing (v1.14.3) -------------------------------------

  // The prompt actually sent: a saved post-processing prompt, or the custom
  // text. Empty falls back to the Rust-side default clean-up prompt.
  const effectivePrompt = (): string => {
    if (promptId === "custom") return customPrompt;
    const found = settings?.post_process_prompts?.find((p) => p.id === promptId);
    return found?.prompt ?? customPrompt ?? "";
  };

  const applyToNote = (id: string, content: string) =>
    setNotes((prev) =>
      prev.map((n) =>
        n.id === id ? { ...n, content, updatedAt: Date.now() } : n,
      ),
    );

  const undoProcess = () => {
    const snap = lastSnapshotRef.current;
    if (!snap) return;
    applyToNote(snap.id, snap.content);
    lastSnapshotRef.current = null;
    toast.message("Note restored.");
  };

  /// Run the selected prompt (+ optional model override) over the whole note.
  const processNoteText = async (noteId: string, content: string) => {
    const text = content.trim();
    if (!text || processingNote) return;
    setProcessingNote(true);
    setProcessElapsed(0);
    const startedAt = Date.now();
    const tick = window.setInterval(
      () => setProcessElapsed(Math.floor((Date.now() - startedAt) / 1000)),
      1000,
    );
    try {
      const res = await commands.notePostProcess(
        text,
        effectivePrompt(),
        ppModel.trim() ? ppModel.trim() : null,
      );
      if (res.status === "ok") {
        const out = res.data.trim();
        if (!out) {
          toast.error("The model returned no output — note unchanged.");
          return;
        }
        lastSnapshotRef.current = { id: noteId, content };
        applyToNote(noteId, out);
        // v1.15.0: mine conservative wrong → right suggestions from what the
        // clean-up changed, so repeat mistakes become permanent fixes.
        setSuggestions(mineCorrectionSuggestions(content, out, corrections.list));
        toast.success("Note processed.", {
          action: { label: "Undo", onClick: undoProcess },
          duration: 8000,
        });
      } else {
        toast.error(res.error);
      }
    } catch (e) {
      toast.error(String(e));
    } finally {
      window.clearInterval(tick);
      setProcessingNote(false);
    }
  };

  const toggleDictation = async () => {
    if (processing || processingNote) return;
    if (recording) {
      setRecording(false);
      setProcessing(true);
      try {
        // v1.14.3: always take the RAW transcript — clean-up now applies to
        // the whole note (below), not just the dictated snippet.
        const res = await commands.noteStopDictation(false);
        if (res.status === "ok") {
          if (res.data && res.data.trim()) {
            const noteId = activeNote.id;
            const updated = insertText(res.data);
            if (cleanupActive) {
              await processNoteText(noteId, updated);
            }
          } else {
            toast.message("No speech detected.");
          }
        } else {
          toast.error(`Dictation failed: ${res.error}`);
        }
      } catch (e) {
        toast.error(`Dictation failed: ${String(e)}`);
      } finally {
        setProcessing(false);
      }
    } else {
      try {
        const res = await commands.noteStartDictation();
        if (res.status === "ok") {
          setRecording(true);
        } else {
          toast.error(res.error);
        }
      } catch (e) {
        toast.error(`Could not start dictation: ${String(e)}`);
      }
    }
  };

  const copyNote = async () => {
    if (!activeNote.content.trim()) return;
    try {
      await writeText(activeNote.content);
      setJustCopied(true);
      window.setTimeout(() => setJustCopied(false), 1500);
    } catch {
      toast.error("Could not copy to clipboard.");
    }
  };

  const addNote = () => {
    const n = blankNote();
    setNotes((prev) => [n, ...prev]);
    setActiveId(n.id);
    requestAnimationFrame(() => textareaRef.current?.focus());
  };

  const deleteNote = (id: string) => {
    setNotes((prev) => {
      const next = prev.filter((n) => n.id !== id);
      const list = next.length ? next : [blankNote()];
      if (id === activeId) setActiveId(list[0].id);
      return list;
    });
  };

  const wordCount = activeNote.content.trim()
    ? activeNote.content.trim().split(/\s+/).length
    : 0;

  // ---- picker options (v1.14.3) --------------------------------------------
  const currentModel = settings?.selected_model ?? "";
  const modelOptions: DropdownOption[] = (models ?? []).map((m) => ({
    value: m.id,
    label: m.name,
  }));
  const changeModel = async (id: string) => {
    try {
      const res = await commands.setActiveModel(id);
      if (res.status === "error") toast.error(res.error);
    } catch (e) {
      toast.error(String(e));
    }
  };

  const promptOptions: DropdownOption[] = [
    ...(settings?.post_process_prompts ?? []).map((p) => ({
      value: p.id,
      label: p.name,
    })),
    { value: "custom", label: "Custom prompt…" },
  ];

  // AI model for processing: "" = the model configured for the provider under
  // Post Process; otherwise one of the provider's suggested models.
  const activeProvider = settings?.post_process_providers?.find(
    (p) => p.id === settings?.post_process_provider_id,
  );
  const configuredPpModel =
    (settings?.post_process_models ?? {})[
      settings?.post_process_provider_id ?? ""
    ] ?? "";
  const ppModelOptions: DropdownOption[] = [
    {
      value: "",
      label: configuredPpModel
        ? `Default (${configuredPpModel})`
        : "Provider default",
    },
    ...(activeProvider?.suggested_models ?? [])
      .filter((m) => m && m !== configuredPpModel)
      .map((m) => ({ value: m, label: m })),
  ];

  return (
    <div className="max-w-4xl w-full mx-auto space-y-4">
      <div className="px-1">
        <h1 className="text-lg font-semibold text-text">Notes</h1>
        <p className="text-sm text-text-subtle mt-1">
          Dictate long-form notes hands-free, then copy them out. Text is
          inserted at your cursor — it is not pasted into other apps.
        </p>
      </div>

      <div className="flex gap-4">
        {/* Saved notes list */}
        <div className="w-56 shrink-0 glass-card p-2 flex flex-col gap-1 max-h-[70vh] overflow-y-auto">
          <Button
            variant="secondary"
            size="sm"
            onClick={addNote}
            className="mb-1 flex items-center justify-center gap-1.5"
          >
            <Plus size={15} /> New note
          </Button>
          {notes.map((n) => {
            const isActive = n.id === activeNote.id;
            return (
              <div
                key={n.id}
                onClick={() => setActiveId(n.id)}
                className={`group rounded-lg px-3 py-2 cursor-pointer transition-colors ${
                  isActive ? "bg-glass-accent-strong" : "hover:bg-white/5"
                }`}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="text-sm text-text truncate">
                    {titleOf(n)}
                  </span>
                  <button
                    type="button"
                    title="Delete note"
                    onClick={(e) => {
                      e.stopPropagation();
                      deleteNote(n.id);
                    }}
                    className="opacity-0 group-hover:opacity-100 text-text-subtle hover:text-pill-urgent transition-opacity"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
                <span className="text-xs text-text-subtle">
                  {new Date(n.updatedAt).toLocaleDateString()}
                </span>
              </div>
            );
          })}
        </div>

        {/* Editor */}
        <div className="flex-1 min-w-0 glass-card p-4 flex flex-col gap-3">
          <input
            value={activeNote.title}
            onChange={(e) => patchActive({ title: e.target.value })}
            placeholder="Note title"
            className="w-full bg-transparent text-base font-medium text-text placeholder:text-text-subtle focus:outline-none"
          />

          {/* Dictation controls */}
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant={recording ? "danger" : "primary"}
              size="md"
              onClick={toggleDictation}
              disabled={processing}
              className="flex items-center gap-2"
            >
              {processing ? (
                <>
                  <Loader2 size={16} className="animate-spin" />{" "}
                  {cleanupActive ? "Transcribing + cleaning…" : "Transcribing…"}
                </>
              ) : recording ? (
                <>
                  <Square size={15} /> Stop · {fmtTime(elapsed)}
                </>
              ) : (
                <>
                  <Mic size={16} /> Dictate
                </>
              )}
            </Button>

            {/* Mode toggle */}
            <div className="flex rounded-lg overflow-hidden border border-glass-border">
              <button
                type="button"
                onClick={() => setPostProcess(false)}
                className={`px-3 py-[5px] text-sm transition-colors ${
                  !postProcess
                    ? "bg-glass-accent-strong text-text"
                    : "text-text-muted hover:bg-white/5"
                }`}
              >
                Transcribe
              </button>
              <button
                type="button"
                onClick={() => setPostProcess(true)}
                disabled={!ppEnabled}
                title={
                  ppEnabled
                    ? "Transcribe, then clean up the WHOLE note with the selected prompt and model"
                    : "Enable post-processing in General to use clean-up"
                }
                className={`px-3 py-[5px] text-sm transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
                  postProcess
                    ? "bg-glass-accent-strong text-text"
                    : "text-text-muted hover:bg-white/5"
                }`}
              >
                Transcribe + clean up
              </button>
            </div>

            <div className="ml-auto flex items-center gap-3">
              <span className="text-xs text-text-subtle">{wordCount} words</span>
              <Button
                variant="secondary"
                size="md"
                onClick={copyNote}
                disabled={!activeNote.content.trim()}
                className="flex items-center gap-1.5"
              >
                {justCopied ? (
                  <>
                    <Check size={15} /> Copied
                  </>
                ) : (
                  <>
                    <Copy size={15} /> Copy
                  </>
                )}
              </Button>
            </div>
          </div>

          {postProcess && !ppEnabled && (
            <p className="text-xs text-pill-warning">
              Clean-up needs post-processing turned on in General — falling back
              to plain transcription until then.
            </p>
          )}

          {/* v1.14.3: processing controls — transcription model (future
              dictations), the prompt + AI model used for whole-note
              processing, and a re-runnable Process note action with Undo. */}
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs text-text-subtle">Model</span>
            <Dropdown
              options={modelOptions}
              selectedValue={currentModel}
              onSelect={changeModel}
              disabled={!models || models.length === 0 || recording}
            />
            {ppEnabled && (
              <>
                <span className="text-xs text-text-subtle ml-2">Prompt</span>
                <Dropdown
                  options={promptOptions}
                  selectedValue={promptId}
                  onSelect={setPromptId}
                  disabled={processingNote}
                />
                <span className="text-xs text-text-subtle ml-2">AI model</span>
                <Dropdown
                  options={ppModelOptions}
                  selectedValue={ppModel}
                  onSelect={setPpModel}
                  disabled={processingNote}
                />
                <Button
                  variant="secondary"
                  size="md"
                  onClick={() =>
                    processNoteText(activeNote.id, activeNote.content)
                  }
                  disabled={
                    processingNote ||
                    processing ||
                    recording ||
                    !activeNote.content.trim()
                  }
                  className="flex items-center gap-1.5"
                  title="Run the selected prompt and AI model over the whole note (Undo available)"
                >
                  {processingNote ? (
                    <>
                      <Loader2 size={15} className="animate-spin" /> Processing…{" "}
                      {processElapsed}s
                    </>
                  ) : (
                    <>
                      <Wand2 size={15} /> Process note
                    </>
                  )}
                </Button>
              </>
            )}
          </div>
          {ppEnabled && promptId === "custom" && (
            <textarea
              value={customPrompt}
              onChange={(e) => setCustomPrompt(e.target.value)}
              rows={2}
              placeholder="Custom processing prompt — e.g. 'Rewrite this note as a client-ready email, NZ English.' Leave empty for a standard clean-up."
              className="w-full bg-white/5 border border-white/10 rounded-md px-2 py-1.5 text-xs text-text placeholder:text-text-subtle focus:outline-none resize-none"
            />
          )}

          {/* v1.15.0: teach the transcriber — select a mis-heard word in the
              note, then click Teach. */}
          {teachWrong === null ? (
            <button
              type="button"
              onClick={() => {
                const ta = textareaRef.current;
                const s = ta?.selectionStart ?? 0;
                const e = ta?.selectionEnd ?? 0;
                setTeachWrong(
                  activeNote.content.slice(s, e).trim().slice(0, 80),
                );
              }}
              className="self-start text-xs text-text-subtle hover:text-aurora-cyan transition-colors flex items-center gap-1.5"
              title="Select a mis-transcribed word in the note first, then click to teach the correction"
            >
              <GraduationCap size={13} /> Teach a correction
            </button>
          ) : (
            <AddCorrectionInline
              initialWrong={teachWrong}
              onDone={() => setTeachWrong(null)}
            />
          )}

          {/* v1.15.0: suggestions mined from the latest clean-up. */}
          {suggestions.length > 0 && (
            <div className="flex flex-wrap items-center gap-2 text-xs">
              <span className="text-text-subtle">
                The clean-up suggests teaching:
              </span>
              {suggestions.map((sug, i) => (
                <span
                  key={`${sug.wrong}-${i}`}
                  className="flex items-center gap-1.5 bg-white/5 border border-white/10 rounded-full px-2.5 py-1"
                >
                  <span className="text-text-muted line-through">
                    {sug.wrong}
                  </span>
                  <span className="text-text">{sug.right}</span>
                  <button
                    type="button"
                    title="Teach this correction"
                    onClick={() => {
                      corrections.add(sug.wrong, sug.right);
                      setSuggestions((prev) => prev.filter((_, j) => j !== i));
                    }}
                    className="text-aurora-cyan hover:opacity-75"
                  >
                    <Plus size={12} />
                  </button>
                </span>
              ))}
              <button
                type="button"
                title="Dismiss suggestions"
                onClick={() => setSuggestions([])}
                className="text-text-subtle hover:text-text"
              >
                <X size={12} />
              </button>
            </div>
          )}

          {/* v1.14.4: read-only while the model rewrites the note — edits made
              during the (potentially long) LLM call would be silently
              overwritten when the processed result lands. */}
          <textarea
            ref={textareaRef}
            value={activeNote.content}
            onChange={(e) => patchActive({ content: e.target.value })}
            readOnly={processingNote}
            placeholder="Start dictating, or type here. Your words land at the cursor."
            spellCheck
            className={`w-full flex-1 min-h-[320px] resize-none bg-transparent text-sm text-text leading-relaxed placeholder:text-text-subtle focus:outline-none ${
              processingNote ? "opacity-60 cursor-wait" : ""
            }`}
          />
        </div>
      </div>
    </div>
  );
};
