/* eslint-disable i18next/no-literal-string */
import React from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { FolderOpen, Loader2, RotateCcw, Wand2, GitMerge, Trash2 } from "lucide-react";
import { AudioPlayer } from "../ui/AudioPlayer";
import { Dropdown } from "../ui/Dropdown";
import type { MeetingsController } from "../settings/meetings/useMeetingsController";
import { baseName, titleOf } from "./model";
import { useRecorder } from "../../stores/recorderStore";

/**
 * Kōrero 1.42: the recordings behind a meeting, and the work you can do on
 * them — re-transcribe with another model, combine two meetings, or delete.
 */
export const AudioPanel: React.FC<{ c: MeetingsController }> = ({ c }) => {
  const m = c.active;
  // A model switch mid-meeting would drop live lines while the new model
  // loads, and the Stop-time rebuild would use a different model.
  const meetingLive = useRecorder((s) => s.recording) || !!c.stopping;
  if (!m) return null;

  const file = (label: string, path: string | null) =>
    path ? (
      <div className="kx-card p-3.5 flex flex-col gap-2">
        <div className="flex items-center justify-between gap-2">
          <div className="min-w-0">
            <div className="kx-heading">{label}</div>
            <div className="kx-meta kx-mono truncate" title={path}>
              {baseName(path)}
            </div>
          </div>
          <button
            type="button"
            className="kx-btn kx-btn-quiet kx-btn-sm shrink-0"
            onClick={() => revealItemInDir(path).catch(() => toast.error("Could not open the folder."))}
          >
            <FolderOpen size={13} /> Show in folder
          </button>
        </div>
        <AudioPlayer src={convertFileSrc(path, "asset")} className="w-full" />
      </div>
    ) : null;

  return (
    <div className="flex flex-col gap-5 max-w-[780px]">
      {!m.micPath && !m.systemPath ? (
        <p className="kx-meta">This meeting has no audio files.</p>
      ) : (
        <div className="flex flex-col gap-3">
          {file(m.imported ? "Imported file" : m.youLabel, m.micPath)}
          {file(m.othersLabel, m.systemPath)}
        </div>
      )}

      <section aria-labelledby="kx-retx" className="kx-card p-4 flex flex-col gap-3">
        <h2 id="kx-retx" className="kx-heading">
          Transcribe again
        </h2>
        <p className="kx-meta">
          Re-run speech-to-text from the saved recording — useful after switching to a different
          speech model, or when a capture went wrong. Your current transcript is kept if it fails.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <span className="kx-meta">Speech model</span>
          <Dropdown
            options={c.modelOptions}
            selectedValue={c.currentModel}
            onSelect={(id) => void c.changeModel(id)}
            disabled={!c.models || c.models.length === 0 || !!c.busy || meetingLive}
          />
          <button type="button" className="kx-btn kx-btn-secondary" onClick={() => void c.onReTranscribe()} disabled={!!c.busy || !c.activeHasAudio}>
            {c.busy === "transcribe" && c.busyMeetingId === m.id ? <Loader2 size={14} className="animate-spin" /> : <RotateCcw size={14} />}
            Re-transcribe
          </button>
          <button type="button" className="kx-btn kx-btn-secondary" onClick={() => void c.onBoth()} disabled={!!c.busy || !c.activeHasAudio}>
            {c.busy === "both" && c.busyMeetingId === m.id ? <Loader2 size={14} className="animate-spin" /> : <Wand2 size={14} />}
            Re-transcribe + notes
          </button>
        </div>
        {c.busy && c.busyMeetingId === m.id && c.task && (
          <div>
            <div className="flex justify-between kx-meta mb-1">
              <span aria-live="polite">Transcribing</span>
              <span className="kx-mono">
                {c.transcribeProgress
                  ? `part ${c.transcribeProgress.window}${c.transcribeProgress.total ? ` of ${c.transcribeProgress.total}` : ""}`
                  : `${c.busyElapsed}s`}
              </span>
            </div>
            <div className={`kx-progress ${c.transcribeProgress?.total ? "" : "kx-progress-indeterminate"}`}>
              <span
                style={
                  c.transcribeProgress?.total
                    ? { width: `${Math.min(100, Math.round((c.transcribeProgress.window / c.transcribeProgress.total) * 100))}%` }
                    : undefined
                }
              />
            </div>
          </div>
        )}
        {c.busy && c.elsewhereJobTitle && (
          <p className="kx-meta">Busy with “{c.elsewhereJobTitle}” — one at a time.</p>
        )}
      </section>

      {c.meetings.length > 1 && (
        <section aria-labelledby="kx-merge" className="kx-card p-4 flex flex-col gap-3">
          <h2 id="kx-merge" className="kx-heading">
            Combine with another meeting
          </h2>
          <p className="kx-meta">
            Makes a NEW meeting with both transcripts in time order. Both originals are kept.
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <Dropdown
              options={c.meetings.filter((x) => x.id !== m.id).map((x) => ({ value: x.id, label: titleOf(x) }))}
              selectedValue={c.mergeWithId}
              onSelect={c.setMergeWithId}
              placeholder="Choose a meeting"
            />
            <button type="button" className="kx-btn kx-btn-secondary" onClick={() => c.mergeMeetings(c.mergeWithId)} disabled={!c.mergeWithId}>
              <GitMerge size={14} /> Combine
            </button>
          </div>
        </section>
      )}

      <section aria-labelledby="kx-del" className="kx-card p-4 flex items-center justify-between gap-3">
        <div>
          <h2 id="kx-del" className="kx-heading">
            Delete this meeting
          </h2>
          <p className="kx-meta">The transcript, the notes and the audio files are removed permanently.</p>
        </div>
        <button type="button" className="kx-btn kx-btn-secondary kx-danger-ghost shrink-0" onClick={() => c.deleteMeeting(m.id)}>
          <Trash2 size={14} /> Delete
        </button>
      </section>
    </div>
  );
};
