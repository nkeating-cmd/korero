import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { LanguageSelector } from "../LanguageSelector";
import { NzEnglishToggle } from "../NzEnglishToggle";
import { TranslateToEnglish } from "../TranslateToEnglish";
import { useModelStore } from "../../../stores/modelStore";
import { useSettings } from "../../../hooks/useSettings";
import type { ModelInfo } from "@/bindings";

/**
 * Korero (P0-NZ, 2026-09-02): whole-file overlay.
 *
 * Upstream gates the entire card on `supportsLanguageSelection || supportsTranslation`, and
 * returns `null` when neither holds. Parakeet V3 reports FALSE for both -- so on the model
 * onboarding recommends, this card did not render at all, and there was no UI anywhere capable of
 * setting the NZ English locale.
 *
 * The NZ switch is not an engine capability: it is a post-transcription text pass that works on
 * every engine. So it is a third, unconditional reason for the card to exist.
 *
 * Forked rather than patched because this file is 43 lines and a third of it changes; the house
 * rule is that a patch is for a fix too small to justify a fork, and a file may not be both an
 * overlay AND a patch target without a `$PatchTargetExemptions` entry.
 */
export const ModelSettingsCard: React.FC = () => {
  const { t } = useTranslation();
  const { currentModel, models } = useModelStore();
  const { getSetting } = useSettings();

  const currentModelInfo = models.find((m: ModelInfo) => m.id === currentModel);

  const supportsLanguageSelection =
    currentModelInfo?.supports_language_selection ?? false;
  const supportsTranslation = currentModelInfo?.supports_translation ?? false;

  // The NZ locale pass is engine-independent, so it is offered whenever the dictation language is
  // English or auto-detect. Mirrors the guard inside NzEnglishToggle so the card does not render
  // an empty SettingsGroup when the toggle hides itself.
  const selectedLanguage = getSetting("selected_language") || "auto";
  const canOfferNzEnglish =
    selectedLanguage === "en" ||
    selectedLanguage === "en-NZ" ||
    selectedLanguage === "auto";

  const hasAnySettings =
    supportsLanguageSelection || supportsTranslation || canOfferNzEnglish;

  // Don't render anything if no model is selected or no settings available
  if (!currentModel || !currentModelInfo || !hasAnySettings) {
    return null;
  }

  return (
    <SettingsGroup
      title={t("settings.modelSettings.title", {
        model: currentModelInfo.name,
      })}
    >
      {supportsLanguageSelection && (
        <LanguageSelector
          descriptionMode="tooltip"
          grouped={true}
          supportedLanguages={currentModelInfo.supported_languages}
        />
      )}
      {canOfferNzEnglish && (
        <NzEnglishToggle descriptionMode="tooltip" grouped={true} />
      )}
      {supportsTranslation && (
        <TranslateToEnglish descriptionMode="tooltip" grouped={true} />
      )}
    </SettingsGroup>
  );
};
