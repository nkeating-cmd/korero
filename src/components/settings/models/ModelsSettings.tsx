/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  ChevronDown,
  Globe,
  Mic2,
  Wand2,
  AudioLines,
  CheckCircle2,
  AlertTriangle,
  Loader2,
  FolderOpen,
} from "lucide-react";
import type { ModelCardStatus } from "@/components/onboarding";
import { ModelCard } from "@/components/onboarding";
import { useModelStore } from "@/stores/modelStore";
import { LANGUAGES } from "@/lib/constants/languages.ts";
import { commands, type ModelInfo } from "@/bindings";

// check if model supports a language based on its supported_languages list
const modelSupportsLanguage = (model: ModelInfo, langCode: string): boolean => {
  return model.supported_languages.includes(langCode);
};

export const ModelsSettings: React.FC = () => {
  const { t } = useTranslation();
  const [switchingModelId, setSwitchingModelId] = useState<string | null>(null);
  const [languageFilter, setLanguageFilter] = useState("all");
  const [languageDropdownOpen, setLanguageDropdownOpen] = useState(false);
  const [languageSearch, setLanguageSearch] = useState("");
  const languageDropdownRef = useRef<HTMLDivElement>(null);
  const languageSearchInputRef = useRef<HTMLInputElement>(null);
  const {
    models,
    currentModel,
    downloadingModels,
    downloadProgress,
    downloadStats,
    verifyingModels,
    extractingModels,
    loading,
    downloadModel,
    cancelDownload,
    selectModel,
    deleteModel,
  } = useModelStore();

  // v1.22.0: live status of the on-device TTS (audio-brief) engine, shown in the
  // Text-to-speech section so Models is the single place to check every model.
  const [ttsState, setTtsState] = useState<
    | { kind: "checking" }
    | { kind: "ready"; path: string }
    | { kind: "missing"; detail: string }
  >({ kind: "checking" });

  const checkTts = async () => {
    setTtsState({ kind: "checking" });
    try {
      const r = await commands.ttsEngineStatus();
      if (r.status === "ok") setTtsState({ kind: "ready", path: r.data });
      else setTtsState({ kind: "missing", detail: r.error });
    } catch (e) {
      setTtsState({ kind: "missing", detail: String(e) });
    }
  };

  useEffect(() => {
    checkTts();
  }, []);

  // click outside handler for language dropdown
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        languageDropdownRef.current &&
        !languageDropdownRef.current.contains(event.target as Node)
      ) {
        setLanguageDropdownOpen(false);
        setLanguageSearch("");
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // focus search input when dropdown opens
  useEffect(() => {
    if (languageDropdownOpen && languageSearchInputRef.current) {
      languageSearchInputRef.current.focus();
    }
  }, [languageDropdownOpen]);

  // filtered languages for dropdown (exclude "auto")
  const filteredLanguages = useMemo(() => {
    return LANGUAGES.filter(
      (lang) =>
        lang.value !== "auto" &&
        lang.label.toLowerCase().includes(languageSearch.toLowerCase()),
    );
  }, [languageSearch]);

  // Get selected language label
  const selectedLanguageLabel = useMemo(() => {
    if (languageFilter === "all") {
      return t("settings.models.filters.allLanguages");
    }
    return LANGUAGES.find((lang) => lang.value === languageFilter)?.label || "";
  }, [languageFilter, t]);

  const getModelStatus = (modelId: string): ModelCardStatus => {
    if (modelId in extractingModels) {
      return "extracting";
    }
    if (modelId in verifyingModels) {
      return "verifying";
    }
    if (modelId in downloadingModels) {
      return "downloading";
    }
    if (switchingModelId === modelId) {
      return "switching";
    }
    if (modelId === currentModel) {
      return "active";
    }
    const model = models.find((m: ModelInfo) => m.id === modelId);
    if (model?.is_downloaded) {
      return "available";
    }
    return "downloadable";
  };

  const getDownloadProgress = (modelId: string): number | undefined => {
    const progress = downloadProgress[modelId];
    return progress?.percentage;
  };

  const getDownloadSpeed = (modelId: string): number | undefined => {
    const stats = downloadStats[modelId];
    return stats?.speed;
  };

  const handleModelSelect = async (modelId: string) => {
    setSwitchingModelId(modelId);
    try {
      await selectModel(modelId);
    } finally {
      setSwitchingModelId(null);
    }
  };

  const handleModelDownload = async (modelId: string) => {
    await downloadModel(modelId);
  };

  const handleModelDelete = async (modelId: string) => {
    const model = models.find((m: ModelInfo) => m.id === modelId);
    const modelName = model?.name || modelId;
    const isActive = modelId === currentModel;

    const confirmed = await ask(
      isActive
        ? t("settings.models.deleteActiveConfirm", { modelName })
        : t("settings.models.deleteConfirm", { modelName }),
      {
        title: t("settings.models.deleteTitle"),
        kind: "warning",
      },
    );

    if (confirmed) {
      try {
        await deleteModel(modelId);
      } catch (err) {
        console.error(`Failed to delete model ${modelId}:`, err);
      }
    }
  };

  const handleModelCancel = async (modelId: string) => {
    try {
      await cancelDownload(modelId);
    } catch (err) {
      console.error(`Failed to cancel download for ${modelId}:`, err);
    }
  };

  // Filter models based on language filter
  const filteredModels = useMemo(() => {
    return models.filter((model: ModelInfo) => {
      if (languageFilter !== "all") {
        if (!modelSupportsLanguage(model, languageFilter)) return false;
      }
      return true;
    });
  }, [models, languageFilter]);

  // Split filtered models into downloaded (including custom) and available sections
  const { downloadedModels, availableModels } = useMemo(() => {
    const downloaded: ModelInfo[] = [];
    const available: ModelInfo[] = [];

    for (const model of filteredModels) {
      if (
        model.is_custom ||
        model.is_downloaded ||
        model.id in downloadingModels ||
        model.id in extractingModels
      ) {
        downloaded.push(model);
      } else {
        available.push(model);
      }
    }

    // Sort: active model first, then non-custom, then custom at the bottom
    downloaded.sort((a, b) => {
      if (a.id === currentModel) return -1;
      if (b.id === currentModel) return 1;
      if (a.is_custom !== b.is_custom) return a.is_custom ? 1 : -1;
      return 0;
    });

    return {
      downloadedModels: downloaded,
      availableModels: available,
    };
  }, [filteredModels, downloadingModels, extractingModels, currentModel]);

  if (loading) {
    return (
      <div className="max-w-3xl w-full mx-auto">
        <div className="flex items-center justify-center py-16">
          <div className="w-8 h-8 border-2 border-logo-primary border-t-transparent rounded-full animate-spin" />
        </div>
      </div>
    );
  }

  return (
    <div className="max-w-3xl w-full mx-auto space-y-4">
      <div className="mb-1">
        <h1 className="text-xl font-semibold mb-2">
          {t("settings.models.title")}
        </h1>
        <p className="text-sm text-text/60">
          Download and manage the on-device models Kōrero uses — speech-to-text,
          the audio-brief voice, and local post-processing — all in one place.
        </p>
      </div>

      {/* v1.22.0: Speech-to-Text section header above the existing manager. */}
      <div className="flex items-center gap-2 pt-2">
        <Mic2 className="h-4 w-4 text-aurora-cyan" />
        <h2 className="text-sm font-semibold text-text">Speech-to-text</h2>
      </div>

      {/* v1.22.0 (AC1): help pick the right model for language + speed/accuracy. */}
      <div className="rounded-xl border border-glass-border bg-glass-surface-thin p-3.5 text-sm leading-relaxed text-text-muted">
        <p className="mb-1.5 font-medium text-text">Choosing a model</p>
        <p>
          <span className="font-medium text-text">Parakeet V3</span> (the default)
          is the fastest, and among the most accurate for English and European
          languages — best for everyday dictation. For{" "}
          <span className="font-medium text-text">te reo Māori</span> or other
          languages, choose a <span className="font-medium text-text">Whisper</span>{" "}
          model (Large or Turbo): they cover 99 languages and apply your custom
          words at the moment of transcription, sharpening names and jargon.
          Larger models are more accurate but slower — pick the smallest that
          reads your speech correctly.
        </p>
      </div>
      {filteredModels.length > 0 ? (
        <div className="space-y-6">
          {/* Downloaded Models Section — header always visible so filter stays accessible */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h2 className="text-sm font-medium text-text/60">
                {t("settings.models.yourModels")}
              </h2>
              {/* Language filter dropdown */}
              <div className="relative" ref={languageDropdownRef}>
                <button
                  type="button"
                  onClick={() => setLanguageDropdownOpen(!languageDropdownOpen)}
                  className={`flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-lg transition-colors ${
                    languageFilter !== "all"
                      ? "bg-logo-primary/20 text-logo-primary"
                      : "bg-mid-gray/10 text-text/60 hover:bg-mid-gray/20"
                  }`}
                >
                  <Globe className="w-3.5 h-3.5" />
                  <span className="max-w-[120px] truncate">
                    {selectedLanguageLabel}
                  </span>
                  <ChevronDown
                    className={`w-3.5 h-3.5 transition-transform ${
                      languageDropdownOpen ? "rotate-180" : ""
                    }`}
                  />
                </button>

                {languageDropdownOpen && (
                  <div className="absolute top-full right-0 mt-1 w-56 bg-background border border-mid-gray/80 rounded-lg shadow-lg z-50 overflow-hidden">
                    <div className="p-2 border-b border-mid-gray/40">
                      <input
                        ref={languageSearchInputRef}
                        type="text"
                        value={languageSearch}
                        onChange={(e) => setLanguageSearch(e.target.value)}
                        onKeyDown={(e) => {
                          if (
                            e.key === "Enter" &&
                            filteredLanguages.length > 0
                          ) {
                            setLanguageFilter(filteredLanguages[0].value);
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          } else if (e.key === "Escape") {
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          }
                        }}
                        placeholder={t(
                          "settings.general.language.searchPlaceholder",
                        )}
                        className="w-full px-2 py-1 text-sm bg-mid-gray/10 border border-mid-gray/40 rounded-md focus:outline-none focus:ring-1 focus:ring-logo-primary"
                      />
                    </div>
                    <div className="max-h-48 overflow-y-auto">
                      <button
                        type="button"
                        onClick={() => {
                          setLanguageFilter("all");
                          setLanguageDropdownOpen(false);
                          setLanguageSearch("");
                        }}
                        className={`w-full px-3 py-1.5 text-sm text-left transition-colors ${
                          languageFilter === "all"
                            ? "bg-logo-primary/20 text-logo-primary font-semibold"
                            : "hover:bg-mid-gray/10"
                        }`}
                      >
                        {t("settings.models.filters.allLanguages")}
                      </button>
                      {filteredLanguages.map((lang) => (
                        <button
                          key={lang.value}
                          type="button"
                          onClick={() => {
                            setLanguageFilter(lang.value);
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          }}
                          className={`w-full px-3 py-1.5 text-sm text-left transition-colors ${
                            languageFilter === lang.value
                              ? "bg-logo-primary/20 text-logo-primary font-semibold"
                              : "hover:bg-mid-gray/10"
                          }`}
                        >
                          {lang.label}
                        </button>
                      ))}
                      {filteredLanguages.length === 0 && (
                        <div className="px-3 py-2 text-sm text-text/50 text-center">
                          {t("settings.general.language.noResults")}
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </div>
            {downloadedModels.map((model: ModelInfo) => (
              <ModelCard
                key={model.id}
                model={model}
                status={getModelStatus(model.id)}
                onSelect={handleModelSelect}
                onDownload={handleModelDownload}
                onDelete={handleModelDelete}
                onCancel={handleModelCancel}
                downloadProgress={getDownloadProgress(model.id)}
                downloadSpeed={getDownloadSpeed(model.id)}
                showRecommended={false}
              />
            ))}
          </div>

          {/* Available Models Section */}
          {availableModels.length > 0 && (
            <div className="space-y-3">
              <h2 className="text-sm font-medium text-text/60">
                {t("settings.models.availableModels")}
              </h2>
              {availableModels.map((model: ModelInfo) => (
                <ModelCard
                  key={model.id}
                  model={model}
                  status={getModelStatus(model.id)}
                  onSelect={handleModelSelect}
                  onDownload={handleModelDownload}
                  onDelete={handleModelDelete}
                  onCancel={handleModelCancel}
                  downloadProgress={getDownloadProgress(model.id)}
                  downloadSpeed={getDownloadSpeed(model.id)}
                  showRecommended={false}
                />
              ))}
            </div>
          )}
        </div>
      ) : (
        <div className="text-center py-8 text-text/50">
          {t("settings.models.noModelsMatch")}
        </div>
      )}

      {/* v1.22.0: Post-processing models — honest about cloud (API key) vs
          fully-local Ollama; the actual provider/model pickers live in Post
          Process, so this avoids a second source of truth. */}
      <section className="space-y-2 pt-2">
        <div className="flex items-center gap-2">
          <Wand2 className="h-4 w-4 text-aurora-purple" />
          <h2 className="text-sm font-semibold text-text">Post-processing</h2>
        </div>
        <div className="space-y-2 rounded-xl border border-glass-border bg-glass-surface-thin p-4 text-sm leading-relaxed text-text-muted">
          <p>
            Post-processing cleans up and reshapes your transcripts — into an
            email, a meeting note, and so on. It runs one of two ways:
          </p>
          <p>
            <span className="font-medium text-text">Cloud provider</span> — fast
            and nothing to download; you just paste an API key. Pick the provider
            and key in <span className="font-medium text-text">Post Process</span>.
          </p>
          <p>
            <span className="font-medium text-text">Fully local (Ollama)</span> —
            no API key, and nothing leaves your machine. Install Ollama, then pull
            a model from{" "}
            <span className="font-medium text-text">Post Process</span>, where the
            model picker and a one-click pull live.
          </p>
        </div>
      </section>

      {/* v1.22.0: Text-to-Speech (audio-brief voice) — live engine status via
          tts_engine_status, reusing the same resolver as the renderer. */}
      <section className="space-y-2 pt-2">
        <div className="flex items-center gap-2">
          <AudioLines className="h-4 w-4 text-aurora-cyan" />
          <h2 className="text-sm font-semibold text-text">
            Text-to-speech (audio brief)
          </h2>
        </div>
        <div className="space-y-3 rounded-xl border border-glass-border bg-glass-surface-thin p-4 text-sm leading-relaxed text-text-muted">
          <p>
            The audio-brief voice reads your notes aloud, fully on-device. It is
            an optional add-on, set up separately from the app.
          </p>

          {ttsState.kind === "checking" && (
            <div className="flex items-center gap-2 text-text-subtle">
              <Loader2 className="h-4 w-4 animate-spin" /> Checking for the voice
              engine…
            </div>
          )}

          {ttsState.kind === "ready" && (
            <div className="space-y-2">
              <div className="flex items-center gap-2 text-pill-positive">
                <CheckCircle2 className="h-4 w-4" />
                <span className="font-medium">Voice engine ready</span>
              </div>
              <p className="break-all font-mono text-xs text-text-subtle">
                {ttsState.path}
              </p>
              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={() => revealItemInDir(ttsState.path).catch(() => {})}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-glass-border px-2.5 py-1.5 text-xs text-text-muted transition-colors hover:text-aurora-cyan"
                >
                  <FolderOpen className="h-3.5 w-3.5" /> Open engine folder
                </button>
                <button
                  type="button"
                  onClick={checkTts}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-glass-border px-2.5 py-1.5 text-xs text-text-muted transition-colors hover:text-aurora-cyan"
                >
                  Re-check
                </button>
              </div>
            </div>
          )}

          {ttsState.kind === "missing" && (
            <div className="space-y-2">
              <div className="flex items-center gap-2 text-pill-warning">
                <AlertTriangle className="h-4 w-4" />
                <span className="font-medium">Voice engine not set up</span>
              </div>
              <p>
                Speech-to-text and post-processing work without it. To enable
                spoken audio briefs, install the local TTS engine, then set the{" "}
                <span className="font-mono text-xs">KORERO_TTS_DIR</span>{" "}
                environment variable to its folder (or place it under{" "}
                <span className="font-mono text-xs">
                  %APPDATA%\com.kyt.korero\tts
                </span>
                ).
              </p>
              <button
                type="button"
                onClick={checkTts}
                className="inline-flex items-center gap-1.5 rounded-lg border border-glass-border px-2.5 py-1.5 text-xs text-text-muted transition-colors hover:text-aurora-cyan"
              >
                Re-check
              </button>
            </div>
          )}
        </div>
      </section>
    </div>
  );
};
