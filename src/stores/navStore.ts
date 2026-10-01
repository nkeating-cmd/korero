// Kōrero 1.42: where the app is looking. One store, so Today, Ctrl K, the
// Activity panel and the recorder can all send you somewhere — including to a
// particular meeting and tab — without threading callbacks through App.tsx.

import { create } from "zustand";

export type Section =
  | "home"
  | "general"
  | "notes"
  | "meetings"
  | "audiobrief"
  | "history"
  | "models"
  | "postprocessing"
  | "advanced"
  | "debug"
  | "help";

export type MeetingTab = "transcript" | "notes" | "audio" | "ask";

export interface MeetingFocus {
  /** A meeting id, or "live" for the recording in progress. */
  id: string;
  tab?: MeetingTab;
  /** Text to highlight in the transcript (from search). */
  query?: string;
}

const COLLAPSE_KEY = "korero.sidebar.collapsed";

const readCollapsed = (): boolean => {
  try {
    return localStorage.getItem(COLLAPSE_KEY) === "1";
  } catch {
    return false;
  }
};

interface NavState {
  section: Section;
  /** Bumped on every navigation, so a page can react to "go here again". */
  navSeq: number;
  meetingFocus: MeetingFocus | null;
  noteFocus: string | null;
  paletteOpen: boolean;
  activityOpen: boolean;
  sidebarCollapsed: boolean;
  go: (section: Section) => void;
  openMeeting: (focus: MeetingFocus) => void;
  takeMeetingFocus: () => MeetingFocus | null;
  openNote: (id: string) => void;
  takeNoteFocus: () => string | null;
  setPaletteOpen: (open: boolean) => void;
  setActivityOpen: (open: boolean) => void;
  toggleSidebar: () => void;
}

export const useNav = create<NavState>((set, get) => ({
  section: "home",
  navSeq: 0,
  meetingFocus: null,
  noteFocus: null,
  paletteOpen: false,
  activityOpen: false,
  sidebarCollapsed: readCollapsed(),
  go: (section) => set((s) => ({ section, navSeq: s.navSeq + 1 })),
  openMeeting: (focus) =>
    set((s) => ({
      section: "meetings",
      meetingFocus: focus,
      navSeq: s.navSeq + 1,
    })),
  takeMeetingFocus: () => {
    const f = get().meetingFocus;
    if (f) set({ meetingFocus: null });
    return f;
  },
  openNote: (id) =>
    set((s) => ({ section: "notes", noteFocus: id, navSeq: s.navSeq + 1 })),
  takeNoteFocus: () => {
    const f = get().noteFocus;
    if (f) set({ noteFocus: null });
    return f;
  },
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  setActivityOpen: (activityOpen) => set({ activityOpen }),
  toggleSidebar: () =>
    set((s) => {
      const next = !s.sidebarCollapsed;
      try {
        localStorage.setItem(COLLAPSE_KEY, next ? "1" : "0");
      } catch {
        /* a remembered layout is a convenience, not state */
      }
      return { sidebarCollapsed: next };
    }),
}));
