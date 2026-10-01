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
import { confirmDestructive } from "../../ui/confirmToast";
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
import i18n from "../../../i18n";
import { formatRelativeTime } from "../../../utils/dateFormat";
import { useSettings } from "../../../hooks/useSettings";
import { noteTitle, useNotes } from "../../../stores/notesStore";
import { useNav } from "../../../stores/navStore";
import { fmtClock } from "../../meetings/model";

/**
 * Kōrero Notes page (v1.12.0; reworked 1.42).
 *
 * A dictation canvas: press Dictate, talk, press Stop, and the words land at
 * your cursor. Since 1.42 the notes, the dictation and the processing run live
 * in `useNotes` (src/stores/notesStore.ts), so leaving this page mid-dictation
 * no longer loses the words or strands the microphone.
 */

const MODE_KEY = "korero.notes.postprocess";
const PROMPT_ID_KEY = "korero.notes.promptId";
const PROMPT_TEXT_KEY = "korero.notes.customPrompt";
const PP_MODEL_KEY = "korero.notes.ppModel";

const readKey = (k: string, fallback = ""): string => {
  try {
    return localStorage.getItem(k) ?? fallback;
  } catch {
    return fallback;
  }
};
const writeKey = (k: string, v: string) => {
  try {
    localStorage.setItem(k, v);
  } catch {
    /* a remembered choice is a convenience */
  }
};

