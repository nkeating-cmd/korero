/**
 * Kōrero 1.43: what the "In use" panel says, as pure functions so every rule
 * is unit-tested (unit-tests/inUse.test.ts). The component only renders.
 */

export type Level = "ok" | "warn" | "off";

export interface DeviceLike {
  name: string;
  is_default?: boolean;
}

const isSyntheticDefault = (name: string) => name.trim().toLowerCase() === "default";

/**
 * The microphone actually in use. The settings store reports "no choice" as
 * "Default" (capital D) and the device list carries a synthetic "Default"
 * entry flagged is_default, so neither can be shown as a device name.
 */
export function micLabel(
  selected: string | null | undefined,
  devices: DeviceLike[],
): { name: string; usingDefault: boolean; missing: boolean } {
  const real = devices.filter((d) => !isSyntheticDefault(d.name));
  const usingDefault = !selected || isSyntheticDefault(selected);
  if (usingDefault) {
    const def = real.find((d) => d.is_default);
    return { name: def?.name ?? "System default", usingDefault: true, missing: false };
  }
  // Only call a chosen mic missing once the real device list has loaded.
  const missing = real.length > 0 && !real.some((d) => d.name === selected);
  return { name: selected, usingDefault: false, missing };
}

export interface SpeechInput {
  modelName: string | null;
  load: "idle" | "loading" | "error";
  downloading?: { name: string; percent: number } | null;
  verifying?: string | null;
  extracting?: string | null;
}

export function speechState(s: SpeechInput): { value: string; detail?: string; level: Level } {
  const busy = s.downloading
    ? `Downloading ${s.downloading.name} · ${Math.max(0, Math.min(100, Math.round(s.downloading.percent)))}%`
    : s.verifying
      ? `Checking ${s.verifying}…`
      : s.extracting
        ? `Unpacking ${s.extracting}…`
        : undefined;
  if (!s.modelName) return { value: "No speech model", detail: busy, level: "warn" };
  if (s.load === "error") return { value: s.modelName, detail: "Couldn't load: open Speech models", level: "warn" };
  if (s.load === "loading") return { value: s.modelName, detail: "Loading…", level: "ok" };
  return { value: s.modelName, detail: busy, level: "ok" };
}

/** TS twin of `history::is_loopback_url` in Rust: "local" is a property of the URL. */
export function isLoopbackUrl(baseUrl: string): boolean {
  const s = baseUrl.trim();
  const rest = s.replace(/^https?:\/\//i, "");
  const authority = rest.split(/[/?#]/)[0] ?? "";
  // Everything before the last "@" is userinfo, not the host (1.43 refuter
  // DEF-01: "http://localhost:11434@evil.example" goes to evil.example).
  const hostPort = authority.slice(authority.lastIndexOf("@") + 1);
  let host: string;
  if (hostPort.startsWith("[")) host = hostPort.slice(1).split("]")[0] ?? "";
  else {
    const i = hostPort.lastIndexOf(":");
    host = i >= 0 ? hostPort.slice(0, i) : hostPort;
  }
  host = host.toLowerCase();
  return host === "localhost" || host === "127.0.0.1" || host === "::1";
}

export interface ProviderLike {
  id: string;
  label: string;
  base_url: string;
  is_local_provider?: boolean;
}

export const APPLE_INTELLIGENCE_ID = "apple_intelligence";

/** Where clean-up, notes and Ask send text. */
export function cleanupDestination(p: ProviderLike | undefined): { local: boolean; text: string } {
  if (!p) return { local: false, text: "no provider chosen" };
  if (p.id === APPLE_INTELLIGENCE_ID) return { local: true, text: "on this computer" };
  if (p.is_local_provider || p.id === "custom") {
    if (isLoopbackUrl(p.base_url)) return { local: true, text: "on this computer" };
    const host = p.base_url.replace(/^https?:\/\//i, "").split("/")[0];
    return { local: false, text: `sends to ${host}` };
  }
  return { local: false, text: `sends to ${p.label}` };
}

export interface CleanupInput {
  enabled: boolean;
  provider: ProviderLike | undefined;
  model: string;
  promptName: string | null;
  routingOn: boolean;
  ollamaUp: boolean | null;
}

export function cleanupState(c: CleanupInput): {
  label: string;
  value: string;
  detail: string;
  level: Level;
  local: boolean;
} {
  const dest = cleanupDestination(c.provider);
  const label = `Clean-up · ${dest.text}`;
  if (!c.enabled) {
    return {
      label,
      value: "Off",
      // Meeting notes and Ask use the same model whatever this switch says.
      detail: c.model ? `Notes and Ask still use ${c.model}` : "Notes and Ask have no model",
      level: "off",
      local: dest.local,
    };
  }
  const prompt = c.promptName ?? "No prompt chosen";
  const detail = c.routingOn ? `${prompt} · varies by app` : prompt;
  if (!c.model) return { label, value: "No model chosen", detail, level: "warn", local: dest.local };
  const ollamaDown = c.provider?.id === "ollama" && c.ollamaUp === false;
  return {
    label,
    value: c.model,
    detail: ollamaDown ? "Ollama isn't running" : detail,
    level: ollamaDown ? "warn" : "ok",
    local: dest.local,
  };
}

/** True only when clean-up is on, the provider is Ollama and `tag` is its model. */
export function isLocalInUse(
  s:
    | {
        post_process_enabled?: boolean;
        post_process_provider_id?: string;
        post_process_models?: Partial<Record<string, string>>;
      }
    | null
    | undefined,
  tag: string,
): boolean {
  return (
    !!s?.post_process_enabled &&
    s.post_process_provider_id === "ollama" &&
    (s.post_process_models ?? {})["ollama"] === tag
  );
}

/** Mic and speech model can't change mid-recording: the stream restarts and the take is lost. */
export function lockedReason(meetingRecording: boolean, dictating: boolean): string | null {
  if (meetingRecording) return "Can't change during a meeting recording";
  if (dictating) return "Can't change while dictating";
  return null;
}
