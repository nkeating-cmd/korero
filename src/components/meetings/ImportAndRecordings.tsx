/* eslint-disable i18next/no-literal-string */
import React from "react";
import { toast } from "sonner";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FileAudio, X, Wand2, RotateCcw, Loader2, Plus, RefreshCw, Trash2, FolderOpen } from "lucide-react";
import { Dropdown } from "../ui/Dropdown";
import type { MeetingsController } from "../settings/meetings/useMeetingsController";
import { baseName } from "./model";

/** Kōrero 1.42: turning an audio file into a meeting. */
export const ImportPanel: React.FC<{ c: MeetingsController }> = ({ c }) => {
  if (!c.importPath) {
    return (
      <div className="kx-page" style={{ maxWidth: 720 }}>
        <h1 className="kx-title">Import a recording</h1>
        <p className="kx-meta mt-1 mb-4">WAV, M4A, MP3, AAC, ALAC, FLAC, OGG, CAF or AIFF.</p>
        <button type="button" className="kx-btn kx-btn-primary" onClick={() => void c.pickImportFile()}>
          Choose a file
        </button>
      </div>
    );
  }
  const p = c.transcribeProgress;
  return (
    <div className="kx-page" style={{ maxWidth: 720 }}>
      <p className="kx-overline">Import</p>
      <div className="flex items-center gap-2 mt-1">
        <FileAudio size={18} className="kx-accent-ink shrink-0" />
        <h1 className="kx-title truncate flex-1">{baseName(c.importPath)}</h1>
        <button
          type="button"
          aria-label="Cancel the import"
          className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm"
          disabled={c.importBusy}
          onClick={() => {
            c.setImportPath(null);
            c.setView({ kind: "meeting" });
          }}
        >
          <X size={16} />
        </button>
      </div>
      <p className="kx-meta mt-1 mb-5">
        Transcribed with your current speech model on this computer, then (if you choose) summarised
        with the prompt below.
      </p>

      <section className="kx-card p-4 flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="kx-meta">Prompt</span>
          <Dropdown
            options={c.promptOptions}
            selectedValue={c.importPromptId}
            onSelect={(id) => {
              c.setImportPromptId(id);
              if (id !== "custom") c.setImportPrompt(c.savedPromptText(id));
            }}
            disabled={c.importBusy}
          />
          <button
            type="button"
            className="kx-btn kx-btn-quiet kx-btn-sm"
            disabled={c.importBusy}
            onClick={async () => {
              const id = await c.savePromptAsNew(c.importPrompt);
              if (id) c.setImportPromptId(id);
            }}
          >
            <Plus size={13} /> Save as new
          </button>
        </div>
        <label htmlFor="kx-import-prompt" className="kx-sr-only">
          Notes prompt
        </label>
        <textarea
          id="kx-import-prompt"
          value={c.importPrompt}
          onChange={(e) => {
            c.setImportPrompt(e.target.value);
            c.setImportPromptId("custom");
          }}
          rows={3}
          className="kx-input"
        />
        <div className="flex flex-wrap gap-2">
          <button type="button" className="kx-btn kx-btn-primary" disabled={c.importBusy || (!!c.busy && !c.importBusy)} onClick={() => void c.runImport(true)}>
            {c.importBusy ? <Loader2 size={14} className="animate-spin" /> : <Wand2 size={14} />}
            Transcribe + notes
          </button>
          <button type="button" className="kx-btn kx-btn-secondary" disabled={c.importBusy || (!!c.busy && !c.importBusy)} onClick={() => void c.runImport(false)}>
            {c.importBusy ? <Loader2 size={14} className="animate-spin" /> : <RotateCcw size={14} />}
            Transcribe only
          </button>
        </div>
        {c.busy && !c.importBusy && c.elsewhereJobTitle && (
          <p className="kx-meta">Busy with “{c.elsewhereJobTitle}” — one at a time.</p>
        )}
        {c.importBusy && (
          <div>
            <div className="flex justify-between kx-meta mb-1">
              <span aria-live="polite">Transcribing the audio…</span>
              <span className="kx-mono">{p ? `part ${p.window}${p.total ? ` of ${p.total}` : ""}` : `${c.busyElapsed}s`}</span>
            </div>
            <div className={`kx-progress ${p?.total ? "" : "kx-progress-indeterminate"}`}>
              <span style={p?.total ? { width: `${Math.min(100, Math.round((p.window / p.total) * 100))}%` } : undefined} />
            </div>
            <p className="kx-meta mt-2">You can leave this page; the meeting appears when it is done.</p>
          </div>
        )}
      </section>
    </div>
  );
};

