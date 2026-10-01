/* eslint-disable i18next/no-literal-string */
import React, { useRef } from "react";
import {
  Check,
  Copy,
  Download,
  Pencil,
  Trash2,
  TriangleAlert,
  FolderOpen,
  Lock,
  Cloud,
  Flag,
  Scissors,
  ArrowLeft,
  Loader2,
  Circle,
} from "lucide-react";
import { toast } from "sonner";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { MeetingsController } from "../settings/meetings/useMeetingsController";
import type { MeetingTab } from "../../stores/navStore";
import { durationOf, fmtDuration, isTrimmed, hiddenCount, sideWords, titleOf, wordCount } from "./model";
import { TranscriptPanel } from "./TranscriptPanel";
import { NotesPanel } from "./NotesPanel";
import { AudioPanel } from "./AudioPanel";
import { AskPanel } from "./AskPanel";

/**
 * Kōrero 1.42: one meeting (A2 on the design canvas). A header that says
 * what it is, then four tabs: Transcript, Notes, Audio, Ask. A meeting fresh
 * from Stop opens with a short wrap-up (C2): what was saved, what to check,
 * and whether the notes are written.
 */

const TABS: { id: MeetingTab; label: string }[] = [
  { id: "transcript", label: "Transcript" },
  { id: "notes", label: "Notes" },
  { id: "audio", label: "Audio" },
  { id: "ask", label: "Ask" },
];

const WRAP_UP_WINDOW_MS = 6 * 3600_000;

