/* eslint-disable i18next/no-literal-string */
import React, { useState } from "react";
import { Check, GraduationCap, Pencil, Plus, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import { Button } from "./Button";
import { useSettings } from "../../hooks/useSettings";

/**
 * Kōrero (v1.15.0): corrections memory — user-taught wrong → right pairs.
 *
 * Applied three ways: deterministic replacement in every transcription
 * (Rust, transcription.rs patch), a glossary injected into note/meeting
 * post-processing prompts, and suggestions mined from Notes clean-ups.
 *
 * This module is the single UI for the list: `useCorrections` (state +
 * mutations), `AddCorrectionInline` (the quick teach form used by Meetings /
 * Notes), and `CorrectionsManager` (the full list, on the Post-processing
 * page).
 */

export type Correction = { wrong: string; right: string };

export const useCorrections = () => {
  const { settings, updateSetting } = useSettings();
  const list: Correction[] = settings?.transcript_corrections ?? [];

  const add = (wrong: string, right: string): boolean => {
    const w = wrong.trim();
    const r = right.trim();
    if (!w || !r) {
      toast.error("Both the wrong and the right text are needed.");
      return false;
    }
    if (w.toLowerCase() === r.toLowerCase() && w === r) {
      toast.error("Those are identical.");
      return false;
    }
    if (list.some((c) => c.wrong.toLowerCase() === w.toLowerCase())) {
      toast.message(`"${w}" already has a correction — edit it under Post-processing.`);
      return false;
    }
    updateSetting("transcript_corrections", [...list, { wrong: w, right: r }]);
    toast.success(`Taught: "${w}" → "${r}"`);
    return true;
  };

  const remove = (index: number) => {
    updateSetting(
      "transcript_corrections",
      list.filter((_, i) => i !== index),
    );
  };

  const update = (index: number, wrong: string, right: string): boolean => {
    const w = wrong.trim();
    const r = right.trim();
    if (!w || !r) {
      toast.error("Both the wrong and the right text are needed.");
      return false;
    }
    if (list.some((c, i) => i !== index && c.wrong.toLowerCase() === w.toLowerCase())) {
      toast.message(`"${w}" already has a correction.`);
      return false;
    }
    updateSetting(
      "transcript_corrections",
      list.map((c, i) => (i === index ? { wrong: w, right: r } : c)),
    );
    toast.success("Correction updated.");
    return true;
  };

  return { list, add, update, remove };
};

/** Compact two-field teach form. Used inline by Meetings, Notes, and the manager. */
export const AddCorrectionInline: React.FC<{
  initialWrong?: string;
  onDone?: () => void;
  autoFocus?: boolean;
}> = ({ initialWrong = "", onDone, autoFocus = true }) => {
  const { add } = useCorrections();
  const [wrong, setWrong] = useState(initialWrong);
  const [right, setRight] = useState("");

  const save = () => {
    if (add(wrong, right)) {
      setWrong("");
      setRight("");
      onDone?.();
    }
  };

  return (
    <div className="flex flex-wrap items-center gap-2">
      <input
        autoFocus={autoFocus && !initialWrong}
        value={wrong}
        onChange={(e) => setWrong(e.target.value)}
        placeholder="What it heard…"
        className="flex-1 min-w-[140px] bg-white/5 border border-white/10 rounded-md px-2 py-1.5 text-sm text-text placeholder:text-text-subtle focus:outline-none"
      />
      <span className="text-text-subtle text-sm shrink-0">→</span>
      <input
        autoFocus={autoFocus && !!initialWrong}
        value={right}
        onChange={(e) => setRight(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") save();
        }}
        placeholder="What it should be…"
        className="flex-1 min-w-[140px] bg-white/5 border border-white/10 rounded-md px-2 py-1.5 text-sm text-text placeholder:text-text-subtle focus:outline-none"
      />
      <Button
        variant="secondary"
        size="sm"
        onClick={save}
        disabled={!wrong.trim() || !right.trim()}
        className="flex items-center gap-1.5 shrink-0"
      >
        <Plus size={14} /> Teach
      </Button>
      {onDone && (
        <button
          type="button"
          onClick={onDone}
          title="Close"
          className="text-text-subtle hover:text-text"
        >
          <X size={14} />
        </button>
      )}
    </div>
  );
};

/** Full list manager for the Post-processing page. */
export const CorrectionsManager: React.FC = () => {
  const { list, update, remove } = useCorrections();
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editWrong, setEditWrong] = useState("");
  const [editRight, setEditRight] = useState("");

  const startEdit = (i: number) => {
    setEditingIndex(i);
    setEditWrong(list[i].wrong);
    setEditRight(list[i].right);
  };
  const cancelEdit = () => setEditingIndex(null);
  const saveEdit = (i: number) => {
    if (update(i, editWrong, editRight)) setEditingIndex(null);
  };

  return (
    <div className="glass-card p-4 space-y-3">
      <div className="flex items-center gap-2">
        <GraduationCap size={16} className="text-aurora-cyan" />
        <h3 className="text-sm font-semibold text-text">Taught corrections</h3>
      </div>
      <p className="text-xs text-text-subtle leading-relaxed">
        Words the transcriber keeps getting wrong. Fixed automatically in every
        transcription (dictation, Notes, Meetings), and the AI clean-up is told
        about them so it catches close variants too. Teach new ones here, or
        select text in a Meetings transcript / Notes and use the Teach button.
      </p>
      <AddCorrectionInline autoFocus={false} />
      {list.length > 0 && (
        <div className="space-y-1">
          {list.map((c, i) =>
            editingIndex === i ? (
              <div
                key={`edit-${i}`}
                className="flex flex-wrap items-center gap-2 text-sm rounded-md px-2 py-1 bg-white/5"
              >
                <input
                  value={editWrong}
                  onChange={(e) => setEditWrong(e.target.value)}
                  className="flex-1 min-w-[120px] bg-white/5 border border-white/10 rounded-md px-2 py-1 text-text focus:outline-none"
                  placeholder="What it heard…"
                />
                <span className="text-text-subtle">→</span>
                <input
                  value={editRight}
                  onChange={(e) => setEditRight(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") saveEdit(i);
                    if (e.key === "Escape") cancelEdit();
                  }}
                  className="flex-1 min-w-[120px] bg-white/5 border border-white/10 rounded-md px-2 py-1 text-text focus:outline-none"
                  placeholder="What it should be…"
                />
                <button
                  type="button"
                  title="Save"
                  onClick={() => saveEdit(i)}
                  disabled={!editWrong.trim() || !editRight.trim()}
                  className="text-text-subtle hover:text-pill-positive transition-colors disabled:opacity-40"
                >
                  <Check size={14} />
                </button>
                <button
                  type="button"
                  title="Cancel"
                  onClick={cancelEdit}
                  className="text-text-subtle hover:text-text transition-colors"
                >
                  <X size={14} />
                </button>
              </div>
            ) : (
              <div
                key={`${c.wrong}-${i}`}
                className="flex items-center gap-2 text-sm rounded-md px-2 py-1 hover:bg-white/5 group"
              >
                <span className="text-text-muted line-through">{c.wrong}</span>
                <span className="text-text-subtle">→</span>
                <span className="text-text flex-1">{c.right}</span>
                <button
                  type="button"
                  title="Edit this correction"
                  onClick={() => startEdit(i)}
                  className="text-text-subtle hover:text-aurora-cyan transition-colors opacity-0 group-hover:opacity-100"
                >
                  <Pencil size={13} />
                </button>
                <button
                  type="button"
                  title="Remove this correction"
                  onClick={() => remove(i)}
                  className="text-text-subtle hover:text-pill-urgent transition-colors"
                >
                  <Trash2 size={13} />
                </button>
              </div>
            ),
          )}
        </div>
      )}
    </div>
  );
};

