import React, { useRef, useState } from "react";
import { Pencil } from "lucide-react";

/**
 * Kōrero 1.46: name the numbered voices of a meeting. "Speaker 1..N" are the voices
 * found in the microphone recording; "Caller 1..N" are the voices found in a call's
 * audio. A name given here replaces the number wherever the transcript is read from
 * then on: on screen, in copy and export, in search, in Ask, and in notes written
 * (or rewritten) after it.
 */
export const SpeakerNames: React.FC<{
  kind: "speaker" | "caller";
  count: number;
  names: string[] | undefined;
  onRename: (names: string[]) => void;
}> = ({ kind, count, names, onRename }) => {
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const chips = useRef<(HTMLButtonElement | null)[]>([]);
  const word = kind === "caller" ? "Caller" : "Speaker";
  // Closing the box puts keyboard focus back on the chip it came from.
  const close = (i: number, save: boolean) => {
    if (save) onRename(withName(names, count, i, draft));
    setEditing(null);
    setTimeout(() => chips.current[i]?.focus(), 0);
  };
  return (
    <>
      {Array.from({ length: count }, (_, i) => {
        const fallback = `${word} ${i + 1}`;
        const name = names?.[i]?.trim() ?? "";
        const label = name || fallback;
        if (editing === i) {
          return (
            <input
              key={i}
              autoFocus
              aria-label={`Name for ${fallback}`}
              placeholder={fallback}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => editing === i && close(i, true)}
              onKeyDown={(e) => {
                if (e.key === "Enter") close(i, true);
                if (e.key === "Escape") close(i, false);
              }}
              className="kx-input min-h-[30px] w-40 text-[13px]"
            />
          );
        }
        return (
          <button
            key={i}
            ref={(el) => {
              chips.current[i] = el;
            }}
            type="button"
            className="kx-chip kx-chip-button gap-1.5 cursor-pointer hover:bg-[var(--kx-raised-2)]"
            aria-label={name ? `Rename ${fallback}, currently ${name}` : `Name ${fallback}`}
            title={name ? `Rename ${fallback} (currently "${name}")` : `Name ${fallback}`}
            onClick={() => {
              setDraft(name);
              setEditing(i);
            }}
          >
            <span className="text-white font-semibold">{label}</span>
            <Pencil size={11} aria-hidden="true" />
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
