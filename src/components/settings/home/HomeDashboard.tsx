/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useState } from "react";
import {
  NotebookPen,
  Sparkles,
  History as HistoryIcon,
  LifeBuoy,
  Mic,
  Copy,
  Check,
} from "lucide-react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { commands, type HistoryEntry } from "../../../bindings";
import i18n from "../../../i18n";
import { formatRelativeTime } from "../../../utils/dateFormat";

/**
 * Kōrero fork (v1.12.0): Home dashboard.
 *
 * The landing surface — quick-nav cards plus recent dictations pulled live from
 * History — so the app opens as a product, not a settings list. `onNavigate`
 * switches sidebar sections (wired from App.tsx).
 */

interface HomeDashboardProps {
  onNavigate?: (section: string) => void;
}

const greeting = () => {
  const h = new Date().getHours();
  if (h < 12) return "Good morning";
  if (h < 18) return "Good afternoon";
  return "Good evening";
};

/**
 * Kōrero (UX round, 2026-09-02): route through the SHARED formatter.
 *
 * This function used to hand-roll a relative-time ladder, and it produced three
 * different formats inside one list — "5 min ago", "2 h ago", then a bare
 * "18/08/2026" for anything older than a day. Three formats in one column is
 * the most visible polish defect on the home screen.
 *
 * It was also the only date code in the app that was not localised. "min ago"
 * and "h ago" were English string literals in a product that ships **20
 * locales**, and the fallback called `toLocaleDateString()` with **no locale
 * argument**, so it followed the operating system rather than the app's own
 * language setting — a user running Kōrero in French on an English Windows got
 * English dates.
 *
 * `formatRelativeTime` already solved all of it: a full second→year ladder on
 * `Intl.RelativeTimeFormat`, with an absolute-time fallback if anything throws.
 * It was sitting in `src/utils/dateFormat.ts`, unused by this file.
 *
 * ⚠ Contract: the shared formatter takes **seconds, as a string**. History
 * timestamps arrive as seconds OR milliseconds, so the normalisation below is
 * load-bearing — inverted, every timestamp reads as 1970.
 */
const relativeTime = (ts: number) => {
  const seconds = ts < 1e12 ? ts : Math.floor(ts / 1000);
  return formatRelativeTime(String(seconds), i18n.language);
};

const QuickAction: React.FC<{
  icon: React.ReactNode;
  title: string;
  subtitle: string;
  onClick: () => void;
}> = ({ icon, title, subtitle, onClick }) => (
  /* v1.32.1: opaque card on the token ladder, with a real hover/active pair.
     Was glass over an animated aurora — four blurred cards competing with the
     background and with each other, none of them leading. */
  <button
    type="button"
    onClick={onClick}
    className="k-card-action flex items-start gap-3.5 p-4 text-left"
  >
    <span className="k-accent-text shrink-0 mt-0.5">{icon}</span>
    <span className="flex flex-col gap-0.5">
      <span className="text-sm font-semibold text-text">{title}</span>
      <span className="text-xs text-text-subtle">{subtitle}</span>
    </span>
  </button>
);

export const HomeDashboard: React.FC<HomeDashboardProps> = ({ onNavigate }) => {
  const [recent, setRecent] = useState<HistoryEntry[] | null>(null);
  const [copiedId, setCopiedId] = useState<number | null>(null);

  const go = (section: string) => onNavigate?.(section);

  const load = async () => {
    try {
      const res = await commands.getHistoryEntries(null, 6);
      if (res.status === "ok") {
        setRecent(res.data.entries.filter((e) => e.transcription_text.trim()));
      } else {
        setRecent([]);
      }
    } catch {
      setRecent([]);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const copyEntry = async (e: HistoryEntry) => {
    const text = e.post_processed_text || e.transcription_text;
    try {
      await writeText(text);
      setCopiedId(e.id);
      window.setTimeout(() => setCopiedId((c) => (c === e.id ? null : c)), 1500);
    } catch {
      /* no-op */
    }
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-7">
      {/* v1.32.1: the greeting is the one focal point on this screen, so it
          gets the display face and a real size step. It was 24px semibold —
          barely above the card titles it was supposed to lead. */}
      <div className="px-1 pt-1">
        <h1 className="type-display text-text">{greeting()}</h1>
        <p className="text-sm text-text-subtle mt-1.5 max-w-[54ch]">
          Press your dictate shortcut anywhere to turn speech into text — or
          start below.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <QuickAction
          icon={<NotebookPen size={20} />}
          title="New note"
          subtitle="Dictate a long note in-app"
          onClick={() => go("notes")}
        />
        <QuickAction
          icon={<Sparkles size={20} />}
          title="Post-processing"
          subtitle="Clean-up & rewrite prompts"
          onClick={() => go("postprocessing")}
        />
        <QuickAction
          icon={<HistoryIcon size={20} />}
          title="History"
          subtitle="Your past dictations"
          onClick={() => go("history")}
        />
        <QuickAction
          icon={<LifeBuoy size={20} />}
          title="Help & shortcuts"
          subtitle="How to drive Kōrero"
          onClick={() => go("help")}
        />
      </div>

      <div className="space-y-2.5">
        <div className="flex items-center justify-between px-1">
          <h2 className="type-overline">Recent dictations</h2>
          <button
            type="button"
            onClick={() => go("history")}
            className="k-accent-text text-xs font-semibold hover:underline"
          >
            View all
          </button>
        </div>

        {/* v1.32.1: opaque. This is a list of transcript text — the thing the
            product exists to produce — and it was set behind a blur. */}
        <div className="k-card p-1.5">
          {recent === null ? (
            <div className="px-4 py-6 text-sm text-text-subtle text-center">
              Loading…
            </div>
          ) : recent.length === 0 ? (
            <div className="px-4 py-8 flex flex-col items-center gap-2 text-center">
              <Mic size={22} className="text-text-subtle" />
              <p className="text-sm text-text-muted">No dictations yet</p>
              <p className="text-xs text-text-subtle">
                Press your dictate shortcut, or start a note, and it will appear
                here.
              </p>
            </div>
          ) : (
            <div>
              {recent.map((e) => (
                <div
                  key={e.id}
                  className="k-row group flex items-center gap-3 px-4 py-3"
                >
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-text truncate">
                      {e.post_processed_text || e.transcription_text}
                    </p>
                    {/* tnum: these are relative times that tick. */}
                    <p className="tnum text-xs text-text-subtle mt-0.5">
                      {relativeTime(e.timestamp)}
                      {e.post_process_requested ? " · cleaned up" : ""}
                    </p>
                  </div>
                  <button
                    type="button"
                    title="Copy"
                    onClick={() => copyEntry(e)}
                    className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 text-text-subtle hover:text-[var(--k-accent-ink)] transition-opacity shrink-0"
                  >
                    {copiedId === e.id ? (
                      <Check size={16} />
                    ) : (
                      <Copy size={16} />
                    )}
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
