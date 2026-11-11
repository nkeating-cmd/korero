// Kōrero 1.46: "write the notes when a call ends".
//
// A per-computer choice, kept beside the "On a call / In the room" one in
// localStorage. Three states, because the transcript goes to whatever AI model is
// set up for notes, and that may be a cloud service:
//
//   - "on" / "off": the person chose.
//   - "unset": nobody has chosen. Notes are then written only when the model runs
//     on this computer. A transcript is never sent to a cloud service on the
//     strength of a default.
//
// Reads never throw: a context without storage behaves as "unset".

const KEY = "korero.meetings.autoNotes";

export type AutoNotesChoice = "on" | "off" | "unset";

/** The stored choice. "on" is remembered together with the notes provider it was
 *  given for, and counts only while that provider is still the one in use: saying
 *  yes to a model on this computer is not saying yes to a cloud service chosen later. */
export const parseChoice = (stored: string | null, providerId: string): AutoNotesChoice => {
  if (stored === "0") return "off";
  if (stored !== null && stored.startsWith("1:")) return stored.slice(2) === providerId ? "on" : "unset";
  return "unset";
};

export const autoNotesChoice = (providerId: string): AutoNotesChoice => {
  try {
    return parseChoice(localStorage.getItem(KEY), providerId);
  } catch {
    return "unset";
  }
};

export const setAutoNotes = (on: boolean, providerId: string): void => {
  try {
    localStorage.setItem(KEY, on ? `1:${providerId}` : "0");
  } catch {
    /* the choice just doesn't persist */
  }
};

/** Whether notes are written without being asked, given the choice and where the model runs. */
export const autoNotesOn = (choice: AutoNotesChoice, providerLocal: boolean): boolean =>
  choice === "on" || (choice === "unset" && providerLocal);

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
 *   - write notes when `autoNotes` (see `autoNotesOn`) and an AI model is set up;
 *   - if the rebuild cannot run or fails, notes come from the live text.
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
