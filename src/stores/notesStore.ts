// Kōrero 1.42: Notes, outside React (fixes N1, N2 and N3 from the 18 August
// sweep).
//
//   N1. Leaving Notes mid-dictation lost the finished dictation: the result of
//       `noteStopDictation` was written into the page's own state, which no
//       longer existed.
//   N2. "Process note" was lost the same way, and its re-entrancy guard was
//       page state, so coming back let a second run start over the first.
//   N3. Leaving mid-dictation left the microphone held with no way to release
//       it: nothing remembered that a dictation was running, and
//       `noteCancelDictation` had no caller.
//
// Same cure as Meetings in 1.41: the store owns the notes, the dictation and
// the processing run. The page only draws them. A dictation that finishes
// while the page is closed is appended to the end of its note; while the page
// is open, the page inserts it at the cursor.

import { create } from "zustand";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { logActivity } from "./activityStore";

export interface Note {
  id: string;
  title: string;
  content: string;
  updatedAt: number;
}

export const NOTES_STORE_KEY = "korero.notes.v1";

const newNoteId = () =>
  (crypto as unknown as { randomUUID?: () => string })?.randomUUID?.() ??
  `n_${Date.now()}_${Math.random()}`;

export const blankNote = (): Note => ({
  id: newNoteId(),
  title: "",
  content: "",
  updatedAt: Date.now(),
});

export const readStoredNotes = (): Note[] => {
  try {
    const raw = localStorage.getItem(NOTES_STORE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.length) {
        // Normalise, so one malformed entry can never take a page down.
        const notes = parsed
          .filter((n): n is Record<string, unknown> => !!n && typeof n === "object")
          .map((n) => ({
            id: typeof n.id === "string" && n.id ? n.id : newNoteId(),
            title: typeof n.title === "string" ? n.title : "",
            content: typeof n.content === "string" ? n.content : "",
            updatedAt: typeof n.updatedAt === "number" ? n.updatedAt : Date.now(),
          }));
        if (notes.length) return notes;
      }
    }
  } catch {
    /* ignore a corrupt store */
  }
  return [blankNote()];
};

export const noteTitle = (n: Pick<Note, "title" | "content">): string => {
  if (n.title.trim()) return n.title.trim();
  const firstLine = n.content.split("\n").find((l) => l.trim());
  return firstLine ? firstLine.trim().slice(0, 40) : "Untitled note";
};

/** The page registers this while it is open, so dictation lands at the cursor. */
export type NoteInserter = (noteId: string, text: string) => string | null;

interface NotesState {
  notes: Note[];
  activeId: string;
  dictation: "idle" | "starting" | "recording" | "finishing";
  dictationNoteId: string | null;
  dictationStartedAt: number | null;
  processing: { noteId: string; startedAt: number } | null;
  snapshot: { id: string; content: string } | null;
  inserter: NoteInserter | null;

  setActive: (id: string) => void;
  addNote: () => string;
  deleteNote: (id: string) => void;
  patchNote: (id: string, patch: Partial<Omit<Note, "id">>) => void;
  setInserter: (fn: NoteInserter | null) => void;
  startDictation: () => Promise<void>;
  stopDictation: (opts: {
    cleanup: boolean;
    prompt: string;
    model: string | null;
    onSuggest?: (before: string, after: string) => void;
  }) => Promise<void>;
  cancelDictation: () => Promise<void>;
  processNote: (opts: {
    noteId: string;
    content: string;
    prompt: string;
    model: string | null;
    onSuggest?: (before: string, after: string) => void;
  }) => Promise<void>;
  undo: () => void;
}

const initial = readStoredNotes();

