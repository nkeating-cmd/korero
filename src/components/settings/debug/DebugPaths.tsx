import React from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../../ui/SettingContainer";

interface DebugPathsProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

// Kōrero fork: Tauri identifier is `com.nkeating.korero`, so AppData
// resolves to %APPDATA%/com.nkeating.korero/ on Windows. Update the
// displayed paths to match.
const APP_DATA_DIR = "com.nkeating.korero";

export const DebugPaths: React.FC<DebugPathsProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();

  return (
    <SettingContainer
      title="Debug Paths"
      description="Display internal file paths and directories for debugging purposes"
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <div className="text-sm text-text-muted space-y-2">
        <div>
          <span className="font-medium">
            {t("settings.debug.paths.appData")}
          </span>{" "}
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span className="font-mono text-xs select-text">
            %APPDATA%/{APP_DATA_DIR}
          </span>
        </div>
        <div>
          <span className="font-medium">
            {t("settings.debug.paths.models")}
          </span>{" "}
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span className="font-mono text-xs select-text">
            %APPDATA%/{APP_DATA_DIR}/models
          </span>
        </div>
        <div>
          <span className="font-medium">
            {t("settings.debug.paths.settings")}
          </span>{" "}
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span className="font-mono text-xs select-text">
            %APPDATA%/{APP_DATA_DIR}/settings_store.json
          </span>
        </div>
      </div>
    </SettingContainer>
  );
};
