/* eslint-disable i18next/no-literal-string */
import React, { useRef, useState } from "react";
import { Loader2, FileUp, Volume2, Wand2 } from "lucide-react";
import { toast } from "sonner";
import { convertFileSrc } from "@tauri-apps/api/core";
import { commands } from "@/bindings";
import { Dropdown } from "@/components/ui/Dropdown";

/**
 * Kōrero (v1.21.0): Audio Brief.
 *
 * Pipeline (all on-device): paste or import text → the local post-processing
 * model (Gemma via Ollama) drafts a SPOKEN script → the local Qwen3-TTS engine
 * renders it to an MP3. Two explicit steps so the script can be edited before
 * it's spoken. Reuses the existing meetingPostProcess (Gemma) and
 * meetingGenerateAudioBrief (Qwen) commands — no transcript leaves the machine.
 */

// Shared rules every format obeys (audio is linear; write for the ear).
const BASE_RULES =
  " Write numbers as words (e.g. three hundred and eighty-three dollars); expand abbreviations on first use; remove URLs, reference codes, IDs and emoji; short one-idea sentences; NZ English; no titles, headings, or speaker labels. Return ONLY the script text.";

// Template formats for the spoken script — each is a Gemma instruction.
const TEMPLATES: { id: string; label: string; prompt: string }[] = [
  {
    id: "standard",
    label: "Standard brief",
    prompt:
      "Rewrite the text below into a short SPOKEN audio brief (150-300 words): lead with the single most important point, then supporting detail, then a clean one-line close." +
      BASE_RULES,
  },
  {
    id: "executive",
    label: "Executive summary",
    prompt:
      "Rewrite the text below into a 120-200 word SPOKEN executive summary: the decision or headline first, then the two or three things that matter and any risk, then what happens next." +
      BASE_RULES,
  },
  {
    id: "actions",
    label: "Action items",
    prompt:
      "From the text below, produce a SPOKEN run-through of the action items: for each, say who does what by when. Open with a one-line context sentence. Under 200 words." +
      BASE_RULES,
  },
  {
    id: "insights",
    label: "Key insights",
    prompt:
      "From the text below, give a SPOKEN rundown of the three or four key insights, each a sentence or two, then one 'so what' line on why it matters. Under 220 words." +
      BASE_RULES,
  },
  {
    id: "narrative",
    label: "Narrative",
    prompt:
      "Retell the text below as a short SPOKEN narrative that flows naturally when heard, about 200-300 words, with a clear beginning, middle and end." +
      BASE_RULES,
  },
];

// Preset Qwen3-TTS speakers (plus the engine's default designed voice).
const SPEAKERS = [
  "Default",
  "Serena",
  "Aiden",
  "Ryan",
  "Vivian",
  "Sohee",
  "Ono Anna",
  "Dylan",
  "Eric",
  "Uncle Fu",
];

// Pace presets → engine --tempo value.
const TEMPOS: { label: string; value: number }[] = [
  { label: "Slower", value: 0.95 },
  { label: "Normal", value: 1.0 },
  { label: "Brisk", value: 1.12 },
  { label: "Fast", value: 1.25 },
];