export const useNotes = create<NotesState>((set, get) => ({
  notes: initial,
  activeId: initial[0].id,
  dictation: "idle",
  dictationNoteId: null,
  dictationStartedAt: null,
  processing: null,
  snapshot: null,
  inserter: null,

  setActive: (activeId) => set({ activeId }),

  addNote: () => {
    const n = blankNote();
    set((s) => ({ notes: [n, ...s.notes], activeId: n.id }));
    return n.id;
  },

  deleteNote: (id) =>
    set((s) => {
      if (s.dictationNoteId === id && s.dictation !== "idle") {
        toast.message("Stop the dictation in this note before deleting it.");
        return {};
      }
      const next = s.notes.filter((n) => n.id !== id);
      const list = next.length ? next : [blankNote()];
      return {
        notes: list,
        activeId: id === s.activeId ? list[0].id : s.activeId,
      };
    }),

  patchNote: (id, patch) =>
    set((s) => ({
      notes: s.notes.map((n) =>
        n.id === id ? { ...n, ...patch, updatedAt: Date.now() } : n,
      ),
    })),

  setInserter: (inserter) => set({ inserter }),

  startDictation: async () => {
    const s = get();
    if (s.dictation !== "idle" || s.processing) return;
    set({ dictation: "starting", dictationNoteId: s.activeId });
    try {
      const res = await commands.noteStartDictation();
      if (res.status === "ok") {
        set({ dictation: "recording", dictationStartedAt: Date.now() });
      } else {
        set({ dictation: "idle", dictationNoteId: null });
        toast.error(res.error);
      }
    } catch (e) {
      set({ dictation: "idle", dictationNoteId: null });
      toast.error(`Could not start dictation: ${String(e)}`);
    }
  },

  stopDictation: async ({ cleanup, prompt, model, onSuggest }) => {
    const s = get();
    if (s.dictation !== "recording") return;
    const noteId = s.dictationNoteId ?? s.activeId;
    set({ dictation: "finishing" });
    try {
      // Always the RAW transcript: clean-up applies to the whole note.
      const res = await commands.noteStopDictation(false);
      if (res.status !== "ok") {
        toast.error(`Dictation failed: ${res.error}`);
        logActivity({ status: "failed", title: "Dictation failed", detail: res.error });
        return;
      }
      const text = res.data?.trim() ?? "";
      if (!text) {
        toast.message("No speech detected.");
        return;
      }
      // At the cursor if the page is open on this note; at the end otherwise.
      let updated = get().inserter?.(noteId, text) ?? null;
      if (updated === null) {
        const note = get().notes.find((n) => n.id === noteId);
        if (!note) {
          // The note was deleted while dictating: keep the words in a new one.
          const n = { ...blankNote(), content: text };
          set((st) => ({ notes: [n, ...st.notes] }));
          updated = text;
          toast.message("Your dictation was saved as a new note.");
        } else {
          const sep = note.content && !/\s$/.test(note.content) ? " " : "";
          updated = note.content + sep + text;
          get().patchNote(noteId, { content: updated });
          toast.success("Dictation added to your note.");
        }
        logActivity({
          status: "done",
          title: "Dictation added to a note",
          target: { section: "notes", noteId },
        });
      }
      if (cleanup && updated) {
        await get().processNote({ noteId, content: updated, prompt, model, onSuggest });
      }
    } catch (e) {
      toast.error(`Dictation failed: ${String(e)}`);
    } finally {
      set({ dictation: "idle", dictationNoteId: null, dictationStartedAt: null });
    }
  },

  cancelDictation: async () => {
    const s = get();
    if (s.dictation !== "recording" && s.dictation !== "starting") return;
    try {
      await commands.noteCancelDictation();
    } catch {
      /* the backend may already have stopped; the UI still resets */
    }
    set({ dictation: "idle", dictationNoteId: null, dictationStartedAt: null });
    toast.message("Dictation cancelled.");
  },

  processNote: async ({ noteId, content, prompt, model, onSuggest }) => {
    const text = content.trim();
    if (!text) return;
    if (get().processing) {
      toast.message("Already processing a note — one at a time.");
      return;
    }
    set({ processing: { noteId, startedAt: Date.now() } });
    try {
      const res = await commands.notePostProcess(text, prompt, model);
      if (res.status !== "ok") {
        toast.error(res.error);
        logActivity({
          status: "failed",
          title: "Processing a note failed",
          detail: res.error,
          target: { section: "notes", noteId },
        });
        return;
      }
      const out = res.data.trim();
      if (!out) {
        toast.error("The model returned no output — note unchanged.");
        return;
      }
      if (!get().notes.some((n) => n.id === noteId)) {
        toast.message("The note was deleted while it was being processed.");
        return;
      }
      set({ snapshot: { id: noteId, content } });
      get().patchNote(noteId, { content: out });
      onSuggest?.(content, out);
      logActivity({
        status: "done",
        title: "Note processed",
        target: { section: "notes", noteId },
      });
      toast.success("Note processed.", {
        action: { label: "Undo", onClick: () => get().undo() },
        duration: 8000,
      });
    } catch (e) {
      toast.error(String(e));
    } finally {
      set({ processing: null });
    }
  },

  undo: () => {
    const snap = get().snapshot;
    if (!snap) return;
    get().patchNote(snap.id, { content: snap.content });
    set({ snapshot: null });
    toast.message("Note restored.");
  },
}));

// ---- persistence ----------------------------------------------------------
// Debounced, and flushed when the window closes. The store lives as long as the
// app, so leaving the Notes page can no longer drop the last half-second.
let lastWritten = JSON.stringify(initial);
let timer: number | null = null;
const write = (payload: string) => {
  try {
    localStorage.setItem(NOTES_STORE_KEY, payload);
    lastWritten = payload;
  } catch {
    /* storage full or unavailable: keep working in memory */
  }
};
useNotes.subscribe((s, prev) => {
  if (s.notes === prev.notes) return;
  if (timer !== null) window.clearTimeout(timer);
  const payload = JSON.stringify(s.notes);
  timer = window.setTimeout(() => {
    timer = null;
    write(payload);
  }, 500);
});
window.addEventListener("beforeunload", () => {
  const payload = JSON.stringify(useNotes.getState().notes);
  if (payload !== lastWritten) write(payload);
});
