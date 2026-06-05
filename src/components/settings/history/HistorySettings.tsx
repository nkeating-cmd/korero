/**
 * Kōrero (v1.4.0) — HistorySettings overlay
 *
 * v1.4.0 enhancements over upstream:
 * - Post-processed text is shown as the primary text when available.
 * - "✦ Post-processed" badge on entries that went through the LLM.
 * - "Show original transcription" toggle expands the raw Whisper output.
 * - Pencil icon opens inline correction: editable textarea, ⌘↵ to save, Esc to cancel.
 * - Inline correction persists via update_post_processed_text Rust command (SQLite).
 * - HistoryUpdatePayload::Updated event keeps the frontend in sync after save.
 * - Copy button copies the effective text (post-processed if present, else raw).
 * - "⚠ Post-processing unavailable" indicator when PP was requested but failed.
 */

import React, { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import {
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  FolderOpen,
  Pencil,
  RotateCcw,
  Sparkles,
  Star,
  Trash2,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  commands,
  events,
  type HistoryEntry,
  type HistoryUpdatePayload,
} from "@/bindings";
import { useOsType } from "@/hooks/useOsType";
import { formatDateTime } from "@/utils/dateFormat";
import { AudioPlayer } from "../../ui/AudioPlayer";
import { Button } from "../../ui/Button";
import { AddCorrectionInline, ClickableWords } from "../../ui/Corrections";

// ---------------------------------------------------------------------------
// Shared UI primitives
// ---------------------------------------------------------------------------

const IconButton: React.FC<{
  onClick: () => void;
  title: string;
  disabled?: boolean;
  active?: boolean;
  children: React.ReactNode;
}> = ({ onClick, title, disabled, active, children }) => (
  <button
    onClick={onClick}
    disabled={disabled}
    className={`p-1.5 rounded-md flex items-center justify-center transition-colors cursor-pointer disabled:cursor-not-allowed disabled:text-text/20 ${
      active
        ? "text-logo-primary hover:text-logo-primary/80"
        : "text-text/50 hover:text-logo-primary"
    }`}
    title={title}
  >
    {children}
  </button>
);

const PAGE_SIZE = 30;

const OpenRecordingsButton: React.FC<{ onClick: () => void; label: string }> = ({
  onClick,
  label,
}) => (
  <Button
    onClick={onClick}
    variant="secondary"
    size="sm"
    className="flex items-center gap-2"
    title={label}
  >
    <FolderOpen className="w-4 h-4" />
    <span>{label}</span>
  </Button>
);

// ---------------------------------------------------------------------------
// Main page component
// ---------------------------------------------------------------------------

export const HistorySettings: React.FC = () => {
  const { t } = useTranslation();
  const osType = useOsType();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [hasMore, setHasMore] = useState(true);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const entriesRef = useRef<HistoryEntry[]>([]);
  const loadingRef = useRef(false);

  useEffect(() => {
    entriesRef.current = entries;
  }, [entries]);

  const loadPage = useCallback(async (cursor?: number) => {
    const isFirstPage = cursor === undefined;
    if (!isFirstPage && loadingRef.current) return;
    loadingRef.current = true;
    if (isFirstPage) setLoading(true);

    try {
      const result = await commands.getHistoryEntries(cursor ?? null, PAGE_SIZE);
      if (result.status === "ok") {
        const { entries: newEntries, has_more } = result.data;
        setEntries((prev) => (isFirstPage ? newEntries : [...prev, ...newEntries]));
        setHasMore(has_more);
      }
    } catch (error) {
      console.error("Failed to load history entries:", error);
    } finally {
      setLoading(false);
      loadingRef.current = false;
    }
  }, []);

  // Initial load
  useEffect(() => {
    loadPage();
  }, [loadPage]);

  // Infinite scroll
  useEffect(() => {
    if (loading) return;
    const sentinel = sentinelRef.current;
    if (!sentinel || !hasMore) return;

    const observer = new IntersectionObserver(
      (observerEntries) => {
        if (observerEntries[0].isIntersecting) {
          const last = entriesRef.current[entriesRef.current.length - 1];
          if (last) loadPage(last.id);
        }
      },
      { threshold: 0 },
    );

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [loading, hasMore, loadPage]);

  // Real-time updates from transcription pipeline
  useEffect(() => {
    const unlisten = events.historyUpdatePayload.listen((event) => {
      const payload: HistoryUpdatePayload = event.payload;
      if (payload.action === "added") {
        setEntries((prev) => [payload.entry, ...prev]);
      } else if (payload.action === "updated") {
        setEntries((prev) =>
          prev.map((e) => (e.id === payload.entry.id ? payload.entry : e)),
        );
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  const toggleSaved = async (id: number) => {
    // Optimistic
    setEntries((prev) => prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)));
    try {
      const result = await commands.toggleHistoryEntrySaved(id);
      if (result.status !== "ok") {
        setEntries((prev) => prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)));
      }
    } catch {
      setEntries((prev) => prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)));
    }
  };

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      console.error("Failed to copy to clipboard:", error);
    }
  };

  const getAudioUrl = useCallback(
    async (fileName: string) => {
      try {
        const result = await commands.getAudioFilePath(fileName);
        if (result.status === "ok") {
          if (osType === "linux") {
            const fileData = await readFile(result.data);
            const blob = new Blob([fileData], { type: "audio/wav" });
            return URL.createObjectURL(blob);
          }
          return convertFileSrc(result.data, "asset");
        }
        return null;
      } catch {
        return null;
      }
    },
    [osType],
  );

  const deleteAudioEntry = async (id: number) => {
    setEntries((prev) => prev.filter((e) => e.id !== id));
    try {
      const result = await commands.deleteHistoryEntry(id);
      if (result.status !== "ok") loadPage();
    } catch {
      loadPage();
    }
  };

  const retryHistoryEntry = async (id: number) => {
    const result = await commands.retryHistoryEntryTranscription(id);
    if (result.status !== "ok") throw new Error(String(result.error));
  };

  const openRecordingsFolder = async () => {
    try {
      const result = await commands.openRecordingsFolder();
      if (result.status !== "ok") throw new Error(String(result.error));
    } catch (error) {
      console.error("Failed to open recordings folder:", error);
    }
  };

  // Optimistic update for inline corrections — event from Rust also arrives
  // but this prevents a visible flicker back to the old value first.
  const handleEntryUpdated = useCallback((updated: HistoryEntry) => {
    setEntries((prev) => prev.map((e) => (e.id === updated.id ? updated : e)));
  }, []);

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  let content: React.ReactNode;

  if (loading) {
    content = (
      <div className="px-4 py-3 text-center text-text/60">
        {t("settings.history.loading")}
      </div>
    );
  } else if (entries.length === 0) {
    content = (
      <div className="px-4 py-3 text-center text-text/60">
        {t("settings.history.empty")}
      </div>
    );
  } else {
    content = (
      <>
        <div className="divide-y divide-mid-gray/20">
          {entries.map((entry) => (
            <HistoryEntryComponent
              key={entry.id}
              entry={entry}
              onToggleSaved={() => toggleSaved(entry.id)}
              onCopyText={copyToClipboard}
              getAudioUrl={getAudioUrl}
              deleteAudio={deleteAudioEntry}
              retryTranscription={retryHistoryEntry}
              onEntryUpdated={handleEntryUpdated}
            />
          ))}
        </div>
        <div ref={sentinelRef} className="h-1" />
      </>
    );
  }

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <div className="space-y-2">
        <div className="px-4 flex items-center justify-between">
          <div>
            <h2 className="text-xs font-medium text-mid-gray uppercase tracking-wide">
              {t("settings.history.title")}
            </h2>
          </div>
          <OpenRecordingsButton
            onClick={openRecordingsFolder}
            label={t("settings.history.openFolder")}
          />
        </div>
        <div className="bg-background border border-mid-gray/20 rounded-lg overflow-visible">
          {content}
        </div>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Per-entry component
