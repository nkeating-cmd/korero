/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { useSettingsStore } from "../../stores/settingsStore";
import { useModelStore } from "../../stores/modelStore";
import { useRecorder } from "../../stores/recorderStore";
import { useAppStatus } from "../../stores/appStatusStore";
import { useNav } from "../../stores/navStore";
import type { Meeting } from "../meetings/model";
import { micLabel } from "../shell/inUseModel";

/**
 * Kōrero 1.42: "Ready to go". One look tells you whether dictation and
 * meetings will work: the mic, New Zealand English, the speech model, the notes
 * model, call audio and the version. Each problem comes with the one button
 * that fixes it.
 *
 * Why it exists: F13. The New Zealand English pass sat switched off for months
 * on both of the maintainer's machines and nothing anywhere said so.
 */

type Level = "ok" | "warn" | "off";

interface Check {
  id: string;
  label: string;
  detail: string;
  level: Level;
  action?: { label: string; run: () => void | Promise<void> };
}

const Dot: React.FC<{ level: Level }> = ({ level }) => (
  <span
    className={`kx-dot mt-[6px] ${level === "ok" ? "kx-dot-ok" : level === "warn" ? "kx-dot-warn" : ""}`}
    aria-label={level === "ok" ? "OK" : level === "warn" ? "Needs a look" : "Off"}
    role="img"
  />
);

