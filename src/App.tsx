import { useEffect, useState, useRef } from "react";
import { toast, Toaster } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { check as checkForUpdate } from "@tauri-apps/plugin-updater";
import { platform } from "@tauri-apps/plugin-os";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ErrorBoundary } from "./components/ErrorBoundary";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { ModelStateEvent, RecordingErrorEvent } from "./lib/types/events";
import "./App.css";
// Kōrero 1.42: the one design system. After App.css so it wins.
import "./styles/kx.css";
import AccessibilityPermissions from "./components/AccessibilityPermissions";
import Onboarding, { AccessibilityOnboarding } from "./components/onboarding";
import { Sidebar, SECTIONS_CONFIG } from "./components/Sidebar";
import { TopBar } from "./components/shell/TopBar";
import { ActivityPanel } from "./components/shell/ActivityPanel";
import { CommandPalette } from "./components/shell/CommandPalette";
import { useNav, type Section } from "./stores/navStore";
import { useRecorder } from "./stores/recorderStore";
import { useAppStatus } from "./stores/appStatusStore";
import { logActivity } from "./stores/activityStore";
import { useSettings } from "./hooks/useSettings";
import { useSettingsStore } from "./stores/settingsStore";
import { commands } from "@/bindings";
import { getLanguageDirection, initializeRTL } from "@/lib/utils/rtl";

const RELEASES_PAGE = "https://github.com/nkeating-cmd/korero/releases/latest";

type OnboardingStep = "accessibility" | "model" | "done";

// Kōrero 1.42: pages that draw their own frame; the rest get the shared one
// (title, one-line intro, the standard page width).
const OWN_FRAME: Section[] = ["home", "notes", "meetings"];
// Pages that bring their own title and intro but sit in the shared frame.
const OWN_HEADER: Section[] = ["models", "help", "audiobrief"];
const PAGE_INTRO: Partial<Record<Section, { title: string; intro: string }>> = {
  general: {
    title: "Dictation & sound",
    intro: "Shortcuts, microphone and sounds for dictating in any app.",
  },
  history: {
    title: "Dictation history",
    intro: "Everything you have dictated, newest first, with the audio.",
  },
  postprocessing: {
    title: "AI clean-up & notes",
    intro: "The model that tidies dictations and writes meeting notes, and the prompts it follows.",
  },
  advanced: {
    title: "Advanced",
    intro: "Pasting, start-up, history and other fine-tuning.",
  },
  debug: { title: "Debug", intro: "Diagnostics for troubleshooting." },
};

const renderSection = (section: Section) => {
  const cfg = SECTIONS_CONFIG[section] ?? SECTIONS_CONFIG.home;
  const Page = cfg.component;
  if (OWN_FRAME.includes(section)) return <Page />;
  if (OWN_HEADER.includes(section)) {
    return (
      <div className="kx-page">
        <Page />
      </div>
    );
  }
  const intro = PAGE_INTRO[section];
  return (
    <div className="kx-page">
      {intro && (
        <header className="max-w-3xl mx-auto kx-page-header">
          <h1 className="kx-title">{intro.title}</h1>
          <p className="kx-meta">{intro.intro}</p>
        </header>
      )}
      <Page />
    </div>
  );
};