export const MeetingView: React.FC<{ c: MeetingsController; onBack?: () => void }> = ({ c, onBack }) => {
  const m = c.active;
  const titleRef = useRef<HTMLInputElement>(null);
  const tabRefs = useRef<Partial<Record<MeetingTab, HTMLButtonElement | null>>>({});

  if (!m) return null;

  const d = durationOf(m);
  const notesRunning = c.jobRunningHere !== null;
  const fresh =
    !m.imported &&
    !m.wrapUpDismissed &&
    Date.now() - m.createdAt < WRAP_UP_WINDOW_MS;

  const onTabKey = (e: React.KeyboardEvent, idx: number) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const next = TABS[(idx + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length];
    c.setTab(next.id);
    tabRefs.current[next.id]?.focus();
  };

  return (
    <div className="flex-1 min-w-0 min-h-0 flex flex-col">
      <div className="px-8 pt-5 shrink-0">
        {onBack && (
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm -ml-2.5 mb-1" onClick={onBack}>
            <ArrowLeft size={14} /> All meetings
          </button>
        )}
        <div className="flex items-center justify-between gap-3 min-h-[30px]">
          <p className="kx-overline truncate">
            {new Date(m.createdAt).toLocaleString(undefined, {
              weekday: "short",
              day: "numeric",
              month: "short",
              hour: "numeric",
              minute: "2-digit",
            })}
          </p>
          <div className="flex items-center gap-1.5 shrink-0">
            <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => void c.copyActive()}>
              {c.copied ? <Check size={14} /> : <Copy size={14} />}
              {c.copied ? "Copied" : "Copy"}
            </button>
            <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => void c.exportActive()} title="Export the transcript and notes to a file">
              <Download size={14} /> Export
            </button>
            <button type="button" aria-label="Delete meeting" title="Delete meeting" className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm kx-danger-ghost" onClick={() => c.deleteMeeting(m.id)}>
              <Trash2 size={15} />
            </button>
          </div>
        </div>
        <div className="flex items-center gap-1.5 mt-0.5">
          <label htmlFor="kx-mtitle" className="kx-sr-only">
            Meeting title
          </label>
          <input
            id="kx-mtitle"
            ref={titleRef}
            value={m.title}
            onChange={(e) => c.patchMeeting(m.id, { title: e.target.value })}
            placeholder={titleOf({ ...m, title: "" })}
            className="flex-1 min-w-0 bg-transparent kx-title text-[22px] border-b border-transparent hover:border-[var(--kx-hairline)] focus:border-[var(--kx-accent-ink)] transition-colors"
          />
          <button type="button" aria-label="Rename meeting" title="Rename" className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm shrink-0" onClick={() => titleRef.current?.focus()}>
            <Pencil size={14} />
          </button>
        </div>

        <div className="flex flex-wrap gap-2 mt-2.5">
          {d && <span className="kx-chip">{fmtDuration(d)}</span>}
          <span className="kx-chip">{m.imported ? "Imported file" : m.systemCaptured ? `${m.youLabel} + ${m.othersLabel}` : `${m.youLabel} only`}</span>
          {(m.flags ?? []).length > 0 && (
            <span className="kx-chip kx-chip-warn">
              <Flag size={11} /> {(m.flags ?? []).length} flagged
            </span>
          )}
          {isTrimmed(m) && (
            <span className="kx-chip kx-chip-accent">
              <Scissors size={11} /> Trimmed · {hiddenCount(m)} hidden
            </span>
          )}
          {/* About the notes model now; worded for whether notes exist yet. */}
          {c.providerLocal === false ? (
            <span className="kx-chip kx-chip-warn" title="The notes model is a cloud service">
              <Cloud size={11} /> {m.processed.trim() ? "Notes by a cloud model" : "Notes would use a cloud model"}
            </span>
          ) : c.providerLocal ? (
            <span className="kx-chip kx-chip-ok" title="The notes model runs on this computer">
              <Lock size={11} /> {m.processed.trim() ? "Notes made on this computer" : "Notes stay on this computer"}
            </span>
          ) : null}
        </div>

        {c.exportedPath && (
          <div className="flex items-center gap-2 mt-3 px-3 py-2 rounded-[9px] border border-[var(--kx-hairline)] bg-[var(--kx-surface)] text-[12.5px] kx-ink-2">
            <Check size={13} className="text-[var(--kx-ok)] shrink-0" />
            <span className="shrink-0">Exported to</span>
            <code className="min-w-0 flex-1 truncate select-all text-white kx-mono" title={c.exportedPath}>
              {c.exportedPath}
            </code>
            <button
              type="button"
              className="kx-btn kx-btn-quiet kx-btn-sm"
              onClick={() => revealItemInDir(c.exportedPath as string).catch(() => toast.error("Could not open the folder."))}
            >
              <FolderOpen size={13} /> Show in folder
            </button>
            <button
              type="button"
              className="kx-btn kx-btn-quiet kx-btn-sm"
              onClick={() =>
                writeText(c.exportedPath as string)
                  .then(() => toast.success("Path copied."))
                  .catch(() => toast.error("Could not copy the path."))
              }
            >
              <Copy size={13} /> Copy path
            </button>
          </div>
        )}

        <div role="tablist" aria-label="Meeting views" className="kx-tabs mt-4">
          {TABS.map((t, i) => (
            <button
              key={t.id}
              ref={(el) => {
                tabRefs.current[t.id] = el;
              }}
              type="button"
              role="tab"
              id={`kx-tab-${t.id}`}
              aria-selected={c.tab === t.id}
              aria-controls={`kx-panel-${t.id}`}
              tabIndex={c.tab === t.id ? 0 : -1}
              onClick={() => c.setTab(t.id)}
              onKeyDown={(e) => onTabKey(e, i)}
              className="kx-tab"
            >
              {t.label}
              {t.id === "notes" && notesRunning && <Loader2 size={13} className="animate-spin kx-accent-ink" aria-label="Writing" />}
              {t.id === "notes" && !notesRunning && m.processed.trim() && (
                <span className="kx-dot kx-dot-ok" aria-label="Notes ready" />
              )}
            </button>
          ))}
        </div>
      </div>

      <div
        role="tabpanel"
        id={`kx-panel-${c.tab}`}
        aria-labelledby={`kx-tab-${c.tab}`}
        className="flex-1 min-h-0 overflow-y-auto px-8 py-5 select-text"
      >
        {fresh && <WrapUp c={c} />}
        {c.tab === "transcript" && <TranscriptPanel c={c} />}
        {c.tab === "notes" && <NotesPanel c={c} />}
        {c.tab === "audio" && <AudioPanel c={c} />}
        {c.tab === "ask" && <AskPanel c={c} />}
      </div>
    </div>
  );
};

