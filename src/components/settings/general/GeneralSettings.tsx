// Kōrero (v1.9.0, UI1): overlay of upstream GeneralSettings.tsx.
// Change: inline latch-mode tip below PushToTalk when PTT is enabled.
// Latch mode is a Kōrero-specific feature — double-tap the shortcut to lock
// recording on without holding the key; tap once to stop.
import React from "react";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import { MicrophoneSelector } from "../MicrophoneSelector";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { PushToTalk } from "../PushToTalk";
import { AudioFeedback } from "../AudioFeedback";
import { useSettings } from "../../../hooks/useSettings";
import { VolumeSlider } from "../VolumeSlider";
import { MuteWhileRecording } from "../MuteWhileRecording";
import { NoiseSuppression } from "../NoiseSuppression";
import { ModelSettingsCard } from "./ModelSettingsCard";

export const GeneralSettings: React.FC = () => {
  const { t } = useTranslation();
  const { audioFeedbackEnabled, getSetting } = useSettings();
  const pushToTalk = getSetting("push_to_talk");
  // Kōrero (v1.10.0): the record-and-clean-up shortcut only registers while
  // post-processing is enabled (see shortcut/mod.rs), so only surface it then.
  const postProcessEnabled = getSetting("post_process_enabled");
  const isLinux = type() === "linux";
  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.general.title")}>
        <ShortcutInput shortcutId="transcribe" grouped={true} />
        {/* Kōrero (v1.14.1): alternative dictation shortcut — same action as
            Transcribe, defaulting to Ctrl+Shift+Enter so it can be pressed
            with the right hand alone (Right Ctrl + Right Shift + Enter). */}
        <ShortcutInput shortcutId="transcribe_alt" grouped={true} />
        {/* Kōrero (v1.10.0): surface the record-and-clean-up shortcut
            (transcribe_with_post_process, default Ctrl+Shift+Space). It records,
            transcribes, then runs your selected post-processing prompt. In
            push-to-talk mode, double-tap it to latch a long hands-free recording
            (tap once to stop). Only shown when post-processing is enabled, since
            the binding only registers then. */}
        {postProcessEnabled && (
          <div>
            <ShortcutInput shortcutId="transcribe_with_post_process" grouped={true} />
            {pushToTalk && (
              <div className="mx-4 mb-3 mt-0.5 px-3 py-2 rounded-lg bg-white/5 border border-white/10 flex items-center gap-2.5">
                <span className="shrink-0 px-2 py-0.5 rounded text-xs font-semibold bg-logo-primary/20 text-logo-primary border border-logo-primary/25">
                  Latch
                </span>
                <span className="text-xs text-mid-gray/75 leading-relaxed">
                  Double-tap to lock on a long hands-free recording, then auto-clean with your post-processing prompt — tap once to stop.
                </span>
              </div>
            )}
          </div>
        )}
        {/* Kōrero (v1.10.0): PushToTalk + latch tip wrapped in a single element so
            SettingsGroup's `divide-y` treats them as ONE row. Previously the tip
            was a sibling child, so a hairline divider was drawn between the PTT
            toggle and its own explanatory note — visually splitting one control
            into two rows. Wrapping removes that stray divider and the tip now
            reads as a sub-note of PushToTalk. Insets aligned to px-4 to match the
            setting rows (was mx-3, which sat ~4px inboard of every other row). */}
        <div>
          <PushToTalk descriptionMode="tooltip" grouped={true} />
          {pushToTalk && (
            <div className="mx-4 mb-3 mt-0.5 px-3 py-2 rounded-lg bg-white/5 border border-white/10 flex items-center gap-2.5">
              <span className="shrink-0 px-2 py-0.5 rounded text-xs font-semibold bg-logo-primary/20 text-logo-primary border border-logo-primary/25">
                Latch
              </span>
              <span className="text-xs text-mid-gray/75 leading-relaxed">
                Double-tap to lock recording on without holding the key — tap once to stop.
              </span>
            </div>
          )}
        </div>
        {/* Cancel shortcut is hidden with push-to-talk (release key cancels) and on Linux (dynamic shortcut instability) */}
        {!isLinux && !pushToTalk && (
          <ShortcutInput shortcutId="cancel" grouped={true} />
        )}
      </SettingsGroup>
      <ModelSettingsCard />
      <SettingsGroup title={t("settings.sound.title")}>
        <MicrophoneSelector descriptionMode="tooltip" grouped={true} />
        <NoiseSuppression descriptionMode="tooltip" grouped={true} />
        <MuteWhileRecording descriptionMode="tooltip" grouped={true} />
        <AudioFeedback descriptionMode="tooltip" grouped={true} />
        <OutputDeviceSelector
          descriptionMode="tooltip"
          grouped={true}
          disabled={!audioFeedbackEnabled}
        />
        <VolumeSlider disabled={!audioFeedbackEnabled} />
      </SettingsGroup>
    </div>
  );
};