/** Ticks once a second from `startedAt`, so elapsed counters survive remounts. */
const useSecondsSince = (startedAt: number | null): number => {
  const [, force] = useState(0);
  useEffect(() => {
    if (startedAt === null) return;
    const t = window.setInterval(() => force((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [startedAt]);
  return startedAt === null ? 0 : Math.floor((Date.now() - startedAt) / 1000);
};

export const NotesSettings: React.FC = () => {
  const { settings, postProcessModelOptions } = useSettings();
  const ppEnabled = settings?.post_process_enabled ?? false;

  const notes = useNotes((s) => s.notes);
  const activeId = useNotes((s) => s.activeId);
  const dictation = useNotes((s) => s.dictation);
  const dictationNoteId = useNotes((s) => s.dictationNoteId);
  const dictationStartedAt = useNotes((s) => s.dictationStartedAt);
  const processing = useNotes((s) => s.processing);

  const [postProcess, setPostProcess] = useState<boolean>(
    () => readKey(MODE_KEY) === "1",
  );
  const [justCopied, setJustCopied] = useState(false);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [promptId, setPromptId] = useState<string>(() => readKey(PROMPT_ID_KEY));
  const [customPrompt, setCustomPrompt] = useState<string>(() =>
    readKey(PROMPT_TEXT_KEY),
  );
  const [ppModel, setPpModel] = useState<string>(() => readKey(PP_MODEL_KEY));
  const corrections = useCorrections();
  const [teachWrong, setTeachWrong] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<Correction[]>([]);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  // Keep the caret so dictated text lands where the cursor was — clicking
  // Stop moves focus off the textarea.
  const lastCaretRef = useRef<{ start: number; end: number } | null>(null);

  const activeNote = notes.find((n) => n.id === activeId) ?? notes[0];
  const recording = dictation === "recording" || dictation === "starting";
  const finishing = dictation === "finishing";
  const processingHere = processing?.noteId === activeNote.id;
  const cleanupActive = postProcess && ppEnabled;
  const elapsed = useSecondsSince(dictationStartedAt);
  const processElapsed = useSecondsSince(processing?.startedAt ?? null);

  useEffect(() => writeKey(MODE_KEY, postProcess ? "1" : "0"), [postProcess]);
  useEffect(() => writeKey(PROMPT_ID_KEY, promptId), [promptId]);
  useEffect(() => writeKey(PROMPT_TEXT_KEY, customPrompt), [customPrompt]);
  useEffect(() => writeKey(PP_MODEL_KEY, ppModel), [ppModel]);

  // A link from Today, search or Activity can ask for a particular note.
  const navSeq = useNav((s) => s.navSeq);
  useEffect(() => {
    const id = useNav.getState().takeNoteFocus();
    if (id && useNotes.getState().notes.some((n) => n.id === id)) {
      useNotes.getState().setActive(id);
    }
  }, [navSeq]);

  useEffect(() => {
    commands
      .getAvailableModels()
      .then((res) =>
        setModels(res.status === "ok" ? res.data.filter((m) => m.is_downloaded) : []),
      )
      .catch(() => setModels([]));
  }, []);

  useEffect(() => {
    if (!promptId && settings?.post_process_selected_prompt_id) {
      setPromptId(settings.post_process_selected_prompt_id);
    }
  }, [settings?.post_process_selected_prompt_id]);

  // While this page is open it inserts dictation at the cursor; when it is
  // closed the store appends to the end of the note instead.
  useEffect(() => {
    const insert = (noteId: string, text: string): string | null => {
      const st = useNotes.getState();
      if (noteId !== st.activeId) return null;
      const note = st.notes.find((n) => n.id === noteId);
      if (!note) return null;
      const t = text.trim();
      const content = note.content;
      const ta = textareaRef.current;
      let caret: { start: number; end: number } | null = null;
      if (ta && document.activeElement === ta) {
        caret = {
          start: ta.selectionStart ?? content.length,
          end: ta.selectionEnd ?? content.length,
        };
      } else if (lastCaretRef.current) {
        const clamp = (n: number) => Math.max(0, Math.min(n, content.length));
        caret = {
          start: clamp(lastCaretRef.current.start),
          end: clamp(lastCaretRef.current.end),
        };
      }
      let next: string;
      let pos: number;
      if (caret) {
        const before = content.slice(0, caret.start);
        const after = content.slice(caret.end);
        const sep = before && !/\s$/.test(before) ? " " : "";
        next = before + sep + t + after;
        pos = before.length + sep.length + t.length;
      } else {
        const sep = content && !/\s$/.test(content) ? " " : "";
        next = content + sep + t;
        pos = next.length;
      }
      lastCaretRef.current = { start: pos, end: pos };
      st.patchNote(noteId, { content: next });
      if (ta) {
        requestAnimationFrame(() => {
          ta.focus();
          ta.setSelectionRange(pos, pos);
        });
      }
      return next;
    };
    useNotes.getState().setInserter(insert);
    return () => useNotes.getState().setInserter(null);
  }, []);

  const effectivePrompt = (): string => {
    if (promptId === "custom") return customPrompt;
    const found = settings?.post_process_prompts?.find((p) => p.id === promptId);
    return found?.prompt ?? customPrompt ?? "";
  };
  const suggest = (before: string, after: string) =>
    setSuggestions(mineCorrectionSuggestions(before, after, corrections.list));

  const toggleDictation = async () => {
    const st = useNotes.getState();
    if (st.dictation === "recording") {
      await st.stopDictation({
        cleanup: cleanupActive,
        prompt: effectivePrompt(),
        model: ppModel.trim() ? ppModel.trim() : null,
        onSuggest: suggest,
      });
    } else if (st.dictation === "idle") {
      await st.startDictation();
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
    useNotes.getState().addNote();
    requestAnimationFrame(() => textareaRef.current?.focus());
  };

  const wordCount = activeNote.content.trim()
    ? activeNote.content.trim().split(/\s+/).length
    : 0;

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

  // Fetched models win; suggestions are a fallback for remote providers only
  // (v1.35.0: a local provider must never offer models that are not installed).
  const activeProvider = settings?.post_process_providers?.find(
    (p) => p.id === settings?.post_process_provider_id,
  );
  const configuredPpModel =
    (settings?.post_process_models ?? {})[settings?.post_process_provider_id ?? ""] ?? "";
  const fetchedPpModels =
    postProcessModelOptions[settings?.post_process_provider_id ?? ""] ?? [];
  const ppModelCandidates =
    fetchedPpModels.length > 0
      ? fetchedPpModels
      : activeProvider?.is_local_provider
        ? []
        : (activeProvider?.suggested_models ?? []);
  const ppModelOptions: DropdownOption[] = [
    {
      value: "",
      label: configuredPpModel ? `Default (${configuredPpModel})` : "Provider default",
    },
    ...ppModelCandidates
      .filter((m) => m && m !== configuredPpModel)
      .map((m) => ({ value: m, label: m })),
  ];

  const dictatingElsewhere =
    recording && dictationNoteId !== null && dictationNoteId !== activeNote.id;

  return (
    <div className="kx-page kx-page-wide flex flex-col gap-4" style={{ maxWidth: 1180 }}>
      <header>
        <h1 className="kx-title">Notes</h1>
        <p className="kx-meta mt-1">
          Dictate long-form notes, then copy them out. Your words land at the
          cursor; they are not pasted into other apps.
        </p>
      </header>

      <div className="flex gap-4 min-h-0">
        <nav aria-label="Notes" className="kx-card w-60 shrink-0 p-2 flex flex-col gap-1 max-h-[72vh] overflow-y-auto">
          <Button variant="secondary" size="sm" onClick={addNote} className="mb-1 w-full">
            <Plus size={15} /> New note
          </Button>
          {notes.map((n) => {
            const isActive = n.id === activeNote.id;
            return (
              <div key={n.id} className="group relative">
                <button
                  type="button"
                  aria-current={isActive ? "true" : undefined}
                  onClick={() => useNotes.getState().setActive(n.id)}
                  className="kx-list-item pr-9"
                >
                  <span className="block text-[13.5px] truncate">{noteTitle(n)}</span>
                  <span className="kx-meta flex items-center gap-1.5">
                    {dictationNoteId === n.id && recording && (
                      <span className="kx-dot kx-dot-alert" aria-hidden="true" />
                    )}
                    {formatRelativeTime(String(Math.floor(n.updatedAt / 1000)), i18n.language)}
                  </span>
                </button>
                <button
                  type="button"
                  aria-label={`Delete note ${noteTitle(n)}`}
                  title="Delete note"
                  onClick={() => {
                    const del = () => useNotes.getState().deleteNote(n.id);
                    if (!n.content.trim() && !n.title.trim()) del();
                    else
                      confirmDestructive(
                        `Delete "${noteTitle(n)}"?`,
                        "The note is removed permanently.",
                        "Delete",
                        del,
                      );
                  }}
                  className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm absolute right-1.5 top-1.5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                >
                  <Trash2 size={14} />
                </button>
              </div>
            );
          })}
        </nav>

        <section aria-label="Note editor" className="kx-card flex-1 min-w-0 p-4 flex flex-col gap-3">
          <label htmlFor="note-title" className="kx-sr-only">
            Note title
          </label>
          <input
            id="note-title"
            value={activeNote.title}
            onChange={(e) =>
              useNotes.getState().patchNote(activeNote.id, { title: e.target.value })
            }
            placeholder="Note title"
            className="w-full bg-transparent kx-title placeholder:text-[var(--kx-ink-2)] focus:outline-none"
          />

          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant={recording ? "danger" : "primary"}
              onClick={toggleDictation}
              disabled={finishing || dictation === "starting" || dictatingElsewhere || !!processing}
            >
              {finishing ? (
                <>
                  <Loader2 size={15} className="animate-spin" />
                  {cleanupActive ? "Transcribing + cleaning…" : "Transcribing…"}
                </>
              ) : recording ? (
                <>
                  <Square size={13} fill="currentColor" /> Stop ·{" "}
                  <span className="kx-mono">{fmtClock(elapsed)}</span>
                </>
              ) : (
                <>
                  <Mic size={15} /> Dictate
                </>
              )}
            </Button>
            {recording && (
              <Button variant="ghost" onClick={() => useNotes.getState().cancelDictation()}>
                <X size={14} /> Cancel
              </Button>
            )}

            <div role="group" aria-label="Dictation mode" className="flex rounded-[9px] overflow-hidden border border-[var(--kx-control)]">
              <button
                type="button"
                aria-pressed={!postProcess}
                onClick={() => setPostProcess(false)}
                className={`px-3 min-h-[34px] text-[13px] ${!postProcess ? "bg-[var(--kx-selected)] text-[var(--kx-accent-ink)] font-semibold" : "text-[var(--kx-ink-soft)] hover:bg-white/5"}`}
              >
                Transcribe
              </button>
              <button
                type="button"
                aria-pressed={postProcess}
                onClick={() => setPostProcess(true)}
                disabled={!ppEnabled}
                title={
                  ppEnabled
                    ? "Transcribe, then clean up the WHOLE note with the selected prompt and model"
                    : "Turn on AI clean-up to use this"
                }
                className={`px-3 min-h-[34px] text-[13px] disabled:opacity-40 disabled:cursor-not-allowed ${postProcess ? "bg-[var(--kx-selected)] text-[var(--kx-accent-ink)] font-semibold" : "text-[var(--kx-ink-soft)] hover:bg-white/5"}`}
              >
                Transcribe + clean up
              </button>
            </div>

            <div className="ml-auto flex items-center gap-3">
              <span className="kx-meta kx-mono">{wordCount} words</span>
              <Button variant="secondary" onClick={copyNote} disabled={!activeNote.content.trim()}>
                {justCopied ? <Check size={15} /> : <Copy size={15} />}
                {justCopied ? "Copied" : "Copy"}
              </Button>
            </div>
          </div>

          {dictatingElsewhere && (
            <p className="kx-banner kx-banner-info">
              A dictation is running in another note. It will land there when you stop it.
            </p>
          )}
          {postProcess && !ppEnabled && (
            <p className="kx-banner kx-banner-warn">
              Clean-up needs AI clean-up turned on. Until then, notes are transcribed only.
            </p>
          )}

          {/* 1.42 polish: labelled fields that wrap as whole columns, instead of
              labels and dropdowns wrapping onto separate lines. */}
          <div className="kx-fields">
            <div className="kx-field">
              <span className="kx-overline">Speech model</span>
              <Dropdown
                className="w-full"
                options={modelOptions}
                selectedValue={currentModel}
                onSelect={changeModel}
                disabled={!models || models.length === 0 || recording}
              />
            </div>
            {ppEnabled && (
              <>
                <div className="kx-field">
                  <span className="kx-overline">Prompt</span>
                  <Dropdown
                    className="w-full"
                    options={promptOptions}
                    selectedValue={promptId}
                    onSelect={setPromptId}
                    disabled={!!processing}
                  />
                </div>
                <div className="kx-field">
                  <span className="kx-overline">AI model</span>
                  <Dropdown
                    className="w-full"
                    options={ppModelOptions}
                    selectedValue={ppModel}
                    onSelect={setPpModel}
                    disabled={!!processing}
                  />
                </div>
                <div className="kx-field items-start">
                  <Button
                    variant="secondary"
                    onClick={() =>
                      useNotes.getState().processNote({
                        noteId: activeNote.id,
                        content: activeNote.content,
                        prompt: effectivePrompt(),
                        model: ppModel.trim() ? ppModel.trim() : null,
                        onSuggest: suggest,
                      })
                    }
                    disabled={!!processing || finishing || recording || !activeNote.content.trim()}
                    title="Run the selected prompt and AI model over the whole note (Undo available)"
                  >
                    {processingHere ? (
                      <>
                        <Loader2 size={15} className="animate-spin" /> Processing…{" "}
                        <span className="kx-mono">{processElapsed}s</span>
                      </>
                    ) : (
                      <>
                        <Wand2 size={15} /> Process note
                      </>
                    )}
                  </Button>
                </div>
              </>
            )}
          </div>
          {ppEnabled && promptId === "custom" && (
            <>
              <label htmlFor="note-prompt" className="kx-sr-only">
                Custom processing prompt
              </label>
              <textarea
                id="note-prompt"
                value={customPrompt}
                onChange={(e) => setCustomPrompt(e.target.value)}
                rows={2}
                placeholder="Custom processing prompt — e.g. 'Rewrite this note as a client-ready email, NZ English.' Leave empty for a standard clean-up."
                className="kx-input"
              />
            </>
          )}

          {teachWrong === null ? (
            <button
              type="button"
              onClick={() => {
                const ta = textareaRef.current;
                const s = ta?.selectionStart ?? 0;
                const e = ta?.selectionEnd ?? 0;
                setTeachWrong(activeNote.content.slice(s, e).trim().slice(0, 80));
              }}
              className="kx-btn kx-btn-quiet kx-btn-sm self-start"
              title="Select a mis-transcribed word in the note first, then click to teach the correction"
            >
              <GraduationCap size={14} /> Teach a correction
            </button>
          ) : (
            <AddCorrectionInline initialWrong={teachWrong} onDone={() => setTeachWrong(null)} />
          )}

          {suggestions.length > 0 && (
            <div className="flex flex-wrap items-center gap-2 text-xs">
              <span className="kx-meta">The clean-up suggests teaching:</span>
              {suggestions.map((sug, i) => (
                <span key={`${sug.wrong}-${i}`} className="kx-chip">
                  <span className="line-through opacity-70">{sug.wrong}</span>
                  <span className="text-white">{sug.right}</span>
                  <button
                    type="button"
                    aria-label={`Teach ${sug.wrong} as ${sug.right}`}
                    onClick={() => {
                      corrections.add(sug.wrong, sug.right);
                      setSuggestions((prev) => prev.filter((_, j) => j !== i));
                    }}
                    className="kx-accent-ink"
                  >
                    <Plus size={12} />
                  </button>
                </span>
              ))}
              <button
                type="button"
                aria-label="Dismiss suggestions"
                onClick={() => setSuggestions([])}
                className="kx-ink-2 hover:text-white"
              >
                <X size={12} />
              </button>
            </div>
          )}

          <label htmlFor="note-body" className="kx-sr-only">
            Note
          </label>
          <textarea
            id="note-body"
            ref={textareaRef}
            value={activeNote.content}
            onChange={(e) =>
              useNotes.getState().patchNote(activeNote.id, { content: e.target.value })
            }
            onSelect={(e) => {
              const ta = e.currentTarget;
              lastCaretRef.current = { start: ta.selectionStart, end: ta.selectionEnd };
            }}
            onBlur={(e) => {
              const ta = e.currentTarget;
              lastCaretRef.current = { start: ta.selectionStart, end: ta.selectionEnd };
            }}
            // Read-only while the model rewrites the note: edits made during the
            // run would be overwritten when the result lands.
            readOnly={processingHere}
            placeholder="Start dictating, or type here. Your words land at the cursor."
            spellCheck
            className={`kx-read w-full max-w-none flex-1 min-h-[340px] resize-none bg-transparent placeholder:text-[var(--kx-ink-2)] focus:outline-none ${
              processingHere ? "opacity-60 cursor-wait" : ""
            }`}
          />
        </section>
      </div>
    </div>
  );
};
