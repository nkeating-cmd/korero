// Korero overlay -- PostProcessingSettings.tsx
// Changes vs upstream (cjpais/Handy):
//   v1.2.0: import PostProcessingToggle; add toggle SettingsGroup at the top
//           of PostProcessingSettings.
//   v1.3.0: import OllamaPullButton; add "Local" badge next to the provider
//           dropdown when is_local_provider; add OllamaPullButton below the
//           model select when is_local_provider.
//
// NOTE: this file is in the overlay (Handy-changes) NOT patched by
// apply-patches.ps1.  The three earlier patches for this file were removed
// in v1.3.1 after the String.Replace all-occurrences bug caused 4x
// duplication of the OllamaPullButton block.

import React, { useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { RefreshCcw } from "lucide-react";
import { commands } from "@/bindings";

import { Alert } from "../../ui/Alert";
import {
  Dropdown,
  SettingContainer,
  SettingsGroup,
  Textarea,
} from "@/components/ui";
import { Button } from "../../ui/Button";
import { ResetButton } from "../../ui/ResetButton";
import { Input } from "../../ui/Input";

import { ProviderSelect } from "../PostProcessingSettingsApi/ProviderSelect";
import { BaseUrlField } from "../PostProcessingSettingsApi/BaseUrlField";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";
import { ModelSelect } from "../PostProcessingSettingsApi/ModelSelect";
import { usePostProcessProviderState } from "../PostProcessingSettingsApi/usePostProcessProviderState";
import { ShortcutInput } from "../ShortcutInput";
import { CorrectionsManager } from "../../ui/Corrections";
import { useSettings } from "../../../hooks/useSettings";
import { PostProcessingToggle } from "../PostProcessingToggle";
import { OllamaPullButton } from "../PostProcessingSettingsApi/OllamaPullButton";

const PostProcessingSettingsApiComponent: React.FC = () => {
  const { t } = useTranslation();
  const state = usePostProcessProviderState();

  return (
    <>
      <SettingContainer
        title={t("settings.postProcessing.api.provider.title")}
        description={t("settings.postProcessing.api.provider.description")}
        descriptionMode="tooltip"
        layout="horizontal"
        grouped={true}
      >
        {/* Korero (v1.3.0): Local badge shown next to provider dropdown when
            is_local_provider is true (e.g. Ollama).  Uses emerald styling to
            distinguish from cloud providers at a glance. */}
        <div className="flex items-center gap-2">
          <ProviderSelect
            options={state.providerOptions}
            value={state.selectedProviderId}
            onChange={state.handleProviderSelect}
          />
          {state.selectedProvider?.is_local_provider && (
            <span className="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-emerald-500/20 text-emerald-400 border border-emerald-500/30">
              Local
            </span>
          )}
        </div>
      </SettingContainer>

      {state.isAppleProvider ? (
        state.appleIntelligenceUnavailable ? (
          <Alert variant="error" contained>
            {t("settings.postProcessing.api.appleIntelligence.unavailable")}
          </Alert>
        ) : null
      ) : (
        <>
          {/* Korero (v1.3.1 fix): show BaseUrlField whenever allow_base_url_edit
              is true, not just for id==="custom". Ollama also has
              allow_base_url_edit:true; the old condition hid its URL field. */}
          {state.selectedProvider?.allow_base_url_edit && (
            <SettingContainer
              title={t("settings.postProcessing.api.baseUrl.title")}
              description={t("settings.postProcessing.api.baseUrl.description")}
              descriptionMode="tooltip"
              layout="horizontal"
              grouped={true}
            >
              <div className="flex items-center gap-2">
                <BaseUrlField
                  value={state.baseUrl}
                  onBlur={state.handleBaseUrlChange}
                  placeholder={t(
                    "settings.postProcessing.api.baseUrl.placeholder",
                  )}
                  disabled={state.isBaseUrlUpdating}
                  className="min-w-[380px]"
                />
              </div>
            </SettingContainer>
          )}

          <SettingContainer
            title={t("settings.postProcessing.api.apiKey.title")}
            description={t("settings.postProcessing.api.apiKey.description")}
            descriptionMode="tooltip"
            layout="horizontal"
            grouped={true}
          >
            <div className="flex items-center gap-2">
              <ApiKeyField
                value={state.apiKey}
                onBlur={state.handleApiKeyChange}
                placeholder={t(
                  "settings.postProcessing.api.apiKey.placeholder",
                )}
                disabled={state.isApiKeyUpdating}
                className="min-w-[320px]"
              />
            </div>
          </SettingContainer>
        </>
      )}

      {!state.isAppleProvider && (
        <SettingContainer
          title={t("settings.postProcessing.api.model.title")}
          description={
            state.isCustomProvider
              ? t("settings.postProcessing.api.model.descriptionCustom")
              : t("settings.postProcessing.api.model.descriptionDefault")
          }
          descriptionMode="tooltip"
          layout="stacked"
          grouped={true}
        >
          <div className="flex items-center gap-2">
            <ModelSelect
              value={state.model}
              options={state.modelOptions}
              disabled={state.isModelUpdating}
              isLoading={state.isFetchingModels}
              placeholder={
                state.modelOptions.length > 0
                  ? t(
                      "settings.postProcessing.api.model.placeholderWithOptions",
                    )
                  : t("settings.postProcessing.api.model.placeholderNoOptions")
              }
              onSelect={state.handleModelSelect}
              onCreate={state.handleModelCreate}
              onBlur={() => {}}
              className="flex-1 min-w-[380px]"
            />
            <ResetButton
              onClick={state.handleRefreshModels}
              disabled={state.isFetchingModels}
              ariaLabel={t("settings.postProcessing.api.model.refreshModels")}
              className="flex h-10 w-10 items-center justify-center"
            >
              <RefreshCcw
                className={`h-4 w-4 ${state.isFetchingModels ? "animate-spin" : ""}`}
              />
            </ResetButton>
          </div>
        </SettingContainer>
      )}

      {/* Korero (v1.3.0): in-app model pull for local providers (Ollama).
          Shows connection status, model storage path, and a pull button.
          Rendered only when is_local_provider is true. */}
      {!state.isAppleProvider && state.selectedProvider?.is_local_provider && (
        <SettingContainer
          title="Pull model"
          description={`Download ${state.model || "the selected model"} from Ollama's registry so it is available locally.`}
          descriptionMode="tooltip"
          layout="stacked"
          grouped={true}
        >
          <OllamaPullButton
            baseUrl={state.baseUrl}
            modelName={state.model}
            onModelPulled={state.handleRefreshModels}
          />
        </SettingContainer>
      )}
    </>
  );
};

const PostProcessingSettingsPromptsComponent: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating, refreshSettings } =
    useSettings();
  const [isCreating, setIsCreating] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [draftText, setDraftText] = useState("");
  const [draftAlias, setDraftAlias] = useState("");
  // Kōrero (v1.17.0, UX roadmap item 3): Raycast-style fuzzy filter over
  // prompt aliases + names. Typing "em" surfaces "Client email body"; Enter
  // selects the top match and clears the filter.
  const [promptQuery, setPromptQuery] = useState("");

  const prompts = getSetting("post_process_prompts") || [];
  const selectedPromptId = getSetting("post_process_selected_prompt_id") || "";
  const selectedPrompt =
    prompts.find((prompt) => prompt.id === selectedPromptId) || null;

  useEffect(() => {
    if (isCreating) return;

    if (selectedPrompt) {
      setDraftName(selectedPrompt.name);
      setDraftText(selectedPrompt.prompt);
      setDraftAlias(selectedPrompt.alias ?? "");
    } else {
      setDraftName("");
      setDraftText("");
      setDraftAlias("");
    }
  }, [
    isCreating,
    selectedPromptId,
    selectedPrompt?.name,
    selectedPrompt?.prompt,
    selectedPrompt?.alias,
  ]);

  // Subsequence fuzzy match, ranked: alias prefix > alias contains >
  // name prefix > name contains > subsequence-of-name. Empty query = all.
  const fuzzyRank = (q: string, alias: string, name: string): number => {
    const query = q.toLowerCase();
    const a = alias.toLowerCase();
    const n = name.toLowerCase();
    if (a.startsWith(query)) return 0;
    if (a.includes(query)) return 1;
    if (n.startsWith(query)) return 2;
    if (n.includes(query)) return 3;
    let i = 0;
    for (const ch of n) if (ch === query[i]) i++;
    return i >= query.length ? 4 : -1;
  };

  const filteredPrompts = promptQuery.trim()
    ? prompts
        .map((p) => ({
          p,
          rank: fuzzyRank(promptQuery.trim(), p.alias ?? "", p.name),
        }))
        .filter((x) => x.rank >= 0)
        .sort((x, y) => x.rank - y.rank)
        .map((x) => x.p)
    : prompts;

  const handlePromptSelect = (promptId: string | null) => {
    if (!promptId) return;
    updateSetting("post_process_selected_prompt_id", promptId);
    setIsCreating(false);
  };

  const handleCreatePrompt = async () => {
    if (!draftName.trim() || !draftText.trim()) return;

    try {
      const result = await commands.addPostProcessPrompt(
        draftName.trim(),
        draftText.trim(),
      );
      if (result.status === "ok") {
        await refreshSettings();
        updateSetting("post_process_selected_prompt_id", result.data.id);
        setIsCreating(false);
      }
    } catch (error) {
      console.error("Failed to create prompt:", error);
    }
  };

  const handleUpdatePrompt = async () => {
    if (!selectedPromptId || !draftName.trim() || !draftText.trim()) return;

    try {
      // Kōrero (v1.18.1) BUG FIX: v1.17.1 wrote the array via
      // updateSetting("post_process_prompts", …), but the settings store has
      // NO updater for that key — edits never persisted (lost on restart).
      // updatePostProcessPromptFull is the real persistence path and carries
      // the alias field the upstream command lacks.
      const result = await commands.updatePostProcessPromptFull(
        selectedPromptId,
        draftName.trim(),
        draftText.trim(),
        draftAlias.trim() ? draftAlias.trim() : null,
      );
      if (result.status === "error") throw new Error(result.error);
      await refreshSettings();
    } catch (error) {
      console.error("Failed to update prompt:", error);
    }
  };

  const handleDeletePrompt = async (promptId: string) => {
    if (!promptId) return;

    try {
      await commands.deletePostProcessPrompt(promptId);
      await refreshSettings();
      setIsCreating(false);
    } catch (error) {
      console.error("Failed to delete prompt:", error);
    }
  };

  const handleCancelCreate = () => {
    setIsCreating(false);
    if (selectedPrompt) {
      setDraftName(selectedPrompt.name);
      setDraftText(selectedPrompt.prompt);
    } else {
      setDraftName("");
      setDraftText("");
    }
  };

  const handleStartCreate = () => {
    setIsCreating(true);
    setDraftName("");
    setDraftText("");
  };

  const hasPrompts = prompts.length > 0;
  const isDirty =
    !!selectedPrompt &&
    (draftName.trim() !== selectedPrompt.name ||
      draftText.trim() !== selectedPrompt.prompt.trim() ||
      draftAlias.trim() !== (selectedPrompt.alias ?? ""));

  return (
    <SettingContainer
      title={t("settings.postProcessing.prompts.selectedPrompt.title")}
      description={t(
        "settings.postProcessing.prompts.selectedPrompt.description",
      )}
      descriptionMode="tooltip"
      layout="stacked"
      grouped={true}
    >
      <div className="space-y-3">
        {/* Kōrero (v1.17.0): alias fuzzy search — Enter selects top match */}
        {prompts.length > 3 && !isCreating && (
          <Input
            type="text"
            value={promptQuery}
            onChange={(e) => setPromptQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && filteredPrompts.length > 0) {
                e.preventDefault();
                handlePromptSelect(filteredPrompts[0].id);
                setPromptQuery("");
              } else if (e.key === "Escape") {
                setPromptQuery("");
              }
            }}
            // eslint-disable-next-line i18next/no-literal-string
            placeholder={'Find a prompt — type an alias ("clean", "email") and press Enter'}
            variant="compact"
          />
        )}
        <div className="flex gap-2">
          <Dropdown
            selectedValue={selectedPromptId || null}
            options={filteredPrompts.map((p) => ({
              value: p.id,
              label: p.alias ? `${p.name}  ·  ${p.alias}` : p.name,
            }))}
            onSelect={(value) => handlePromptSelect(value)}
            placeholder={
              prompts.length === 0
                ? t("settings.postProcessing.prompts.noPrompts")
                : t("settings.postProcessing.prompts.selectPrompt")
            }
            disabled={
              isUpdating("post_process_selected_prompt_id") || isCreating
            }
            className="flex-1"
          />
          <Button
            onClick={handleStartCreate}
            variant="primary"
            size="md"
            disabled={isCreating}
          >
            {t("settings.postProcessing.prompts.createNew")}
          </Button>
        </div>

        {!isCreating && hasPrompts && selectedPrompt && (
          <div className="space-y-3">
            <div className="flex gap-3">
              <div className="space-y-2 flex flex-col flex-1">
                <label className="text-sm font-semibold">
                  {t("settings.postProcessing.prompts.promptLabel")}
                </label>
                <Input
                  type="text"
                  value={draftName}
                  onChange={(e) => setDraftName(e.target.value)}
                  placeholder={t(
                    "settings.postProcessing.prompts.promptLabelPlaceholder",
                  )}
                  variant="compact"
                />
              </div>
              <div className="space-y-2 flex flex-col w-36">
                {/* eslint-disable-next-line i18next/no-literal-string */}
                <label className="text-sm font-semibold">Alias</label>
                <Input
                  type="text"
                  value={draftAlias}
                  onChange={(e) => setDraftAlias(e.target.value)}
                  // eslint-disable-next-line i18next/no-literal-string
                  placeholder="e.g. clean"
                  variant="compact"
                />
              </div>
            </div>

            <div className="space-y-2 flex flex-col">
              <label className="text-sm font-semibold">
                {t("settings.postProcessing.prompts.promptInstructions")}
              </label>
              <Textarea
                value={draftText}
                onChange={(e) => setDraftText(e.target.value)}
                placeholder={t(
                  "settings.postProcessing.prompts.promptInstructionsPlaceholder",
                )}
              />
              <p className="text-xs text-mid-gray/70">
                <Trans
                  i18nKey="settings.postProcessing.prompts.promptTip"
                  components={{ code: <code /> }}
                />
              </p>
            </div>

            <div className="flex gap-2 pt-2">
              <Button
                onClick={handleUpdatePrompt}
                variant="primary"
                size="md"
                disabled={!draftName.trim() || !draftText.trim() || !isDirty}
              >
                {t("settings.postProcessing.prompts.updatePrompt")}
              </Button>
              <Button
                onClick={() => handleDeletePrompt(selectedPromptId)}
                variant="secondary"
                size="md"
                disabled={!selectedPromptId || prompts.length <= 1}
              >
                {t("settings.postProcessing.prompts.deletePrompt")}
              </Button>
            </div>
          </div>
        )}

        {!isCreating && !selectedPrompt && (
          <div className="p-3 bg-mid-gray/5 rounded-md border border-mid-gray/20">
            <p className="text-sm text-mid-gray">
              {hasPrompts
                ? t("settings.postProcessing.prompts.selectToEdit")
                : t("settings.postProcessing.prompts.createFirst")}
            </p>
          </div>
        )}

        {isCreating && (
          <div className="space-y-3">
            <div className="space-y-2 block flex flex-col">
              <label className="text-sm font-semibold text-text">
                {t("settings.postProcessing.prompts.promptLabel")}
              </label>
              <Input
                type="text"
                value={draftName}
                onChange={(e) => setDraftName(e.target.value)}
                placeholder={t(
                  "settings.postProcessing.prompts.promptLabelPlaceholder",
                )}
                variant="compact"
              />
            </div>

            <div className="space-y-2 flex flex-col">
              <label className="text-sm font-semibold">
                {t("settings.postProcessing.prompts.promptInstructions")}
              </label>
              <Textarea
                value={draftText}
                onChange={(e) => setDraftText(e.target.value)}
                placeholder={t(
                  "settings.postProcessing.prompts.promptInstructionsPlaceholder",
                )}
              />
              <p className="text-xs text-mid-gray/70">
                <Trans
                  i18nKey="settings.postProcessing.prompts.promptTip"
                  components={{ code: <code /> }}
                />
              </p>
            </div>

            <div className="flex gap-2 pt-2">
              <Button
                onClick={handleCreatePrompt}
                variant="primary"
                size="md"
                disabled={!draftName.trim() || !draftText.trim()}
              >
                {t("settings.postProcessing.prompts.createPrompt")}
              </Button>
              <Button
                onClick={handleCancelCreate}
                variant="secondary"
                size="md"
              >
                {t("settings.postProcessing.prompts.cancel")}
              </Button>
            </div>
          </div>
        )}
      </div>
    </SettingContainer>
  );
};