// ---------------------------------------------------------------------------

interface HistoryEntryProps {
  entry: HistoryEntry;
  onToggleSaved: () => void;
  onCopyText: (text: string) => void;
  getAudioUrl: (fileName: string) => Promise<string | null>;
  deleteAudio: (id: number) => Promise<void>;
  retryTranscription: (id: number) => Promise<void>;
  onEntryUpdated: (entry: HistoryEntry) => void;
}

const HistoryEntryComponent: React.FC<HistoryEntryProps> = ({
  entry,
  onToggleSaved,
  onCopyText,
  getAudioUrl,
  deleteAudio,
  retryTranscription,
  onEntryUpdated,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");
  const [saving, setSaving] = useState(false);
  const [showOriginal, setShowOriginal] = useState(false);
  // v1.16.0: click any word in the transcript to teach a correction —
  // History is where mistakes get noticed, so it's a teaching surface.
  const [teachWord, setTeachWord] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Derived state
  const hasPostProcessed =
    entry.post_process_requested &&
    entry.post_processed_text !== null &&
    entry.post_processed_text !== undefined &&
    entry.post_processed_text.trim().length > 0;
  const ppFailed =
    entry.post_process_requested &&
    (entry.post_processed_text === null ||
      entry.post_processed_text === undefined ||
      entry.post_processed_text.trim().length === 0);
  const effectiveText = hasPostProcessed
    ? (entry.post_processed_text as string)
    : entry.transcription_text;
  const hasTranscription = effectiveText.trim().length > 0;

  // Auto-focus + position cursor at end when entering edit mode
  useEffect(() => {
    if (editing && textareaRef.current) {
      textareaRef.current.focus();
      const len = textareaRef.current.value.length;
      textareaRef.current.setSelectionRange(len, len);
    }
  }, [editing]);

  const handleLoadAudio = useCallback(
    () => getAudioUrl(entry.file_name),
    [getAudioUrl, entry.file_name],
  );

  const handleCopyText = () => {
    if (!hasTranscription) return;
    onCopyText(effectiveText);
    setShowCopied(true);
    setTimeout(() => setShowCopied(false), 2000);
  };

  const handleStartEdit = () => {
    setEditText(entry.post_processed_text ?? "");
    setEditing(true);
    setShowOriginal(false); // collapse original when entering edit
  };

  const handleCancelEdit = () => {
    setEditing(false);
    setEditText("");
  };

  const handleSaveEdit = async () => {
    if (saving) return;
    setSaving(true);
    try {
      const result = await commands.updatePostProcessedText(entry.id, editText);
      if (result.status === "ok") {
        // Optimistic update before the Rust event arrives
        onEntryUpdated({ ...entry, post_processed_text: editText });
        setEditing(false);
        setEditText("");
      } else {
        toast.error("Failed to save correction");
      }
    } catch (error) {
      console.error("Failed to save post-processed text:", error);
      toast.error("Failed to save correction");
    } finally {
      setSaving(false);
    }
  };

  const handleTextareaKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Escape") {
      handleCancelEdit();
    } else if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      void handleSaveEdit();
    }
  };

  const handleDeleteEntry = async () => {
    try {
      await deleteAudio(entry.id);
    } catch (error) {
      console.error("Failed to delete entry:", error);
      toast.error(t("settings.history.deleteError"));
    }
  };

  const handleRetranscribe = async () => {
    try {
      setRetrying(true);
      await retryTranscription(entry.id);
    } catch (error) {
      console.error("Failed to re-transcribe:", error);
      toast.error(t("settings.history.retranscribeError"));
    } finally {
      setRetrying(false);
    }
  };

  const formattedDate = formatDateTime(String(entry.timestamp), i18n.language);

  return (
    <div className="px-4 py-2 pb-5 flex flex-col gap-3">

      {/* Header row -------------------------------------------------------- */}
      <div className="flex justify-between items-center">
        <p className="text-sm font-medium">{formattedDate}</p>
        <div className="flex items-center">
          {editing ? (
            <>
              <IconButton
                onClick={() => { void handleSaveEdit(); }}
                disabled={saving}
                title="Save correction (⌘↵)"
              >
                {saving ? (
                  <span className="h-4 w-4 border-2 border-current border-t-transparent rounded-full animate-spin block" />
                ) : (
                  <Check width={16} height={16} />
                )}
              </IconButton>
              <IconButton
                onClick={handleCancelEdit}
                disabled={saving}
                title="Cancel (Esc)"
              >
                <X width={16} height={16} />
              </IconButton>
            </>
          ) : (
            <>
              <IconButton
                onClick={handleCopyText}
                disabled={!hasTranscription || retrying}
                title={t("settings.history.copyToClipboard")}
              >
                {showCopied ? (
                  <Check width={16} height={16} />
                ) : (
                  <Copy width={16} height={16} />
                )}
              </IconButton>
              {hasPostProcessed && (
                <IconButton
                  onClick={handleStartEdit}
                  disabled={retrying}
                  title="Edit post-processed text"
                >
                  <Pencil width={16} height={16} />
                </IconButton>
              )}
              <IconButton
                onClick={onToggleSaved}
                disabled={retrying}
                active={entry.saved}
                title={
                  entry.saved
                    ? t("settings.history.unsave")
                    : t("settings.history.save")
                }
              >
                <Star
                  width={16}
                  height={16}
                  fill={entry.saved ? "currentColor" : "none"}
                />
              </IconButton>
              <IconButton
                onClick={handleRetranscribe}
                disabled={retrying}
                title={t("settings.history.retranscribe")}
              >
                <RotateCcw
                  width={16}
                  height={16}
                  style={
                    retrying
                      ? { animation: "spin 1s linear infinite reverse" }
                      : undefined
                  }
                />
              </IconButton>
              <IconButton
                onClick={handleDeleteEntry}
                disabled={retrying}
                title={t("settings.history.delete")}
              >
                <Trash2 width={16} height={16} />
              </IconButton>
            </>
          )}
        </div>
      </div>

      {/* Post-processing status badge -------------------------------------- */}
      {hasPostProcessed && !editing && (
        <div className="flex items-center gap-1.5">
          <Sparkles className="h-3 w-3 text-cyan-400/70 flex-shrink-0" />
          <span className="text-xs text-cyan-400/70 font-medium">
            Post-processed
          </span>
        </div>
      )}
      {ppFailed && !editing && (
        <div className="flex items-center gap-1.5">
          <span className="text-xs text-pill-warning/70">
            ⚠ Post-processing unavailable
          </span>
        </div>
      )}

      {/* Text area — edit mode -------------------------------------------- */}
      {editing ? (
        <div className="flex flex-col gap-1.5">
          <textarea
            ref={textareaRef}
            value={editText}
            onChange={(e) => setEditText(e.target.value)}
            onKeyDown={handleTextareaKeyDown}
            rows={4}
            className="w-full text-sm italic bg-mid-gray/10 border border-mid-gray/30 rounded-md px-3 py-2 text-text/90 resize-y focus:outline-none focus:border-cyan-400/50 select-text cursor-text"
            placeholder="Post-processed text…"
          />
          <p className="text-xs text-mid-gray/40">⌘↵ to save · Esc to cancel</p>
        </div>
      ) : (
        /* Text area — display mode ---------------------------------------- */
        <div className="flex flex-col gap-1.5">
          <p
            className={`italic text-sm pb-1 ${
              retrying
                ? ""
                : hasTranscription
                  ? "text-text/90 select-text cursor-text whitespace-pre-wrap break-words"
                  : "text-text/40"
            }`}
            style={
              retrying
                ? { animation: "transcribe-pulse 3s ease-in-out infinite" }
                : undefined
            }
          >
            {retrying && (
              <style>{`
                @keyframes transcribe-pulse {
                  0%, 100% { color: color-mix(in srgb, var(--color-text) 40%, transparent); }
                  50%       { color: color-mix(in srgb, var(--color-text) 90%, transparent); }
                }
              `}</style>
            )}
            {retrying ? (
              t("settings.history.transcribing")
            ) : hasTranscription ? (
              <ClickableWords
                text={effectiveText}
                onWordClick={(w) => setTeachWord(w)}
              />
            ) : (
              t("settings.history.transcriptionFailed")
            )}
          </p>

          {/* v1.16.0: teach form, prefilled from the clicked word. */}
          {teachWord !== null && (
            <AddCorrectionInline
              initialWrong={teachWord}
              onDone={() => setTeachWord(null)}
            />
          )}

          {/* Show / hide original raw transcription ---------------------- */}
          {hasPostProcessed && !retrying && (
            <div className="flex flex-col gap-1">
              <button
                onClick={() => setShowOriginal((v) => !v)}
                className="flex items-center gap-1 text-xs text-mid-gray/40 hover:text-mid-gray/70 transition-colors w-fit"
              >
                {showOriginal ? (
                  <ChevronUp width={12} height={12} />
                ) : (
                  <ChevronDown width={12} height={12} />
                )}
                {showOriginal
                  ? "Hide original"
                  : "Show original transcription"}
              </button>
              {showOriginal && (
                <p className="text-xs text-mid-gray/60 italic select-text cursor-text whitespace-pre-wrap break-words bg-mid-gray/10 rounded-md px-3 py-2 mt-0.5">
                  {/* v1.16.0: the RAW transcript is where the transcriber's
                      actual mistakes live — clickable too. */}
                  <ClickableWords
                    text={entry.transcription_text}
                    onWordClick={(w) => setTeachWord(w)}
                  />
                </p>
              )}
            </div>
          )}
        </div>
      )}

      <AudioPlayer onLoadRequest={handleLoadAudio} className="w-full" />
    </div>
  );
};
