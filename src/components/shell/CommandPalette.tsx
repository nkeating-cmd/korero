/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  Search,
  Circle,
  Square,
  Flag,
  NotebookPen,
  Upload,
  Activity,
  Languages,
  RefreshCw,
  PanelLeft,
  UsersRound,
  Mic,
  SlidersHorizontal,
  ExternalLink,
  Loader2,
} from "lucide-react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useNav } from "../../stores/navStore";
import { useRecorder } from "../../stores/recorderStore";
import { useMeetingJobs } from "../../stores/meetingJobsStore";
import { useNotes } from "../../stores/notesStore";
import { useSettingsStore } from "../../stores/settingsStore";
import {
  groupHits,
  search,
  type SearchDoc,
  type SearchHit,
  type SearchKind,
} from "../../lib/search";
import { loadSearchSources, SETTINGS_CATALOGUE, type LoadedSources } from "./searchSources";
import { titleOf } from "../meetings/model";

/**
 * Kōrero 1.42: Ctrl K. One box that finds any meeting (title, transcript or
 * notes), note, dictation or setting — macron-insensitive — and runs the
 * common actions. Keyboard first: ↑ ↓ to move, Enter to open, Esc to close.
 */

interface ActionDef {
  id: string;
  title: string;
  keywords: string;
  hint?: string;
  icon: React.ReactNode;
  run: () => void | Promise<void>;
}

const KIND_LABEL: Record<SearchKind, string> = {
  action: "Actions",
  meeting: "Meetings",
  note: "Notes",
  dictation: "Dictations",
  setting: "Settings",
};

const KIND_ICON: Record<Exclude<SearchKind, "action">, React.ReactNode> = {
  meeting: <UsersRound size={16} />,
  note: <NotebookPen size={16} />,
  dictation: <Mic size={16} />,
  setting: <SlidersHorizontal size={16} />,
};

interface Row {
  key: string;
  kind: SearchKind;
  title: string;
  meta?: string;
  hit?: SearchHit;
  icon: React.ReactNode;
  hint?: string;
  run: () => void;
}

