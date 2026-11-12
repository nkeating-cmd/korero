import type { Page } from "@playwright/test";

/**
 * Stands in for the Rust side so the real frontend can run in a plain browser.
 * Everything here is invented: no real meeting, name, setting or custom word
 * belongs in this file.
 */

export interface FixtureSeg {
  source: "you" | "others";
  text: string;
  start_ms: number;
  speaker: number | null;
}

const seg = (
  source: "you" | "others",
  text: string,
  start_ms: number,
  speaker: number | null = null,
): FixtureSeg => ({ source, text, start_ms, speaker });

const HOUR_MS = 3_600_000;

/** A two-sided call: you, a named caller and an unnamed one. Six lines, four turns. */
export const callMeeting = () => ({
  id: "fixture-call",
  title: "Budget call (fixture)",
  // The app stores each side's joined text beside the timed lines.
  you: "Kia ora koutou. Yes. Let's do that.",
  others: "Mōrena. Shall we start with the budget?",
  processed: "**Actions**\n- Mere: send the figures by Friday",
  processPrompt: "",
  createdAt: Date.now() - HOUR_MS,
  systemCaptured: true,
  micPath: "C:/fixtures/call-you.wav",
  systemPath: "C:/fixtures/call-others.wav",
  youLabel: "You",
  othersLabel: "Others",
  imported: false,
  mode: "call",
  callersFound: 2,
  callerNames: ["Mere", ""],
  durationSec: 120,
  transcript: [
    seg("you", "Kia ora koutou.", 0),
    seg("others", "Mōrena.", 2000, 0),
    seg("others", "Shall we start", 4000, 1),
    seg("others", "with the budget?", 6000, 1),
    seg("you", "Yes.", 9000),
    seg("you", "Let's do that.", 11000),
  ],
});

/** A call nobody has named: it still carries the dated placeholder. Recorded on
 *  5 November 2026, with notes already written. */
export const unnamedMeeting = () => ({
  ...callMeeting(),
  id: "fixture-unnamed",
  title: "Meeting 5 Nov 2026, 6:03 pm",
  createdAt: new Date(2026, 10, 5, 18, 3).getTime(),
});

/** One imported recording: there are no two sides to rebuild. */
export const importedMeeting = () => ({
  ...callMeeting(),
  id: "fixture-import",
  title: "Imported file (fixture)",
  processed: "",
  imported: true,
  systemCaptured: false,
  micPath: "C:/fixtures/imported.wav",
  systemPath: null,
  callersFound: 0,
  callerNames: [],
  transcript: [
    seg("you", "Tēnā koe.", 0),
    seg("you", "This came from a file.", 3000),
  ],
});

const SETTINGS = {
  bindings: {},
  post_process_providers: [
    {
      id: "ollama",
      label: "Ollama",
      base_url: "http://localhost:11434/v1",
      allow_base_url_edit: true,
      models_endpoint: null,
    },
  ],
  post_process_provider_id: "ollama",
  post_process_models: { ollama: "fixture-model" },
  post_process_api_keys: { ollama: "" },
  post_process_prompts: [],
  post_process_enabled: true,
  custom_words: [],
  selected_language: "en-NZ",
  selected_model: "fixture-speech-model",
  debug_mode: false,
  app_language: "en",
};

export interface Invocation {
  cmd: string;
  args: unknown;
}

/** Install before `page.goto`. Commands not listed here answer null. */
export async function installTauriMock(
  page: Page,
  meetings: unknown[],
): Promise<void> {
  await page.addInitScript(
    ({ settings, stored }) => {
      const w = window as unknown as Record<string, unknown>;
      const calls: { cmd: string; args: unknown }[] = [];
      let nextId = 0;
      const answers: Record<string, unknown> = {
        get_app_settings: settings,
        get_default_settings: settings,
        get_available_models: [],
        get_current_model: "",
        check_custom_sounds: { start: false, stop: false },
        has_any_models_available: true,
        get_windows_microphone_permission_status: {
          supported: false,
          overall_access: "allowed",
        },
        meetings_store_load: stored,
        meeting_provider_is_local: true,
        meeting_recording_status: null,
        "plugin:app|version": "9.8.7",
        // What a successful run hands back, so the screens can show the result.
        meeting_call_transcript: {
          segments: [
            {
              source: "you",
              text: "Kia ora koutou, rebuilt.",
              start_ms: 0,
              speaker: null,
            },
            {
              source: "others",
              text: "Mōrena, rebuilt.",
              start_ms: 2000,
              speaker: 0,
            },
          ],
          callers: 1,
          rebuilt: ["you", "others"],
          failed: 0,
          notes: [],
          audio_s: 120,
          took_s: 4,
        },
        meeting_transcribe_merge: [
          { source: "you", text: "Tēnā koe, transcribed again." },
        ],
      };
      w.__koreroCalls = calls;
      w.__TAURI_OS_PLUGIN_INTERNALS__ = {
        platform: "windows",
        os_type: "windows",
        arch: "x86_64",
        eol: "\r\n",
        family: "windows",
        version: "10.0",
        exe_extension: "exe",
      };
      w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
      w.__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main", windowLabel: "main" },
        },
        transformCallback: (cb: unknown) => {
          nextId += 1;
          w["_" + nextId] = cb;
          return nextId;
        },
        unregisterCallback: () => {},
        convertFileSrc: (path: string) => "asset://" + path,
        invoke: async (cmd: string, args: unknown) => {
          calls.push({ cmd, args });
          if (cmd === "meeting_post_process") {
            // The notes model: asked for a title it answers the way models do (quotes,
            // a full stop); asked for notes it writes a short set.
            const prompt = String(
              (args as { prompt?: string } | null)?.prompt ?? "",
            );
            return prompt.startsWith("Name this meeting")
              ? '"Budget forecast and hiring."'
              : "**Actions**\n- Mere: send the revised figures";
          }
          return cmd in answers ? answers[cmd] : null;
        },
      };
    },
    { settings: SETTINGS, stored: JSON.stringify(meetings) },
  );
}

/** The arguments of the first call to one backend command, or null if it was never called. */
export const argsOf = (page: Page, cmd: string): Promise<unknown> =>
  page.evaluate((name) => {
    const calls = (
      window as unknown as { __koreroCalls: { cmd: string; args: unknown }[] }
    ).__koreroCalls;
    return calls.find((c) => c.cmd === name)?.args ?? null;
  }, cmd);

/** The backend commands the page has called so far, in order. */
export const invoked = (page: Page): Promise<string[]> =>
  page.evaluate(() =>
    (
      window as unknown as { __koreroCalls: { cmd: string }[] }
    ).__koreroCalls.map((c) => c.cmd),
  );