export const AudioBriefSettings: React.FC = () => {
  const [source, setSource] = useState("");
  const [transcript, setTranscript] = useState("");
  const [drafting, setDrafting] = useState(false);
  const [rendering, setRendering] = useState(false);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [templateId, setTemplateId] = useState(TEMPLATES[0].id);
  const [speaker, setSpeaker] = useState(SPEAKERS[0]); // "Default"
  const [tempo, setTempo] = useState(1.12); // "Brisk"

  const fileInputRef = useRef<HTMLInputElement>(null);

  // Import via a native <input type=file> + FileReader (pure web). Avoids the
  // Tauri fs-plugin scope handling for arbitrary user-picked paths.
  const onFilePicked = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      setSource(String(reader.result ?? ""));
      toast.success("Imported text.");
    };
    reader.onerror = () => toast.error("Could not read that file.");
    reader.readAsText(file);
    e.target.value = ""; // let the same file be re-imported
  };

  const draftTranscript = async () => {
    const text = source.trim();
    if (!text) {
      toast.message("Paste or import some text first.");
      return;
    }
    if (drafting) return;
    setDrafting(true);
    try {
      const tpl = TEMPLATES.find((t) => t.id === templateId) ?? TEMPLATES[0];
      const r = await commands.meetingPostProcess(text, tpl.prompt);
      if (r.status !== "ok") throw new Error(r.error);
      setTranscript(r.data.trim());
      setAudioUrl(null);
      toast.success("Draft ready — edit it, then render audio.");
    } catch (e) {
      toast.error(`Draft failed: ${String(e)}`);
    } finally {
      setDrafting(false);
    }
  };

  const renderAudio = async () => {
    const text = transcript.trim();
    if (!text) {
      toast.message("Draft or write a script first.");
      return;
    }
    if (rendering) return;
    setRendering(true);
    setAudioUrl(null);
    try {
      const r = await commands.meetingGenerateAudioBrief(
        text,
        speaker === "Default" ? null : speaker,
        tempo,
      );
      if (r.status !== "ok") throw new Error(r.error);
      setAudioUrl(convertFileSrc(r.data, "asset"));
      toast.success("Audio brief ready.");
    } catch (e) {
      toast.error(`Audio render failed: ${String(e)}`);
    } finally {
      setRendering(false);
    }
  };

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-4">
      <div>
        <h2 className="text-lg font-semibold text-text">Audio brief</h2>
        <p className="mt-1 text-xs text-text-subtle">
          Paste or import text. The local model drafts a spoken script (Gemma),
          then renders it to audio on-device (Qwen3-TTS). Nothing leaves your
          machine.
        </p>
      </div>

      {/* Source */}
      <div className="space-y-1">
        <div className="flex items-center justify-between">
          <label className="text-xs text-text-subtle">Source text</label>
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            className="flex items-center gap-1 text-xs text-text-subtle transition-colors hover:text-aurora-cyan"
          >
            <FileUp size={13} /> Import .txt / .md
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept=".txt,.md,.markdown,text/plain,text/markdown"
            onChange={onFilePicked}
            className="hidden"
          />
        </div>
        <textarea
          value={source}
          onChange={(e) => setSource(e.target.value)}
          rows={6}
          placeholder="Paste any text, or import a .txt / .md document…"
          className="w-full resize-y rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-2 text-sm text-text placeholder:text-text-subtle focus:outline-none"
        />
        {/* v1.21.0: use the app's themed Dropdown (portal-based, readable on
            dark) — native <select> option popups render OS-default white and
            the near-white option text was invisible. */}
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 pt-1">
          <div className="flex items-center gap-1.5 text-xs text-text-subtle">
            Format
            <Dropdown
              options={TEMPLATES.map((t) => ({ value: t.id, label: t.label }))}
              selectedValue={templateId}
              onSelect={setTemplateId}
            />
          </div>
          <div className="flex items-center gap-1.5 text-xs text-text-subtle">
            Voice
            <Dropdown
              options={SPEAKERS.map((s) => ({ value: s, label: s }))}
              selectedValue={speaker}
              onSelect={setSpeaker}
            />
          </div>
          <div className="flex items-center gap-1.5 text-xs text-text-subtle">
            Pace
            <Dropdown
              options={TEMPOS.map((t) => ({
                value: String(t.value),
                label: t.label,
              }))}
              selectedValue={String(tempo)}
              onSelect={(v) => setTempo(Number(v))}
            />
          </div>
        </div>
        <button
          type="button"
          onClick={draftTranscript}
          disabled={drafting || !source.trim()}
          className="flex items-center gap-1.5 rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-1.5 text-sm transition-colors hover:text-aurora-cyan disabled:opacity-50"
        >
          {drafting ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <Wand2 size={14} />
          )}
          {drafting ? "Drafting…" : "Draft script (Gemma)"}
        </button>
      </div>

      {/* Spoken script */}
      {(transcript || drafting) && (
        <div className="space-y-1">
          <label className="text-xs text-text-subtle">
            Spoken script (editable)
          </label>
          <textarea
            value={transcript}
            onChange={(e) => setTranscript(e.target.value)}
            rows={8}
            placeholder="The drafted script will appear here…"
            className="w-full resize-y rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-2 text-sm text-text placeholder:text-text-subtle focus:outline-none"
          />
          <button
            type="button"
            onClick={renderAudio}
            disabled={rendering || !transcript.trim()}
            className="flex items-center gap-1.5 rounded-lg border border-glass-border bg-glass-surface-thin px-3 py-1.5 text-sm transition-colors hover:text-aurora-cyan disabled:opacity-50"
          >
            {rendering ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Volume2 size={14} />
            )}
            {rendering ? "Rendering…" : "Render audio (Qwen3-TTS)"}
          </button>
          {rendering && (
            <p className="flex items-center gap-1.5 text-xs text-text-subtle">
              <Loader2 size={12} className="animate-spin" /> Rendering on-device —
              GPU-bound, can take a few minutes. Leave it running.
            </p>
          )}
        </div>
      )}

      {audioUrl && !rendering && (
        <audio controls src={audioUrl} className="mt-1 w-full" />
      )}
    </div>
  );
};
