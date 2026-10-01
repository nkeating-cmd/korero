/* eslint-disable i18next/no-literal-string */
import React from "react";
import { Loader2, Sparkles, TriangleAlert } from "lucide-react";
import { Markdown } from "../ui/Markdown";
import type { MeetingsController } from "../settings/meetings/useMeetingsController";

/** Kōrero 1.42: ask the notes model a question about this meeting. */
const SUGGESTIONS = [
  "What did we decide?",
  "List the actions, with who owns each one",
  "What is still unresolved?",
  "Draft a short follow-up email",
];

export const AskPanel: React.FC<{ c: MeetingsController }> = ({ c }) => {
  const m = c.active;
  if (!m) return null;
  return (
    <div className="flex flex-col gap-4 max-w-[780px]">
      <div className="flex gap-2">
        <label htmlFor="kx-ask" className="kx-sr-only">
          Ask a question about this meeting
        </label>
        <input
          id="kx-ask"
          value={c.askQuestion}
          onChange={(e) => c.setAskQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void c.ask();
          }}
          placeholder="Ask anything about this meeting"
          className="kx-input min-h-[40px]"
        />
        <button type="button" className="kx-btn kx-btn-primary shrink-0" disabled={c.asking || !c.askQuestion.trim()} onClick={() => void c.ask()}>
          {c.asking ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />} Ask
        </button>
      </div>
      <div className="flex flex-wrap gap-2">
        {SUGGESTIONS.map((s) => (
          <button
            key={s}
            type="button"
            className="kx-chip kx-chip-button cursor-pointer hover:bg-[var(--kx-raised-2)] min-h-[28px]"
            onClick={() => c.setAskQuestion(s)}
          >
            {s}
          </button>
        ))}
      </div>
      {c.providerLocal === false && (
        <p className="kx-meta flex items-start gap-1.5 text-[var(--kx-warn)]">
          <TriangleAlert size={13} className="mt-0.5 shrink-0" /> Asking sends this transcript to {c.ppLabel}, a cloud model.
        </p>
      )}
      {c.askAnswer && (
        <article className="kx-card px-5 py-4">
          <p className="kx-overline mb-2">{c.askAnswer.q}</p>
          <div className="md-body kx-read max-w-none">
            <Markdown>{c.askAnswer.a}</Markdown>
          </div>
        </article>
      )}
      {!c.askAnswer && !c.asking && (
        <p className="kx-meta">Answers come from the transcript (and honour any trim), using {c.ppLabel}.</p>
      )}
    </div>
  );
};
