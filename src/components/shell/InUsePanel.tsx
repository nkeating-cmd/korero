/* eslint-disable i18next/no-literal-string */
import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AudioLines, Check, Download, Lock, Mic, Settings2, Sparkles } from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { useModelStore } from "../../stores/modelStore";
import { useNav, type Section } from "../../stores/navStore";
import { useRecorder } from "../../stores/recorderStore";
import { cleanupState, lockedReason, micLabel, speechState, type Level } from "./inUseModel";

/**
 * Kōrero 1.43: "In use". The three things that decide what a dictation turns
 * into (the microphone, the speech model and the clean-up model with its
 * prompt) always visible at the foot of the sidebar, on every page, each a
 * click away from being changed. Replaces the speech-model-only picker.
 *
 * Rules live in `inUseModel.ts` (unit-tested). Safety rules, from the 1.43
 * security review: the clean-up row always says where text goes; picking a
 * model never switches clean-up on; switching it on for a provider that sends
 * text off the computer goes through AI clean-up settings; the microphone and
 * speech model can't change mid-recording (the take would be lost).
 */

type Which = "mic" | "speech" | "cleanup";

interface Row {
  id: Which;
  icon: React.ReactNode;
  label: string;
  value: string;
  detail?: string;
  level: Level;
  mono?: boolean;
}

// Shape as well as colour (A11Y-04): ok is a filled circle, warn a diamond,
// off a hollow ring.
const dotClass = (l: Level) =>
  l === "ok" ? "kx-dot-ok" : l === "warn" ? "kx-dot-warn" : "!bg-transparent border border-[var(--kx-ink-2)]";
const levelWord = (l: Level) => (l === "warn" ? "Needs a look. " : "");

const MenuSection: React.FC<{ title: string; children: React.ReactNode }> = ({ title, children }) => (
  <div className="py-1" role="group" aria-label={title}>
    <div className="kx-overline px-2.5 pt-1.5 pb-1" aria-hidden="true">
      {title}
    </div>
    {children}
  </div>
);

