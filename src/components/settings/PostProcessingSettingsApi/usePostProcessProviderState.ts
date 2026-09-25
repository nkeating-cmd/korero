/**
 * Korero -- usePostProcessProviderState
 *
 * Promoted into the Handy-changes overlay at v1.35.0. It was previously an
 * upstream file carrying two surgical patches (the suggested_models loop and
 * its deps-array re-anchor). The v1.35.0 change touches four separate places
 * in the file, which is past the point where single-line Find/Replace patches
 * stay readable -- the same reasoning that moved PostProcessingSettings.tsx
 * into the overlay at v1.3.1.
 *
 * v1.35.0 -- the model dropdown tells the truth about local providers
 * -------------------------------------------------------------------
 * Reported: the Ollama model dropdown listed models that are NOT installed
 * (gemma3:4b, gemma3:27b, llama3.2:3b, ...) and omitted the ones that ARE.
 *
 * Three faults, all real, found in order:
 *
 *   1. BACKEND. fetch_post_process_models() refused to fetch whenever the
 *      provider's API key was empty and the id was not "custom". Ollama has
 *      no API key, so the refresh button returned
 *      "API key is required for Ollama (local)" and the installed list could
 *      never be obtained -- not by refresh, not by anything. Fixed by a
 *      patch on shortcut/mod.rs that exempts is_local_provider.
 *
 *   2. NO AUTO-FETCH. handleProviderSelect() only fetches when the provider
 *      has an API key (or is "custom"), so selecting Ollama fetched nothing,
 *      and there was no mount-time fetch at all. The dropdown therefore had
 *      only the static list to show. Fixed by the local-provider auto-fetch
 *      effect below.
 *
 *   3. SUGGESTIONS PRESENTED AS INSTALLED. suggested_models was merged into
 *      modelOptions for every provider. For a remote provider that is
 *      correct -- every catalogue model is usable the moment you have a key.
 *      For a LOCAL provider it is a lie: a model you have not pulled is not
 *      runnable, and picking it 404s at inference time. Fixed by excluding
 *      suggested_models from modelOptions for local providers and exposing
 *      them separately as pullCandidates, which the UI offers through the
 *      existing OllamaPullButton flow.
 *
 * The dropdown is still creatable, so a user can type any tag they like and
 * pull it -- nothing that was reachable before became unreachable.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSettings } from "../../../hooks/useSettings";
import { commands, type PostProcessProvider } from "@/bindings";
import type { ModelOption } from "./types";
import type { DropdownOption } from "../../ui/Dropdown";

type PostProcessProviderState = {
  providerOptions: DropdownOption[];
  selectedProviderId: string;
  selectedProvider: PostProcessProvider | undefined;
  isCustomProvider: boolean;
  isAppleProvider: boolean;
  isLocalProvider: boolean;
  appleIntelligenceUnavailable: boolean;
  baseUrl: string;
  handleBaseUrlChange: (value: string) => void;
  isBaseUrlUpdating: boolean;
  apiKey: string;
  handleApiKeyChange: (value: string) => void;
  isApiKeyUpdating: boolean;
  model: string;
  handleModelChange: (value: string) => void;
  modelOptions: ModelOption[];
  /** Models the local runtime actually reports as installed. Empty for remote
   *  providers and for a local provider that has not been reached yet. */
  installedModels: string[];
  /** True once a fetch for this provider has returned at least one model. */
  hasFetchedModels: boolean;
  /** Suggested models that are NOT installed -- pull candidates, not picks.
   *  Only populated for local providers. */
  pullCandidates: string[];
  isModelUpdating: boolean;
  isFetchingModels: boolean;
  handleProviderSelect: (providerId: string) => void;
  handleModelSelect: (value: string) => void;
  handleModelCreate: (value: string) => void;
  handleRefreshModels: () => void;
};

const APPLE_PROVIDER_ID = "apple_intelligence";

