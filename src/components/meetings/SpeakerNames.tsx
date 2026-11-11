import React, { useState } from "react";
import { Pencil } from "lucide-react";

/**
 * Kōrero 1.46: name the numbered voices of a meeting. "Speaker 1..N" are the voices
 * found in the microphone recording; "Caller 1..N" are the voices found in a call's
 * audio. A name given here replaces the number everywhere the transcript is read:
 * on screen, in copy and export, in search, in Ask and in the notes.
 */
export const SpeakerNames: React.FC<{
  kind: "speaker" | "caller";
  count: number;
  names: string[] | undefined;
  onRename: (names: string[]) => void;
}> = ({ kind, count, names, onRename }) => {
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const word = kind === "caller" ? "Caller" : "Speaker";
  const save = (i: number) => {
    onRename(withName(names, count, i, draft));
    setEditing(null);
  };
  return (
    <>
      {Array.from({ length: count }, (_, i) => {
        const fallback = `${word} ${i + 1}`;
        const label = names?.[i]?.trim() || fallback;
        if (editing === i) {
          return (
            <input
              key={i}
              autoFocus
              aria-label={`Name for ${fallback}`}
              placeholder={fallback}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => save(i)}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                if (e.key === "Escape") setEditing(null);
              }}
              className="kx-input min-h-[30px] w-40 text-[13px]"
            />
          );
        }
        return (
          <button
            key={i}
            type="button"
            className="kx-chip kx-chip-button gap-1.5 cursor-pointer hover:bg-[var(--kx-raised-2)]"
            title={`Name this ${kind} (currently "${label}")`}
            onClick={() => {
              setDraft(names?.[i]?.trim() ?? "");
              setEditing(i);
            }}
          >
            <span className="text-white font-semibold">{label}</span>
            <Pencil size={11} />
          </button>
        );
      })}
    </>
  );
};

/** The names list with entry `i` set (trimmed; blank clears it), padded to `count`. */
export const withName = (names: string[] | undefined, count: number, i: number, name: string): string[] => {
  const next = Array.from({ length: Math.max(count, i + 1) }, (_, k) => names?.[k]?.trim() ?? "");
  next[i] = name.trim();
  return next;
};