const Item: React.FC<{
  checked?: boolean;
  onSelect: () => void;
  children: React.ReactNode;
  sub?: string;
  hint?: string;
  icon?: React.ReactNode;
  disabled?: boolean;
}> = ({ checked, onSelect, children, sub, hint, icon, disabled }) => (
  <button
    type="button"
    role={checked === undefined ? "menuitem" : "menuitemradio"}
    aria-checked={checked === undefined ? undefined : checked}
    // aria-disabled, not disabled: unavailable choices stay reachable with the
    // arrow keys and are announced as such (A11Y-03).
    aria-disabled={disabled || undefined}
    tabIndex={-1}
    className={`kx-menu-item gap-2 ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
    onClick={disabled ? undefined : onSelect}
  >
    <span className="w-4 shrink-0 flex justify-center" aria-hidden="true">
      {checked ? <Check size={14} /> : icon}
    </span>
    <span className="flex-1 min-w-0 py-1">
      <span className="block truncate">{children}</span>
      {sub && <span className="block truncate text-[11.5px] font-normal text-[var(--kx-ink-2)]">{sub}</span>}
    </span>
    {hint && <span className="kx-meta shrink-0 !text-[11.5px]">{hint}</span>}
  </button>
);

export const InUsePanel: React.FC<{ collapsed: boolean }> = ({ collapsed }) => {
  const {
    settings,
    audioDevices,
    refreshAudioDevices,
    updateSetting,
    updatePostProcessModel,
    postProcessModelOptions,
    fetchPostProcessModels,
  } = useSettings();
  const models = useModelStore((s) => s.models);
  const currentModel = useModelStore((s) => s.currentModel);
  const selectModel = useModelStore((s) => s.selectModel);
  const downloadProgress = useModelStore((s) => s.downloadProgress);
  const verifying = useModelStore((s) => s.verifyingModels);
  const extracting = useModelStore((s) => s.extractingModels);
  const meetingRecording = useRecorder((s) => s.recording);
  const go = useNav((s) => s.go);

  const [open, setOpen] = useState<Which | null>(null);
  const [locked, setLocked] = useState<string | null>(null);
  const [maxH, setMaxH] = useState(420);
  const [ollamaUp, setOllamaUp] = useState<boolean | null>(null);
  const [load, setLoad] = useState<"idle" | "loading" | "error">("idle");
  const rootRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const rowRefs = useRef<Partial<Record<Which, HTMLButtonElement | null>>>({});

  const provider = settings?.post_process_providers?.find((p) => p.id === settings?.post_process_provider_id);
  const ppEnabled = settings?.post_process_enabled ?? false;
  const ppModel = (settings?.post_process_models ?? {})[settings?.post_process_provider_id ?? ""] ?? "";
  const prompts = settings?.post_process_prompts ?? [];
  const promptId = settings?.post_process_selected_prompt_id ?? null;
  const routingOn = (settings?.post_process_app_routes ?? []).length > 0;

  // Ollama status: on mount, when the menu opens and when the window regains focus.
  const probeOllama = useCallback(() => {
    if (!provider || provider.id !== "ollama") {
      setOllamaUp(null);
      return;
    }
    commands
      .ollamaStatus(provider.base_url)
      .then((s) => setOllamaUp(s.running))
      .catch(() => setOllamaUp(null));
  }, [provider?.id, provider?.base_url]);
  useEffect(() => {
    probeOllama();
    window.addEventListener("focus", probeOllama);
    return () => window.removeEventListener("focus", probeOllama);
  }, [probeOllama]);

  // Speech model load state (the old picker tracked this; keep it).
  useEffect(() => {
    const un = listen<{ event_type: string }>("model-state-changed", (e) => {
      const t = e.payload.event_type;
      if (t === "loading_started") setLoad("loading");
      else if (t === "loading_failed") setLoad("error");
      else setLoad("idle");
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(null);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  // Focus the checked item (or the first) when a menu opens.
  useLayoutEffect(() => {
    if (!open || !menuRef.current) return;
    const items = Array.from(menuRef.current.querySelectorAll<HTMLButtonElement>("[role^='menuitem']"));
    (items.find((b) => b.getAttribute("aria-checked") === "true") ?? items[0])?.focus();
  }, [open, locked]);

  const close = (focusRow = true) => {
    const was = open;
    setOpen(null);
    if (focusRow && was) rowRefs.current[was]?.focus();
  };

  const onMenuKey = (e: React.KeyboardEvent) => {
    const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>("[role^='menuitem']") ?? []);
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    const to = (n: number) => {
      e.preventDefault();
      items[(n + items.length) % items.length]?.focus();
    };
    if (e.key === "ArrowDown") to(i + 1);
    else if (e.key === "ArrowUp") to(i - 1);
    else if (e.key === "Home") to(0);
    else if (e.key === "End") to(items.length - 1);
    else if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") {
      // Leave the menu by its row, so Tab and Shift+Tab carry on from there (A11Y-08).
      e.preventDefault();
      close();
    }
  };

  const toggle = async (id: Which) => {
    if (open === id) return close();
    const row = rowRefs.current[id];
    if (row) {
      const r = row.getBoundingClientRect();
      setMaxH(Math.max(160, Math.min(420, (collapsed ? r.bottom : r.top) - 16)));
    }
    let reason: string | null = null;
    if (id !== "cleanup") {
      const dictating = await commands.isRecording().catch(() => false);
      reason = lockedReason(meetingRecording, dictating);
    }
    setLocked(reason);
    setOpen(id);
    if (id === "mic" && !reason) void refreshAudioDevices();
    if (id === "cleanup") {
      probeOllama();
      if (provider) void fetchPostProcessModels(provider.id).catch(() => {});
    }
  };

  // Focus goes back to the row, not to the page body (A11Y-01).
  const goTo = (s: Section) => {
    close();
    go(s);
  };
  // The menu said "unlocked" when it opened; check again at the moment of
  // change, and treat an unknown state as locked (refuter DEF-04).
  const stillLocked = async (): Promise<string | null> => {
    const dictating = await commands.isRecording().catch(() => true);
    return lockedReason(useRecorder.getState().recording, dictating);
  };
  const attempt = async (what: string, fn: () => Promise<unknown>) => {
    close();
    try {
      await fn();
    } catch (e) {
      toast.error(`Couldn't change the ${what}`, { description: String(e) });
    }
  };

  // ---- rows ----------------------------------------------------------------
  const mic = micLabel(settings?.selected_microphone, audioDevices);
  const speechModel = models.find((m) => m.id === currentModel);
  const dlId = Object.keys(downloadProgress)[0];
  const dlName = (id?: string) => models.find((m) => m.id === id)?.name ?? "a model";
  const speech = speechState({
    modelName: speechModel?.name ?? null,
    load,
    downloading: dlId ? { name: dlName(dlId), percent: downloadProgress[dlId]?.percentage ?? 0 } : null,
    verifying: Object.keys(verifying)[0] ? dlName(Object.keys(verifying)[0]) : null,
    extracting: Object.keys(extracting)[0] ? dlName(Object.keys(extracting)[0]) : null,
  });
  const promptName = prompts.find((p) => p.id === promptId)?.name ?? null;
  const cleanup = cleanupState({ enabled: ppEnabled, provider, model: ppModel, promptName, routingOn, ollamaUp });
  const nz = settings?.selected_language === "en-NZ";

  const rows: Row[] = [
    {
      id: "mic",
      icon: <Mic size={15} />,
      label: mic.usingDefault ? "Microphone · system default" : "Microphone",
      value: mic.missing ? `${mic.name} (not connected)` : mic.name,
      level: mic.missing ? "warn" : "ok",
    },
    {
      id: "speech",
      icon: <AudioLines size={15} />,
      label: nz ? "Speech · NZ English" : "Speech",
      value: speech.value,
      detail: speech.detail,
      level: speech.level,
    },
    {
      id: "cleanup",
      icon: <Sparkles size={15} />,
      label: cleanup.label,
      value: cleanup.value,
      detail: cleanup.detail,
      level: cleanup.level,
      mono: cleanup.level !== "off" && !!ppModel && cleanup.value === ppModel,
    },
  ];

  // ---- menus ---------------------------------------------------------------
  const lockNote = locked ? (
    <div id="kx-inuse-lock" className="flex items-center gap-2 px-2.5 py-2 text-[12.5px] text-[var(--kx-ink-soft)]">
      <Lock size={13} aria-hidden="true" /> {locked}
    </div>
  ) : null;

  const footer = (label: string, section: Section) => (
    <div className="border-t border-[var(--kx-hairline)] mt-1 pt-1">
      <Item onSelect={() => goTo(section)} icon={<Settings2 size={14} />}>
        {label}
      </Item>
    </div>
  );

  const localModels = provider ? postProcessModelOptions[provider.id] ?? [] : [];
  const modelChoices = Array.from(new Set([ppModel, ...localModels].filter(Boolean)));

  const menu = (id: Which) => {
    if (id === "mic")
      return (
        <>
          {lockNote}
          <MenuSection title="Microphone">
            {audioDevices.map((d) => {
              const isDefault = d.name.toLowerCase() === "default";
              const checked = isDefault ? mic.usingDefault : d.name === settings?.selected_microphone;
              return (
                <Item
                  key={d.name}
                  checked={checked}
                  disabled={!!locked}
                  sub={isDefault && mic.usingDefault && mic.name !== "System default" ? `Now: ${mic.name}` : undefined}
                  onSelect={() =>
                    void attempt("microphone", async () => {
                      const why = await stillLocked();
                      if (why) throw new Error(why);
                      await updateSetting("selected_microphone", d.name);
                    })
                  }
                >
                  {isDefault ? "System default" : d.name}
                </Item>
              );
            })}
          </MenuSection>
          {footer("Dictation & sound settings", "general")}
        </>
      );
    if (id === "speech")
      return (
        <>
          {lockNote}
          <MenuSection title="Speech model">
            {models
              .filter((m) => m.is_downloaded)
              .map((m) => (
                <Item
                  key={m.id}
                  checked={m.id === currentModel}
                  disabled={!!locked}
                  hint={m.size_mb ? `${(m.size_mb / 1024).toFixed(1)} GB` : undefined}
                  onSelect={() =>
                    void attempt("speech model", async () => {
                      const why = await stillLocked();
                      if (why) throw new Error(why);
                      if (!(await selectModel(m.id))) throw new Error("the model didn't load");
                    })
                  }
                >
                  {m.name}
                </Item>
              ))}
          </MenuSection>
          {footer("Speech models", "models")}
        </>
      );
    return (
      <>
        <MenuSection title={`AI model · ${cleanup.local ? "on this computer" : provider?.label ?? "no provider"}`}>
          {modelChoices.map((m) => (
            <Item
              key={m}
              checked={m === ppModel}
              onSelect={() => void attempt("AI model", () => updatePostProcessModel(provider!.id, m))}
            >
              <span className="kx-mono text-[12.5px]">{m}</span>
            </Item>
          ))}
          <Item onSelect={() => goTo("postprocessing")} icon={<Download size={14} />}>
            Find a model for this computer…
          </Item>
        </MenuSection>
        <MenuSection title={routingOn ? "Prompt (default; per-app routing is on)" : "Prompt"}>
          {prompts.map((p) => (
            <Item
              key={p.id}
              checked={p.id === promptId}
              onSelect={() => void attempt("prompt", () => updateSetting("post_process_selected_prompt_id", p.id))}
            >
              {p.name}
            </Item>
          ))}
        </MenuSection>
        <div className="border-t border-[var(--kx-hairline)] mt-1 pt-1">
          {ppEnabled ? (
            <Item
              icon={<span className="kx-dot kx-dot-ok" />}
              onSelect={() => void attempt("clean-up setting", () => updateSetting("post_process_enabled", false))}
            >
              Turn clean-up off
            </Item>
          ) : cleanup.local ? (
            <Item
              icon={<span className="kx-dot" />}
              onSelect={() => void attempt("clean-up setting", () => updateSetting("post_process_enabled", true))}
            >
              Turn clean-up on
            </Item>
          ) : (
            // Switching on a provider that sends text off this computer is a
            // decision for the settings page, where the provider is named.
            <Item icon={<span className="kx-dot" />} sub={`It ${cleanup.label.replace("Clean-up · ", "")}`} onSelect={() => goTo("postprocessing")}>
              Turn clean-up on…
            </Item>
          )}
          <Item onSelect={() => goTo("postprocessing")} icon={<Settings2 size={14} />}>
            AI clean-up settings
          </Item>
        </div>
      </>
    );
  };

  const fullLabel = (r: Row) => `${r.label}: ${r.value}${r.detail ? ` · ${r.detail}` : ""}`;

  return (
    <div
      ref={rootRef}
      role="group"
      aria-label="In use"
      className={`relative flex ${collapsed ? "flex-col items-center gap-1" : "flex-col gap-0.5"}`}
    >
      {rows.map((r) =>
        collapsed ? (
          <button
            key={r.id}
            ref={(el) => {
              rowRefs.current[r.id] = el;
            }}
            type="button"
            aria-haspopup="menu"
            aria-expanded={open === r.id}
            title={fullLabel(r)}
            aria-label={`${levelWord(r.level)}${fullLabel(r)}. Change`}
            onClick={() => void toggle(r.id)}
            className={`kx-btn kx-btn-ghost kx-btn-icon relative ${open === r.id ? "bg-[var(--kx-raised-2)]" : ""}`}
          >
            {r.icon}
            <span className={`kx-dot absolute top-1.5 right-1.5 !w-[6px] !h-[6px] ${dotClass(r.level)}`} aria-hidden="true" />
          </button>
        ) : (
          <button
            key={r.id}
            ref={(el) => {
              rowRefs.current[r.id] = el;
            }}
            type="button"
            aria-haspopup="menu"
            aria-expanded={open === r.id}
            aria-label={`${levelWord(r.level)}${fullLabel(r)}. Change`}
            title={fullLabel(r)}
            onClick={() => void toggle(r.id)}
            className={`w-full flex items-center gap-2.5 min-h-[42px] pl-2.5 pr-1.5 rounded-[8px] text-left transition-colors hover:bg-[var(--kx-raised)] ${
              open === r.id ? "bg-[var(--kx-raised)]" : ""
            }`}
          >
            <span className="relative text-[var(--kx-ink-2)] shrink-0" aria-hidden="true">
              {r.icon}
              <span
                className={`kx-dot absolute -top-0.5 -right-1 !w-[6px] !h-[6px] ring-2 ring-[var(--kx-sidebar)] ${dotClass(r.level)}`}
              />
            </span>
            <span className="flex-1 min-w-0">
              <span className="block truncate text-[11px] leading-[14px] text-[var(--kx-ink-2)]">{r.label}</span>
              <span
                className={`block truncate leading-[17px] ${r.mono ? "kx-mono text-[12px]" : "text-[12.5px]"} ${
                  r.level === "off" ? "text-[var(--kx-ink-2)]" : r.level === "warn" ? "text-[var(--kx-warn-ink)]" : "text-[var(--kx-ink-read)]"
                }`}
              >
                {r.value}
              </span>
              {r.detail && <span className="block truncate text-[11.5px] leading-[15px] text-[var(--kx-ink-2)]">{r.detail}</span>}
            </span>
          </button>
        ),
      )}
      {open && (
        <div
          ref={menuRef}
          role="menu"
          aria-label={rows.find((r) => r.id === open)?.label}
          aria-describedby={locked && open !== "cleanup" ? "kx-inuse-lock" : undefined}
          onKeyDown={onMenuKey}
          style={{ maxHeight: maxH }}
          className={`kx-menu absolute z-50 w-[300px] !max-h-none overflow-y-auto ${
            collapsed ? "left-[52px] bottom-0" : "left-0 bottom-[calc(100%+6px)]"
          }`}
        >
          {menu(open)}
        </div>
      )}
    </div>
  );
};
