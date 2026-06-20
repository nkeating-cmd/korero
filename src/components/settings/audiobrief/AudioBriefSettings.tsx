/* eslint-disable i18next/no-literal-string */
import React, { useRef, useState } from "react";
import { Loader2, FileUp, Volume2, Wand2, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { convertFileSrc } from "@tauri-apps/api/core";
import { commands } from "@/bindings";
import { Dropdown } from "@/components/ui/Dropdown";

/**
 * Kōrero (v1.22.0): Audio Brief.
 *
 * Pipeline (all on-device): paste or import text → the local post-processing
 * model drafts a SPOKEN script → the local TTS engine renders it to an MP3. Two
 * explicit steps so the script can be edited before it's spoken. Reuses the
 * existing meetingPostProcess + meetingGenerateAudioBrief commands — no
 * transcript leaves the machine.
 *
 * v1.22.0 polish: logical two-step layout (Format sits with Draft; Voice + Pace
 * sit with Render), aligned label/control stacks on an 8pt rhythm, the unified
 * primary buttons, a wrapped audio preview, and a graceful "engine not set up"
 * state when the on-device voice engine is absent.
 */

// Shared rules every format obeys (audio is linear; write for the ear).
const BASE_RULES =
  " Write numbers as words (e.g. three hundred and eighty-three dollars); expand abbreviations on first use; remove URLs, reference codes, IDs and emoji; short one-idea sentences; NZ English; no titles, headings, or speaker labels. Return ONLY the script text.";

// Template formats for the spoken script — each is a model instruction.
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

// Preset TTS speakers (plus the engine's default designed voice).
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
  const [engineMissing, setEngineMissing] = useState(false);
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
      setEngineMissing(false);
      setAudioUrl(convertFileSrc(r.data, "asset"));
      toast.success("Audio brief ready.");
    } catch (e) {
      const msg = String(e);
      // The backend says "...voice engine not found..." when no local TTS engine
      // is installed — surface a helpful setup card instead of a bare toast.
      if (/not found|no such|couldn'?t (find|locate)|engine/i.test(msg)) {
        setEngineMissing(true);
      }
      toast.error(`Audio render failed: ${msg}`);
    } finally {
      setRendering(false);
    }
  };

  const inputClass =
    "w-full resize-y rounded-lg border border-glass-border bg-glass-surface-thin px-3.5 py-3 text-sm leading-relaxed text-text placeholder:text-text-subtle transition-colors focus:outline-none";
  const stepLabel =
    "text-xs font-semibold uppercase tracking-wider text-text-subtle";
  const fieldLabel = "text-xs font-medium text-text-subtle";

  return (
    <div className="mx-auto w-full max-w-3xl space-y-6 px-6 py-6">
      {/* Header */}
      <header className="space-y-2.5">
        <h2 className="text-xl font-semibold tracking-tight text-text">
          Audio brief
        </h2>
        <p className="max-w-prose text-sm leading-relaxed text-text-muted">
          Turn any text into a spoken brief — entirely on your device. The local
          model drafts a script you can edit, then the on-device voice reads it
          aloud.
        </p>
        <span className="inline-flex items-center gap-1.5 rounded-full border border-glass-border bg-glass-surface-thin px-2.5 py-1 text-xs font-medium text-text-muted">
          <ShieldCheck size={13} className="text-aurora-cyan" />
          On-device · nothing leaves your machine
        </span>
      </header>

      {/* Step 1 — Source → Draft */}
      <section className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <span className={stepLabel}>1 · Source text</span>
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            className="inline-flex items-center gap-1.5 text-xs text-text-muted transition-colors hover:text-aurora-cyan"
          >
            <FileUp size={14} /> Import .txt / .md
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
          aria-label="Source text"
          onChange={(e) => setSource(e.target.value)}
          rows={6}
          placeholder="Paste any text, or import a .txt / .md document…"
          className={inputClass}
        />
        <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-3">
          <div className="flex flex-col gap-1.5">
            <label className={fieldLabel}>Format</label>
            <Dropdown
              options={TEMPLATES.map((t) => ({ value: t.id, label: t.label }))}
              selectedValue={templateId}
              onSelect={setTemplateId}
            />
          </div>
          <button
            type="button"
            onClick={draftTranscript}
            disabled={drafting || !source.trim()}
            className={transcript ? "korero-btn" : "korero-btn-primary"}
          >
            {drafting ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Wand2 size={14} />
            )}
            {drafting ? "Drafting…" : "Draft script"}
          </button>
        </div>
      </section>

      {/* Step 2 — Script → Render */}
      {(transcript || drafting) && (
        <section className="space-y-3 border-t border-glass-border pt-6">
          <span className={stepLabel}>2 · Spoken script</span>
          <textarea
            value={transcript}
            aria-label="Spoken script"
            onChange={(e) => setTranscript(e.target.value)}
            rows={8}
            placeholder="The drafted script will appear here — edit freely before rendering…"
            className={inputClass}
          />
          <div className="flex flex-wrap items-end gap-4">
            <div className="flex flex-wrap items-end gap-4">
              <div className="flex flex-col gap-1.5">
                <label className={fieldLabel}>Voice</label>
                <Dropdown
                  options={SPEAKERS.map((s) => ({ value: s, label: s }))}
                  selectedValue={speaker}
                  onSelect={setSpeaker}
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className={fieldLabel}>Pace</label>
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
              onClick={renderAudio}
              disabled={rendering || !transcript.trim()}
              className="korero-btn-primary"
            >
              {rendering ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <Volume2 size={14} />
              )}
              {rendering ? "Rendering…" : "Render audio"}
            </button>
          </div>
          {rendering && (
            <p className="flex items-center gap-2 text-xs text-text-subtle">
              <Loader2 size={12} className="animate-spin" /> Rendering on-device —
              GPU-bound, this can take a few minutes. You can leave it running.
            </p>
          )}
        </section>
      )}

      {/* Audio preview */}
      {audioUrl && !rendering && (
        <div className="space-y-2 rounded-lg border border-glass-border bg-glass-surface-thin p-3">
          <div className="flex items-center gap-1.5 text-xs font-medium text-text-subtle">
            <Volume2 size={13} className="text-aurora-cyan" /> Preview
          </div>
          <audio controls src={audioUrl} className="w-full" />
        </div>
      )}

      {/* Engine-not-set-up help (shown after a render fails because no local
          TTS engine is installed). Everything else on the page still works. */}
      {engineMissing && (
        <div className="space-y-1.5 rounded-lg border border-glass-border bg-glass-surface-thin p-3.5 text-xs leading-relaxed text-text-muted">
          <p className="font-medium text-text">
            The on-device voice engine isn't set up yet
          </p>
          <p>
            Audio Brief needs a local text-to-speech engine. Point Kōrero at one
            by setting the{" "}
            <code className="rounded bg-glass-surface px-1 py-0.5 text-text">
              KORERO_TTS_DIR
            </code>{" "}
            environment variable to the engine folder, or place it at{" "}
            <code className="rounded bg-glass-surface px-1 py-0.5 text-text">
              %APPDATA%\com.nkeating.korero\tts
            </code>
            . Drafting scripts and everything else on this page work without it.
          </p>
        </div>
      )}
    </div>
  );
};
