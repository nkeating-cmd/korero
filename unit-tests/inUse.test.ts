// Kōrero 1.43: the rules behind the "In use" panel. Run with `bun test unit-tests`.

import { describe, expect, test } from "bun:test";
import {
  cleanupDestination,
  cleanupState,
  isLocalInUse,
  isLoopbackUrl,
  lockedReason,
  micLabel,
  speechState,
} from "../src/components/shell/inUseModel";

const devices = [
  { name: "Default", is_default: true }, // synthetic entry from the settings store
  { name: "Headset microphone (USB)", is_default: true },
  { name: "Laptop microphone array", is_default: false },
];
const ollama = { id: "ollama", label: "Ollama", base_url: "http://localhost:11434/v1", is_local_provider: true };

describe("micLabel", () => {
  test("system default names the real device, not the synthetic entry", () => {
    expect(micLabel("Default", devices)).toEqual({ name: "Headset microphone (USB)", usingDefault: true, missing: false });
    expect(micLabel("default", devices).name).toBe("Headset microphone (USB)");
    expect(micLabel(null, devices).usingDefault).toBe(true);
  });
  test("falls back to 'System default' when the real default is unknown", () => {
    expect(micLabel("Default", [{ name: "Default", is_default: true }]).name).toBe("System default");
  });
  test("a chosen device that has gone is flagged, once the list has loaded", () => {
    expect(micLabel("Webcam microphone", devices).missing).toBe(true);
    expect(micLabel("Webcam microphone", [{ name: "Default", is_default: true }]).missing).toBe(false);
    expect(micLabel("Laptop microphone array", devices).missing).toBe(false);
  });
});

describe("speechState", () => {
  test("ready", () => expect(speechState({ modelName: "Parakeet V3", load: "idle" })).toEqual({ value: "Parakeet V3", detail: undefined, level: "ok" }));
  test("loading", () => expect(speechState({ modelName: "Parakeet V3", load: "loading" }).detail).toBe("Loading…"));
  test("load error warns", () => expect(speechState({ modelName: "Parakeet V3", load: "error" }).level).toBe("warn"));
  test("downloading shows the percentage", () =>
    expect(speechState({ modelName: "Parakeet V3", load: "idle", downloading: { name: "Whisper Turbo", percent: 44.6 } }).detail).toBe(
      "Downloading Whisper Turbo · 45%",
    ));
  test("verifying and extracting", () => {
    expect(speechState({ modelName: "A", load: "idle", verifying: "B" }).detail).toBe("Checking B…");
    expect(speechState({ modelName: "A", load: "idle", extracting: "B" }).detail).toBe("Unpacking B…");
  });
  test("no model warns", () => expect(speechState({ modelName: null, load: "idle" }).level).toBe("warn"));
});

describe("destination (SEC-143-01)", () => {
  test("loopback Ollama is on this computer", () => expect(cleanupDestination(ollama)).toEqual({ local: true, text: "on this computer" }));
  test("a 'local' provider on a remote host is not local", () => {
    const d = cleanupDestination({ ...ollama, base_url: "http://192.168.1.20:11434/v1" });
    expect(d.local).toBe(false);
    expect(d.text).toBe("sends to 192.168.1.20:11434");
  });
  test("a cloud provider is named", () =>
    expect(cleanupDestination({ id: "deepseek", label: "DeepSeek", base_url: "https://api.deepseek.com/v1" })).toEqual({
      local: false,
      text: "sends to DeepSeek",
    }));
  test("Apple Intelligence is local", () =>
    expect(cleanupDestination({ id: "apple_intelligence", label: "Apple", base_url: "" }).local).toBe(true));
  test("loopback parsing", () => {
    expect(isLoopbackUrl("http://127.0.0.1:11434/v1")).toBe(true);
    expect(isLoopbackUrl("http://[::1]:11434")).toBe(true);
    expect(isLoopbackUrl("http://localhost.evil.example/v1")).toBe(false);
    expect(isLoopbackUrl("https://api.openai.com/v1")).toBe(false);
  });
});

describe("cleanupState", () => {
  const base = { enabled: true, provider: ollama, model: "gemma4:12b", promptName: "Clean transcript", routingOn: false, ollamaUp: true };
  test("on, local", () => {
    const s = cleanupState(base);
    expect(s).toMatchObject({ label: "Clean-up · on this computer", value: "gemma4:12b", detail: "Clean transcript", level: "ok", local: true });
  });
  test("off still says what notes and Ask use", () => {
    const s = cleanupState({ ...base, enabled: false });
    expect(s.value).toBe("Off");
    expect(s.level).toBe("off");
    expect(s.detail).toContain("gemma4:12b");
  });
  test("no model warns", () => expect(cleanupState({ ...base, model: "" }).level).toBe("warn"));
  test("Ollama down warns", () => expect(cleanupState({ ...base, ollamaUp: false })).toMatchObject({ level: "warn", detail: "Ollama isn't running" }));
  test("routing on", () => expect(cleanupState({ ...base, routingOn: true }).detail).toBe("Clean transcript · varies by app"));
  test("cloud destination is in the label", () =>
    expect(cleanupState({ ...base, provider: { id: "deepseek", label: "DeepSeek", base_url: "https://api.deepseek.com/v1" } }).label).toBe(
      "Clean-up · sends to DeepSeek",
    ));
});

describe("isLocalInUse (RT-F-07)", () => {
  const s = { post_process_enabled: true, post_process_provider_id: "ollama", post_process_models: { ollama: "qwen3.5:4b", openai: "gpt" } };
  test("true only for the active Ollama model", () => {
    expect(isLocalInUse(s, "qwen3.5:4b")).toBe(true);
    expect(isLocalInUse(s, "gemma4:12b")).toBe(false);
  });
  test("a stale Ollama tag under a cloud provider is not in use", () =>
    expect(isLocalInUse({ ...s, post_process_provider_id: "openai" }, "qwen3.5:4b")).toBe(false));
  test("off is not in use", () => expect(isLocalInUse({ ...s, post_process_enabled: false }, "qwen3.5:4b")).toBe(false));
});

describe("lock while recording (RT-F-08)", () => {
  test("meeting and dictation lock; idle does not", () => {
    expect(lockedReason(true, false)).toContain("meeting");
    expect(lockedReason(false, true)).toContain("dictating");
    expect(lockedReason(false, false)).toBeNull();
  });
});
