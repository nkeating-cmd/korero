import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

/**
 * Korero (P0-NZ, 2026-09-02) -- the New Zealand English switch.
 *
 * WHY THIS IS NOT PART OF THE LANGUAGE PICKER.
 *
 * `en-NZ` is a Korero locale tag, not an engine language: it is folded to bare `en` before any
 * model sees it, and its only effect is the deterministic post-engine pass (macrons, NZ place
 * names, NZ spelling). That pass runs for EVERY engine.
 *
 * The language picker cannot carry it. `ModelSettingsCard` renders `LanguageSelector` only when
 * the model reports `supports_language_selection`, and Parakeet V3 -- the model onboarding
 * recommends, and the one Nic runs -- reports `false` for that AND for `supports_translation`, so
 * the entire card used to render `null`. There was literally no UI able to set this.
 *
 * So it is its own control, always available, and it writes the EXISTING `selected_language`
 * field. That is deliberate: a new settings field would regenerate `bindings.ts`, which carries 35
 * of the repo's 119 surgical patches. Reusing the string costs zero patches there.
 */

const NZ_LOCALE = "en-NZ";
const BASE_LOCALE = "en";

interface NzEnglishToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const NzEnglishToggle: React.FC<NzEnglishToggleProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const selected = getSetting("selected_language") || "auto";

    // Only meaningful when English is what is being spoken. Repairing NZ spelling and restoring
    // macrons in a French transcript is not a feature, it is corruption -- so the control hides
    // itself rather than offering a switch that would do harm.
    if (
      selected !== NZ_LOCALE &&
      selected !== BASE_LOCALE &&
      selected !== "auto"
    ) {
      return null;
    }

    return (
      <ToggleSwitch
        checked={selected === NZ_LOCALE}
        onChange={(enabled) =>
          updateSetting("selected_language", enabled ? NZ_LOCALE : BASE_LOCALE)
        }
        isUpdating={isUpdating("selected_language")}
        label={t("settings.modelSettings.nzEnglish.label", {
          defaultValue: "New Zealand English",
        })}
        description={t("settings.modelSettings.nzEnglish.description", {
          defaultValue:
            "Restores macrons in te reo Maori, corrects New Zealand place names, and uses NZ spelling. Applied after transcription, so it works on every model. Does not change the speech model's language.",
        })}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
