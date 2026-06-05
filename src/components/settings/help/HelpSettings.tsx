/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useState } from "react";
import { toast } from "sonner";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { Button } from "../../ui/Button";
import { LogLevelSelector, LogDirectory } from "../debug";
import { useSettings } from "../../../hooks/useSettings";
import { commands } from "../../../bindings";

/**
 * Kōrero fork (v1.12.0): Help & Guide page.
 *
 * Sits between Post Process and About in the sidebar. Plain-English guidance on
 * how to use Kōrero — the transcription model and its load/unload behaviour,
 * shortcuts, post-processing, troubleshooting — plus a Diagnostics block that
 * surfaces the log-level selector + log folder (previously buried in the Debug
 * section behind debug_mode) so a normal user can raise logging when reporting
 * an issue. Crash-report controls slot into Diagnostics once the panic-hook
 * backend lands.
 *
 * Guidance text is intentionally literal (not i18n) — it is fork-specific and
 * would bloat the locale files; the file-level eslint-disable above covers it.
 */

/** One guidance entry: a heading and a short body, divider-separated by the group. */
const Guide: React.FC<{ title: string; children: React.ReactNode }> = ({
  title,
  children,
}) => (
  <div className="px-4 py-3 space-y-1">
    <h3 className="text-sm font-medium text-text">{title}</h3>
    <div className="text-sm text-text-muted leading-relaxed space-y-1.5">
      {children}
    </div>
  </div>
);

/** Accent callout used for the fixed-bug note and caveats. */
const Callout: React.FC<{
  tone?: "info" | "warning" | "positive";
  children: React.ReactNode;
}> = ({ tone = "info", children }) => {
  const border =
    tone === "warning"
      ? "border-pill-warning"
      : tone === "positive"
        ? "border-pill-positive"
        : "border-aurora-cyan";
  return (
    <div
      className={`mt-2 rounded-r-lg border-l-2 ${border} bg-glass-surface-thin px-3 py-2 text-sm text-text-muted`}
    >
      {children}
    </div>
  );
};

