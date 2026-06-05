import React from "react";
import { useTranslation } from "react-i18next";
import {
  AudioWaveform,
  Cog,
  FlaskConical,
  History,
  Info,
  Sparkles,
  Cpu,
  LifeBuoy,
  NotebookPen,
  Home,
  Users,
} from "lucide-react";
// Korero fork: removed upstream HandyHand glyph + wordmark from the sidebar
// header. The app icon + window title bar carry brand recognition; the
// in-app wordmark just cost vertical space.

import {
  GeneralSettings,
  AdvancedSettings,
  HistorySettings,
  DebugSettings,
  AboutSettings,
  PostProcessingSettings,
  ModelsSettings,
} from "./settings";
// Kōrero fork: Help & Guide page imported directly from the overlay (not via the
// upstream settings barrel) so adding it needs no patch to settings/index.ts.
import { HelpSettings } from "./settings/help/HelpSettings";
import { NotesSettings } from "./settings/notes/NotesSettings";
import { HomeDashboard } from "./settings/home/HomeDashboard";
import { MeetingsSettings } from "./settings/meetings/MeetingsSettings";

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

interface IconProps {
  width?: number | string;
  height?: number | string;
  size?: number | string;
  className?: string;
  [key: string]: any;
}

interface SectionConfig {
  labelKey: string;
  icon: React.ComponentType<IconProps>;
  component: React.ComponentType;
  enabled: (settings: any) => boolean;
  // Kōrero: optional literal fallback used as the i18n defaultValue, so a
  // section can ship without adding a key to every locale file.
  label?: string;
}

export const SECTIONS_CONFIG = {
  home: { labelKey: "sidebar.home", label: "Home", icon: Home, component: HomeDashboard, enabled: () => true },
  general: { labelKey: "sidebar.general", icon: AudioWaveform, component: GeneralSettings, enabled: () => true },
  models: { labelKey: "sidebar.models", icon: Cpu, component: ModelsSettings, enabled: () => true },
  advanced: { labelKey: "sidebar.advanced", icon: Cog, component: AdvancedSettings, enabled: () => true },
  history: { labelKey: "sidebar.history", icon: History, component: HistorySettings, enabled: () => true },
  notes: { labelKey: "sidebar.notes", label: "Notes", icon: NotebookPen, component: NotesSettings, enabled: () => true },
  meetings: { labelKey: "sidebar.meetings", label: "Meetings", icon: Users, component: MeetingsSettings, enabled: () => true },
  // Kōrero (v1.14.6): always visible — hiding this tab while post-processing
  // was off made the feature undiscoverable (its enable toggle lives INSIDE
  // this page).
  postprocessing: { labelKey: "sidebar.postProcessing", icon: Sparkles, component: PostProcessingSettings, enabled: () => true },
  debug: { labelKey: "sidebar.debug", icon: FlaskConical, component: DebugSettings, enabled: (s) => s?.debug_mode ?? false },
  help: { labelKey: "sidebar.help", label: "Help", icon: LifeBuoy, component: HelpSettings, enabled: () => true },
  about: { labelKey: "sidebar.about", icon: Info, component: AboutSettings, enabled: () => true },
} as const satisfies Record<string, SectionConfig>;

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}

import { useSettings } from "../hooks/useSettings";

/**
 * Korero fork sidebar — aurora active state (was yellow).
 * Active nav item uses cyan #5DD8FF tint over glass + soft cyan shadow halo.
 */
export const Sidebar: React.FC<SidebarProps> = ({ activeSection, onSectionChange }) => {
  const { t } = useTranslation();
  const { settings } = useSettings();

  const availableSections = Object.entries(SECTIONS_CONFIG)
    .filter(([_, config]) => config.enabled(settings))
    .map(([id, config]) => ({ id: id as SidebarSection, ...config }));

  return (
    <div
      className="flex flex-col w-44 h-full items-center px-3 py-2 border-e border-glass-border"
      style={{
        backgroundColor: "rgba(255, 255, 255, 0.04)",
        backdropFilter: "blur(30px) saturate(180%)",
        WebkitBackdropFilter: "blur(30px) saturate(180%)",
        boxShadow: "inset -1px 0 0 0 rgba(255, 255, 255, 0.06)",
      }}
    >
      {/* Kōrero: wordmark removed from sidebar header (2026-05-17).
          The app icon in the taskbar + window title bar carry brand
          recognition already; an in-app wordmark just costs vertical space. */}
      <div className="flex flex-col w-full items-stretch gap-1 pt-3">
        {availableSections.map((section) => {
          const Icon = section.icon;
          const isActive = activeSection === section.id;

          return (
            <button
              key={section.id}
              type="button"
              onClick={() => onSectionChange(section.id)}
              className={`flex gap-2.5 items-center px-3 py-2 w-full rounded-lg text-left transition-all duration-200 ${
                isActive
                  ? "text-text font-semibold"
                  : "text-text-muted hover:bg-white/8 hover:text-text"
              }`}
              style={
                isActive
                  ? {
                      // Kōrero (v1.12.0): brand cyan sourced from the --color-aurora-cyan
                      // token via color-mix, so the active nav state tracks the design
                      // system instead of a hardcoded literal.
                      backgroundColor:
                        "color-mix(in srgb, var(--color-aurora-cyan) 18%, transparent)",
                      boxShadow:
                        "inset 0 1px 0 0 rgba(255, 255, 255, 0.30), 0 4px 14px 0 color-mix(in srgb, var(--color-aurora-cyan) 28%, transparent), 0 0 0 1px color-mix(in srgb, var(--color-aurora-cyan) 35%, transparent)",
                    }
                  : undefined
              }
            >
              {(() => {
                const label = t(section.labelKey, {
                  defaultValue: "label" in section ? section.label : undefined,
                });
                return (
                  <>
                    <Icon width={18} height={18} className="shrink-0" />
                    <span className="text-sm truncate" title={label}>
                      {label}
                    </span>
                  </>
                );
              })()}
            </button>
          );
        })}
      </div>
    </div>
  );
};
