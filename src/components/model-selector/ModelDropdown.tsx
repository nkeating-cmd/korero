import React from "react";
import { useTranslation } from "react-i18next";
import type { ModelInfo } from "@/bindings";
import {
  getTranslatedModelName,
  getTranslatedModelDescription,
} from "../../lib/utils/modelTranslation";

interface ModelDropdownProps {
  models: ModelInfo[];
  currentModelId: string;
  onModelSelect: (modelId: string) => void;
}

const ModelDropdown: React.FC<ModelDropdownProps> = ({
  models,
  currentModelId,
  onModelSelect,
}) => {
  const { t } = useTranslation();
  const downloadedModels = models.filter((m) => m.is_downloaded);

  const handleModelClick = (modelId: string) => {
    onModelSelect(modelId);
  };

  return (
    <div className="kx-menu absolute bottom-full start-0 mb-2 w-64 max-h-[60vh] z-50">
      {downloadedModels.length > 0 ? (
        <div>
          {downloadedModels.map((model) => (
            <div
              key={model.id}
              onClick={() => handleModelClick(model.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  handleModelClick(model.id);
                }
              }}
              tabIndex={0}
              role="button"
              aria-current={currentModelId === model.id ? "true" : undefined}
              className="kx-menu-item py-2"
            >
              <div className="flex items-center justify-between w-full min-w-0">
                <div className="min-w-0">
                  <div className="text-[13.5px]">
                    {getTranslatedModelName(model, t)}
                    {model.is_custom && (
                      <span className="ms-1.5 kx-overline">
                        {t("modelSelector.custom")}
                      </span>
                    )}
                  </div>
                  <div className="kx-meta font-normal pe-4 whitespace-normal">
                    {getTranslatedModelDescription(model, t)}
                  </div>
                </div>
                {currentModelId === model.id && (
                  <div className="text-[12px] kx-accent-ink shrink-0">
                    {t("modelSelector.active")}
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="px-3 py-2 kx-meta">
          {t("modelSelector.noModelsAvailable")}
        </div>
      )}
    </div>
  );
};

export default ModelDropdown;