export const HelpSettings: React.FC = () => {
  const { settings } = useSettings();
  const [saveCrash, setSaveCrash] = useState(true);

  useEffect(() => {
    if (settings) setSaveCrash(settings.save_crash_reports ?? true);
  }, [settings?.save_crash_reports]);

  const onToggleCrash = async (value: boolean) => {
    setSaveCrash(value);
    try {
      await commands.setSaveCrashReports(value);
    } catch (e) {
      toast.error(`Could not update crash report setting: ${String(e)}`);
    }
  };

  const openCrashFolder = async () => {
    try {
      await commands.openCrashReportsDir();
    } catch (e) {
      toast.error(`Could not open crash reports folder: ${String(e)}`);
    }
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <div className="px-4 pt-1">
        <h1 className="text-lg font-semibold text-text">Help &amp; Guide</h1>
        <p className="text-sm text-text-subtle mt-1">
          How Kōrero works, how to drive it from the keyboard, and what to do
          when something misbehaves.
        </p>
      </div>

      <SettingsGroup title="Getting started">
        <Guide title="The basic loop">
          <p>
            Kōrero turns speech into text, fully on your machine. Press your
            dictate shortcut, speak, then release — Kōrero transcribes your
            audio and types the result straight into whatever app holds your
            cursor. No audio or text leaves your computer.
          </p>
        </Guide>
        <Guide title="Where the controls live">
          <p>
            General (shortcuts, microphone, post-processing toggle), Models
            (which speech model is active), Advanced (idle unload, noise
            suppression, retention), Post Process (prompts and provider), and
            History (past dictations). This page links to each where relevant.
          </p>
        </Guide>
      </SettingsGroup>

      <SettingsGroup title="The transcription model">
        <Guide title="First use and downloads">
          <p>
            The speech model (Parakeet V3 by default) downloads once on first
            use — that single step needs internet. After it has downloaded,
            transcription is fully offline. Manage or switch models in
            Settings → Models.
          </p>
        </Guide>
        <Guide title="Loading and unloading">
          <p>
            For speed, the model is held in memory. Kōrero pre-warms it at
            startup and keeps it resident while you are working. To free memory
            it unloads the model after a period of inactivity.
          </p>
          <p>
            Set that window in Settings → Advanced → Model unload timeout.
            Never keeps the model resident for the fastest possible first word;
            Immediately frees memory after every dictation but adds a reload
            delay each time; the minute options sit in between.
          </p>
          <Callout tone="positive">
            After an unload, your next dictation reloads the model
            automatically — expect a brief pause on the first word, then normal
            speed.
          </Callout>
        </Guide>
      </SettingsGroup>

      <SettingsGroup title="Shortcuts">
        <Guide title="Push-to-talk or toggle">
          <p>
            In push-to-talk you hold the shortcut while speaking and release to
            finish. In toggle mode you tap once to start and tap again to stop.
            Choose the mode and set every binding in Settings → General.
          </p>
        </Guide>
        <Guide title="Dictate">
          <p>
            Your main shortcut: speak, release, and the text is inserted at the
            cursor.
          </p>
        </Guide>
        <Guide title="Dictate and clean up">
          <p>
            A separate shortcut (Ctrl+Shift+Space by default) that transcribes
            and then runs your chosen post-processing prompt before inserting
            the result. It is only active while post-processing is turned on.
          </p>
        </Guide>
        <Guide title="Hands-free latch">
          <p>
            Double-tap the dictate shortcut to lock recording on for a long,
            hands-free dictation; a single tap stops it. Useful for notes and
            long passages where you do not want to hold a key.
          </p>
        </Guide>
        <Guide title="Cancel">
          <p>
            Press Esc (or your cancel shortcut) while recording to discard the
            current capture without transcribing it.
          </p>
        </Guide>
      </SettingsGroup>

      <SettingsGroup title="Post-processing">
        <Guide title="What it does">
          <p>
            Post-processing runs your raw transcript through an AI prompt — to
            tidy it up, or reshape it into an email, a Slack or WhatsApp
            message, a meeting note, and so on.
          </p>
        </Guide>
        <Guide title="Turn it on">
          <p>
            Enable Post-processing in Settings → General. A Post Process section
            then appears in the sidebar where you manage prompts and the
            provider.
          </p>
        </Guide>
        <Guide title="Provider and API key">
          <p>
            Pick a provider and paste its API key (DeepSeek is the default).
            Keys are stored in your operating system keychain, never written to
            disk in plain text.
          </p>
        </Guide>
        <Guide title="Using it">
          <p>
            Trigger it with the Dictate and clean up shortcut, or re-process a
            past entry from History. The active prompt is chosen in Post
            Process.
          </p>
        </Guide>
      </SettingsGroup>

      <SettingsGroup title="Troubleshooting">
        <Guide title="A shortcut stopped responding">
          <p>
            A model-reload edge case could previously leave dictation stuck
            until you restarted the app. That is fixed in this version.
          </p>
          <Callout tone="info">
            If a shortcut ever stops responding: dictate once more (the model
            reloads itself). If it is still stuck, restart Kōrero, then turn on
            Trace logging below, reproduce the problem, and send the newest log
            file.
          </Callout>
        </Guide>
        <Guide title="Nothing was typed after I spoke">
          <p>
            Check that the correct input device is selected in Settings →
            General, that you began speaking after the start sound, and that the
            target app had focus. Very short clips can produce no text.
          </p>
        </Guide>
        <Guide title="The model will not load, or the first word is slow">
          <p>
            The first dictation after launch — or after an idle unload — takes a
            moment while the model loads. Set Model unload timeout to Never to
            keep it resident. Also confirm the model finished downloading in
            Settings → Models.
          </p>
        </Guide>
        <Guide title="Microphone is blocked">
          <p>
            Grant microphone permission to Kōrero in Windows Settings. On a
            permission error Kōrero offers a direct link to the right settings
            page.
          </p>
        </Guide>
        <Guide title="Accuracy in noisy rooms">
          <p>
            Optional noise suppression (Settings → Advanced) can help in genuinely
            noisy spaces.
          </p>
          <Callout tone="warning">
            Front-end denoising can slightly reduce accuracy in quiet
            conditions, so it is off by default. A/B test on your own microphone
            before leaving it on.
          </Callout>
        </Guide>
      </SettingsGroup>

      <SettingsGroup
        title="Diagnostics and logs"
        description="Raise the detail, reproduce the issue, then open the log folder and share the newest file."
      >
        <LogLevelSelector grouped={true} />
        <LogDirectory grouped={true} />
        <SettingContainer
          title="Save crash reports"
          description="On a fatal error, write a timestamped crash report with a backtrace to a folder you can open below. Panics are always written to the log regardless of this setting."
          descriptionMode="inline"
          grouped={true}
        >
          <ToggleSwitch
            checked={saveCrash}
            onChange={onToggleCrash}
            label="Save crash reports"
            description="Write a crash report file on a fatal error."
          />
        </SettingContainer>
        <SettingContainer
          title="Crash reports folder"
          description="Open the folder where saved crash reports are kept."
          descriptionMode="inline"
          grouped={true}
        >
          <Button variant="secondary" size="md" onClick={openCrashFolder}>
            Open folder
          </Button>
        </SettingContainer>
      </SettingsGroup>
    </div>
  );
};
