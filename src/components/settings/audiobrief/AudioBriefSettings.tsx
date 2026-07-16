/* eslint-disable i18next/no-literal-string */
import React, { useRef } from "react";
import {
  Loader2,
  FileUp,
  Volume2,
  Wand2,
  ShieldCheck,
  Download,
  FolderOpen,
  Copy,
} from "lucide-react";
import { toast } from "sonner";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { convertFileSrc } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Dropdown } from "@/components/ui/Dropdown";
import {
  useAudioBriefStore,
  TEMPLATES,
  SPEAKERS,
  TEMPOS,
  STYLES,
} from "@/stores/audioBriefStore";

/**
 * Kōrero (v1.22.0): Audio Brief.
 *
 * Pipeline (all on-device): paste or import text → the local post-processing
 * model drafts a SPOKEN script → the local TTS engine renders it to an MP3. Two
 * explicit steps so the script can be edited before it's spoken. Reuses the
 * existing meetingPostProcess + meetingGenerateAudioBrief commands — no
 * transcript leaves the machine.
 *
 * v1.22.0 (background-safe): all working state AND the draft/render actions live
 * in a Zustand
 * store (audioBriefStore), NOT in component state. The settings panel unmounts
 * the inactive section, so component state used to reset whenever you left this
 * tab mid-run. Now a draft or render keeps running in the BACKGROUND and its
 * result (the script, and the saved MP3 path) is captured by the store even
 * while you're elsewhere; returning to the tab simply re-binds to it. This file
 * is now a thin view over that store.
 */

export const AudioBriefSettings: React.FC = () => {
  const {
    source,
    transcript,
    drafting,
    rendering,
    audioPath,
    engineMissing,
    templateId,
    speaker,
    tempo,
    style,
    setSource,
    setTranscript,
    setTemplateId,
    setSpeaker,
    setTempo,
    setStyle,
    draftTranscript,
    renderAudio,
  } = useAudioBriefStore();

  const fileInputRef = useRef<HTMLInputElement>(null);

  // Derive the playable asset URL from the stored on-disk path, so it survives
  // tab switches (the path is the canonical value held in the store).
  const audioUrl = audioPath ? convertFileSrc(audioPath, "asset") : null;

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
          aloud. A draft or render keeps going if you switch tabs.
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
                <label className={fieldLabel}>Style</label>
                <Dropdown
                  options={STYLES.map((s) => ({ value: s.value, label: s.label }))}
                  selectedValue={style}
                  onSelect={setStyle}
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
              GPU-bound, this can take a few minutes. You can switch tabs; it
              keeps running and the audio is saved when it finishes.
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
          <div className="flex items-center gap-3 text-xs text-text-muted">
            <a
              href={audioUrl}
              download="audio-brief.mp3"
              className="inline-flex items-center gap-1 transition-colors hover:text-aurora-cyan"
            >
              <Download size={13} /> Download
            </a>
            {audioPath && (
              <button
                type="button"
                onClick={() =>
                  revealItemInDir(audioPath).catch(() =>
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
          {/* v1.25.0 (UX batch, audit #9): both paths are long and typo-prone —
              make them one-click copyable. */}
          <div className="flex gap-3 pt-0.5">
            {(
              [
                ["Copy variable name", "KORERO_TTS_DIR"],
                ["Copy folder path", "%APPDATA%\\com.nkeating.korero\\tts"],
              ] as const
            ).map(([label, value]) => (
              <button
                key={label}
                type="button"
                onClick={() =>
                  writeText(value)
                    .then(() => toast.success("Copied."))
                    .catch(() => toast.error("Couldn't copy."))
                }
                className="inline-flex items-center gap-1 text-aurora-cyan transition-colors hover:text-text"
              >
                <Copy size={12} /> {label}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};
