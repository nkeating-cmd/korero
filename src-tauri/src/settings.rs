use log::{debug, warn};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt;

/// Kōrero: event emitted when one or more keychain writes fail. Payload is a
/// list of provider IDs that did not persist. Frontend listens on
/// `korero://keychain-error` and surfaces a toast so the user knows their key
/// isn't being saved (otherwise the failure is silent and manifests later as
/// "the app forgot my key").
pub const KEYCHAIN_ERROR_EVENT: &str = "korero://keychain-error";

#[derive(Debug, Clone, Serialize, Type)]
pub struct KeychainErrorPayload {
    pub failed_providers: Vec<String>,
    pub phase: &'static str, // "save" | "migrate"
}

fn emit_keychain_error(app: &AppHandle, phase: &'static str, failed_providers: Vec<String>) {
    if failed_providers.is_empty() {
        return;
    }
    let payload = KeychainErrorPayload {
        failed_providers,
        phase,
    };
    if let Err(err) = app.emit(KEYCHAIN_ERROR_EVENT, &payload) {
        warn!(
            "secret_store: failed to emit {} event: {}",
            KEYCHAIN_ERROR_EVENT, err
        );
    }
}

pub const APPLE_INTELLIGENCE_PROVIDER_ID: &str = "apple_intelligence";
pub const APPLE_INTELLIGENCE_DEFAULT_MODEL_ID: &str = "Apple Intelligence";

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// Custom deserializer to handle both old numeric format (1-5) and new string format ("trace", "debug", etc.)
impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LogLevelVisitor;

        impl<'de> Visitor<'de> for LogLevelVisitor {
            type Value = LogLevel;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or integer representing log level")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<LogLevel, E> {
                match value.to_lowercase().as_str() {
                    "trace" => Ok(LogLevel::Trace),
                    "debug" => Ok(LogLevel::Debug),
                    "info" => Ok(LogLevel::Info),
                    "warn" => Ok(LogLevel::Warn),
                    "error" => Ok(LogLevel::Error),
                    _ => Err(E::unknown_variant(
                        value,
                        &["trace", "debug", "info", "warn", "error"],
                    )),
                }
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<LogLevel, E> {
                match value {
                    1 => Ok(LogLevel::Trace),
                    2 => Ok(LogLevel::Debug),
                    3 => Ok(LogLevel::Info),
                    4 => Ok(LogLevel::Warn),
                    5 => Ok(LogLevel::Error),
                    _ => Err(E::invalid_value(de::Unexpected::Unsigned(value), &"1-5")),
                }
            }
        }

        deserializer.deserialize_any(LogLevelVisitor)
    }
}

impl From<LogLevel> for tauri_plugin_log::LogLevel {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => tauri_plugin_log::LogLevel::Trace,
            LogLevel::Debug => tauri_plugin_log::LogLevel::Debug,
            LogLevel::Info => tauri_plugin_log::LogLevel::Info,
            LogLevel::Warn => tauri_plugin_log::LogLevel::Warn,
            LogLevel::Error => tauri_plugin_log::LogLevel::Error,
        }
    }
}

