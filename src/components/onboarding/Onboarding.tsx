/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Keyboard,
  Sparkles,
  NotebookPen,
  Users,
  Download,
  ArrowRight,
  type LucideIcon,
} from "lucide-react";
import type { ModelInfo } from "@/bindings";
import type { ModelCardStatus } from "./ModelCard";
import ModelCard from "./ModelCard";
import HandyTextLogo from "../icons/HandyTextLogo";
import { useModelStore } from "../../stores/modelStore";

interface OnboardingProps {
  onModelSelected: () => void;
}

/**
 * Kōrero (v1.14.6): onboarding is now two steps —
 *   1. A welcome screen explaining what the app does and how it works
 *      (dictation shortcuts, optional AI clean-up via Ollama or a cloud
 *      provider, Notes, Meetings) so a fresh install isn't dropped straight
 *      onto a bare model list.
 *   2. The model picker (cards now shrink-0 — previously, as flex children of
 *      a column scroll area they COMPRESSED to fit the window instead of
 *      scrolling, which is why a fresh install showed them bunched up).
 */

// LucideIcon (the library's own component type) — a hand-rolled ComponentType
// fails tsc on propTypes variance (Lucide's `size` is string | number).
const FEATURES: {
  icon: LucideIcon;
  title: string;
  body: string;
}[] = [
  {
    icon: Keyboard,
    title: "Dictate anywhere",
    body: "Hold Ctrl+Space, speak, release — the text lands wherever your cursor is, in any app. Double-tap to latch hands-free, and Ctrl+Shift+Enter works one-handed with just your right hand.",
  },
  {
    icon: Sparkles,
    title: "AI clean-up (optional)",
    body: "Ctrl+Shift+Space transcribes, then tidies the text with a prompt you choose. Runs fully locally if you install Ollama (ollama.com), or with a cloud provider — set it up any time under Post-processing.",
  },
  {
    icon: NotebookPen,
    title: "Notes",
    body: "A dictation canvas inside the app: ramble long-form, clean up the whole note with one click, and copy it out when you're done.",
  },
  {
    icon: Users,
    title: "Meetings",
    body: "Record both sides of a call — your mic and the system audio — with a live transcript while it runs. Use a headset for a clean You/Others split.",
  },
  {
    icon: Download,
    title: "First step: pick a transcription model",
    body: "On the next screen, choose a speech model. It downloads once and then everything runs offline — Parakeet V3 is the recommended pick for NZ English.",
  },
];

