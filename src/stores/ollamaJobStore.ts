/**
 * Kōrero 1.43: "Download and use" for a local AI model, as one job that lives
 * outside any component.
 *
 * WHY A STORE. The 1.43 red team (RT-F-04) found that a chain held inside the
 * card dies with it: leave the page mid-download and the card comes back idle,
 * offering a second download, while the first one later switches clean-up to
 * its model over whatever you chose meanwhile. Here there is one job, one
 * progress listener, and the final switch is skipped if your settings changed
 * since the download started.
 *
 * ORDER. Ollama ready → download → test run → switch. Nothing is switched
 * unless the test run worked (RT-F-01: a failed download used to read as
 * success, because command errors come back as values, not exceptions).
 */

import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import { useSettingsStore } from "./settingsStore";

// ---------------------------------------------------------------- progress

export interface PullEvent {
  model?: string;
  status: string;
  digest?: string;
  total?: number;
  completed?: number;
}

export interface Progress {
  status: string;
  layers: Record<string, { total: number; completed: number }>;
  doneBytes: number;
  totalBytes: number;
  /** Bytes already on disk when a layer was first seen (a resumed download). */
  resumedBytes: number;
  rateBps: number | null;
  sampleAt: number;
  sampleBytes: number;
}

export const emptyProgress = (now = 0): Progress => ({
  status: "",
  layers: {},
  doneBytes: 0,
  totalBytes: 0,
  resumedBytes: 0,
  rateBps: null,
  sampleAt: now,
  sampleBytes: 0,
});

/**
 * Ollama reports progress per layer, one at a time. Overwriting a single
 * total made the bar jump 100 → 0 between layers (RT-F-10). Sum the layers,
 * never go backwards, and measure the rate from new bytes only.
 */
export function reduceProgress(p: Progress, ev: PullEvent, now: number): Progress {
  const layers = { ...p.layers };
  let resumedBytes = p.resumedBytes;
  if (ev.digest && ev.total && ev.total > 0) {
    const prev = layers[ev.digest];
    const completed = Math.min(ev.total, Math.max(prev?.completed ?? 0, ev.completed ?? 0));
    if (!prev && completed > 0) resumedBytes += completed;
    layers[ev.digest] = { total: ev.total, completed };
  }
  const sum = (k: "total" | "completed") => Object.values(layers).reduce((a, l) => a + l[k], 0);
  const totalBytes = Math.max(p.totalBytes, sum("total"));
  const doneBytes = Math.max(p.doneBytes, sum("completed"));
  let { rateBps, sampleAt, sampleBytes } = p;
  const fresh = doneBytes - resumedBytes;
  if (now - sampleAt >= 1000) {
    const inst = ((fresh - sampleBytes) * 1000) / (now - sampleAt);
    rateBps = rateBps === null ? inst : rateBps * 0.7 + inst * 0.3;
    sampleAt = now;
    sampleBytes = fresh;
  }
  return { status: ev.status || p.status, layers, doneBytes, totalBytes, resumedBytes, rateBps, sampleAt, sampleBytes };
}

export function etaSeconds(p: Progress): number | null {
  if (!p.rateBps || p.rateBps <= 0 || p.totalBytes <= 0) return null;
  return Math.max(0, Math.round((p.totalBytes - p.doneBytes) / p.rateBps));
}

// ---------------------------------------------------------------- chain

export type Step = "idle" | "starting" | "pulling" | "testing" | "switching" | "done" | "error" | "cancelled";

export interface TestResult {
  seconds: number;
  sample: string;
  gpu_share: number | null;
}

/** Everything the chain touches, injectable for tests. */
export interface ChainDeps {
  status: () => Promise<{ installed: boolean; running: boolean }>;
  start: () => Promise<boolean>;
  pull: (model: string) => Promise<void>;
  test: (model: string) => Promise<TestResult>;
  use: (model: string) => Promise<void>;
  /** What clean-up is set to right now (provider + model + on/off). */
  snapshot: () => string;
  onStep: (s: Step, extra?: { error?: string; result?: TestResult; note?: string }) => void;
}

export const CANCELLED = "Download cancelled.";

export async function runChain(model: string, d: ChainDeps): Promise<void> {
  try {
    const st = await d.status();
    if (!st.installed) {
      d.onStep("error", { error: "Ollama isn't installed on this computer yet." });
      return;
    }
    if (!st.running) {
      d.onStep("starting");
      if (!(await d.start())) {
        d.onStep("error", { error: "Ollama didn't start. Open it from the Start menu, then try again." });
        return;
      }
    }
    const before = d.snapshot();
    d.onStep("pulling");
    await d.pull(model);
    d.onStep("testing");
    const result = await d.test(model);
    if (d.snapshot() !== before) {
      d.onStep("done", {
        result,
        note: "Your clean-up settings changed during the download, so Kōrero didn't switch to it.",
      });
      return;
    }
    d.onStep("switching");
    await d.use(model);
    d.onStep("done", { result });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    d.onStep(msg === CANCELLED ? "cancelled" : "error", { error: msg });
  }
}

/** Command results come back as values; turn an error value into a throw. */
export function ok<T>(r: { status: "ok"; data: T } | { status: "error"; error: string }): T {
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

// ---------------------------------------------------------------- store

interface OllamaJob {
  step: Step;
  model: string | null;
  progress: Progress;
  error: string | null;
  note: string | null;
  result: TestResult | null;
  /** Start "Download and use" for `model`. Refused while a job runs. */
  start: (model: string) => boolean;
  cancel: () => void;
  reset: () => void;
}

const BUSY: Step[] = ["starting", "pulling", "testing", "switching"];
let listening = false;

const cleanupSnapshot = () => {
  const s = useSettingsStore.getState().settings;
  return JSON.stringify([s?.post_process_enabled, s?.post_process_provider_id, s?.post_process_models?.["ollama"]]);
};

export const useOllamaJob = create<OllamaJob>((set, get) => ({
  step: "idle",
  model: null,
  progress: emptyProgress(),
  error: null,
  note: null,
  result: null,

  start: (model) => {
    if (BUSY.includes(get().step)) return false;
    if (!listening) {
      listening = true;
      void listen<PullEvent>("ollama-pull-progress", (e) => {
        const s = get();
        if (s.step !== "pulling" || (e.payload.model && e.payload.model !== s.model)) return;
        set({ progress: reduceProgress(s.progress, e.payload, Date.now()) });
      });
    }
    set({ step: "starting", model, progress: emptyProgress(Date.now()), error: null, note: null, result: null });
    const base = () =>
      useSettingsStore.getState().settings?.post_process_providers?.find((p) => p.id === "ollama")?.base_url ??
      "http://localhost:11434/v1";
    void runChain(model, {
      status: () => commands.ollamaStatus(base()),
      start: async () => ok(await commands.ollamaStart(base())),
      pull: async (m) => {
        ok(await commands.pullOllamaModel(m));
      },
      test: async (m) => ok(await commands.ollamaTestModel(m)),
      use: async (m) => {
        ok(await commands.useLocalOllamaModel(m));
        await useSettingsStore.getState().refreshSettings();
      },
      snapshot: cleanupSnapshot,
      onStep: (step, extra) =>
        set({ step, error: extra?.error ?? null, note: extra?.note ?? null, result: extra?.result ?? get().result }),
    });
    return true;
  },

  cancel: () => {
    void commands.cancelOllamaPull();
  },

  reset: () => {
    if (!BUSY.includes(get().step)) set({ step: "idle", model: null, error: null, note: null, result: null });
  },
}));