/**
 * v1.16.0: renders text where every word is click-to-teach. Words stay
 * selectable (they're spans, not buttons) — a click that was actually a
 * text-selection drag is ignored, so copying still works. Surrounding
 * punctuation is stripped from the taught word.
 */
export const ClickableWords: React.FC<{
  text: string;
  onWordClick: (word: string) => void;
}> = ({ text, onWordClick }) => (
  <>
    {text.split(/(\s+)/).map((part, i) => {
      if (!part.trim()) return part;
      const core = part.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
      if (!core) return part;
      return (
        <span
          key={i}
          title={`Teach a correction for "${core}"`}
          onClick={() => {
            // Ignore clicks that were really a selection drag.
            if (window.getSelection()?.toString()) return;
            onWordClick(core);
          }}
          className="cursor-pointer rounded-sm hover:bg-white/10 hover:text-aurora-cyan transition-colors"
        >
          {part}
        </span>
      );
    })}
  </>
);

// ---------------------------------------------------------------------------
// Phase 3: suggestion mining — compare a raw transcript with its LLM-cleaned
// version and propose conservative wrong → right candidates.
// ---------------------------------------------------------------------------

const tokenise = (s: string): string[] =>
  s
    .split(/\s+/)
    .map((w) => w.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, ""))
    .filter((w) => w.length >= 3);

const levenshtein = (a: string, b: string): number => {
  const m = a.length;
  const n = b.length;
  if (!m) return n;
  if (!n) return m;
  let prev = Array.from({ length: n + 1 }, (_, i) => i);
  for (let i = 1; i <= m; i++) {
    const cur = [i];
    for (let j = 1; j <= n; j++) {
      cur[j] = Math.min(
        prev[j] + 1,
        cur[j - 1] + 1,
        prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
    }
    prev = cur;
  }
  return prev[n];
};

/**
 * Conservative mining: pair words that DISAPPEARED in the clean-up with the
 * most-similar word that APPEARED, requiring ≥0.6 similarity. Case-only
 * sentence-capitalisation noise is excluded; macron/case fixes of real words
 * (kōrero, ProjectIQ) survive. Capped at 3 suggestions per run.
 */
export const mineCorrectionSuggestions = (
  raw: string,
  processed: string,
  existing: Correction[],
): Correction[] => {
  const rawWords = new Set(tokenise(raw));
  const procWords = new Set(tokenise(processed));
  const rawOnly = [...rawWords].filter((w) => !procWords.has(w)).slice(0, 200);
  const procOnly = [...procWords].filter((w) => !rawWords.has(w)).slice(0, 200);
  const out: Correction[] = [];

  for (const w of rawOnly) {
    if (existing.some((c) => c.wrong.toLowerCase() === w.toLowerCase())) continue;
    let best: string | null = null;
    let bestSim = 0;
    for (const p of procOnly) {
      const sim =
        1 -
        levenshtein(w.toLowerCase(), p.toLowerCase()) /
          Math.max(w.length, p.length);
      if (sim > bestSim) {
        bestSim = sim;
        best = p;
      }
    }
    if (!best || best === w) continue;
    const lcEqual = w.toLowerCase() === best.toLowerCase();
    // Sentence-start capitalisation is noise, not a correction.
    const firstCharOnly = lcEqual && w.slice(1) === best.slice(1);
    if (firstCharOnly) continue;
    if (lcEqual || bestSim >= 0.6) {
      out.push({ wrong: w, right: best });
      if (out.length >= 3) break;
    }
  }
  return out;
};