/** C2: what happened at Stop, honestly, with the next step one click away. */
const WrapUp: React.FC<{ c: MeetingsController }> = ({ c }) => {
  const m = c.active;
  if (!m) return null;
  const { you: youWords, others: othersWords } = sideWords(m);
  const warnings = m.captureWarnings ?? [];
  const running = c.jobRunningHere;
  const flags = m.flags ?? [];
  const hasAudio = !!m.micPath || !!m.systemPath;

  return (
    <section aria-label="What happened" className="kx-card p-4 mb-5">
      <div className="flex items-center justify-between mb-3">
        <h2 className="kx-heading">What happened</h2>
        <button
          type="button"
          className="kx-btn kx-btn-quiet kx-btn-sm"
          onClick={() => c.patchMeeting(m.id, { wrapUpDismissed: true })}
        >
          Hide this
        </button>
      </div>
      <ol className="flex flex-col gap-3">
        {hasAudio && (
          <li className="flex gap-3 items-start">
            <span className="kx-step kx-step-done" aria-label="Done">
              <Check size={14} />
            </span>
            <div>
              <div className="text-[13.5px] text-white">Audio saved</div>
              <div className="kx-meta">
                {m.micPath && m.systemPath ? "Two recordings, You and Others" : "One recording"} · see the Audio tab
              </div>
            </div>
          </li>
        )}
        <li className="flex gap-3 items-start">
          <span className={`kx-step ${youWords + othersWords > 0 ? "kx-step-done" : "kx-step-warn"}`}>
            {youWords + othersWords > 0 ? <Check size={14} /> : <TriangleAlert size={13} />}
          </span>
          <div>
            <div className="text-[13.5px] text-white">
              {youWords + othersWords > 0 ? "Transcript ready" : "No speech was transcribed"}
            </div>
            <div className="kx-meta">
              {m.youLabel} {youWords.toLocaleString()} words · {m.othersLabel} {othersWords.toLocaleString()} words
            </div>
          </div>
        </li>
        {warnings.map((w, i) => (
          <li key={i} className="flex gap-3 items-start">
            <span className="kx-step kx-step-warn" aria-label="Check">
              <TriangleAlert size={13} />
            </span>
            <div className="text-[13px] text-[var(--kx-warn-ink)]">{w}</div>
          </li>
        ))}
        {flags.length > 0 && (
          <li className="flex gap-3 items-start">
            <span className="kx-step kx-step-done" aria-label="Done">
              <Flag size={13} />
            </span>
            <div>
              <div className="text-[13.5px] text-white">
                {flags.length} moment{flags.length === 1 ? "" : "s"} flagged
              </div>
              <div className="kx-meta">The notes give each one its own line.</div>
            </div>
          </li>
        )}
        <li className="flex gap-3 items-start">
          {running ? (
            <span className="kx-step kx-step-running" aria-label="Working">
              <Loader2 size={14} className="animate-spin" />
            </span>
          ) : m.processed.trim() ? (
            <span className="kx-step kx-step-done" aria-label="Done">
              <Check size={14} />
            </span>
          ) : (
            <span className="kx-step kx-step-todo" aria-label="Not started">
              <Circle size={6} fill="currentColor" />
            </span>
          )}
          <div className="flex-1">
            <div className="text-[13.5px] text-white">
              {running ? "Writing notes" : m.processed.trim() ? "Notes ready" : "Notes"}
            </div>
            <div className="kx-meta">
              {running
                ? `${wordCount(running.live).toLocaleString()} words so far · ${c.ppLabel}`
                : m.processed.trim()
                  ? "Open the Notes tab to read, edit or refine them."
                  : c.ppLabel === "not configured"
                    ? "Set up AI clean-up to have notes written for you."
                    : `Written by ${c.ppLabel}.`}
            </div>
            {!running && !m.processed.trim() && c.activeHasTranscript && (
              <button
                type="button"
                className="kx-btn kx-btn-primary kx-btn-sm mt-2"
                disabled={!!c.busy}
                onClick={() => void c.onPostProcess()}
              >
                Write notes
              </button>
            )}
          </div>
        </li>
      </ol>
    </section>
  );
};
