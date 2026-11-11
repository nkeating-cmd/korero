// Kōrero 1.46: "write the notes when a meeting ends" (on unless switched off).
//
// A small per-computer choice, kept beside the "On a call / In the room" one in
// localStorage. Reads never throw: a browser context without storage means "on".

const KEY = "korero.meetings.autoNotes";

export const autoNotesEnabled = (): boolean => {
  try {
    return localStorage.getItem(KEY) !== "0";
  } catch {
    return true;
  }
};

export const setAutoNotes = (on: boolean): void => {
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    /* the choice just doesn't persist */
  }
};

/** Whether an AI model is set up for notes: a provider is chosen and it has a model. */
export const notesModelReady = (
  s: { post_process_provider_id?: string | null; post_process_models?: Partial<Record<string, string>> | null } | null | undefined,
): boolean => {
  const id = s?.post_process_provider_id ?? "";
  return !!id && !!(s?.post_process_models ?? {})[id]?.trim();
};

/**
 * What a call does by itself after Stop. Pure, so the rules are testable:
 *   - rebuild the transcript when at least one side's recording is a WAV;
 *   - write notes when that is switched on and an AI model is set up;
 *   - if the rebuild cannot run or does not land, notes come from the live text.
 */
export const afterStopPlan = (a: {
  micWav: boolean;
  systemWav: boolean;
  hasLiveText: boolean;
  autoNotes: boolean;
  modelReady: boolean;
}): { rebuild: boolean; notes: boolean; notesFromLive: boolean } => {
  const rebuild = a.micWav || a.systemWav;
  const notes = a.autoNotes && a.modelReady && (rebuild || a.hasLiveText);
  return { rebuild, notes, notesFromLive: notes && a.hasLiveText };
};
