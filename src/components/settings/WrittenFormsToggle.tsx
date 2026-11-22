import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface WrittenFormsToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/** Kōrero 1.50: spoken numbers, money, dates and times written as figures. */
export const WrittenFormsToggle: React.FC<WrittenFormsToggleProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    // Absent from an older store means on: the Rust default.
    const enabled = getSetting("written_forms_enabled") ?? true;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(enabled) => updateSetting("written_forms_enabled", enabled)}
        isUpdating={isUpdating("written_forms_enabled")}
        label={t("settings.debug.writtenForms.label")}
        description={t("settings.debug.writtenForms.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
