import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";

import ModelSelector from "../model-selector";
import PrivacyPills from "./PrivacyPills";

/**
 * Kōrero footer — UpdateChecker removed (the plugin was dropped in our fork
 * to prevent auto-updates pulling upstream Handy releases). Shows the
 * currently-selected model and the app version only.
 */
const Footer: React.FC = () => {
  const [version, setVersion] = useState("");

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("1.0.0");
      }
    };
    fetchVersion();
  }, []);

  return (
    <div className="w-full border-t border-glass-border pt-3">
      <div className="flex justify-between items-center text-xs px-4 pb-3 text-text-muted">
        <div className="flex items-center gap-4">
          <ModelSelector />
        </div>
        <div className="flex items-center gap-2">
          <PrivacyPills />
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span>v{version}</span>
        </div>
      </div>
    </div>
  );
};

export default Footer;
