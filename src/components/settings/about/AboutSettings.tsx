/* eslint-disable i18next/no-literal-string */
import React, { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Button } from "../../ui/Button";
import { AppDataDirectory } from "../AppDataDirectory";
import { AppLanguageSelector } from "../AppLanguageSelector";
import { LogDirectory } from "../debug";

/**
 * Kōrero fork (v1.12.0): About page.
 *
 * Forked from upstream Handy to (1) drop the upstream "Support Development"
 * donate row — not relevant to this personal fork — and (2) point Source Code
 * at this fork's repository instead of cjpais/Handy. The Acknowledgments
 * section is kept to credit Handy + whisper.cpp per the MIT licence.
 */

const KORERO_REPO_URL = "https://github.com/nkeating-cmd/korero";

// Korero (v1.28.0): a real support address, so someone who installs this build
// has somewhere to write that is not a GitHub issue. Business contact by design --
// the maintainer's personal address is deliberately not shipped in the binary.
const SUPPORT_EMAIL = "nic@kyt.nz";

export const AboutSettings: React.FC = () => {
  const { t } = useTranslation();
  const [version, setVersion] = useState("");

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("unknown");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.about.title")}>
        <AppLanguageSelector descriptionMode="tooltip" grouped={true} />
        <SettingContainer
          title={t("settings.about.version.title")}
          description={t("settings.about.version.description")}
          grouped={true}
        >
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span className="text-sm font-mono">v{version}</span>
        </SettingContainer>

        <SettingContainer
          title={t("settings.about.sourceCode.title")}
          description={t("settings.about.sourceCode.description")}
          grouped={true}
        >
          <Button
            variant="secondary"
            size="md"
            onClick={() => openUrl(KORERO_REPO_URL)}
          >
            {t("settings.about.sourceCode.button")}
          </Button>
        </SettingContainer>

        <SettingContainer
          title="Support"
          description="Questions, bug reports, or feedback"
          grouped={true}
        >
          <Button
            variant="secondary"
            size="md"
            onClick={() => openUrl(`mailto:${SUPPORT_EMAIL}?subject=${encodeURIComponent(`Korero v${version} support`)}`)}
          >
            {SUPPORT_EMAIL}
          </Button>
        </SettingContainer>

        <AppDataDirectory descriptionMode="tooltip" grouped={true} />
        <LogDirectory grouped={true} />
      </SettingsGroup>

      <SettingsGroup title="About Kōrero">
        <SettingContainer
          title="What it is"
          description="A personal, on-device speech app — dictation, meeting notes, and spoken audio briefs. Your audio and text never leave your computer."
          grouped={true}
          layout="stacked"
        >
          <div className="text-sm text-text-muted leading-relaxed">
            Kōrero is a personal fork of Handy (MIT) by CJ Pais. Speech-to-text
            is powered by whisper.cpp and NVIDIA Parakeet via the transcribe-rs
            runtime. Spoken audio briefs are produced by a local Qwen3-TTS
            engine. Post-processing runs either on a cloud provider with your
            own API key, or fully locally via Ollama. With thanks to all of
            these open-source projects.
          </div>
        </SettingContainer>
      </SettingsGroup>
    </div>
  );
};