/// Kōrero (v1.15.0): one user-taught transcription correction. `wrong` is
/// what the model keeps producing; `right` is what it should be.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct TranscriptCorrection {
    pub wrong: String,
    pub right: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ShortcutBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_binding: String,
    pub current_binding: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct LLMPrompt {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// Kōrero (v1.17.0, UX roadmap item 3): short alias for Raycast-style
    /// fuzzy search in the prompts UI ("clean", "email", …). Optional —
    /// existing installs deserialise to None via serde default.
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PostProcessProvider {
    pub id: String,
    pub label: String,
    pub base_url: String,
    #[serde(default)]
    pub allow_base_url_edit: bool,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub supports_structured_output: bool,
    /// Static model list shown in the UI without requiring an API call.
    /// Populated for providers with a well-known, stable model catalogue.
    /// The UI merges this with any dynamically-fetched models from the API.
    #[serde(default)]
    pub suggested_models: Vec<String>,
    /// True when the provider runs locally (e.g. Ollama).
    /// Adds a "Local" badge in the UI and enables the in-app model pull workflow.
    #[serde(default)]
    pub is_local_provider: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    None,
    Top,
    Bottom,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ModelUnloadTimeout {
    Never,
    Immediately,
    Min2,
    Min5,
    Min10,
    Min15,
    Hour1,
    Sec15, // Debug mode only
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum PasteMethod {
    CtrlV,
    Direct,
    None,
    ShiftInsert,
    CtrlShiftV,
    ExternalScript,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardHandling {
    DontModify,
    CopyToClipboard,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AutoSubmitKey {
    Enter,
    CtrlEnter,
    CmdEnter,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecordingRetentionPeriod {
    Never,
    PreserveLimit,
    Days3,
    Weeks2,
    Months3,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardImplementation {
    Tauri,
    HandyKeys,
}

impl Default for KeyboardImplementation {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        return KeyboardImplementation::Tauri;
        #[cfg(not(target_os = "linux"))]
        return KeyboardImplementation::HandyKeys;
    }
}

impl Default for ModelUnloadTimeout {
    fn default() -> Self {
        // Kōrero: keep model warm longer during a working session.
        ModelUnloadTimeout::Min15
    }
}

impl Default for PasteMethod {
    fn default() -> Self {
        // Default to CtrlV for macOS and Windows, Direct for Linux
        #[cfg(target_os = "linux")]
        return PasteMethod::Direct;
        #[cfg(not(target_os = "linux"))]
        return PasteMethod::CtrlV;
    }
}

impl Default for ClipboardHandling {
    fn default() -> Self {
        ClipboardHandling::DontModify
    }
}

impl Default for AutoSubmitKey {
    fn default() -> Self {
        AutoSubmitKey::Enter
    }
}

impl ModelUnloadTimeout {
    pub fn to_minutes(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Min2 => Some(2),
            ModelUnloadTimeout::Min5 => Some(5),
            ModelUnloadTimeout::Min10 => Some(10),
            ModelUnloadTimeout::Min15 => Some(15),
            ModelUnloadTimeout::Hour1 => Some(60),
            ModelUnloadTimeout::Sec15 => Some(0), // Special case for debug - handled separately
        }
    }

    pub fn to_seconds(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Sec15 => Some(15),
            _ => self.to_minutes().map(|m| m * 60),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SoundTheme {
    Marimba,
    Pop,
    /// Kōrero (v1.17.1, UX roadmap item 4): asymmetric cues — sharp,
    /// high-frequency start tone vs soft, descending stop tone. The
    /// asymmetry makes start/stop unambiguous without visual attention.
    Aurora,
    Custom,
}

impl SoundTheme {
    fn as_str(&self) -> &'static str {
        match self {
            SoundTheme::Marimba => "marimba",
            SoundTheme::Pop => "pop",
            SoundTheme::Aurora => "aurora",
            SoundTheme::Custom => "custom",
        }
    }

    pub fn to_start_path(&self) -> String {
        format!("resources/{}_start.wav", self.as_str())
    }

    pub fn to_stop_path(&self) -> String {
        format!("resources/{}_stop.wav", self.as_str())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum TypingTool {
    Auto,
    Wtype,
    Kwtype,
    Dotool,
    Ydotool,
    Xdotool,
}

impl Default for TypingTool {
    fn default() -> Self {
        TypingTool::Auto
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WhisperAcceleratorSetting {
    Auto,
    Cpu,
    Gpu,
}

impl Default for WhisperAcceleratorSetting {
    fn default() -> Self {
        WhisperAcceleratorSetting::Auto
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum OrtAcceleratorSetting {
    Auto,
    Cpu,
    Cuda,
    #[serde(rename = "directml")]
    DirectMl,
    Rocm,
}

impl Default for OrtAcceleratorSetting {
    fn default() -> Self {
        OrtAcceleratorSetting::Auto
    }
}

// Kōrero: secrets live in the OS keychain, not on disk.
//
// SecretMap is still the in-memory cache the rest of the app reads from — the
// frontend, the LLM client, and the settings command surface all expect a
// `HashMap<String, String>`-shaped view. The change is at the persistence
// boundary:
//
//   * `Serialize` emits empty strings for every key, so `settings_store.json`
//     never contains plaintext secrets even if the in-memory map is populated.
//   * `Deserialize` reads the JSON verbatim, which lets us migrate legacy
//     plaintext keys on first load (see `migrate_plaintext_to_keyring`).
//   * `hydrate_from_keyring` repopulates the in-memory map from the OS store
//     after every load.
//   * `persist_to_keyring` writes the in-memory map into the OS store before
//     every save.
//
// Result: a backup of `settings_store.json` is no longer a credential leak.
#[derive(Clone, Deserialize, Type)]
#[serde(transparent)]
pub(crate) struct SecretMap(HashMap<String, String>);

impl Serialize for SecretMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        // Preserve the key set (provider IDs) so the on-disk shape stays
        // schema-compatible with anything that reads it raw, but blank every
        // value. Real secrets are stored separately in the OS keychain.
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for key in self.0.keys() {
            map.serialize_entry(key, "")?;
        }
        map.end()
    }
}

impl fmt::Debug for SecretMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted: HashMap<&String, &str> = self
            .0
            .iter()
            .map(|(k, v)| (k, if v.is_empty() { "" } else { "[REDACTED]" }))
            .collect();
        redacted.fmt(f)
    }
}

impl std::ops::Deref for SecretMap {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SecretMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl SecretMap {
    /// Pull every known provider's key out of the OS keychain into the
    /// in-memory map. Existing in-memory values are overwritten — the keychain
    /// is the source of truth post-migration. Missing keychain entries leave
    /// the in-memory value as an empty string.
    pub fn hydrate_from_keyring(&mut self) {
        for provider_id in self.0.keys().cloned().collect::<Vec<_>>() {
            let key = crate::secret_store::load_api_key(&provider_id).unwrap_or_default();
            self.0.insert(provider_id, key);
        }
    }

    /// Push every value in the in-memory map into the OS keychain.
    /// Empty values trigger a delete so revoked keys don't linger.
    ///
    /// Returns the list of provider IDs whose write failed, so the caller
    /// can surface a UI signal. An empty vec means everything persisted
    /// cleanly.
    pub fn persist_to_keyring(&self) -> Vec<String> {
        let mut failures = Vec::new();
        for (provider_id, value) in self.0.iter() {
            if !crate::secret_store::save_api_key(provider_id, value) {
                failures.push(provider_id.clone());
            }
        }
        failures
    }

    /// One-shot migration of legacy plaintext keys from a freshly deserialised
    /// settings JSON. Any non-empty value is copied to the keychain and then
    /// blanked in the in-memory map (so the next write to disk is clean).
    ///
    /// Returns `(migrated_any, failures)`:
    ///   * `migrated_any` is `true` if at least one key was migrated and the
    ///     caller should re-write the settings JSON to commit the blanks.
    ///   * `failures` lists provider IDs whose keychain write failed — their
    ///     plaintext is preserved in the in-memory map for the current session
    ///     (do not blank), and the caller should surface a UI signal so the
    ///     user knows a re-save is needed.
    pub fn migrate_plaintext_to_keyring(&mut self) -> (bool, Vec<String>) {
        let mut migrated = false;
        let mut failures = Vec::new();
        for (provider_id, value) in self.0.iter_mut() {
            if value.is_empty() {
                continue;
            }
            if crate::secret_store::save_api_key(provider_id, value) {
                debug!(
                    "secret_store: migrated plaintext key for '{}' into OS keychain",
                    provider_id
                );
                value.clear();
                migrated = true;
            } else {
                warn!(
                    "secret_store: keychain unavailable, leaving plaintext key for '{}' in place for this session",
                    provider_id
                );
                failures.push(provider_id.clone());
            }
        }
        (migrated, failures)
    }
}

/* still handy for composing the initial JSON in the store ------------- */
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct AppSettings {
    pub bindings: HashMap<String, ShortcutBinding>,
    pub push_to_talk: bool,
    pub audio_feedback: bool,
    #[serde(default = "default_audio_feedback_volume")]
    pub audio_feedback_volume: f32,
    #[serde(default = "default_sound_theme")]
    pub sound_theme: SoundTheme,
    #[serde(default = "default_start_hidden")]
    pub start_hidden: bool,
    #[serde(default = "default_autostart_enabled")]
    pub autostart_enabled: bool,
    #[serde(default = "default_update_checks_enabled")]
    pub update_checks_enabled: bool,
    #[serde(default = "default_model")]
    pub selected_model: String,
    #[serde(default = "default_always_on_microphone")]
    pub always_on_microphone: bool,
    #[serde(default)]
    pub selected_microphone: Option<String>,
    #[serde(default)]
    pub clamshell_microphone: Option<String>,
    #[serde(default)]
    pub selected_output_device: Option<String>,
    #[serde(default = "default_translate_to_english")]
    pub translate_to_english: bool,
    #[serde(default = "default_selected_language")]
    pub selected_language: String,
    #[serde(default = "default_overlay_position")]
    pub overlay_position: OverlayPosition,
    #[serde(default = "default_debug_mode")]
    pub debug_mode: bool,
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
    #[serde(default)]
    pub custom_words: Vec<String>,
    // Kōrero (v1.15.0): user-taught corrections (wrong → right), applied
    // deterministically to every transcription AFTER custom-word fuzzy
    // matching, and injected as a glossary into note/meeting post-processing.
    #[serde(default)]
    pub transcript_corrections: Vec<TranscriptCorrection>,
    // Kōrero (v1.11.0): optional RNNoise mic denoiser. Only effective when the
    // capture device runs at 48 kHz. Default OFF (denoising can hurt ASR accuracy).
    #[serde(default)]
    pub denoise_enabled: bool,
    // Kōrero (v1.12.0): write a crash report file on panic (panic logging is
    // always on regardless). Default ON.
    #[serde(default = "default_save_crash_reports")]
    pub save_crash_reports: bool,
    #[serde(default)]
    pub model_unload_timeout: ModelUnloadTimeout,
    #[serde(default = "default_word_correction_threshold")]
    pub word_correction_threshold: f64,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
    #[serde(default = "default_recording_retention_period")]
    pub recording_retention_period: RecordingRetentionPeriod,
    #[serde(default)]
    pub paste_method: PasteMethod,
    #[serde(default)]
    pub clipboard_handling: ClipboardHandling,
    #[serde(default = "default_auto_submit")]
    pub auto_submit: bool,
    #[serde(default)]
    pub auto_submit_key: AutoSubmitKey,
    #[serde(default = "default_post_process_enabled")]
    pub post_process_enabled: bool,
    #[serde(default = "default_post_process_provider_id")]
    pub post_process_provider_id: String,
    #[serde(default = "default_post_process_providers")]
    pub post_process_providers: Vec<PostProcessProvider>,
    #[serde(default = "default_post_process_api_keys")]
    pub post_process_api_keys: SecretMap,
    #[serde(default = "default_post_process_models")]
    pub post_process_models: HashMap<String, String>,
    #[serde(default = "default_post_process_prompts")]
    pub post_process_prompts: Vec<LLMPrompt>,
    #[serde(default)]
    pub post_process_selected_prompt_id: Option<String>,
    #[serde(default)]
    pub mute_while_recording: bool,
    #[serde(default)]
    pub append_trailing_space: bool,
    #[serde(default = "default_app_language")]
    pub app_language: String,
    #[serde(default)]
    pub experimental_enabled: bool,
    #[serde(default)]
    pub lazy_stream_close: bool,
    #[serde(default)]
    pub keyboard_implementation: KeyboardImplementation,
    #[serde(default = "default_show_tray_icon")]
    pub show_tray_icon: bool,
    #[serde(default = "default_paste_delay_ms")]
    pub paste_delay_ms: u64,
    #[serde(default = "default_typing_tool")]
    pub typing_tool: TypingTool,
    pub external_script_path: Option<String>,
    #[serde(default)]
    pub custom_filler_words: Option<Vec<String>>,
    #[serde(default)]
    pub whisper_accelerator: WhisperAcceleratorSetting,
    #[serde(default)]
    pub ort_accelerator: OrtAcceleratorSetting,
    #[serde(default = "default_whisper_gpu_device")]
    pub whisper_gpu_device: i32,
    #[serde(default)]
    pub extra_recording_buffer_ms: u64,
}

fn default_model() -> String {
    "".to_string()
}

fn default_always_on_microphone() -> bool {
    false
}

fn default_translate_to_english() -> bool {
    false
}

fn default_start_hidden() -> bool {
    false
}

fn default_autostart_enabled() -> bool {
    false
}

fn default_update_checks_enabled() -> bool {
    true
}

fn default_selected_language() -> String {
    // Kōrero: default to English. "auto" was upstream default but caused
    // mistriggers on NZ accents in early testing.
    "en".to_string()
}

fn default_overlay_position() -> OverlayPosition {
    #[cfg(target_os = "linux")]
    return OverlayPosition::None;
    #[cfg(not(target_os = "linux"))]
    return OverlayPosition::Bottom;
}

fn default_debug_mode() -> bool {
    false
}

fn default_log_level() -> LogLevel {
    LogLevel::Debug
}

fn default_save_crash_reports() -> bool {
    true
}

fn default_word_correction_threshold() -> f64 {
    0.18
}

fn default_paste_delay_ms() -> u64 {
    60
}

fn default_auto_submit() -> bool {
    false
}

fn default_history_limit() -> usize {
    5
}

fn default_recording_retention_period() -> RecordingRetentionPeriod {
    RecordingRetentionPeriod::PreserveLimit
}

fn default_audio_feedback_volume() -> f32 {
    1.0
}

fn default_sound_theme() -> SoundTheme {
    SoundTheme::Marimba
}

fn default_post_process_enabled() -> bool {
    false
}

fn default_app_language() -> String {
    tauri_plugin_os::locale()
        .map(|l| l.replace('_', "-"))
        .unwrap_or_else(|| "en".to_string())
}

fn default_show_tray_icon() -> bool {
    true
}

fn default_post_process_provider_id() -> String {
    // Korero: DeepSeek V4 as default for cost (about 12x cheaper than
    // Claude per token). User can switch to Anthropic for premium prompts.
    "deepseek".to_string()
}

fn default_post_process_providers() -> Vec<PostProcessProvider> {
    let mut providers = vec![
        PostProcessProvider {
            id: "openai".to_string(),
            label: "OpenAI".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
            suggested_models: vec![],
            is_local_provider: false,
        },
        PostProcessProvider {
            id: "zai".to_string(),
            label: "Z.AI".to_string(),
            base_url: "https://api.z.ai/api/paas/v4".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
            suggested_models: vec![],
            is_local_provider: false,
        },
        PostProcessProvider {
            id: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
            suggested_models: vec![],
            is_local_provider: false,
        },
        PostProcessProvider {
            id: "anthropic".to_string(),
            label: "Anthropic".to_string(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
            suggested_models: vec![],
            is_local_provider: false,
        },
        PostProcessProvider {
            id: "groq".to_string(),
            label: "Groq".to_string(),
            base_url: "https://api.groq.com/openai/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
            suggested_models: vec![],
            is_local_provider: false,
        },
        PostProcessProvider {
            // Korero: added 2026-05-14. DeepSeek V4 — OpenAI-compatible API.
            // Roughly 12x cheaper than Claude per token. Default for high-volume
            // post-processing (clean transcript, WhatsApp/Slack formatting).
            id: "deepseek".to_string(),
            label: "DeepSeek".to_string(),
            base_url: "https://api.deepseek.com/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
            suggested_models: vec![],
            is_local_provider: false,
        },
        PostProcessProvider {
            id: "cerebras".to_string(),
            label: "Cerebras".to_string(),
            base_url: "https://api.cerebras.ai/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
            suggested_models: vec![],
            is_local_provider: false,
        },
    ];

    // Note: We always include Apple Intelligence on macOS ARM64 without checking availability
    // at startup. The availability check is deferred to when the user actually tries to use it
    // (in actions.rs). This prevents crashes on macOS 26.x beta where accessing
    // SystemLanguageModel.default during early app initialization causes SIGABRT.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        providers.push(PostProcessProvider {
            id: APPLE_INTELLIGENCE_PROVIDER_ID.to_string(),
            label: "Apple Intelligence".to_string(),
            base_url: "apple-intelligence://local".to_string(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: true,
            suggested_models: vec![],
            is_local_provider: true,
        });
    }

    // AWS Bedrock via Mantle (OpenAI-compatible endpoint)
    providers.push(PostProcessProvider {
        id: "bedrock_mantle".to_string(),
        label: "AWS Bedrock (Mantle)".to_string(),
        base_url: "https://bedrock-mantle.us-east-1.api.aws/v1".to_string(),
        allow_base_url_edit: false,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: true,
        suggested_models: vec![],
        is_local_provider: false,
    });

    // Kōrero (v1.1.0): Google Gemini via the OpenAI-compatible endpoint.
    // supports_structured_output: false -- Google's compat layer accepts JSON mode
    // but strict schema mode is not fully equivalent to OpenAI's. Enable once
    // confirmed working end-to-end. /models returns "models/gemini-*" prefixed IDs.
    // Kōrero (v1.3.0): suggested_models populated so the dropdown works without
    // an API call. Ordered fastest/cheapest → largest for easy scanning.
    providers.push(PostProcessProvider {
        id: "gemini".to_string(),
        label: "Google Gemini".to_string(),
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai/".to_string(),
        allow_base_url_edit: false,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: false,
        suggested_models: vec![
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-flash-lite".to_string(),
            "gemini-2.5-pro".to_string(),
            "gemini-2.0-flash".to_string(),
            "gemini-2.0-flash-lite".to_string(),
            "gemini-1.5-flash".to_string(),
            "gemini-1.5-pro".to_string(),
        ],
        is_local_provider: false,
    });

    // Kōrero (v1.1.0): Ollama local inference. Default model gemma3:4b (~4-6 GB RAM,
    // 128K context, GPU-accelerated via DirectML on Windows). User can change the
    // base URL if running Ollama on a non-standard port.
    // Kōrero (v1.3.0): is_local_provider=true shows the "Local" badge and enables
    // the in-app pull workflow. suggested_models covers the most common options so
    // the dropdown is useful before the user clicks "Fetch models".
    providers.push(PostProcessProvider {
        id: "ollama".to_string(),
        label: "Ollama (local)".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        allow_base_url_edit: true,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: false,
        suggested_models: vec![
            // Kōrero (v1.14.6): Gemma 4 12B — released 2026-06-03, Apache 2.0,
            // near-26B-MoE quality, ~8-9 GB VRAM at Ollama's default quant
            // (16 GB for the full model). Listed first as the best local
            // option on capable hardware. NOTE: tag follows the gemma3:12b
            // convention but was unverified at commit time — if the in-app
            // pull 404s, confirm at ollama.com/library/gemma4.
            "gemma4:12b".to_string(),
            "gemma3:4b".to_string(),
            "gemma3:12b".to_string(),
            "gemma3:27b".to_string(),
            "llama3.2:3b".to_string(),
            "llama3.1:8b".to_string(),
            "mistral:7b".to_string(),
            "phi4:14b".to_string(),
            "qwen2.5:7b".to_string(),
        ],
        is_local_provider: true,
    });

    // Custom provider always comes last. URL defaulted to :8080 to distinguish
    // from Ollama's :11434 default -- avoids two identical-looking entries.
    providers.push(PostProcessProvider {
        id: "custom".to_string(),
        label: "Custom".to_string(),
        base_url: "http://localhost:8080/v1".to_string(),
        allow_base_url_edit: true,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: false,
        suggested_models: vec![],
        is_local_provider: false,
    });

    providers
}

fn default_post_process_api_keys() -> SecretMap {
    let mut map = HashMap::new();
    for provider in default_post_process_providers() {
        map.insert(provider.id, String::new());
    }
    SecretMap(map)
}

fn default_model_for_provider(provider_id: &str) -> String {
    if provider_id == APPLE_INTELLIGENCE_PROVIDER_ID {
        return APPLE_INTELLIGENCE_DEFAULT_MODEL_ID.to_string();
    }
    // Korero: pre-select sensible default model per provider
    match provider_id {
        "deepseek" => "deepseek-chat".to_string(),
        "anthropic" => "claude-sonnet-4-6".to_string(),
        "openai" => "gpt-4o-mini".to_string(),
        "groq" => "llama-3.3-70b-versatile".to_string(),
        // Korero (v1.2.0): bumped from gemini-2.0-flash to gemini-2.5-flash.
        // Flash 2.5 is faster and higher quality for complex prompts (Red Team,
        // meeting notes). Migration in ensure_post_process_defaults() auto-
        // upgrades existing installs that still have the old default.
        "gemini" => "gemini-2.5-flash".to_string(),
        // Kōrero (v1.1.0): gemma3:4b (not gemma:4b -- that's the older Gemma 1/2 tag).
        // Run `ollama pull gemma3:4b` before first use. ~4-6 GB RAM.
        "ollama" => "gemma3:4b".to_string(),
        _ => String::new(),
    }
}

fn default_post_process_models() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for provider in default_post_process_providers() {
        map.insert(
            provider.id.clone(),
            default_model_for_provider(&provider.id),
        );
    }
    map
}

/// Kōrero (v1.18.1) BUG FIX: the v1.17.1 prompt editor saved edits via the
/// frontend's `updateSetting("post_process_prompts", …)`, but settingsStore
/// has no updater for that key — edits changed local React state, logged
/// "No handler for setting", and were silently LOST on restart. This command
/// is the persistence path: full prompt update including the alias field
/// (which the upstream update_post_process_prompt command doesn't carry).
#[tauri::command]
#[specta::specta]
pub fn update_post_process_prompt_full(
    app: tauri::AppHandle,
    prompt_id: String,
    name: String,
    prompt: String,
    alias: Option<String>,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    let Some(p) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == prompt_id)
    else {
        return Err("Prompt not found.".to_string());
    };
    p.name = name;
    p.prompt = prompt;
    p.alias = alias;
    write_settings(&app, settings);
    Ok(())
}

fn default_post_process_prompts() -> Vec<LLMPrompt> {
    vec![
        LLMPrompt {
            id: "korero_clean_transcript".to_string(),
            name: "Clean transcript (NZ English)".to_string(),
            alias: Some("clean".to_string()),
            // Kōrero (v1.7.0): added rule 6 (coherent prose); relaxed "word order"
            // constraint to "do not add content" so rule 6 can smooth sentence
            // boundaries without conflicting with the earlier "do not reorder" guard.
            // Migration in ensure_post_process_defaults() upgrades existing installs
            // that still carry the v1.6.0 default text.
            prompt: "Clean this transcript using NZ English spelling (colour, organise, whānau, etc.):\n1. Fix spelling, capitalisation, and punctuation errors\n2. Convert number words to digits (twenty-five → 25, ten percent → 10%, five dollars → $5)\n3. Replace spoken punctuation with symbols (period → ., comma → ,, question mark → ?)\n4. Remove filler words (um, uh, like as filler)\n5. Preserve te reo Māori words exactly as spoken\n6. Ensure sentences and paragraphs read as coherent prose — join fragments that clearly belong together and smooth any sentence boundaries broken by dictation\n\nPreserve exact meaning. Do not add content or invent details. Use NZ English throughout.\n\nReturn only the cleaned transcript.\n\nTranscript:\n${output}".to_string(),
        },
        LLMPrompt {
            id: "korero_client_email".to_string(),
            name: "Client email body".to_string(),
            alias: Some("email".to_string()),
            prompt: "Convert this dictation into a direct, warm-but-professional email body for a client. Use NZ English. Keep all specifics. Do NOT add a greeting or signoff — the sender will add those. Drop filler words and false starts. Return only the email body text.\n\nDictation:\n${output}".to_string(),
        },
        LLMPrompt {
            id: "korero_whatsapp_slack".to_string(),
            name: "WhatsApp / Slack message".to_string(),
            alias: Some("chat".to_string()),
            prompt: "Convert this dictation into a terse Slack or WhatsApp message. NZ English, sentence case, no formatting, no preamble. Drop all filler. Keep it under 3 sentences if at all possible. Return only the message text.\n\nDictation:\n${output}".to_string(),
        },
        LLMPrompt {
            id: "korero_meeting_note".to_string(),
            name: "Meeting note (Decision/Action/Question)".to_string(),
            alias: Some("meeting".to_string()),
            prompt: "Restructure this dictation as a meeting note with three sections:\n\n**Decisions:** (what was decided)\n**Actions:** (who does what, by when)\n**Open questions:** (what's still unresolved)\n\nUse NZ English. Drop chronological filler. Each bullet should be one sentence. Return only the structured note.\n\nDictation:\n${output}".to_string(),
        },
        // Kōrero (v1.16.0): out-of-the-box prompt for dictation that blends
        // NZ English and te reo Māori — restores macrons, never translates
        // te reo, fixes common mis-hearings. Merged into existing installs by
        // the prompt-defaults sync.
        LLMPrompt {
            id: "korero_reo_blend".to_string(),
            name: "NZ English + te reo Māori".to_string(),
            alias: Some("reo".to_string()),
            prompt: "Tidy this dictation into clear New Zealand English that naturally blends te reo Māori and English. Rules:\n\n1. Keep every te reo Māori word or phrase in te reo — never translate it to English\n2. Restore macrons (ā ē ī ō ū): Māori, whānau, kōrero, Aotearoa, hapū, iwi, marae, tikanga, taonga, kaupapa, mahi, tamariki, mokopuna, mōrena\n3. Fix obvious mis-hearings of te reo (e.g. 'far no' → 'whānau', 'koe deer' → 'kia ora', 'fakapapa' → 'whakapapa', 'curry row' → 'kōrero')\n4. Fix punctuation and sentence breaks; use New Zealand English spelling (organise, colour); keep the speaker's meaning and tone exactly\n\nReturn ONLY the corrected text — no commentary.\n\nDictation:\n${output}".to_string(),
        },
        LLMPrompt {
            id: "korero_red_team".to_string(),
            name: "Red Team review".to_string(),
            alias: Some("redteam".to_string()),
            prompt: "I just dictated a draft argument or plan. Red-team it:\n\n1. Identify the weakest premise\n2. List three counterarguments a smart critic would raise\n3. Note any missing evidence or unstated assumptions\n4. THEN return the cleaned-up version of my dictation (NZ English)\n\nBe direct and clinical. Do not flatter. Format as four short sections with bold labels.\n\nDictation:\n${output}".to_string(),
        },
    ]
}

fn default_whisper_gpu_device() -> i32 {
    -1 // auto
}

fn default_typing_tool() -> TypingTool {
    TypingTool::Auto
}

fn ensure_post_process_defaults(settings: &mut AppSettings) -> bool {
    let mut changed = false;
    for provider in default_post_process_providers() {
        // Use match to do a single lookup - either sync existing or add new
        match settings
            .post_process_providers
            .iter_mut()
            .find(|p| p.id == provider.id)
        {
            Some(existing) => {
                // Sync supports_structured_output field for existing providers (migration)
                if existing.supports_structured_output != provider.supports_structured_output {
                    debug!(
                        "Updating supports_structured_output for provider '{}' from {} to {}",
                        provider.id,
                        existing.supports_structured_output,
                        provider.supports_structured_output
                    );
                    existing.supports_structured_output = provider.supports_structured_output;
                    changed = true;
                }
                // Kōrero (v1.3.0): sync suggested_models and is_local_provider.
                // These fields are new in v1.3.0; existing deserialized providers
                // have empty/false defaults from #[serde(default)]. Replace with
                // the canonical default values so upgrades get the full model list
                // (Gemini) and the Local badge + pull workflow (Ollama).
                if existing.suggested_models != provider.suggested_models {
                    existing.suggested_models = provider.suggested_models.clone();
                    changed = true;
                }
                if existing.is_local_provider != provider.is_local_provider {
                    existing.is_local_provider = provider.is_local_provider;
                    changed = true;
                }
            }
            None => {
                // Provider doesn't exist, add it
                settings.post_process_providers.push(provider.clone());
                changed = true;
            }
        }

        if !settings.post_process_api_keys.contains_key(&provider.id) {
            settings
                .post_process_api_keys
                .insert(provider.id.clone(), String::new());
            changed = true;
        }

        let default_model = default_model_for_provider(&provider.id);
        match settings.post_process_models.get_mut(&provider.id) {
            Some(existing) => {
                // Korero (v1.2.0): auto-upgrade gemini-2.0-flash -> gemini-2.5-flash.
                // 2.5 is faster and higher quality. Users who have manually changed
                // their model to something other than the old default are unaffected.
                if provider.id == "gemini" && existing.as_str() == "gemini-2.0-flash" {
                    debug!("Migrating gemini model from gemini-2.0-flash to gemini-2.5-flash");
                    *existing = default_model.clone();
                    changed = true;
                } else if existing.is_empty() && !default_model.is_empty() {
                    *existing = default_model.clone();
                    changed = true;
                }
            }
            None => {
                settings
                    .post_process_models
                    .insert(provider.id.clone(), default_model);
                changed = true;
            }
        }
    }

    // Kōrero (v1.7.0): migrate built-in prompt text for existing users.
    // For each default prompt: if the user's stored prompt still carries the
    // previous default text (sentinel check), upgrade it to the current version.
    // Users who edited their prompt away from the default are unaffected.
    //
    // Current migration: korero_clean_transcript — adds rule 6 (coherent prose).
    // Detection: the v1.6.0 default contained "Preserve exact meaning and word order."
    // as its constraint sentence; the v1.7.0 text uses "Preserve exact meaning." only.
    for default_prompt in &default_post_process_prompts() {
        match settings
            .post_process_prompts
            .iter_mut()
            .find(|p| p.id == default_prompt.id)
        {
            Some(existing) => {
                if existing.id == "korero_clean_transcript"
                    && existing.prompt.contains("Preserve exact meaning and word order.")
                {
                    debug!("Migrating korero_clean_transcript prompt to v1.7.0 (rule 6 + relaxed word-order constraint)");
                    existing.prompt = default_prompt.prompt.clone();
                    changed = true;
                }
            }
            None => {
                // Prompt ID missing entirely — add it
                settings.post_process_prompts.push(default_prompt.clone());
                changed = true;
            }
        }
    }

    changed
}

fn default_custom_words() -> Vec<String> {
    // Kōrero: generic NZ-English seed dictionary. Improves recognition of
    // te reo Māori and NZ-isms that Whisper/Parakeet routinely mis-transcribe.
    // Contains NO personal data (see 2026-05-30 note below). Edit via
    // Settings → Custom Words UI to add your own names/companies locally.
    vec![
        // Kōrero (2026-05-30): personal People + Companies/clients entries were
        // removed from this shipped default so the public build / installer never
        // discloses the maintainer's contacts. Add your own via Settings -> Custom Words.
        // Generic tooling / product terms (no personal data)
        "Monday.com", "Copilot", "M365", "OneDrive", "Replit",
        "Cowork", "Fireflies", "Tauri", "MCP", "Anthropic", "Kōrero",
        // Te reo Māori
        "whānau", "mihi", "kōrero", "mahi", "Aotearoa",
        "Tāmaki", "iwi", "hapū", "tangata",
        // NZ-isms / acronyms
        "GST", "IRD", "ACC", "EPA", "FY26", "FY27",
        "KiwiSaver", "Plunket", "Te Whatu Ora",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Kōrero (v1.7.0, S1): in-memory settings cache registered as Tauri managed
/// state.  After the setup hook seeds this from the initial disk+keychain load,
/// every `get_settings()` call reads from RAM instead of hitting the store file
/// and the OS keychain.  `write_settings()` updates both the cache and disk
/// atomically so readers always see a consistent value.
pub struct SettingsCache(pub Arc<RwLock<AppSettings>>);

pub const SETTINGS_STORE_PATH: &str = "settings_store.json";

pub fn get_default_settings() -> AppSettings {
    #[cfg(target_os = "windows")]
    let default_shortcut = "ctrl+space";
    #[cfg(target_os = "macos")]
    let default_shortcut = "option+space";
    #[cfg(target_os = "linux")]
    let default_shortcut = "ctrl+space";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let default_shortcut = "alt+space";

    let mut bindings = HashMap::new();
    bindings.insert(
        "transcribe".to_string(),
        ShortcutBinding {
            id: "transcribe".to_string(),
            name: "Transcribe".to_string(),
            description: "Converts your speech into text.".to_string(),
            default_binding: default_shortcut.to_string(),
            current_binding: default_shortcut.to_string(),
        },
    );
    // Kōrero (v1.14.1): alternative dictation shortcut, chosen to be pressable
    // with the RIGHT hand alone (Right Ctrl + Right Shift + Enter) for
    // one-handed use — e.g. holding the baby with the left. NOTE: the OS
    // shortcut layer doesn't distinguish left/right modifiers, so this must
    // not collide with other generic combos (ctrl+shift+space is taken by
    // post-processing). Same action as "Transcribe"; rebindable in General.
    // Missing-binding merge in load_or_create_app_settings adds this to
    // existing installs automatically.
    bindings.insert(
        "transcribe_alt".to_string(),
        ShortcutBinding {
            id: "transcribe_alt".to_string(),
            name: "Transcribe (alternative)".to_string(),
            description: "A second shortcut for dictation — handy one-handed (default: Right Ctrl + Right Shift + Enter).".to_string(),
            default_binding: "ctrl+shift+enter".to_string(),
            current_binding: "ctrl+shift+enter".to_string(),
        },
    );
    #[cfg(target_os = "windows")]
    let default_post_process_shortcut = "ctrl+shift+space";
    #[cfg(target_os = "macos")]
    let default_post_process_shortcut = "option+shift+space";
    #[cfg(target_os = "linux")]
    let default_post_process_shortcut = "ctrl+shift+space";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let default_post_process_shortcut = "alt+shift+space";

    bindings.insert(
        "transcribe_with_post_process".to_string(),
        ShortcutBinding {
            id: "transcribe_with_post_process".to_string(),
            name: "Transcribe with Post-Processing".to_string(),
            description: "Converts your speech into text and applies AI post-processing."
                .to_string(),
            default_binding: default_post_process_shortcut.to_string(),
            current_binding: default_post_process_shortcut.to_string(),
        },
    );
    bindings.insert(
        "cancel".to_string(),
        ShortcutBinding {
            id: "cancel".to_string(),
            name: "Cancel".to_string(),
            description: "Cancels the current recording.".to_string(),
            default_binding: "escape".to_string(),
            current_binding: "escape".to_string(),
        },
    );

    AppSettings {
        bindings,
        push_to_talk: true,
        audio_feedback: false,
        audio_feedback_volume: default_audio_feedback_volume(),
        sound_theme: default_sound_theme(),
        start_hidden: default_start_hidden(),
        autostart_enabled: default_autostart_enabled(),
        update_checks_enabled: default_update_checks_enabled(),
        selected_model: "".to_string(),
        always_on_microphone: false,
        selected_microphone: None,
        clamshell_microphone: None,
        selected_output_device: None,
        translate_to_english: false,
        selected_language: default_selected_language(),
        overlay_position: default_overlay_position(),
        debug_mode: false,
        log_level: default_log_level(),
        custom_words: default_custom_words(),
        transcript_corrections: Vec::new(),
        denoise_enabled: false,
        save_crash_reports: default_save_crash_reports(),
        model_unload_timeout: ModelUnloadTimeout::default(),
        word_correction_threshold: default_word_correction_threshold(),
        history_limit: default_history_limit(),
        recording_retention_period: default_recording_retention_period(),
        paste_method: PasteMethod::default(),
        clipboard_handling: ClipboardHandling::default(),
        auto_submit: default_auto_submit(),
        auto_submit_key: AutoSubmitKey::default(),
        post_process_enabled: default_post_process_enabled(),
        post_process_provider_id: default_post_process_provider_id(),
        post_process_providers: default_post_process_providers(),
        post_process_api_keys: default_post_process_api_keys(),
        post_process_models: default_post_process_models(),
        post_process_prompts: default_post_process_prompts(),
        post_process_selected_prompt_id: Some("korero_clean_transcript".to_string()),
        mute_while_recording: false,
        append_trailing_space: true,  // Kōrero: smoother inline dictation
        app_language: default_app_language(),
        experimental_enabled: false,
        lazy_stream_close: false,
        keyboard_implementation: KeyboardImplementation::default(),
        show_tray_icon: default_show_tray_icon(),
        paste_delay_ms: default_paste_delay_ms(),
        typing_tool: default_typing_tool(),
        external_script_path: None,
        custom_filler_words: None,
        whisper_accelerator: WhisperAcceleratorSetting::default(),
        ort_accelerator: OrtAcceleratorSetting::default(),
        whisper_gpu_device: default_whisper_gpu_device(),
        extra_recording_buffer_ms: 0,
    }
}

impl AppSettings {
    pub fn active_post_process_provider(&self) -> Option<&PostProcessProvider> {
        self.post_process_providers
            .iter()
            .find(|provider| provider.id == self.post_process_provider_id)
    }

    pub fn post_process_provider(&self, provider_id: &str) -> Option<&PostProcessProvider> {
        self.post_process_providers
            .iter()
            .find(|provider| provider.id == provider_id)
    }

    pub fn post_process_provider_mut(
        &mut self,
        provider_id: &str,
    ) -> Option<&mut PostProcessProvider> {
        self.post_process_providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
    }
}

pub fn load_or_create_app_settings(app: &AppHandle) -> AppSettings {
    // Initialize store
    let store = app
        .store(crate::portable::store_path(SETTINGS_STORE_PATH))
        .expect("Failed to initialize store");

    let mut settings = if let Some(settings_value) = store.get("settings") {
        // Parse the entire settings object
        match serde_json::from_value::<AppSettings>(settings_value) {
            Ok(mut settings) => {
                debug!("Found existing settings: {:?}", settings);
                let default_settings = get_default_settings();
                let mut updated = false;

                // Merge default bindings into existing settings
                for (key, value) in default_settings.bindings {
                    if !settings.bindings.contains_key(&key) {
                        debug!("Adding missing binding: {}", key);
                        settings.bindings.insert(key, value);
                        updated = true;
                    }
                }

                if updated {
                    debug!("Settings updated with new bindings");
                    store.set("settings", serde_json::to_value(&settings).unwrap());
                }

                settings
            }
            Err(e) => {
                warn!("Failed to parse settings: {}", e);
                // Fall back to default settings if parsing fails
                let default_settings = get_default_settings();
                store.set("settings", serde_json::to_value(&default_settings).unwrap());
                default_settings
            }
        }
    } else {
        let default_settings = get_default_settings();
        store.set("settings", serde_json::to_value(&default_settings).unwrap());
        default_settings
    };

    if ensure_post_process_defaults(&mut settings) {
        store.set("settings", serde_json::to_value(&settings).unwrap());
    }

    // Kōrero: one-shot migration of any plaintext keys still in the JSON,
    // then hydrate from the keychain so the returned struct has live keys.
    // The custom Serialize impl on SecretMap guarantees the next disk write
    // will not re-emit plaintext.
    //
    // BEFORE migrating, back up settings_store.json so a partial keychain
    // failure can't silently destroy the user's plaintext keys. Migration
    // blanks values per-provider; if one provider's keychain write succeeds
    // but another fails, the succeeded provider's plaintext is gone forever
    // without this backup. The backup is taken only on the FIRST migration
    // attempt — once .pre-keychain.json.bak exists, we leave it alone so
    // a successful migration doesn't itself overwrite the safety net.
    if settings
        .post_process_api_keys
        .values()
        .any(|v| !v.is_empty())
    {
        if let Err(err) = backup_settings_before_keychain_migration(app) {
            warn!(
                "secret_store: pre-migration backup failed ({err}). Aborting migration to preserve plaintext."
            );
            // No event here — the plaintext is preserved and the user can
            // retry next launch. Emitting would be noise on every startup
            // until the backup path becomes writable.
        } else {
            let (migrated, failures) =
                settings.post_process_api_keys.migrate_plaintext_to_keyring();
            if migrated {
                debug!("secret_store: rewriting settings store after plaintext migration");
                store.set("settings", serde_json::to_value(&settings).unwrap());
            }
            emit_keychain_error(app, "migrate", failures);
        }
    }
    settings.post_process_api_keys.hydrate_from_keyring();

    // Kōrero (v1.11.0): seed the denoiser flag from the loaded setting at startup
    // (write_settings keeps it in sync thereafter).
    crate::denoise::set_enabled(settings.denoise_enabled);

    settings
}

/// Copy `settings_store.json` to `settings_store.pre-keychain.json.bak`
/// before keychain migration runs. Idempotent — preserves the first-ever
/// backup so a successful migration never overwrites the safety net.
///
/// Returns Err if the source can't be located OR the copy itself fails. The
/// caller treats a backup failure as a hard stop on migration (better to keep
/// plaintext on disk than lose it irrecoverably).
fn backup_settings_before_keychain_migration(app: &AppHandle) -> Result<(), String> {
    let src = crate::portable::resolve_app_data(app, SETTINGS_STORE_PATH)
        .map_err(|e| format!("could not resolve settings path: {e}"))?;
    if !src.exists() {
        // Clean install, nothing to back up. Treat as success — the migration
        // path will find no plaintext anyway and immediately no-op.
        return Ok(());
    }
    let backup = src.with_file_name("settings_store.pre-keychain.json.bak");
    if backup.exists() {
        debug!(
            "secret_store: keychain migration backup already exists at {:?}, leaving it untouched",
            backup
        );
        return Ok(());
    }
    std::fs::copy(&src, &backup)
        .map_err(|e| format!("failed to copy {:?} -> {:?}: {e}", src, backup))?;
    debug!(
        "secret_store: keychain migration backup written to {:?}",
        backup
    );
    Ok(())
}

pub fn get_settings(app: &AppHandle) -> AppSettings {
    // Kōrero (v1.7.0, S1): fast path — read from in-memory cache when available.
    // The cache is seeded in the setup hook after the first disk+keychain load.
    // try_state() returns None only during that initial load itself, so the slow
    // path below runs exactly once per process lifetime.
    if let Some(cache) = app.try_state::<SettingsCache>() {
        return cache
            .0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
    }

    // Slow path: disk + OS keychain.  Used only during setup before the cache
    // is registered.
    let store = app
        .store(crate::portable::store_path(SETTINGS_STORE_PATH))
        .expect("Failed to initialize store");

    let mut settings = if let Some(settings_value) = store.get("settings") {
        serde_json::from_value::<AppSettings>(settings_value).unwrap_or_else(|_| {
            let default_settings = get_default_settings();
            store.set("settings", serde_json::to_value(&default_settings).unwrap());
            default_settings
        })
    } else {
        let default_settings = get_default_settings();
        store.set("settings", serde_json::to_value(&default_settings).unwrap());
        default_settings
    };

    if ensure_post_process_defaults(&mut settings) {
        store.set("settings", serde_json::to_value(&settings).unwrap());
    }

    // Kōrero: hydrate API keys from the OS keychain on every read. The disk
    // copy is always blanked by SecretMap::serialize, so this is the only path
    // by which the frontend / LLM client sees real secret material.
    settings.post_process_api_keys.hydrate_from_keyring();

    settings
}

pub fn write_settings(app: &AppHandle, settings: AppSettings) {
    // Kōrero (v1.11.0): keep the denoiser's process-global flag in sync with the
    // persisted setting on every write (read before `settings` is moved below).
    crate::denoise::set_enabled(settings.denoise_enabled);

    let store = app
        .store(crate::portable::store_path(SETTINGS_STORE_PATH))
        .expect("Failed to initialize store");

    // Kōrero: write secrets to the keychain BEFORE writing the JSON. The
    // SecretMap Serialize impl writes empty strings to disk regardless, so the
    // ordering is purely belt-and-braces — if anything reads the JSON between
    // the two writes it will get blanks, not plaintext.
    let failures = settings.post_process_api_keys.persist_to_keyring();
    emit_keychain_error(app, "save", failures);
    store.set("settings", serde_json::to_value(&settings).unwrap());

    // Kōrero (v1.7.0, S1): update the in-memory cache so subsequent
    // get_settings() calls reflect the new values without a disk round-trip.
    if let Some(cache) = app.try_state::<SettingsCache>() {
        *cache.0.write().unwrap_or_else(|e| e.into_inner()) = settings;
    }
}

pub fn get_bindings(app: &AppHandle) -> HashMap<String, ShortcutBinding> {
    let settings = get_settings(app);

    settings.bindings
}

pub fn get_stored_binding(app: &AppHandle, id: &str) -> ShortcutBinding {
    let bindings = get_bindings(app);

    let binding = bindings.get(id).unwrap().clone();

    binding
}

pub fn get_history_limit(app: &AppHandle) -> usize {
    get_settings(app).history_limit
}

pub fn get_recording_retention_period(app: &AppHandle) -> RecordingRetentionPeriod {
    get_settings(app).recording_retention_period
}