export const HealthPanel: React.FC<{ meetings: Meeting[] | null }> = ({ meetings }) => {
  const { settings } = useSettings();
  const audioDevices = useSettingsStore((s) => s.audioDevices);
  const models = useModelStore((s) => s.models);
  const currentModel = useModelStore((s) => s.currentModel);
  const testing = useRecorder((s) => s.testing);
  const lastTest = useRecorder((s) => s.lastTest);
  const update = useAppStatus((s) => s.updateAvailable);
  const [version, setVersion] = useState("");
  const [ollama, setOllama] = useState<{ installed: boolean; running: boolean } | null>(null);
  const [loaded, setLoaded] = useState<boolean | null>(null);

  const provider = settings?.post_process_providers?.find(
    (p) => p.id === settings?.post_process_provider_id,
  );
  const ppEnabled = settings?.post_process_enabled ?? false;
  const ppModel =
    (settings?.post_process_models ?? {})[settings?.post_process_provider_id ?? ""] ?? "";
  const isOllama = provider?.id === "ollama";

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
    commands
      .getModelLoadStatus()
      .then((r) => setLoaded(r.status === "ok" ? r.data.is_loaded : null))
      .catch(() => setLoaded(null));
  }, [currentModel]);

  useEffect(() => {
    if (!ppEnabled || !isOllama || !provider) {
      setOllama(null);
      return;
    }
    commands
      .ollamaStatus(provider.base_url)
      .then((s) => setOllama({ installed: s.installed, running: s.running }))
      .catch(() => setOllama(null));
  }, [ppEnabled, isOllama, provider?.base_url]);

  const checks: Check[] = [];

  // Microphone
  // Kōrero 1.43: shared with the In use panel. The store reports "no choice"
  // as "Default" and lists a synthetic "Default" device, so the old
  // lowercase compare never named the real device.
  const mic = micLabel(settings?.selected_microphone, audioDevices);
  checks.push({
    id: "mic",
    label: "Microphone",
    detail: mic.missing ? `${mic.name} is not connected` : mic.usingDefault ? `${mic.name} · system default` : mic.name,
    level: mic.missing ? "warn" : "ok",
    action: mic.missing ? { label: "Choose", run: () => useNav.getState().go("general") } : undefined,
  });

  // New Zealand English
  const nz = settings?.selected_language === "en-NZ";
  checks.push({
    id: "nz",
    label: "New Zealand English",
    detail: nz ? "On · macrons, NZ places and spelling" : "Off · macrons and NZ spelling are not applied",
    level: nz ? "ok" : "warn",
    action: nz
      ? undefined
      : {
          label: "Turn on",
          run: async () => {
            await useSettingsStore.getState().updateSetting("selected_language", "en-NZ");
            toast.success("New Zealand English is on.");
          },
        },
  });

  // Speech model
  const model = models.find((m) => m.id === currentModel);
  checks.push({
    id: "model",
    label: "Speech model",
    detail: model
      ? `${model.name}${loaded ? " · loaded" : " · loads when you dictate"}`
      : "No model chosen",
    level: model ? "ok" : "warn",
    action: model ? undefined : { label: "Choose", run: () => useNav.getState().go("models") },
  });

  // AI clean-up / notes model
  if (!ppEnabled) {
    checks.push({
      id: "ai",
      label: "AI clean-up & notes",
      detail: "Off · meetings get transcripts only",
      level: "off",
      action: { label: "Set up", run: () => useNav.getState().go("postprocessing") },
    });
  } else if (!provider?.is_local_provider) {
    checks.push({
      id: "ai",
      label: "AI clean-up & notes",
      detail: `${provider?.label ?? "Cloud"}${ppModel ? ` · ${ppModel}` : ""} · transcripts leave this computer`,
      level: "warn",
      action: { label: "Change", run: () => useNav.getState().go("postprocessing") },
    });
  } else if (isOllama && ollama && !ollama.running) {
    checks.push({
      id: "ai",
      label: "Notes model",
      detail: ollama.installed ? "Ollama is not running" : "Ollama is not installed",
      level: "warn",
      action: ollama.installed
        ? {
            label: "Start",
            run: async () => {
              const r = await commands.ollamaStart(provider?.base_url ?? "");
              if (r.status === "ok") {
                toast.success("Ollama started.");
                setOllama({ installed: true, running: true });
              } else toast.error(r.error);
            },
          }
        : { label: "Set up", run: () => useNav.getState().go("postprocessing") },
    });
  } else {
    checks.push({
      id: "ai",
      label: "Notes model",
      detail: `${ppModel || provider.label}${isOllama ? " · Ollama running" : " · on this computer"}`,
      level: ppModel ? "ok" : "warn",
      action: ppModel ? undefined : { label: "Choose", run: () => useNav.getState().go("postprocessing") },
    });
  }

  // Call audio: this session's test wins; otherwise the last recorded meeting.
  const lastRecorded = (meetings ?? [])
    .filter((m) => !m.imported)
    .sort((a, b) => b.createdAt - a.createdAt)[0];
  const lastMeetingOk =
    lastRecorded &&
    lastRecorded.systemCaptured &&
    !(lastRecorded.captureWarnings ?? []).some((w) => /system|call audio|others/i.test(w));
  const callDetail = testing
    ? "Testing… play some audio"
    : lastTest
      ? lastTest.systemOk && lastTest.micOk
        ? "Both sides captured in your test"
        : lastTest.systemOk
          ? "Test: call audio OK, microphone silent"
          : "Test: no call audio captured"
      : lastRecorded
        ? lastMeetingOk
          ? "Both sides captured in your last meeting"
          : "Your last meeting missed some call audio"
        : "Not tested yet";
  const callLevel: Level = testing
    ? "off"
    : lastTest
      ? lastTest.systemOk && lastTest.micOk
        ? "ok"
        : "warn"
      : lastRecorded
        ? lastMeetingOk
          ? "ok"
          : "warn"
        : "off";
  checks.push({
    id: "call",
    label: "Call audio",
    detail: callDetail,
    level: callLevel,
    action:
      callLevel === "ok" || testing
        ? undefined
        : { label: "Test", run: () => useRecorder.getState().testCapture() },
  });

  // Version
  checks.push({
    id: "version",
    label: "Version",
    detail: update ? `${version} · ${update.version} is available` : version || "…",
    level: update ? "warn" : "ok",
    action: update
      ? {
          label: "Install",
          run: async () => {
            const r = await commands.installUpdate();
            if (r.status === "error") toast.error(r.error);
          },
        }
      : undefined,
  });

  const problems = checks.filter((c) => c.level === "warn").length;

  return (
    <section aria-labelledby="today-ready" className="kx-card px-4 pt-3.5 pb-1.5">
      <h2 id="today-ready" className="kx-heading">
        {problems === 0 ? "Ready to go" : `Ready to go · ${problems} to look at`}
      </h2>
      <p className="kx-meta mb-2">Dictation and meetings, checked.</p>
      <div className="kx-divide">
        {checks.map((c) => (
          <div key={c.id} className="flex gap-2.5 py-2.5 items-start">
            <Dot level={c.level} />
            <div className="flex-1 min-w-0">
              <div className="text-[13px] text-white">{c.label}</div>
              <div className="kx-meta break-words">{c.detail}</div>
            </div>
            {c.action && (
              <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm self-center shrink-0" onClick={() => void c.action?.run()}>
                {c.action.label}
              </button>
            )}
          </div>
        ))}
      </div>
    </section>
  );
};
