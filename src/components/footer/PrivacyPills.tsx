/* eslint-disable i18next/no-literal-string */
import React from "react";
import { useSettings } from "../../hooks/useSettings";

/**
 * Kōrero (UX roadmap item 2): granular privacy indicators.
 *
 * Two persistent pills in the footer:
 *  - Audio → On-Device   (always true: transcription is local by design)
 *  - Text  → On-Device | <provider> (cloud)
 *
 * "On-Device" copy is deliberate (UX roadmap item 1 — device-bound language
 * outperforms generic "private"/"offline" wording for trust).
 *
 * Text pill logic:
 *  - post-processing disabled            → On-Device (cyan)
 *  - enabled + is_local_provider         → On-Device via <label> (cyan)
 *  - enabled + cloud provider            → <label> (cloud) (amber)
 */
const PrivacyPills: React.FC = () => {
  const { settings } = useSettings();
  if (!settings) return null;

  const enabled = settings.post_process_enabled ?? false;
  const providerId = settings.post_process_provider_id;
  const provider = (settings.post_process_providers ?? []).find(
    (p) => p.id === providerId,
  );

  const textLocal = !enabled || (provider?.is_local_provider ?? false);
  const textLabel = !enabled
    ? "Text → On-Device"
    : provider
      ? provider.is_local_provider
        ? `Text → On-Device (${provider.label})`
        : `Text → ${provider.label} (cloud)`
      : "Text → On-Device";

  const pillBase =
    "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium tracking-wide whitespace-nowrap";

  return (
    <div className="flex items-center gap-1.5" aria-label="Privacy status">
      <span
        className={`${pillBase} glass-pill-accent`}
        title="Speech is transcribed entirely on this computer. Audio never leaves your machine."
      >
        Audio → On-Device
      </span>
      <span
        className={`${pillBase} ${textLocal ? "glass-pill-accent" : "pill-warning"}`}
        title={
          textLocal
            ? "Transcript text stays on this computer."
            : "AI clean-up sends transcript text to the selected cloud provider over HTTPS."
        }
      >
        {textLabel}
      </span>
    </div>
  );
};

export default PrivacyPills;