export const CommandPalette: React.FC = () => {
  const open = useNav((s) => s.paletteOpen);
  const setOpen = useNav((s) => s.setPaletteOpen);
  const recording = useRecorder((s) => s.recording);
  const selectedLanguage = useSettingsStore((s) => s.settings?.selected_language);
  const [query, setQuery] = useState("");
  const [sources, setSources] = useState<LoadedSources | null>(null);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);

  const close = () => setOpen(false);

  useEffect(() => {
    if (!open) return;
    returnFocus.current = document.activeElement as HTMLElement | null;
    setQuery("");
    setActive(0);
    setSources(null);
    let cancelled = false;
    void loadSearchSources().then((s) => {
      if (!cancelled) setSources(s);
    });
    requestAnimationFrame(() => inputRef.current?.focus());
    return () => {
      cancelled = true;
      returnFocus.current?.focus?.();
    };
  }, [open]);

  const nzOn = selectedLanguage === "en-NZ";

  const actions: ActionDef[] = useMemo(() => {
    const nav = useNav.getState();
    const list: ActionDef[] = [];
    if (!recording) {
      list.push({
        id: "record",
        title: "Record a meeting",
        keywords: "start meeting record call capture",
        icon: <Circle size={14} fill="currentColor" />,
        run: async () => {
          const ok = await useRecorder.getState().start();
          if (ok) nav.openMeeting({ id: "live" });
        },
      });
    } else {
      list.push(
        {
          id: "stop",
          title: "Stop the recording",
          keywords: "stop end meeting finish",
          icon: <Square size={13} fill="currentColor" />,
          run: () => useRecorder.getState().stop(),
        },
        {
          id: "flag",
          title: "Flag this moment",
          keywords: "mark bookmark important moment",
          icon: <Flag size={15} />,
          run: () => {
            useRecorder.getState().flag();
            toast.success("Moment flagged.");
          },
        },
        {
          id: "live",
          title: "Show the live meeting",
          keywords: "recording live transcript",
          icon: <UsersRound size={16} />,
          run: () => nav.openMeeting({ id: "live" }),
        },
        {
          id: "popout",
          title: "Pop out the recorder",
          keywords: "companion small window always on top",
          icon: <ExternalLink size={15} />,
          run: () => useRecorder.getState().openCompanion(),
        },
      );
    }
    list.push(
      {
        id: "note",
        title: "Start a new note",
        keywords: "new note write dictate",
        icon: <NotebookPen size={16} />,
        run: () => {
          const id = useNotes.getState().addNote();
          nav.openNote(id);
        },
      },
      {
        id: "import",
        title: "Import meeting audio",
        keywords: "import file m4a wav mp3 upload transcribe",
        icon: <Upload size={15} />,
        run: async () => {
          if (useRecorder.getState().recording || useMeetingJobs.getState().stopping) {
            toast.message("Finish the meeting first: importing uses the same speech engine.");
            return;
          }
          try {
            const sel = await openFileDialog({
              multiple: false,
              filters: [
                {
                  name: "Audio files",
                  extensions: ["wav", "m4a", "mp3", "aac", "flac", "ogg", "caf", "aiff", "aif"],
                },
              ],
            });
            if (typeof sel === "string") nav.openMeeting({ id: `import:${sel}` });
          } catch (e) {
            toast.error(`Could not open the file picker: ${String(e)}`);
          }
        },
      },
      {
        id: "activity",
        title: "Show Activity",
        keywords: "background jobs running done progress",
        icon: <Activity size={16} />,
        run: () => nav.setActivityOpen(true),
      },
      {
        id: "nz",
        title: nzOn ? "Turn New Zealand English off" : "Turn New Zealand English on",
        keywords: "nz english macrons te reo spelling language",
        hint: nzOn ? "On" : "Off",
        icon: <Languages size={16} />,
        run: async () => {
          await useSettingsStore
            .getState()
            .updateSetting("selected_language", nzOn ? "en" : "en-NZ");
          toast.success(nzOn ? "New Zealand English is off." : "New Zealand English is on.");
        },
      },
      {
        id: "updates",
        title: "Check for updates",
        keywords: "update version upgrade",
        icon: <RefreshCw size={15} />,
        run: async () => {
          const r = await commands.triggerUpdateCheck();
          if (r.status === "ok") toast.message("Checking GitHub for a newer Kōrero…");
          else toast.error(r.error);
        },
      },
      {
        id: "sidebar",
        title: "Collapse or expand the sidebar",
        keywords: "sidebar navigation compact rail",
        icon: <PanelLeft size={16} />,
        run: () => nav.toggleSidebar(),
      },
    );
    return list;
  }, [recording, nzOn]);

  const actionDocs: SearchDoc[] = useMemo(
    () =>
      actions.map((a) => ({
        id: `action:${a.id}`,
        kind: "action" as const,
        title: a.title,
        keywords: a.keywords,
      })),
    [actions],
  );

  const runFor = (doc: SearchDoc, q: string): (() => void) => {
    const nav = useNav.getState();
    const [kind, ...rest] = doc.id.split(":");
    const id = rest.join(":");
    switch (kind) {
      case "action":
        return () => void actions.find((a) => a.id === id)?.run();
      case "meeting":
        return () => {
          const inTitle = doc.title.toLowerCase().includes(q.trim().toLowerCase());
          nav.openMeeting({
            id,
            tab: q.trim() && !inTitle ? "transcript" : undefined,
            query: q.trim() || undefined,
          });
        };
      case "note":
        return () => nav.openNote(id);
      case "dictation":
        return () => nav.go("history");
      case "setting":
        return () => {
          const s = SETTINGS_CATALOGUE.find((x) => x.id === id);
          if (s) nav.go(s.section);
        };
      default:
        return () => {};
    }
  };

  const rows: { kind: SearchKind; label: string; rows: Row[] }[] = useMemo(() => {
    const q = query.trim();
    if (!q) {
      const suggested: Row[] = actions.slice(0, recording ? 4 : 3).map((a) => ({
        key: `action:${a.id}`,
        kind: "action",
        title: a.title,
        hint: a.hint,
        icon: a.icon,
        run: () => void a.run(),
      }));
      const recent: Row[] = (sources?.meetings ?? [])
        .slice()
        .sort((a, b) => b.createdAt - a.createdAt)
        .slice(0, 4)
        .map((m) => ({
          key: `meeting:${m.id}`,
          kind: "meeting",
          title: titleOf(m),
          meta: new Date(m.createdAt).toLocaleDateString(undefined, {
            weekday: "short",
            day: "numeric",
            month: "short",
          }),
          icon: KIND_ICON.meeting,
          run: () => useNav.getState().openMeeting({ id: m.id }),
        }));
      return [
        { kind: "action" as SearchKind, label: "Suggested", rows: suggested },
        ...(recent.length
          ? [{ kind: "meeting" as SearchKind, label: "Recent meetings", rows: recent }]
          : []),
      ];
    }
    const docs = [...actionDocs, ...(sources?.docs ?? [])];
    return groupHits(search(docs, q, 5)).map((g) => ({
      kind: g.kind,
      label: KIND_LABEL[g.kind],
      rows: g.hits.map((h) => {
        const action = h.doc.kind === "action" ? actions.find((a) => `action:${a.id}` === h.doc.id) : undefined;
        return {
          key: h.doc.id,
          kind: h.doc.kind,
          title: h.doc.title,
          meta: h.doc.meta,
          hit: h,
          hint: action?.hint,
          icon: action ? action.icon : KIND_ICON[h.doc.kind as Exclude<SearchKind, "action">],
          run: runFor(h.doc, q),
        };
      }),
    }));
  }, [query, sources, actions, actionDocs, recording]);

  const flat = rows.flatMap((g) => g.rows);
  const activeRow = flat[Math.min(active, Math.max(0, flat.length - 1))];

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    if (!activeRow) return;
    document
      .getElementById(`kx-cmd-${cssId(activeRow.key)}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeRow?.key]);

  if (!open) return null;

  const choose = (row: Row | undefined) => {
    if (!row) return;
    close();
    // After the dialog has gone, so focus lands on the destination.
    requestAnimationFrame(() => row.run());
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (flat.length ? (i + 1) % flat.length : 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (flat.length ? (i - 1 + flat.length) % flat.length : 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(activeRow);
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") {
      // The input is the only stop; keep focus inside the dialog.
      e.preventDefault();
    }
  };

  const loading = sources === null && query.trim().length > 0;
  const nothing = !loading && query.trim() && flat.length === 0;

  return (
    <div
      className="fixed inset-0 z-[60] flex items-start justify-center"
      style={{ background: "rgba(0,0,0,0.62)", paddingTop: "12vh" }}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Search and commands"
        className="kx-raised kx-pop w-[640px] max-w-[calc(100vw-32px)] overflow-hidden"
        style={{ boxShadow: "0 8px 24px rgba(0,0,0,.6), 0 24px 56px rgba(0,0,0,.5)" }}
        onKeyDown={onKeyDown}
      >
        <div className="flex items-center gap-2.5 px-4 border-b border-[#2f2f33]">
          <Search size={17} className="kx-ink-2 shrink-0" aria-hidden="true" />
          <label htmlFor="kx-cmd-input" className="kx-sr-only">
            Search meetings, notes, dictations and settings, or run a command
          </label>
          <input
            id="kx-cmd-input"
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search meetings, notes, dictations, settings…"
            role="combobox"
            aria-expanded="true"
            aria-controls="kx-cmd-list"
            aria-activedescendant={activeRow ? `kx-cmd-${cssId(activeRow.key)}` : undefined}
            aria-autocomplete="list"
            autoComplete="off"
            spellCheck={false}
            className="kx-bare-input flex-1 min-h-[54px] bg-transparent text-[16px] text-white placeholder:text-[var(--kx-ink-2)]"
          />
          {loading && <Loader2 size={15} className="animate-spin kx-ink-2" aria-label="Loading" />}
          <span className="kx-kbd">Esc</span>
        </div>

        <div
          id="kx-cmd-list"
          ref={listRef}
          role="listbox"
          aria-label="Results"
          className="max-h-[56vh] overflow-y-auto p-2"
        >
          {nothing && (
            <p className="px-3 py-6 text-center kx-meta">
              Nothing matches “{query.trim()}”. Searches titles, transcripts,
              notes and settings, with or without macrons.
            </p>
          )}
          {rows.map((g) => (
            <div key={g.label} role="group" aria-label={g.label}>
              <div className="kx-overline px-2.5 pt-2.5 pb-1">{g.label}</div>
              {g.rows.map((r) => {
                const i = flat.indexOf(r);
                const isActive = r === activeRow;
                return (
                  <div
                    key={r.key}
                    id={`kx-cmd-${cssId(r.key)}`}
                    role="option"
                    aria-selected={isActive}
                    onMouseMove={() => setActive(i)}
                    onClick={() => choose(r)}
                    className={`flex gap-3 items-start px-2.5 py-2.5 rounded-[9px] cursor-pointer ${isActive ? "bg-[var(--kx-selected)]" : ""}`}
                  >
                    <span className={`mt-0.5 shrink-0 ${isActive ? "kx-accent-ink" : "kx-ink-2"}`}>{r.icon}</span>
                    <span className="flex-1 min-w-0">
                      <span className="flex items-baseline justify-between gap-3">
                        <span className={`truncate text-[13.5px] ${isActive ? "font-semibold text-white" : "text-white"}`}>
                          {r.title}
                        </span>
                        <span className={`shrink-0 text-[12px] ${isActive ? "text-[var(--kx-selected-ink)]" : "kx-ink-2"}`}>
                          {r.hint ??
                            [r.meta, r.hit && r.hit.matches > 1 && r.kind === "meeting" ? `${r.hit.matches} matches` : ""]
                              .filter(Boolean)
                              .join(" · ")}
                        </span>
                      </span>
                      {r.hit?.snippet && (
                        <span className={`block text-[12.5px] mt-0.5 truncate ${isActive ? "text-[var(--kx-selected-ink)]" : "kx-ink-2"}`}>
                          {r.hit.snippet.before}
                          <mark className="bg-transparent text-white font-semibold">{r.hit.snippet.match}</mark>
                          {r.hit.snippet.after}
                        </span>
                      )}
                    </span>
                    {isActive && <span className="kx-kbd self-center">Enter</span>}
                  </div>
                );
              })}
            </div>
          ))}
        </div>

        <div className="flex flex-wrap gap-x-5 gap-y-1 px-4 py-2.5 border-t border-[#2f2f33] kx-meta">
          <span>
            <span className="kx-mono">↑ ↓</span> move
          </span>
          <span>
            <span className="kx-mono">Enter</span> open
          </span>
          <span>Macrons optional: “whanau” finds “whānau”</span>
        </div>
      </div>
    </div>
  );
};

const cssId = (s: string) => s.replace(/[^a-zA-Z0-9_-]/g, "_");
