import React from "react";
import { useTranslation } from "react-i18next";
import {
  Home,
  NotebookPen,
  UsersRound,
  Headphones,
  History,
  AudioLines,
  Boxes,
  Wand2,
  SlidersHorizontal,
  FlaskConical,
  LifeBuoy,
  PanelLeftClose,
  PanelLeftOpen,
  type LucideIcon,
} from "lucide-react";
import {
  GeneralSettings,
  AdvancedSettings,
  HistorySettings,
  DebugSettings,
  PostProcessingSettings,
  ModelsSettings,
} from "./settings";
import { NotesSettings } from "./settings/notes/NotesSettings";
import { MeetingsSettings } from "./settings/meetings/MeetingsSettings";
import { AudioBriefSettings } from "./settings/audiobrief/AudioBriefSettings";
import { TodayPage } from "./today/TodayPage";
import { HelpAndAbout } from "./settings/help/HelpAndAbout";
import { InUsePanel } from "./shell/InUsePanel";
import { useSettings } from "../hooks/useSettings";
import { useNav, type Section } from "../stores/navStore";
import { useRecorder } from "../stores/recorderStore";

/**
 * Kōrero 1.42 sidebar: grouped by what you are doing — capture, look back,
 * set up — instead of a flat wall of eleven items. Collapses to an icon rail
 * (remembered), and collapses on its own when the window is narrow.
 */

export type SidebarSection = Section;

interface SectionConfig {
  labelKey: string;
  label: string;
  icon: LucideIcon;
  component: React.ComponentType;
  enabled: (settings: { debug_mode?: boolean } | null | undefined) => boolean;
}

export const SECTIONS_CONFIG: Record<Section, SectionConfig> = {
  home: { labelKey: "sidebar.home", label: "Today", icon: Home, component: TodayPage, enabled: () => true },
  notes: { labelKey: "sidebar.notes", label: "Notes", icon: NotebookPen, component: NotesSettings, enabled: () => true },
  meetings: { labelKey: "sidebar.meetings", label: "Meetings", icon: UsersRound, component: MeetingsSettings, enabled: () => true },
  audiobrief: { labelKey: "sidebar.audioBrief", label: "Audio briefs", icon: Headphones, component: AudioBriefSettings, enabled: () => true },
  history: { labelKey: "sidebar.history", label: "Dictation history", icon: History, component: HistorySettings, enabled: () => true },
  general: { labelKey: "sidebar.general", label: "Dictation & sound", icon: AudioLines, component: GeneralSettings, enabled: () => true },
  models: { labelKey: "sidebar.models", label: "Speech models", icon: Boxes, component: ModelsSettings, enabled: () => true },
  postprocessing: { labelKey: "sidebar.postProcessing", label: "AI clean-up & notes", icon: Wand2, component: PostProcessingSettings, enabled: () => true },
  advanced: { labelKey: "sidebar.advanced", label: "Advanced", icon: SlidersHorizontal, component: AdvancedSettings, enabled: () => true },
  debug: { labelKey: "sidebar.debug", label: "Debug", icon: FlaskConical, component: DebugSettings, enabled: (s) => s?.debug_mode ?? false },
  help: { labelKey: "sidebar.help", label: "Help & about", icon: LifeBuoy, component: HelpAndAbout, enabled: () => true },
};

const GROUPS: { title: string | null; items: Section[] }[] = [
  { title: null, items: ["home"] },
  { title: "Capture", items: ["notes", "meetings", "audiobrief"] },
  { title: "Library", items: ["history"] },
  { title: "Set up", items: ["general", "models", "postprocessing", "advanced", "debug"] },
];

/** Window width, for the narrow-window auto-collapse. */
export const useWindowWidth = (): number => {
  const [w, setW] = React.useState(() => window.innerWidth);
  React.useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
};

export const AUTO_COLLAPSE_BELOW = 980;

export const Sidebar: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { settings } = useSettings();
  const section = useNav((s) => s.section);
  const go = useNav((s) => s.go);
  const userCollapsed = useNav((s) => s.sidebarCollapsed);
  const toggle = useNav((s) => s.toggleSidebar);
  const recording = useRecorder((s) => s.recording);
  const width = useWindowWidth();
  const narrow = width < AUTO_COLLAPSE_BELOW;
  const collapsed = userCollapsed || narrow;

  const item = (id: Section) => {
    const cfg = SECTIONS_CONFIG[id];
    if (!cfg.enabled(settings)) return null;
    const Icon = cfg.icon;
    // 1.42 renamed several sections in plain English; other languages keep
    // their translated names until those catch up.
    const label = (i18n.language ?? "en").startsWith("en")
      ? cfg.label
      : t(cfg.labelKey, { defaultValue: cfg.label });
    const isActive = section === id;
    return (
      <button
        key={id}
        type="button"
        aria-current={isActive ? "page" : undefined}
        aria-label={collapsed ? label : undefined}
        title={collapsed ? label : undefined}
        onClick={() => go(id)}
        className="kx-nav-item relative"
      >
        <Icon size={17} />
        {!collapsed && <span className="truncate">{label}</span>}
        {id === "meetings" && recording && (
          <span
            className={`kx-dot kx-dot-alert ${collapsed ? "absolute top-2 right-2" : "ml-auto"}`}
            aria-label="Recording"
          />
        )}
      </button>
    );
  };

  return (
    <nav
      aria-label="Main"
      className={`${collapsed ? "kx-nav-collapsed w-[64px]" : "w-[228px]"} shrink-0 h-full flex flex-col px-2.5 py-3 gap-px border-e border-[var(--kx-hairline-soft)] bg-[var(--kx-sidebar)] transition-[width] duration-150`}
    >
      <div className="flex-1 min-h-0 overflow-y-auto overflow-x-hidden flex flex-col gap-px">
        {GROUPS.map((g, gi) => {
          const items = g.items.map(item).filter(Boolean);
          if (items.length === 0) return null;
          return (
            <div key={gi} className="flex flex-col gap-px">
              {g.title &&
                (collapsed ? (
                  <div className="my-2 mx-2 border-t border-[var(--kx-hairline-soft)]" aria-hidden="true" />
                ) : (
                  <div className="kx-overline px-2.5 pt-4 pb-1.5">{g.title}</div>
                ))}
              {items}
            </div>
          );
        })}
      </div>

      <div className="flex flex-col gap-px pt-2">
        {item("help")}
        <div className="mt-1.5 pt-1.5 border-t border-[var(--kx-hairline-soft)]">
          <div className={`flex items-center ${collapsed ? "flex-col gap-1 pb-1" : "justify-between pl-2.5 pb-0.5"}`}>
            {!collapsed && <span className="kx-overline">In use</span>}
            {!narrow && (
              <button
                type="button"
                onClick={toggle}
                aria-label={collapsed ? "Expand the sidebar" : "Collapse the sidebar"}
                title={collapsed ? "Expand the sidebar" : "Collapse the sidebar"}
                className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm shrink-0"
              >
                {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
              </button>
            )}
          </div>
          <InUsePanel collapsed={collapsed} />
        </div>
      </div>
    </nav>
  );
};
