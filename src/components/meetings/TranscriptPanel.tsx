/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import { GraduationCap, Pencil, Scissors, TriangleAlert, X, Flag } from "lucide-react";
import { AddCorrectionInline } from "../ui/Corrections";
import type { MeetingsController } from "../settings/meetings/useMeetingsController";
import { Highlighted, containsFolded } from "./Highlighted";
import { fmtClock, fmtTrimMs, hiddenCount, isTrimmed, segInWindow, segMs } from "./model";

/**
 * Kōrero 1.42: the transcript tab. The reading surface leads; speaker labels,
 * trim controls and edit buttons stay quiet until you reach for them. Trim
 * rules are unchanged from v1.27.0: a trim is a view, never an edit.
 */
export const TranscriptPanel: React.FC<{ c: MeetingsController }> = ({ c }) => {
  const m = c.active;
  const firstHitRef = useRef<HTMLDivElement | null>(null);
  const [labelDraft, setLabelDraft] = useState<string>("");

  // From search: scroll to the first line that matches, once.
  useEffect(() => {
    if (c.highlight && firstHitRef.current) {
      firstHitRef.current.scrollIntoView({ block: "center" });
    }
  }, [c.highlight, m?.id]);

  if (!m) return null;
  const ordered = (m.transcript ?? []).length > 0;
  const flags = m.flags ?? [];
  let firstHitAssigned = false;

  const speaker = (key: "you" | "others") => {
    const label = key === "you" ? m.youLabel : m.othersLabel;
    const fallback = key === "you" ? "You" : "Others";
    if (c.editingLabel === key) {
      return (
        <input
          autoFocus
          aria-label={`Rename the ${fallback} speaker`}
          value={labelDraft}
          onChange={(e) => setLabelDraft(e.target.value)}
          onBlur={() => {
            const v = labelDraft.trim() || fallback;
            c.patchMeeting(m.id, key === "you" ? { youLabel: v } : { othersLabel: v });
            c.setEditingLabel(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === "Escape") (e.target as HTMLInputElement).blur();
          }}
          className="kx-input min-h-[30px] w-40 text-[13px]"
        />
      );
    }
    return (
      <button
        type="button"
        className="kx-chip kx-chip-button gap-1.5 cursor-pointer hover:bg-[var(--kx-raised-2)]"
        title={`Rename this speaker (currently "${label}")`}
        onClick={() => {
          setLabelDraft(label);
          c.setEditingLabel(key);
        }}
      >
        <span className={key === "you" ? "kx-accent-ink font-semibold" : "text-white font-semibold"}>{label}</span>
        <Pencil size={11} />
      </button>
    );
  };

  return (
    <div className="flex flex-col gap-4 max-w-[860px]">
      <div className="flex flex-wrap items-center gap-2">
        <span className="kx-meta">Speakers</span>
        {speaker("you")}
        {speaker("others")}
        <div className="flex-1" />
        {c.teachWrong === null && (
          <button
            type="button"
            className="kx-btn kx-btn-quiet kx-btn-sm"
            title="Select a mis-heard word in the transcript first, then click to teach the correction"
            onClick={() => c.setTeachWrong((window.getSelection()?.toString() ?? "").trim().slice(0, 80))}
          >
            <GraduationCap size={14} /> Teach a correction
          </button>
        )}
      </div>
      {c.teachWrong !== null && (
        <AddCorrectionInline initialWrong={c.teachWrong} onDone={() => c.setTeachWrong(null)} />
      )}

      {!m.systemCaptured && !m.imported && (
        <p className="kx-banner kx-banner-warn">
          <TriangleAlert size={16} />
          <span>The call's audio was not captured for this meeting — only your mic was recorded.</span>
        </p>
      )}

      {isTrimmed(m) && (
        <div className="kx-card p-3.5 flex flex-col gap-2.5" style={{ borderLeft: "2px solid var(--kx-accent-ink)" }}>
          <div className="flex flex-wrap items-center gap-3">
            <button
              type="button"
              onClick={c.clearTrim}
              aria-label={`Clear trim, ${hiddenCount(m)} segments hidden`}
              className="kx-chip kx-chip-accent kx-chip-button cursor-pointer font-semibold"
            >
              <Scissors size={12} /> Trimmed · {hiddenCount(m)} hidden <X size={12} />
            </button>
            <span className="kx-mono kx-meta">
              {m.trimStartMs != null ? fmtTrimMs(m.trimStartMs) : "0:00.0"} –{" "}
              {m.trimEndMs != null ? fmtTrimMs(m.trimEndMs) : "end"}
            </span>
          </div>
          <div className="flex flex-wrap items-end gap-3">
            <div className="flex flex-col gap-1">
              <label htmlFor="korero-trim-in" className="kx-overline">
                Start (mm:ss.s)
              </label>
              <input
                id="korero-trim-in"
                type="text"
                inputMode="decimal"
                value={c.trimInDraft}
                placeholder="0:00.0"
                onChange={(e) => c.setTrimInDraft(e.target.value)}
                onBlur={c.commitTrimInputs}
                onKeyDown={(e) => {
                  if (e.key === "Enter") c.commitTrimInputs();
                }}
                aria-invalid={c.trimError !== null}
                className="kx-input kx-mono w-28 min-h-[32px] text-[12.5px]"
              />
            </div>
            <div className="flex flex-col gap-1">
              <label htmlFor="korero-trim-out" className="kx-overline">
                End (mm:ss.s)
              </label>
              <input
                id="korero-trim-out"
                type="text"
                inputMode="decimal"
                value={c.trimOutDraft}
                placeholder="end"
                onChange={(e) => c.setTrimOutDraft(e.target.value)}
                onBlur={c.commitTrimInputs}
                onKeyDown={(e) => {
                  if (e.key === "Enter") c.commitTrimInputs();
                }}
                aria-invalid={c.trimError !== null}
                className="kx-input kx-mono w-28 min-h-[32px] text-[12.5px]"
              />
            </div>
          </div>
          {c.trimError && (
            <p role="alert" className="text-[12.5px] text-[var(--kx-alert)]">
              {c.trimError}
            </p>
          )}
          <p className="kx-meta">
            Trimming hides lines from copy, export, search and notes. The audio is not changed,
            and you can undo it any time.
          </p>
        </div>
      )}

      {flags.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="kx-meta flex items-center gap-1.5">
            <Flag size={13} className="text-[var(--kx-warn)]" /> Flagged
          </span>
          {flags.map((f) => (
            <span key={f.atSec} className="kx-chip kx-mono">
              {fmtClock(f.atSec)}
              {f.label ? ` · ${f.label}` : ""}
            </span>
          ))}
        </div>
      )}

      {c.highlight && (
        <p className="kx-meta">
          Showing matches for <mark className="kx-mark">{c.highlight}</mark>.{" "}
          <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => c.setHighlight("")}>
            Clear
          </button>
        </p>
      )}

      {!ordered ? (
        <div className="flex flex-col gap-4">
          {(
            [
              { key: "you" as const, label: m.youLabel, text: m.you },
              { key: "others" as const, label: m.othersLabel, text: m.others },
            ]
          ).map((row) => (
            <div key={row.key}>
              <h3 className={`kx-overline mb-1 ${row.key === "you" ? "kx-accent-ink" : ""}`}>{row.label}</h3>
              <p className="kx-read whitespace-pre-wrap">
                <Highlighted text={row.text.trim() || "—"} query={c.highlight} />
              </p>
            </div>
          ))}
        </div>
      ) : (
        <div className="flex flex-col">
          {(m.transcript ?? []).map((s, i) => {
            if (!s.text.trim() && c.editingSegIdx !== i) return null;
            const label = s.source === "you" ? m.youLabel : m.othersLabel;
            const excluded = !segInWindow(m, s);
            const flagged = flags.some((f) => Math.abs(f.atSec * 1000 - segMs(s)) < 6000);
            const hit = !!c.highlight && containsFolded(s.text, c.highlight);
            const isFirstHit = hit && !firstHitAssigned;
            if (isFirstHit) firstHitAssigned = true;
            return (
              <div
                key={i}
                ref={isFirstHit ? firstHitRef : undefined}
                className={`kx-seg ${excluded ? "kx-seg-out" : flagged ? "kx-seg-flag" : ""}`}
                title={
                  excluded
                    ? "Outside the current trim — hidden from copy, export, search and notes, but still on the record."
                    : undefined
                }
              >
                <span className={`truncate text-[12.5px] font-semibold pt-[3px] ${s.source === "you" ? "kx-accent-ink" : "text-[var(--kx-ink-soft)]"}`}>
                  {label}
                  <span className="block kx-mono font-normal kx-ink-2 text-[11px]">{fmtClock(segMs(s) / 1000)}</span>
                </span>
                <div className="min-w-0">
                  {c.editingSegIdx === i ? (
                    <input
                      autoFocus
                      aria-label="Edit this line"
                      defaultValue={s.text.trim()}
                      onBlur={(e) => c.saveSegment(i, e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") c.saveSegment(i, (e.target as HTMLInputElement).value);
                        else if (e.key === "Escape") c.setEditingSegIdx(null);
                      }}
                      className="kx-input text-[14.5px]"
                    />
                  ) : (
                    <p className="kx-read m-0 whitespace-pre-wrap break-words">
                      <Highlighted text={s.text.trim()} query={c.highlight} />
                    </p>
                  )}
                </div>
                <div className="kx-seg-actions">
                  <button
                    type="button"
                    onClick={() => c.setTrimPoint("start", segMs(s))}
                    aria-label={`Start the trim at this line (${fmtTrimMs(segMs(s))})`}
                    title={`Start here — hide everything before ${fmtTrimMs(segMs(s))}`}
                  >
                    Start here
                  </button>
                  <button
                    type="button"
                    onClick={() => c.setTrimPoint("end", segMs(s))}
                    aria-label={`End the trim at this line (${fmtTrimMs(segMs(s))})`}
                    title={`End here — hide everything after ${fmtTrimMs(segMs(s))}`}
                  >
                    End here
                  </button>
                  <button type="button" onClick={() => c.setEditingSegIdx(i)} aria-label="Edit this line" title="Edit this line">
                    <Pencil size={11} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