const Onboarding: React.FC<OnboardingProps> = ({ onModelSelected }) => {
  const { t } = useTranslation();
  const {
    models,
    downloadModel,
    selectModel,
    downloadingModels,
    verifyingModels,
    extractingModels,
    downloadProgress,
    downloadStats,
  } = useModelStore();
  const [step, setStep] = useState<"welcome" | "models">("welcome");
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);

  const isDownloading = selectedModelId !== null;

  // Watch for the selected model to finish downloading + verifying + extracting
  useEffect(() => {
    if (!selectedModelId) return;

    const model = models.find((m) => m.id === selectedModelId);
    const stillDownloading = selectedModelId in downloadingModels;
    const stillVerifying = selectedModelId in verifyingModels;
    const stillExtracting = selectedModelId in extractingModels;

    if (
      model?.is_downloaded &&
      !stillDownloading &&
      !stillVerifying &&
      !stillExtracting
    ) {
      // Model is ready — select it and transition
      selectModel(selectedModelId).then((success) => {
        if (success) {
          onModelSelected();
        } else {
          toast.error(t("onboarding.errors.selectModel"));
          setSelectedModelId(null);
        }
      });
    }
  }, [
    selectedModelId,
    models,
    downloadingModels,
    verifyingModels,
    extractingModels,
    selectModel,
    onModelSelected,
  ]);

  const handleDownloadModel = async (modelId: string) => {
    setSelectedModelId(modelId);

    // Error toast is handled centrally by the model-download-failed event listener
    // in modelStore — no toast here to avoid duplicates.
    const success = await downloadModel(modelId);
    if (!success) {
      setSelectedModelId(null);
    }
  };

  const getModelStatus = (modelId: string): ModelCardStatus => {
    if (modelId in extractingModels) return "extracting";
    if (modelId in verifyingModels) return "verifying";
    if (modelId in downloadingModels) return "downloading";
    return "downloadable";
  };

  const getModelDownloadProgress = (modelId: string): number | undefined => {
    return downloadProgress[modelId]?.percentage;
  };

  const getModelDownloadSpeed = (modelId: string): number | undefined => {
    return downloadStats[modelId]?.speed;
  };

  // ---- Step 1: welcome / what-it-does ------------------------------------
  if (step === "welcome") {
    return (
      <div className="h-screen w-screen flex flex-col items-center px-8 pt-10 pb-8 gap-6 overflow-y-auto">
        {/* Text wordmark, NOT the HandyTextLogo component: the established
            "drop HandyTextLogo render" patch substring-replaces every
            occurrence of that JSX line, so using the component here would be
            silently stripped at build time. */}
        <div className="flex flex-col items-center gap-2 shrink-0">
          <h1 className="text-4xl font-semibold tracking-tight text-text">
            Kōrero
          </h1>
          <p className="text-text-muted text-base max-w-md font-medium mx-auto text-center">
            Fast, private speech-to-text — everything runs on your machine.
          </p>
        </div>

        <div className="max-w-[640px] w-full flex flex-col gap-3 shrink-0">
          {FEATURES.map((f) => (
            <div
              key={f.title}
              className="glass-card p-4 flex items-start gap-3 shrink-0"
            >
              <f.icon size={20} className="text-aurora-cyan shrink-0 mt-0.5" />
              <div className="min-w-0">
                <h3 className="text-sm font-semibold text-text">{f.title}</h3>
                <p className="text-sm text-text-muted leading-relaxed mt-0.5">
                  {f.body}
                </p>
              </div>
            </div>
          ))}
        </div>

        <div className="flex flex-col items-center gap-2 shrink-0 pb-2">
          <button
            type="button"
            onClick={() => setStep("models")}
            className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-glass-accent-strong text-text font-medium text-sm hover:bg-white/15 transition-colors border border-white/10"
          >
            Choose your transcription model <ArrowRight size={16} />
          </button>
          <p className="text-xs text-text-subtle">
            Downloads once, then works fully offline.
          </p>
        </div>
      </div>
    );
  }

  // ---- Step 2: model picker -----------------------------------------------
  return (
    <div className="h-screen w-screen flex flex-col px-8 pt-10 pb-6 gap-6 inset-0">
      {/* Kōrero fork: tightened hero header — bigger wordmark, calmer subtitle */}
      <div className="flex flex-col items-center gap-3 shrink-0">
        {/* Korero: wordmark removed; the app icon carries brand recognition. */}
        <p className="text-text-muted text-base max-w-md font-medium mx-auto text-center">
          {t("onboarding.subtitle")}
        </p>
      </div>

      <div className="max-w-[640px] w-full mx-auto flex-1 flex flex-col min-h-0">
        {/* v1.14.6: every card wrapped shrink-0 — flex children of a column
            scroll area otherwise compress to fit instead of scrolling. */}
        <div className="flex flex-col gap-3 pb-6 overflow-y-auto pr-1">
          {models
            .filter((m: ModelInfo) => !m.is_downloaded)
            .filter((model: ModelInfo) => model.is_recommended)
            .map((model: ModelInfo) => (
              <div key={model.id} className="shrink-0">
                <ModelCard
                  model={model}
                  variant="featured"
                  status={getModelStatus(model.id)}
                  disabled={isDownloading}
                  onSelect={handleDownloadModel}
                  onDownload={handleDownloadModel}
                  downloadProgress={getModelDownloadProgress(model.id)}
                  downloadSpeed={getModelDownloadSpeed(model.id)}
                />
              </div>
            ))}

          {models
            .filter((m: ModelInfo) => !m.is_downloaded)
            .filter((model: ModelInfo) => !model.is_recommended)
            .sort(
              (a: ModelInfo, b: ModelInfo) =>
                Number(a.size_mb) - Number(b.size_mb),
            )
            .map((model: ModelInfo) => (
              <div key={model.id} className="shrink-0">
                <ModelCard
                  model={model}
                  status={getModelStatus(model.id)}
                  disabled={isDownloading}
                  onSelect={handleDownloadModel}
                  onDownload={handleDownloadModel}
                  downloadProgress={getModelDownloadProgress(model.id)}
                  downloadSpeed={getModelDownloadSpeed(model.id)}
                />
              </div>
            ))}
        </div>
      </div>
    </div>
  );
};

export default Onboarding;
