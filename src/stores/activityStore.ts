// Kōrero 1.42: the Activity log — what finished, what failed and what needs a
// look, across the whole app.
//
// Running work is NOT stored here: the Activity panel derives it live from the
// stores that own the work (meetingJobs, notes, recorder), so it can never
// show a job as running after it ended. This log only records outcomes, for
// this session, newest first.

import { create } from "zustand";
import type { Section } from "./navStore";

export type ActivityStatus = "done" | "attention" | "failed";

export interface ActivityTarget {
  section: Section;
  meetingId?: string;
  tab?: "transcript" | "notes" | "audio" | "ask";
  noteId?: string;
}

export interface ActivityItem {
  id: string;
  status: ActivityStatus;
  title: string;
  detail?: string;
  at: number;
  target?: ActivityTarget;
}

interface ActivityState {
  items: ActivityItem[];
  /** Outcomes added since the panel was last opened. */
  unseen: number;
  add: (item: Omit<ActivityItem, "id" | "at"> & { at?: number }) => void;
  markSeen: () => void;
  dismiss: (id: string) => void;
  clearDone: () => void;
}

const MAX_ITEMS = 60;
let seq = 0;

export const useActivity = create<ActivityState>((set) => ({
  items: [],
  unseen: 0,
  add: (item) =>
    set((s) => ({
      items: [
        { ...item, id: `a${Date.now()}_${seq++}`, at: item.at ?? Date.now() },
        ...s.items,
      ].slice(0, MAX_ITEMS),
      unseen: s.unseen + 1,
    })),
  markSeen: () => set({ unseen: 0 }),
  dismiss: (id) => set((s) => ({ items: s.items.filter((i) => i.id !== id) })),
  clearDone: () =>
    set((s) => ({ items: s.items.filter((i) => i.status !== "done") })),
}));

/** Fire-and-forget logging for stores that are not React components. */
export const logActivity = (
  item: Omit<ActivityItem, "id" | "at"> & { at?: number },
): void => useActivity.getState().add(item);
