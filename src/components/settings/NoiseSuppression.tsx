import React from "react";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

// Kōrero (v1.11.0): toggle for the optional RNNoise microphone denoiser.
// Mirrors MuteWhileRecording. Uses literal label/description (no i18n key needed).
interface NoiseSuppressionToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const NoiseSuppression: React.FC<NoiseSuppressionToggleProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("denoise_enabled") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("denoise_enabled", value)}
        isUpdating={isUpdating("denoise_enabled")}
        label="Noise suppression"
        description="Reduce steady background noise before transcription (RNNoise). Best in noisy rooms; may not help — or may slightly hurt — in quiet ones. Requires a 48 kHz microphone."
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
