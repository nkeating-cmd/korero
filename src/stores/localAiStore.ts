/**
 * Kōrero 1.49 (B10, D-014): Kōrero's own local AI and "Check local AI", as
 * state that lives outside any page.
 *
 * - `addRuntime()`: download the runtime this computer needs, then point
 *   clean-up at it. One job, like the 1.43 model download: leaving the page
 *   does not stop it, and a second click does not start a second download.
 * - `check(full)`: run the doctor and keep the last result, so Today, In use
 *   and the settings page show the same answer.
 */

import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { commands, type DoctorReport, type RuntimeProgress, type RuntimeStatus } from "@/bindings";
import { useSettingsStore } from "./settingsStore";

export type RuntimeStep = "idle" | "downloading" | "starting" | "done" | "error" | "cancelled";

const CANCELLED = "Download cancelled.";
const ALREADY = "The local AI is already downloading.";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

interface LocalAiState {
  runtime: RuntimeStatus | null;
  step: RuntimeStep;
  doneBytes: number;
  totalBytes: number;
  error: string | null;
  report: DoctorReport | null;
  checking: boolean;
  checkError: string | null;

  refreshRuntime: () => Promise<RuntimeStatus | null>;
  addRuntime: () => Promise<boolean>;
  cancelRuntime: () => Promise<void>;
  resetRuntime: () => void;
  /** Check every file again and replace damaged ones. */
  repairRuntime: () => Promise<boolean>;
  /** Go back to what clean-up used before; keep the files. */
  stopUsingRuntime: () => Promise<boolean>;
  /** Delete the runtime and its models. */
  removeRuntime: () => Promise<boolean>;
  /** After a page reload mid-download: show it, and wait for it to end. */
  followInstall: () => Promise<void>;
  check: (full: boolean) => Promise<DoctorReport | null>;
}

export const useLocalAi = create<LocalAiState>((set, get) => ({
  runtime: null,
  step: "idle",
  doneBytes: 0,
  totalBytes: 0,
  error: null,
  report: null,
  checking: false,
  checkError: null,

  refreshRuntime: async () => {
    const r = await commands.localAiRuntimeStatus();
    if (r.status === "error") return null;
    set({ runtime: r.data });
    return r.data;
  },

  addRuntime: async () => {
    const busy: RuntimeStep[] = ["downloading", "starting"];
    if (busy.includes(get().step)) return false;
    set({ step: "downloading", doneBytes: 0, totalBytes: 0, error: null });
    const unlisten = await listen<RuntimeProgress>("local-ai-runtime-progress", (e) =>
      set({ doneBytes: e.payload.done_bytes, totalBytes: e.payload.total_bytes }),
    );
    try {
      const got = await commands.localAiRuntimeInstall();
      if (got.status === "error") {
        if (got.error === ALREADY) {
          // Started before this page loaded: follow it, not an error.
          await get().followInstall();
          return false;
        }
        set(got.error === CANCELLED ? { step: "cancelled" } : { step: "error", error: got.error });
        return false;
      }
      set({ step: "starting" });
      const used = await commands.localAiRuntimeUse();
      if (used.status === "error") {
        set({ step: "error", error: used.error });
        return false;
      }
      await useSettingsStore.getState().refreshSettings();
      set({ step: "done" });
      return true;
    } catch (e) {
      set({ step: "error", error: String(e instanceof Error ? e.message : e) });
      return false;
    } finally {
      unlisten();
      void get().refreshRuntime();
    }
  },

  cancelRuntime: async () => {
    await commands.localAiRuntimeCancel();
  },

  resetRuntime: () => set({ step: "idle", error: null, doneBytes: 0, totalBytes: 0 }),

  repairRuntime: async () => {
    if (["downloading", "starting"].includes(get().step)) return false;
    set({ step: "downloading", doneBytes: 0, totalBytes: 0, error: null });
    const unlisten = await listen<RuntimeProgress>("local-ai-runtime-progress", (e) =>
      set({ doneBytes: e.payload.done_bytes, totalBytes: e.payload.total_bytes }),
    );
    try {
      const r = await commands.localAiRuntimeRepair();
      if (r.status === "error") {
        set(r.error === CANCELLED ? { step: "cancelled" } : { step: "error", error: r.error });
        return false;
      }
      set({ step: "done" });
      return true;
    } finally {
      unlisten();
      void get().refreshRuntime();
    }
  },

  stopUsingRuntime: async () => {
    const r = await commands.localAiRuntimeStopUsing();
    await useSettingsStore.getState().refreshSettings();
    await get().refreshRuntime();
    if (r.status === "error") {
      set({ error: r.error });
      return false;
    }
    return true;
  },

  removeRuntime: async () => {
    const r = await commands.localAiRuntimeRemove();
    await useSettingsStore.getState().refreshSettings();
    await get().refreshRuntime();
    if (r.status === "error") {
      set({ error: r.error });
      return false;
    }
    set({ step: "idle", error: null });
    return true;
  },

  followInstall: async () => {
    if (["downloading", "starting"].includes(get().step) && get().totalBytes > 0) return;
    set({ step: "downloading", error: null });
    const unlisten = await listen<RuntimeProgress>("local-ai-runtime-progress", (e) =>
      set({ doneBytes: e.payload.done_bytes, totalBytes: e.payload.total_bytes }),
    );
    try {
      for (;;) {
        const s = await get().refreshRuntime();
        if (!s?.installing) break;
        await wait(2000);
      }
    } finally {
      unlisten();
      const s = get().runtime;
      set({ step: s && s.installed.includes("core") ? "done" : "idle" });
    }
  },

  check: async (full) => {
    if (get().checking) return get().report;
    set({ checking: true, checkError: null });
    try {
      const r = await commands.localAiCheck(full);
      if (r.status === "error") {
        set({ checkError: r.error });
        return null;
      }
      set({ report: r.data });
      return r.data;
    } catch (e) {
      set({ checkError: String(e instanceof Error ? e.message : e) });
      return null;
    } finally {
      set({ checking: false });
    }
  },
}));
