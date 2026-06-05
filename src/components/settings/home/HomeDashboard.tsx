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

const relativeTime = (ts: number) => {
  // History timestamps may be seconds or ms; normalise to ms.
  const ms = ts < 1e12 ? ts * 1000 : ts;
  const diff = Date.now() - ms;
  const mins = Math.round(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  const hrs = Math.round(mins / 60);
  if (hrs < 24) return `${hrs} h ago`;
  return new Date(ms).toLocaleDateString();
};

const QuickAction: React.FC<{
  icon: React.ReactNode;
  title: string;
  subtitle: string;
  onClick: () => void;
}> = ({ icon, title, subtitle, onClick }) => (
  <button
    type="button"
    onClick={onClick}
    className="glass-card glass-card-interactive flex items-start gap-3 p-4 text-left"
  >
    <span className="text-aurora-cyan shrink-0 mt-0.5">{icon}</span>
    <span className="flex flex-col">
      <span className="text-sm font-medium text-text">{title}</span>
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
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <div className="px-1 pt-1">
        <h1 className="text-2xl font-semibold text-text tracking-tight">
          {greeting()}
        </h1>
        <p className="text-sm text-text-subtle mt-1">
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

      <div className="space-y-2">
        <div className="flex items-center justify-between px-1">
          <h2 className="text-xs font-semibold text-text-muted uppercase tracking-wider">
            Recent dictations
          </h2>
          <button
            type="button"
            onClick={() => go("history")}
            className="text-xs text-aurora-cyan hover:underline"
          >
            View all
          </button>
        </div>

        <div className="glass-card p-1.5">
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
            <div className="divide-y divide-glass-border">
              {recent.map((e) => (
                <div
                  key={e.id}
                  className="group flex items-center gap-3 px-4 py-2.5"
                >
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-text truncate">
                      {e.post_processed_text || e.transcription_text}
                    </p>
                    <p className="text-xs text-text-subtle">
                      {relativeTime(e.timestamp)}
                      {e.post_process_requested ? " · cleaned up" : ""}
                    </p>
                  </div>
                  <button
                    type="button"
                    title="Copy"
                    onClick={() => copyEntry(e)}
                    className="opacity-0 group-hover:opacity-100 text-text-subtle hover:text-aurora-cyan transition-opacity shrink-0"
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