function App() {
  const { t, i18n } = useTranslation();
  const [onboardingStep, setOnboardingStep] = useState<OnboardingStep | null>(
    null,
  );
  // Track if this is a returning user who just needs to grant permissions
  // (vs a new user who needs full onboarding including model selection)
  const [isReturningUser, setIsReturningUser] = useState(false);
  const currentSection = useNav((s) => s.section);
  const { settings, updateSetting } = useSettings();
  const direction = getLanguageDirection(i18n.language);
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const hasCompletedPostOnboardingInit = useRef(false);

  // Kōrero (v1.16.0): update notification — Rust checks the fork's GitHub
  // releases once at startup (8 s delayed, silent on failure) and emits this
  // when a newer version exists.
  // v1.18.0: the toast action now installs in place via the updater plugin
  // (signature-verified, fork-repo endpoint only) and restarts. If the
  // install path fails for any reason — portable build, blocked installer,
  // signature mismatch — fall back to opening the release page.
  useEffect(() => {
    const un = listen<{ version: string; url: string }>(
      "korero://update-available",
      (e) => {
        useAppStatus.getState().setUpdateAvailable(e.payload);
        toast.message(`Kōrero v${e.payload.version} is available`, {
          duration: 15000,
          action: {
            label: "Install now",
            onClick: () => {
              toast.promise(
                commands.installUpdate().then((r) => {
                  if (r.status === "error") throw new Error(r.error);
                }),
                {
                  loading: "Downloading update…",
                  success: "Update installed — restarting…",
                  error: () => {
                    openUrl(e.payload.url).catch(() => {});
                    return "Install failed — opening the release page instead.";
                  },
                },
              );
            },
          },
        });
      },
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Kōrero 1.42: the dictation pill's Fix button asks for a settings page.
  useEffect(() => {
    const un = listen<string>("korero://open-section", (e) => {
      const section = e.payload as Section;
      if (section in SECTIONS_CONFIG) useNav.getState().go(section);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Kōrero 1.42: "Check for updates" (tray menu and Ctrl K) emits this. The
  // listener used to live in the footer's UpdateChecker, which 1.42 retired
  // with the footer, so it is handled here and always mounted.
  useEffect(() => {
    const un = listen("check-for-updates", async () => {
      const id = toast.loading("Checking for updates…");
      try {
        const update = await checkForUpdate();
        if (!update) {
          toast.success("Kōrero is up to date.", { id });
          return;
        }
        const payload = { version: update.version, url: RELEASES_PAGE };
        useAppStatus.getState().setUpdateAvailable(payload);
        toast.message(`Kōrero v${update.version} is available`, {
          id,
          duration: 15000,
          action: {
            label: "Install now",
            onClick: () => {
              toast.promise(
                commands.installUpdate().then((r) => {
                  if (r.status === "error") throw new Error(r.error);
                }),
                {
                  loading: "Downloading update…",
                  success: "Update installed — restarting…",
                  error: () => {
                    openUrl(RELEASES_PAGE).catch(() => {});
                    return "Install failed — opening the release page instead.";
                  },
                },
              );
            },
          },
        });
      } catch (e) {
        toast.error(`Couldn't check for updates: ${String(e)}`, { id });
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    checkOnboardingStatus();
  }, []);

  // Initialize RTL direction when language changes
  useEffect(() => {
    initializeRTL(i18n.language);
  }, [i18n.language]);

  // Kōrero (v1.25.0, upstream cherry-pick — Handy #1665): only one audio
  // player at a time. History rows, meeting audio, and audio-brief previews
  // each render their own <audio>; without this they play simultaneously.
  // 'play' doesn't bubble, so listen in the capture phase at document level.
  useEffect(() => {
    const onPlay = (e: Event) => {
      const started = e.target;
      if (!(started instanceof HTMLAudioElement)) return;
      document.querySelectorAll("audio").forEach((a) => {
        if (a !== started && !a.paused) a.pause();
      });
    };
    document.addEventListener("play", onPlay, true);
    return () => document.removeEventListener("play", onPlay, true);
  }, []);

  // Kōrero (2026-05-17 PM, T2.4a — inlined from apply-patches.ps1):
  // Surface OS-keychain write failures so the user knows their API key didn't
  // persist. Without this the failure is silent and manifests later as
  // "the app forgot my key", with the actual cause buried in handy.log.
  // Rust emits `korero://keychain-error` with { failed_providers, phase }
  // when persist_to_keyring or migrate_plaintext_to_keyring partially fails.
  useEffect(() => {
    const unlistenPromise = listen<{ failed_providers: string[]; phase: string }>(
      "korero://keychain-error",
      (event) => {
        const providers = event.payload.failed_providers.join(", ");
        const action = event.payload.phase === "migrate" ? "migrate" : "save";
        const plural = event.payload.failed_providers.length === 1 ? "" : "s";
        toast.error(
          `Couldn't ${action} API key${plural} for: ${providers}. OS keychain may be locked or unavailable.`,
          { duration: 10000 }
        );
      }
    );
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // Initialize Enigo, shortcuts, and refresh audio devices when main app loads
  useEffect(() => {
    if (onboardingStep === "done" && !hasCompletedPostOnboardingInit.current) {
      hasCompletedPostOnboardingInit.current = true;
      Promise.all([
        commands.initializeEnigo(),
        commands.initializeShortcuts(),
      ]).catch((e) => {
        console.warn("Failed to initialize:", e);
      });
      refreshAudioDevices();
      refreshOutputDevices();
      // 1.42: a meeting still recording (the window was reloaded) is picked
      // up by the recorder, so every page shows it, not just Meetings.
      void useRecorder.getState().restore();
    }
  }, [onboardingStep, refreshAudioDevices, refreshOutputDevices]);

  // 1.42: Ctrl K opens search from anywhere in the app.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        const nav = useNav.getState();
        nav.setPaletteOpen(!nav.paletteOpen);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  // Handle keyboard shortcuts for debug mode toggle
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      // Check for Ctrl+Shift+D (Windows/Linux) or Cmd+Shift+D (macOS)
      const isDebugShortcut =
        event.shiftKey &&
        event.key.toLowerCase() === "d" &&
        (event.ctrlKey || event.metaKey);

      if (isDebugShortcut) {
        event.preventDefault();
        const currentDebugMode = settings?.debug_mode ?? false;
        updateSetting("debug_mode", !currentDebugMode);
      }
    };

    // Add event listener when component mounts
    document.addEventListener("keydown", handleKeyDown);

    // Cleanup event listener when component unmounts
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [settings?.debug_mode, updateSetting]);

  // Listen for recording errors from the backend and show a toast
  useEffect(() => {
    const unlisten = listen<RecordingErrorEvent>("recording-error", (event) => {
      const { error_type, detail } = event.payload;

      if (error_type === "microphone_permission_denied") {
        const currentPlatform = platform();
        const platformKey = `errors.micPermissionDenied.${currentPlatform}`;
        const description = t(platformKey, {
          defaultValue: t("errors.micPermissionDenied.generic"),
        });
        toast.error(t("errors.micPermissionDeniedTitle"), { description });
      } else if (error_type === "no_input_device") {
        toast.error(t("errors.noInputDeviceTitle"), {
          description: t("errors.noInputDevice"),
        });
        logActivity({
          status: "failed",
          title: "Dictation could not start: no microphone",
          target: { section: "general" },
        });
      } else {
        toast.error(
          t("errors.recordingFailed", { error: detail ?? "Unknown error" }),
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for paste failures and show a toast.
  // The technical error detail is logged to handy.log on the Rust side
  // (see actions.rs `error!("Failed to paste transcription: ...")`),
  // so we show a localized, user-friendly message here instead of the raw error.
  useEffect(() => {
    // Kōrero 1.42: the payload says whether Rust put the text on the
    // clipboard instead (older builds sent nothing, which reads as false).
    const unlisten = listen<boolean | null>("paste-error", (e) => {
      const copied = e.payload === true;
      toast.error(t("errors.pasteFailedTitle"), {
        description: copied
          ? "It's on the clipboard instead — press Ctrl V where you want it."
          : t("errors.pasteFailed"),
      });
      logActivity({
        status: "attention",
        title: "A dictation could not be pasted",
        detail: copied
          ? "It was copied to the clipboard instead, and it is in Dictation history."
          : "It is in Dictation history.",
        target: { section: "history" },
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for model loading failures and show a toast
  useEffect(() => {
    const unlisten = listen<ModelStateEvent>("model-state-changed", (event) => {
      if (event.payload.event_type === "loading_failed") {
        toast.error(
          t("errors.modelLoadFailed", {
            model:
              event.payload.model_name || t("errors.modelLoadFailedUnknown"),
          }),
          {
            description: event.payload.error,
          },
        );
      } else if (
        event.payload.event_type === "unloaded" &&
        event.payload.error
      ) {
        // Kōrero (v1.22.0, ST2): a rare transcription-engine panic unloads the
        // model (it auto-reloads on the next attempt). Surface it instead of a
        // silent failure so the user knows to try that dictation again.
        toast.error("Transcription engine hit an error and reloaded.", {
          description: "Please try that dictation again.",
        });
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  const revealMainWindowForPermissions = async () => {
    try {
      await commands.showMainWindowCommand();
    } catch (e) {
      console.warn("Failed to show main window for permission onboarding:", e);
    }
  };

  const checkOnboardingStatus = async () => {
    try {
      // Check if they have any models available
      const result = await commands.hasAnyModelsAvailable();
      const hasModels = result.status === "ok" && result.data;
      const currentPlatform = platform();

      if (hasModels) {
        // Returning user - check if they need to grant permissions first
        setIsReturningUser(true);

        if (currentPlatform === "macos") {
          try {
            const [hasAccessibility, hasMicrophone] = await Promise.all([
              checkAccessibilityPermission(),
              checkMicrophonePermission(),
            ]);
            if (!hasAccessibility || !hasMicrophone) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check macOS permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        if (currentPlatform === "windows") {
          try {
            const microphoneStatus =
              await commands.getWindowsMicrophonePermissionStatus();
            if (
              microphoneStatus.supported &&
              microphoneStatus.overall_access === "denied"
            ) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check Windows microphone permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        setOnboardingStep("done");
      } else {
        // New user - start full onboarding
        setIsReturningUser(false);
        setOnboardingStep("accessibility");
      }
    } catch (error) {
      console.error("Failed to check onboarding status:", error);
      setOnboardingStep("accessibility");
    }
  };

  const handleAccessibilityComplete = () => {
    // Returning users already have models, skip to main app
    // New users need to select a model
    setOnboardingStep(isReturningUser ? "done" : "model");
  };

  const handleModelSelected = () => {
    // Transition to main app - user has started a download
    setOnboardingStep("done");
  };

  // Still checking onboarding status
  if (onboardingStep === null) {
    return null;
  }

  if (onboardingStep === "accessibility") {
    return <AccessibilityOnboarding onComplete={handleAccessibilityComplete} />;
  }

  if (onboardingStep === "model") {
    return <Onboarding onModelSelected={handleModelSelected} />;
  }

  const fullBleed = currentSection === "meetings";

  return (
    <div
      dir={direction}
      className="h-screen flex flex-col select-none cursor-default bg-[var(--kx-ground)]"
    >
      <Toaster
        theme="dark"
        toastOptions={{
          unstyled: true,
          classNames: {
            toast: "glass-card-thick flex items-center gap-3 text-sm text-text px-4 py-3 w-[356px]",
            title: "font-semibold text-text",
            description: "text-[var(--kx-ink-2)]",
            actionButton: "kx-btn kx-btn-secondary kx-btn-sm",
            cancelButton: "kx-btn kx-btn-ghost kx-btn-sm",
          },
        }}
      />
      <div className="flex-1 flex overflow-hidden">
        <Sidebar />
        <div className="flex-1 min-w-0 flex flex-col">
          <TopBar />
          <div className="flex-1 min-h-0 flex">
            <main
              className={`flex-1 min-w-0 min-h-0 ${fullBleed ? "overflow-hidden flex flex-col" : "overflow-y-auto"}`}
            >
              <AccessibilityPermissions />
              {/* Keyed so switching sections replays the page transition. */}
              <div
                key={currentSection}
                className={`korero-page kx-cq ${fullBleed ? "flex-1 min-h-0 flex flex-col" : ""}`}
              >
                {renderSection(currentSection)}
              </div>
            </main>
            <ActivityPanel />
          </div>
        </div>
      </div>
      <CommandPalette />
    </div>
  );
}

// Kōrero (v1.7.0, B4): wrap in ErrorBoundary so render crashes show a fallback
// rather than a blank settings window. The boundary is outside App so it
// catches errors thrown by any child, including hooks inside App itself.
function AppRoot() {
  return (
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  );
}

export default AppRoot;