/** Kōrero 1.42: recordings on disk (recovery) and where they are kept. */
export const RecordingsPanel: React.FC<{ c: MeetingsController }> = ({ c }) => (
  <div className="kx-page" style={{ maxWidth: 820 }}>
    <h1 className="kx-title">Recordings & storage</h1>
    <p className="kx-meta mt-1 mb-5">
      Every meeting is written to disk while it records, so it can be recovered even if Kōrero
      closed unexpectedly. Recordings are kept for 30 days.
    </p>

    <section aria-labelledby="kx-recs" className="kx-card">
      <div className="kx-card-header">
        <h2 id="kx-recs" className="kx-heading">
          Recordings on disk
        </h2>
        <button type="button" className="kx-btn kx-btn-ghost kx-btn-sm" onClick={() => void c.loadRecordings()}>
          <RefreshCw size={13} /> Refresh
        </button>
      </div>
      {c.recordings === null ? (
        <p className="px-4 py-5 kx-meta text-center">Loading…</p>
      ) : c.recordings.length === 0 ? (
        <p className="px-4 py-5 kx-meta text-center">No saved recordings.</p>
      ) : (
        <div className="kx-divide">
          {c.recordings.map((f) => (
            <div key={f.path} className="flex items-center gap-3 px-4 py-2.5">
              <FileAudio size={16} className="kx-ink-2 shrink-0" />
              <div className="flex-1 min-w-0">
                <p className="text-[13.5px] truncate">{f.file_name}</p>
                <p className="kx-meta">{f.modified ? new Date(f.modified * 1000).toLocaleString() : ""}</p>
              </div>
              <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => void c.transcribeRecording(f)} disabled={c.busyFile === f.path || !!c.busy}>
                {c.busyFile === f.path ? <Loader2 size={13} className="animate-spin" /> : <RotateCcw size={13} />} Transcribe
              </button>
              <button
                type="button"
                aria-label={`Delete ${f.file_name}`}
                title="Delete this recording from disk"
                className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm kx-danger-ghost"
                onClick={() => void c.deleteRecording(f)}
                disabled={c.busyFile === f.path}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
    </section>

    <section aria-labelledby="kx-storage" className="kx-card p-4 mt-5 flex flex-col gap-3">
      <h2 id="kx-storage" className="kx-heading">
        Where recordings are saved
      </h2>
      {c.dirs === null ? (
        <p className="kx-meta">Loading…</p>
      ) : (
        <>
          <div className="flex items-start justify-between gap-3">
            <p className="kx-meta kx-mono select-all break-all" title={c.dirs.recordingDir}>
              {c.dirs.recordingDir}
              {!c.dirs.recordingIsCustom && <span className="kx-ink-2"> (default)</span>}
            </p>
            <div className="flex shrink-0 items-center gap-2">
              <button
                type="button"
                aria-label="Show this folder"
                className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm"
                onClick={() => revealItemInDir(c.dirs!.recordingDir).catch(() => toast.error("Could not open the folder."))}
              >
                <FolderOpen size={15} />
              </button>
              <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => void c.changeRecordingDir()} disabled={c.dirBusy}>
                Change…
              </button>
              {c.dirs.recordingIsCustom && (
                <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => void c.resetRecordingDir()} disabled={c.dirBusy}>
                  Reset
                </button>
              )}
            </div>
          </div>
          {c.dirs.recordingIsCustom && (
            <div className="flex items-center justify-between gap-3 pt-3 border-t border-[var(--kx-hairline-soft)]">
              <p className="kx-meta">Recordings made before the change are still in the default folder.</p>
              <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm shrink-0" onClick={() => void c.moveExistingRecordings()} disabled={c.dirBusy || !c.storeReady}>
                {c.dirBusy && <Loader2 size={13} className="animate-spin" />} Move existing
              </button>
            </div>
          )}
          <p className="kx-meta">
            A local drive works best: cloud-synced folders (OneDrive, Dropbox) can churn while a
            meeting records. Exports ask where to save and remember your last folder
            ({c.dirs.exportSeedDir}).
          </p>
        </>
      )}
    </section>
  </div>
);
