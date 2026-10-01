/* eslint-disable i18next/no-literal-string */
import React, { useState } from "react";
import {
  Copy,
  Pencil,
  Volume2,
  Loader2,
  Sparkles,
  TriangleAlert,
  Download,
  FolderOpen,
  Plus,
  Wand2,
  RotateCcw,
  Cpu,
  ChevronDown,
  ChevronUp,
} from "lucide-react";
import { toast } from "sonner";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Markdown } from "../ui/Markdown";
import { Dropdown } from "../ui/Dropdown";
import type { MeetingsController } from "../settings/meetings/useMeetingsController";
import { notesAreStale, wordCount } from "./model";

/**
 * Kōrero 1.42: the notes tab — the deliverable. The notes lead on a reading
 * surface; the prompt and the model are one line underneath, and stay folded
 * away until you want to change them.
 */
export const NotesPanel: React.FC<{ c: MeetingsController }> = ({ c }) => {
  const m = c.active;
  const [showPrompt, setShowPrompt] = useState(false);
  if (!m) return null;

  const running = c.jobRunningHere;
  const stale = notesAreStale(m);
  const hasNotes = !!m.processed.trim();

  return (
    <div className="flex flex-col gap-4 max-w-[780px]">
      {running && (
        <section aria-live="polite" className="kx-card p-4">
          <div className="flex items-center justify-between mb-2">
            <h2 className="kx-heading flex items-center gap-2">
              <Loader2 size={14} className="animate-spin kx-accent-ink" />
              {running.kind === "refine" ? "Refining the notes" : "Writing the notes"}
            </h2>
            <span className="kx-meta kx-mono">
              {wordCount(running.live).toLocaleString()} words · {c.busyElapsed}s
            </span>
          </div>
          {running.live.trim() ? (
            <div className="md-body kx-read">
              <Markdown>{running.live}</Markdown>
            </div>
          ) : (
            <p className="kx-meta">Waiting for the model to start writing… a local model can take a moment to load.</p>
          )}
          <p className="kx-meta mt-3">You can leave this page; the notes are saved when they finish.</p>
        </section>
      )}

      {!running && !hasNotes && (
        <section className="kx-card p-6 flex flex-col items-start gap-2">
          <h2 className="kx-heading">No notes yet</h2>
          <p className="kx-meta">
            {c.activeHasTranscript
              ? `The notes model reads the transcript and writes decisions, actions and key points. It uses ${c.ppLabel}.`
              : "There is no transcript to summarise yet. Re-transcribe it from the Audio tab."}
          </p>
          <div className="flex gap-2 mt-2">
            <button type="button" className="kx-btn kx-btn-primary" disabled={!!c.busy || !c.activeHasTranscript} onClick={() => void c.onPostProcess()}>
              <Sparkles size={14} /> Write notes
            </button>
            {c.elsewhereJobTitle && (
              <span className="kx-meta self-center">Busy with “{c.elsewhereJobTitle}” — one at a time</span>
            )}
          </div>
        </section>
      )}

      {hasNotes && !running && (
        <>
          {stale && (
            <p role="status" className="kx-banner kx-banner-warn">
              <TriangleAlert size={16} />
              <span>These notes were written before the current trim. Regenerate them to match.</span>
            </p>
          )}
          <div className="flex flex-wrap items-center gap-2">
            <span className="kx-meta flex items-center gap-1.5">
              <Cpu size={13} /> {c.ppLabel}
              {c.providerLocal ? " · on this computer" : c.providerLocal === false ? " · cloud" : ""}
            </span>
            <div className="flex-1" />
            <button type="button" className="kx-btn kx-btn-ghost kx-btn-sm" onClick={() => void c.copyProcessed()}>
              <Copy size={14} /> Copy notes
            </button>
            {!c.editingNotes && (
              <button
                type="button"
                className="kx-btn kx-btn-ghost kx-btn-sm"
                onClick={() => {
                  c.setNotesDraft(m.processed);
                  c.setEditingNotes(true);
                }}
              >
                <Pencil size={14} /> Edit
              </button>
            )}
            <button
              type="button"
              className="kx-btn kx-btn-ghost kx-btn-sm"
              onClick={() => void c.genAudioBrief()}
              disabled={c.briefBusy || stale}
              title={
                stale
                  ? "These notes predate the current trim — regenerate them before rendering audio."
                  : "A spoken summary, rendered on this computer"
              }
            >
              {c.briefBusy ? <Loader2 size={14} className="animate-spin" /> : <Volume2 size={14} />}
              {c.briefBusy ? "Rendering…" : "Audio brief"}
            </button>
          </div>

          {c.editingNotes ? (
            <div className="flex flex-col gap-2">
              <label htmlFor="kx-notes-edit" className="kx-sr-only">
                Edit the notes
              </label>
              <textarea
                id="kx-notes-edit"
                value={c.notesDraft}
                onChange={(e) => c.setNotesDraft(e.target.value)}
                rows={16}
                className="kx-input kx-mono text-[13px]"
              />
              <div className="flex gap-2">
                <button type="button" className="kx-btn kx-btn-primary kx-btn-sm" onClick={c.saveNotes}>
                  Save
                </button>
                <button type="button" className="kx-btn kx-btn-ghost kx-btn-sm" onClick={() => c.setEditingNotes(false)}>
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <article className="kx-card px-6 py-5">
              <div className="md-body kx-read max-w-none">
                <Markdown>{m.processed.trim()}</Markdown>
              </div>
            </article>
          )}

          {c.briefBusy && (
            <p className="kx-meta flex items-center gap-1.5">
              <Loader2 size={12} className="animate-spin" /> Rendering audio on this computer — this can
              take a few minutes for long notes. Leave it running.
            </p>
          )}
          {c.briefUrl && !c.briefBusy && (
            <div className="kx-card p-3 flex flex-col gap-2">
              <audio controls src={c.briefUrl} className="w-full" />
              <div className="flex gap-2">
                <a href={c.briefUrl} download="audio-brief.mp3" className="kx-btn kx-btn-quiet kx-btn-sm">
                  <Download size={13} /> Download
                </a>
                {c.briefPath && (
                  <button
                    type="button"
                    className="kx-btn kx-btn-quiet kx-btn-sm"
                    onClick={() => revealItemInDir(c.briefPath as string).catch(() => toast.error("Could not open the folder."))}
                  >
                    <FolderOpen size={13} /> Show in folder
                  </button>
                )}
              </div>
            </div>
          )}

          {!c.editingNotes && (
            <div className="flex gap-2">
              <label htmlFor="kx-refine" className="kx-sr-only">
                Tell the AI how to improve these notes
              </label>
              <input
                id="kx-refine"
                value={c.feedback}
                onChange={(e) => c.setFeedback(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void c.refineNotes();
                }}
                placeholder="Tell the AI how to improve these notes — e.g. “shorter, and put actions first”"
                disabled={c.refining}
                className="kx-input"
              />
              <button type="button" className="kx-btn kx-btn-secondary shrink-0" disabled={c.refining || !c.feedback.trim() || !!c.busy} onClick={() => void c.refineNotes()}>
                {c.refining ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />}
                {c.refining ? "Refining…" : "Refine"}
              </button>
            </div>
          )}
        </>
      )}

      <section aria-label="Prompt and regenerate" className="kx-card">
        <div className="flex flex-wrap items-center gap-2 px-4 py-3">
          <span className="kx-meta">Prompt</span>
          <Dropdown
            options={c.promptOptions}
            selectedValue={c.meetingPromptId}
            onSelect={(id) => {
              c.setMeetingPromptId(id);
              if (id !== "custom") c.setCustomPrompt(c.savedPromptText(id));
            }}
          />
          <button type="button" className="kx-btn kx-btn-ghost kx-btn-sm" onClick={() => setShowPrompt((v) => !v)} aria-expanded={showPrompt}>
            {showPrompt ? <ChevronUp size={14} /> : <ChevronDown size={14} />} {showPrompt ? "Hide prompt" : "Edit prompt"}
          </button>
          <div className="flex-1" />
          <button
            type="button"
            className="kx-btn kx-btn-secondary kx-btn-sm"
            onClick={() => void c.onPostProcess()}
            disabled={!!c.busy || !c.activeHasTranscript}
            title="Write the notes again from the current transcript"
          >
            <RotateCcw size={13} /> {hasNotes ? "Regenerate" : "Write notes"}
          </button>
          <button
            type="button"
            className="kx-btn kx-btn-secondary kx-btn-sm"
            onClick={() => void c.onBoth()}
            disabled={!!c.busy || !c.activeHasAudio}
            title="Re-transcribe from the recording, then write the notes"
          >
            <Wand2 size={13} /> Re-transcribe + notes
          </button>
        </div>
        {showPrompt && (
          <div className="px-4 pb-4 flex flex-col gap-2">
            <label htmlFor="kx-prompt" className="kx-sr-only">
              Notes prompt for this meeting
            </label>
            <textarea
              id="kx-prompt"
              value={c.customPrompt}
              onChange={(e) => {
                c.setCustomPrompt(e.target.value);
                c.setMeetingPromptId("custom");
              }}
              rows={3}
              className="kx-input"
            />
            <button
              type="button"
              className="kx-btn kx-btn-quiet kx-btn-sm self-start"
              onClick={async () => {
                const id = await c.savePromptAsNew(c.customPrompt);
                if (id) c.setMeetingPromptId(id);
              }}
            >
              <Plus size={13} /> Save as a new prompt
            </button>
          </div>
        )}
        {c.providerLocal === false && (
          <p className="kx-meta px-4 pb-3 flex items-start gap-1.5 text-[var(--kx-warn)]">
            <TriangleAlert size={13} className="mt-0.5 shrink-0" />
            {c.ppLabel} is a cloud model: writing notes sends this transcript off your computer.
            Choose a local model (Ollama) to keep it here.
          </p>
        )}
        {c.busy && c.elsewhereJobTitle && (
          <p className="kx-meta px-4 pb-3">Busy with “{c.elsewhereJobTitle}” — one at a time.</p>
        )}
      </section>
    </div>
  );
};
