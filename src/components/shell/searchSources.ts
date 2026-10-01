// Kōrero 1.42: what Ctrl K searches. Loaded fresh each time the palette opens
// (cheap: one read of meetings.json, the notes in memory, the latest 400
// dictations), so results are never stale for more than a keystroke.

import { commands } from "@/bindings";
import type { SearchDoc } from "../../lib/search";
import { meetingsBridge } from "../../stores/meetingJobsStore";
import { noteTitle, useNotes } from "../../stores/notesStore";
import type { Section } from "../../stores/navStore";
import {
  durationOf,
  fmtDuration,
  normaliseMeetings,
  titleOf,
  visibleSegs,
  combine,
  type Meeting,
} from "../meetings/model";

export interface SettingEntry {
  id: string;
  title: string;
  section: Section;
  keywords: string;
  meta: string;
}

/** Settings people actually look for, with the words they use for them. */
export const SETTINGS_CATALOGUE: SettingEntry[] = [
  { id: "nz", title: "New Zealand English", section: "general", meta: "Dictation & sound", keywords: "nz english macrons te reo maori spelling language en-nz place names" },
  { id: "shortcut", title: "Dictation shortcuts", section: "general", meta: "Dictation & sound", keywords: "keyboard shortcut hotkey ctrl space binding key" },
  { id: "ptt", title: "Push to talk", section: "general", meta: "Dictation & sound", keywords: "hold key latch hands-free" },
  { id: "mic", title: "Microphone", section: "general", meta: "Dictation & sound", keywords: "input device mic headset audio" },
  { id: "noise", title: "Noise suppression", section: "general", meta: "Dictation & sound", keywords: "rnnoise denoise background noise" },
  { id: "feedback", title: "Sound feedback", section: "general", meta: "Dictation & sound", keywords: "beep sounds volume output device" },
  { id: "models", title: "Speech models", section: "models", meta: "Set up", keywords: "parakeet whisper download model accuracy speed gpu" },
  { id: "ai", title: "AI clean-up provider", section: "postprocessing", meta: "AI clean-up & notes", keywords: "post-processing llm ollama deepseek openai anthropic claude gemini api key provider" },
  { id: "prompts", title: "Prompts", section: "postprocessing", meta: "AI clean-up & notes", keywords: "post-processing prompt template meeting notes email" },
  { id: "corrections", title: "Taught corrections", section: "postprocessing", meta: "AI clean-up & notes", keywords: "corrections teach wrong right glossary" },
  { id: "words", title: "Custom words", section: "advanced", meta: "Advanced", keywords: "vocabulary dictionary names custom words" },
  { id: "paste", title: "Paste method", section: "advanced", meta: "Advanced", keywords: "paste clipboard typing output" },
  { id: "overlay", title: "Recording pill position", section: "advanced", meta: "Advanced", keywords: "overlay pill position show hide" },
  { id: "autostart", title: "Start with Windows", section: "advanced", meta: "Advanced", keywords: "autostart launch login start hidden tray" },
  { id: "history", title: "Dictation history", section: "history", meta: "Library", keywords: "history past dictations retry" },
  { id: "briefs", title: "Audio briefs", section: "audiobrief", meta: "Capture", keywords: "tts speech spoken summary audio brief" },
  { id: "storage", title: "Meeting recordings folder", section: "meetings", meta: "Meetings", keywords: "storage folder recordings wav disk location" },
  { id: "about", title: "Help & about", section: "help", meta: "Version, logs, guide", keywords: "help about version update logs support guide" },
];

export interface LoadedSources {
  docs: SearchDoc[];
  meetings: Meeting[];
}

const meetingMeta = (m: Meeting): string => {
  const parts = [new Date(m.createdAt).toLocaleDateString(undefined, { day: "numeric", month: "short" })];
  const d = durationOf(m);
  if (d) parts.push(fmtDuration(d));
  if (m.processed.trim()) parts.push("notes");
  return parts.join(" · ");
};

export const loadSearchSources = async (): Promise<LoadedSources> => {
  const docs: SearchDoc[] = [];
  let meetings: Meeting[] = [];

  try {
    const res = await meetingsBridge.load();
    if (res.ok && res.data.trim()) meetings = normaliseMeetings(JSON.parse(res.data));
  } catch {
    /* meetings unreadable: search the rest */
  }
  for (const m of meetings) {
    docs.push({
      id: `meeting:${m.id}`,
      kind: "meeting",
      title: titleOf(m),
      // Search honours the trim, as the Meetings search always has.
      body: `${combine(m.you, m.others, m.youLabel, m.othersLabel, visibleSegs(m))}\n${m.processed}`,
      meta: meetingMeta(m),
      at: m.createdAt,
    });
  }

  for (const n of useNotes.getState().notes) {
    if (!n.content.trim() && !n.title.trim()) continue;
    docs.push({
      id: `note:${n.id}`,
      kind: "note",
      title: noteTitle(n),
      body: n.content,
      meta: `${n.content.trim() ? n.content.trim().split(/\s+/).length : 0} words`,
      at: n.updatedAt,
    });
  }

  try {
    const res = await commands.getHistoryEntries(null, 400);
    if (res.status === "ok") {
      for (const e of res.data.entries) {
        const text = (e.post_processed_text || e.transcription_text || "").trim();
        if (!text) continue;
        const ms = e.timestamp < 1e12 ? e.timestamp * 1000 : e.timestamp;
        docs.push({
          id: `dictation:${e.id}`,
          kind: "dictation",
          title: text.length > 90 ? `${text.slice(0, 90)}…` : text,
          body: text,
          meta: new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short" }),
          at: ms,
        });
      }
    }
  } catch {
    /* history unavailable: search the rest */
  }

  for (const s of SETTINGS_CATALOGUE) {
    docs.push({
      id: `setting:${s.id}`,
      kind: "setting",
      title: s.title,
      keywords: s.keywords,
      meta: s.meta,
    });
  }

  return { docs, meetings };
};