export const PostProcessingSettingsApi = React.memo(
  PostProcessingSettingsApiComponent,
);
PostProcessingSettingsApi.displayName = "PostProcessingSettingsApi";

export const PostProcessingSettingsPrompts = React.memo(
  PostProcessingSettingsPromptsComponent,
);
PostProcessingSettingsPrompts.displayName = "PostProcessingSettingsPrompts";

export const PostProcessingSettings: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      {/* Korero (v1.2.0): enable/disable toggle surfaced here so the user can
          turn off post-processing without hunting through Advanced > Experimental. */}
      <SettingsGroup title={t("settings.advanced.groups.experimental")}>
        <PostProcessingToggle descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.postProcessing.hotkey.title")}>
        <ShortcutInput
          shortcutId="transcribe_with_post_process"
          descriptionMode="tooltip"
          grouped={true}
        />
      </SettingsGroup>

      <SettingsGroup title={t("settings.postProcessing.api.title")}>
        <PostProcessingSettingsApi />
      </SettingsGroup>

      <SettingsGroup title={t("settings.postProcessing.prompts.title")}>
        <PostProcessingSettingsPrompts />
      </SettingsGroup>

      {/* Kōrero (v1.15.0): corrections memory — teach the transcriber the
          words it keeps getting wrong. */}
      <CorrectionsManager />
    </div>
  );
};