export const usePostProcessProviderState = (): PostProcessProviderState => {
  const {
    settings,
    isUpdating,
    setPostProcessProvider,
    updatePostProcessBaseUrl,
    updatePostProcessApiKey,
    updatePostProcessModel,
    fetchPostProcessModels,
    postProcessModelOptions,
  } = useSettings();

  // Settings are guaranteed to have providers after migration
  const providers = settings?.post_process_providers || [];

  const selectedProviderId = useMemo(() => {
    return settings?.post_process_provider_id || providers[0]?.id || "openai";
  }, [providers, settings?.post_process_provider_id]);

  const selectedProvider = useMemo(() => {
    return (
      providers.find((provider) => provider.id === selectedProviderId) ||
      providers[0]
    );
  }, [providers, selectedProviderId]);

  const isAppleProvider = selectedProvider?.id === APPLE_PROVIDER_ID;
  // Korero (v1.35.0): Apple Intelligence is flagged is_local_provider too, but
  // it has no model catalogue and no fetchable list -- it is excluded here so
  // "local" in this hook means "a local server we can enumerate", i.e. Ollama.
  const isLocalProvider =
    selectedProvider?.is_local_provider === true && !isAppleProvider;

  const [appleIntelligenceUnavailable, setAppleIntelligenceUnavailable] =
    useState(false);

  // Use settings directly as single source of truth
  const baseUrl = selectedProvider?.base_url ?? "";
  const apiKey = settings?.post_process_api_keys?.[selectedProviderId] ?? "";
  const model = settings?.post_process_models?.[selectedProviderId] ?? "";

  const providerOptions = useMemo<DropdownOption[]>(() => {
    return providers.map((provider) => ({
      value: provider.id,
      label: provider.label,
    }));
  }, [providers]);

  const handleProviderSelect = useCallback(
    async (providerId: string) => {
      // Clear error state on any selection attempt (allows dismissing the error)
      setAppleIntelligenceUnavailable(false);

      if (providerId === selectedProviderId) return;

      // Check Apple Intelligence availability before selecting
      if (providerId === APPLE_PROVIDER_ID) {
        const available = await commands.checkAppleIntelligenceAvailable();
        if (!available) {
          setAppleIntelligenceUnavailable(true);
          // Don't return - still set the provider so dropdown shows the selection
          // The backend gracefully handles unavailable Apple Intelligence
        }
      }

      await setPostProcessProvider(providerId);

      // Auto-fetch available models for the new provider so the model dropdown
      // reflects what's actually valid. Without this, a stale model value from
      // a previous provider/base_url can persist and silently 404 at runtime.
      // Skip when the provider isn't configured yet (no API key / empty base URL)
      // to avoid unnecessary backend errors.
      if (providerId !== APPLE_PROVIDER_ID) {
        const provider = providers.find((p) => p.id === providerId);
        const apiKey = settings?.post_process_api_keys?.[providerId] ?? "";
        const hasBaseUrl = (provider?.base_url ?? "").trim() !== "";
        const hasApiKey = apiKey.trim() !== "";
        // Korero (v1.35.0): a local provider needs no API key -- a base URL is
        // the whole configuration. This clause is why selecting Ollama used to
        // fetch nothing.
        const isLocal = provider?.is_local_provider === true;

        const configured =
          provider?.id === "custom" || isLocal ? hasBaseUrl : hasApiKey;

        if (configured) {
          void fetchPostProcessModels(providerId);
        }
      }
    },
    [
      selectedProviderId,
      setPostProcessProvider,
      fetchPostProcessModels,
      providers,
      settings,
    ],
  );

  const handleBaseUrlChange = useCallback(
    (value: string) => {
      if (!selectedProvider || selectedProvider.id !== "custom") {
        return;
      }
      const trimmed = value.trim();
      if (trimmed && trimmed !== baseUrl) {
        void updatePostProcessBaseUrl(selectedProvider.id, trimmed);
      }
    },
    [selectedProvider, baseUrl, updatePostProcessBaseUrl],
  );

  const handleApiKeyChange = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (trimmed !== apiKey) {
        void updatePostProcessApiKey(selectedProviderId, trimmed);
      }
    },
    [apiKey, selectedProviderId, updatePostProcessApiKey],
  );

  const handleModelChange = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (trimmed !== model) {
        void updatePostProcessModel(selectedProviderId, trimmed);
      }
    },
    [model, selectedProviderId, updatePostProcessModel],
  );

  const handleModelSelect = useCallback(
    (value: string) => {
      void updatePostProcessModel(selectedProviderId, value.trim());
    },
    [selectedProviderId, updatePostProcessModel],
  );

  const handleModelCreate = useCallback(
    (value: string) => {
      void updatePostProcessModel(selectedProviderId, value);
    },
    [selectedProviderId, updatePostProcessModel],
  );

  // Korero (v1.35.0): keyed record of auto-fetch attempts. fetchPostProcessModels
  // deliberately does NOT cache an empty result on failure, so "no models yet"
  // is indistinguishable from "never asked" in the store. Without this guard an
  // unreachable Ollama would re-trigger the effect on every render for as long
  // as the pane stayed open. One attempt per (provider, base URL); the refresh
  // button clears the key so a manual retry is always honoured.
  const autoFetchAttempted = useRef<Set<string>>(new Set());
  const autoFetchKey = `${selectedProviderId}|${baseUrl}`;

  const handleRefreshModels = useCallback(() => {
    if (isAppleProvider) return;
    // Mark this (provider, base URL) as attempted so the auto-fetch effect
    // does not fire a second, redundant request behind this one.
    autoFetchAttempted.current.add(autoFetchKey);
    void fetchPostProcessModels(selectedProviderId);
  }, [
    autoFetchKey,
    fetchPostProcessModels,
    isAppleProvider,
    selectedProviderId,
  ]);

  const availableModelsRaw = postProcessModelOptions[selectedProviderId] || [];
  const hasFetchedModels = availableModelsRaw.length > 0;

  // Korero (v1.35.0): the local-provider auto-fetch. A local runtime is
  // enumerable the moment it is running -- there is no key to wait for and no
  // per-request cost -- so the installed list is fetched on mount and whenever
  // the provider or base URL changes, rather than waiting for a refresh click
  // the user has no reason to suspect they need.
  useEffect(() => {
    if (!isLocalProvider) return;
    if (!baseUrl.trim()) return;
    if (autoFetchAttempted.current.has(autoFetchKey)) return;
    autoFetchAttempted.current.add(autoFetchKey);
    void fetchPostProcessModels(selectedProviderId);
  }, [
    autoFetchKey,
    baseUrl,
    fetchPostProcessModels,
    isLocalProvider,
    selectedProviderId,
  ]);

  const installedSet = useMemo(() => {
    const s = new Set<string>();
    for (const m of availableModelsRaw) {
      const trimmed = m?.trim();
      if (trimmed) s.add(trimmed);
    }
    return s;
  }, [availableModelsRaw]);

  const modelOptions = useMemo<ModelOption[]>(() => {
    const seen = new Set<string>();
    const options: ModelOption[] = [];

    const upsert = (value: string | null | undefined, label?: string) => {
      const trimmed = value?.trim();
      if (!trimmed || seen.has(trimmed)) return;
      seen.add(trimmed);
      options.push({ value: trimmed, label: label ?? trimmed });
    };

    // Add available models from API
    for (const candidate of availableModelsRaw) {
      upsert(candidate);
    }

    // Korero (v1.3.0): add suggested models from the provider's static list.
    // These ensure the dropdown is useful before the user clicks "Fetch models".
    // Deduplicated against API results by the seen Set above.
    //
    // Korero (v1.35.0): NOT for local providers. A suggested model that has not
    // been pulled cannot run, so offering it as a pick is offering a failure.
    // Local suggestions go out through pullCandidates instead.
    if (!isLocalProvider) {
      for (const candidate of selectedProvider?.suggested_models ?? []) {
        upsert(candidate);
      }
    }

    // Ensure current model is in the list. Korero (v1.35.0): on a local
    // provider whose installed list we actually have, say plainly when the
    // configured model is not among it -- this is the state Nic was in, with
    // gemma3:4b selected and never pulled.
    const notInstalled =
      isLocalProvider && hasFetchedModels && !installedSet.has(model.trim());
    upsert(model, notInstalled ? `${model.trim()} -- not installed` : undefined);

    return options;
  }, [
    availableModelsRaw,
    hasFetchedModels,
    installedSet,
    isLocalProvider,
    model,
    selectedProvider,
  ]);

  const pullCandidates = useMemo<string[]>(() => {
    if (!isLocalProvider) return [];
    const out: string[] = [];
    const seen = new Set<string>();
    for (const candidate of selectedProvider?.suggested_models ?? []) {
      const trimmed = candidate?.trim();
      if (!trimmed || seen.has(trimmed) || installedSet.has(trimmed)) continue;
      seen.add(trimmed);
      out.push(trimmed);
    }
    return out;
  }, [installedSet, isLocalProvider, selectedProvider]);

  const isBaseUrlUpdating = isUpdating(
    `post_process_base_url:${selectedProviderId}`,
  );
  const isApiKeyUpdating = isUpdating(
    `post_process_api_key:${selectedProviderId}`,
  );
  const isModelUpdating = isUpdating(
    `post_process_model:${selectedProviderId}`,
  );
  const isFetchingModels = isUpdating(
    `post_process_models_fetch:${selectedProviderId}`,
  );

  const isCustomProvider = selectedProvider?.id === "custom";

  return {
    providerOptions,
    selectedProviderId,
    selectedProvider,
    isCustomProvider,
    isAppleProvider,
    isLocalProvider,
    appleIntelligenceUnavailable,
    baseUrl,
    handleBaseUrlChange,
    isBaseUrlUpdating,
    apiKey,
    handleApiKeyChange,
    isApiKeyUpdating,
    model,
    handleModelChange,
    modelOptions,
    installedModels: availableModelsRaw,
    hasFetchedModels,
    pullCandidates,
    isModelUpdating,
    isFetchingModels,
    handleProviderSelect,
    handleModelSelect,
    handleModelCreate,
    handleRefreshModels,
  };
};
