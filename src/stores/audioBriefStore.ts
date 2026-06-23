// Kōrero (v1.22.0): Audio Brief state + actions in a Zustand store.
//
// WHY a store and not component state: the settings panel mounts only the
// ACTIVE section's component (App.tsx renders SECTIONS_CONFIG[section].component),
// so navigating away from Audio Brief UNMOUNTS it. With component-local state a
// draft/render in flight would lose its UI hookup and the typed text, drafted
// script and rendered audio would all reset on return.
//
// Holding everything here fixes that: the store is a module-level singleton that
// outlives the view, and draftTranscript()/renderAudio() are store methods (not
// bound to the React lifecycle), so they keep running in the BACKGROUND and write
// their results (the script, and the on-disk MP3 path) into the store even while
// you're on another tab. Returning to the tab re-binds to the same state.
//
// Scope: SESSION memory (survives tab switches, cleared on app restart). The MP3
// itself is written to disk by the backend regardless, so "audio created" is
// always saved on disk and reachable via Show in folder.

import { create } from "zustand";
import { toast } from "sonner";
import { commands } from "@/bindings";

// Shared rules every format obeys (audio is linear; write for the ear).
const BASE_RULES =
  " Write numbers as words (e.g. three hundred and eighty-three dollars); expand abbreviations on first use; remove URLs, reference codes, IDs and emoji; short one-idea sentences; NZ English; no titles, headings, or speaker labels. Return ONLY the script text.";

// Template formats for the spoken script — each is a model instruction.
export const TEMPLATES: { id: string; label: string; prompt: string }[] = [
  {
    id: "standard",
    label: "Standard brief",
    prompt:
      "Rewrite the text below into a short SPOKEN audio brief (150-300 words): lead with the single most important point, then supporting detail, then a clean one-line close." +
      BASE_RULES,
  },
  {
    id: "executive",
    label: "Executive summary",
    prompt:
      "Rewrite the text below into a 120-200 word SPOKEN executive summary: the decision or headline first, then the two or three things that matter and any risk, then what happens next." +
      BASE_RULES,
  },
  {
    id: "actions",
    label: "Action items",
    prompt:
      "From the text below, produce a SPOKEN run-through of the action items: for each, say who does what by when. Open with a one-line context sentence. Under 200 words." +
      BASE_RULES,
  },
  {
    id: "insights",
    label: "Key insights",
    prompt:
      "From the text below, give a SPOKEN rundown of the three or four key insights, each a sentence or two, then one 'so what' line on why it matters. Under 220 words." +
      BASE_RULES,
  },
  {
    id: "narrative",
    label: "Narrative",
    prompt:
      "Retell the text below as a short SPOKEN narrative that flows naturally when heard, about 200-300 words, with a clear beginning, middle and end." +
      BASE_RULES,
  },
];

// Preset TTS speakers (plus the engine's default designed voice).
export const SPEAKERS = [
  "Default",
  "Serena",
  "Aiden",
  "Ryan",
  "Vivian",
  "Sohee",
  "Ono Anna",
  "Dylan",
  "Eric",
  "Uncle Fu",
];

// Pace presets → engine --tempo value.
export const TEMPOS: { label: string; value: number }[] = [
  { label: "Slower", value: 0.95 },
  { label: "Normal", value: 1.0 },
  { label: "Brisk", value: 1.12 },
  { label: "Fast", value: 1.25 },
];

// Delivery styles → the engine's --style (instruct). "" = the voice's natural
// delivery. Applies to both the designed default voice and the named speakers.
export const STYLES: { label: string; value: string }[] = [
  { label: "Natural", value: "" },
  { label: "Warm & friendly", value: "warm and friendly" },
  { label: "Professional", value: "professional and clear" },
  { label: "Energetic", value: "energetic and upbeat" },
  { label: "Calm", value: "calm and measured" },
  { label: "Authoritative", value: "authoritative and confident" },
  { label: "Conversational", value: "relaxed and conversational" },
];

interface AudioBriefState {
  // Durable working state.
  source: string;
  transcript: string;
  audioPath: string | null; // the rendered MP3 path on disk (canonical)
  engineMissing: boolean;
  templateId: string;
  speaker: string;
  tempo: number;
  style: string;
  // Transient progress flags (NOT persisted — they belong to the live run).
  drafting: boolean;
  rendering: boolean;
  // Setters.
  setSource: (v: string) => void;
  setTranscript: (v: string) => void;
  setTemplateId: (v: string) => void;
  setSpeaker: (v: string) => void;
  setTempo: (v: number) => void;
  setStyle: (v: string) => void;
  // Background-safe actions.
  draftTranscript: () => Promise<void>;
  renderAudio: () => Promise<void>;
}

export const useAudioBriefStore = create<AudioBriefState>((set, get) => ({
  source: "",
  transcript: "",
  audioPath: null,
  engineMissing: false,
  templateId: TEMPLATES[0].id,
  speaker: SPEAKERS[0], // "Default"
  tempo: 1.12, // "Brisk"
  style: STYLES[0].value, // "" = natural delivery
  drafting: false,
  rendering: false,

  setSource: (v) => set({ source: v }),
  setTranscript: (v) => set({ transcript: v }),
  setTemplateId: (v) => set({ templateId: v }),
  setSpeaker: (v) => set({ speaker: v }),
  setTempo: (v) => set({ tempo: v }),
  setStyle: (v) => set({ style: v }),

  // Draft a spoken script from the source text via the local post-processing
  // model. Reads/writes the store (not React state), so it completes and the
  // script is captured even if the user has navigated away from the tab.
  draftTranscript: async () => {
    const { source, drafting, templateId } = get();
    const text = source.trim();
    if (!text) {
      toast.message("Paste or import some text first.");
      return;
    }
    if (drafting) return;
    set({ drafting: true });
    try {
      const tpl = TEMPLATES.find((t) => t.id === templateId) ?? TEMPLATES[0];
      const r = await commands.meetingPostProcess(text, tpl.prompt);
      if (r.status !== "ok") throw new Error(r.error);
      // The script changed, so any previously rendered audio is now stale.
      set({ transcript: r.data.trim(), audioPath: null });
      toast.success("Draft ready — edit it, then render audio.");
    } catch (e) {
      toast.error(`Draft failed: ${String(e)}`);
    } finally {
      set({ drafting: false });
    }
  },

  // Render the current script to an MP3 via the local TTS engine. The backend
  // writes the MP3 to disk and runs to completion regardless of the UI, so the
  // render genuinely continues in the background; on success the on-disk path is
  // stored (and the MP3 stays saved on disk for Show in folder / Download).
  renderAudio: async () => {
    const { transcript, rendering, speaker, style, tempo } = get();
    const text = transcript.trim();
    if (!text) {
      toast.message("Draft or write a script first.");
      return;
    }
    if (rendering) return;
    set({ rendering: true, audioPath: null });
    try {
      const r = await commands.meetingGenerateAudioBrief(
        text,
        speaker === "Default" ? null : speaker,
        style || null,
        tempo,
      );
      if (r.status !== "ok") throw new Error(r.error);
      set({ engineMissing: false, audioPath: r.data });
      toast.success("Audio brief ready.");
    } catch (e) {
      const msg = String(e);
      // The backend says "...voice engine not found..." when no local TTS engine
      // is installed — surface a helpful setup card instead of a bare toast.
      if (/not found|no such|couldn'?t (find|locate)|engine/i.test(msg)) {
        set({ engineMissing: true });
      }
      toast.error(`Audio render failed: ${msg}`);
    } finally {
      set({ rendering: false });
    }
  },
}));
