use crate::actions::process_transcription_output;
use crate::managers::{
    history::{HistoryManager, PaginatedHistory},
    transcription::TranscriptionManager,
};
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
#[specta::specta]
pub async fn get_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    history_manager
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_history_entry_saved(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_audio_file_path(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    file_name: String,
) -> Result<String, String> {
    let path = history_manager.get_audio_file_path(&file_name);
    path.to_str()
        .ok_or_else(|| "Invalid file path".to_string())
        .map(|s| s.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_history_entry(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_transcription(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    id: i64,
) -> Result<(), String> {
    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {} not found", id))?;

    let audio_path = history_manager.get_audio_file_path(&entry.file_name);
    let samples = crate::audio_toolkit::read_wav_samples(&audio_path)
        .map_err(|e| format!("Failed to load audio: {}", e))?;

    if samples.is_empty() {
        return Err("Recording has no audio samples".to_string());
    }

    transcription_manager.initiate_model_load();

    let tm = Arc::clone(&transcription_manager);
    let transcription = tauri::async_runtime::spawn_blocking(move || tm.transcribe(samples))
        .await
        .map_err(|e| format!("Transcription task panicked: {}", e))?
        .map_err(|e| e.to_string())?;

    if transcription.is_empty() {
        return Err("Recording contains no speech".to_string());
    }

    let processed =
        process_transcription_output(&app, &transcription, entry.post_process_requested).await;
    history_manager
        .update_transcription(
            id,
            transcription,
            processed.post_processed_text,
            processed.post_process_prompt,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn update_history_limit(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    limit: usize,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.history_limit = limit;
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_period(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: String,
) -> Result<(), String> {
    use crate::settings::RecordingRetentionPeriod;

    let retention_period = match period.as_str() {
        "never" => RecordingRetentionPeriod::Never,
        "preserve_limit" => RecordingRetentionPeriod::PreserveLimit,
        "days3" => RecordingRetentionPeriod::Days3,
        "weeks2" => RecordingRetentionPeriod::Weeks2,
        "months3" => RecordingRetentionPeriod::Months3,
        _ => return Err(format!("Invalid retention period: {}", period)),
    };

    let mut settings = crate::settings::get_settings(&app);
    settings.recording_retention_period = retention_period;
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Kōrero (v1.40.0, M5 / backlog T7): "Tidy te reo" — run the `korero_reo_blend`
// post-processing prompt on ONE history entry, on demand, against a LOOPBACK
// Ollama only. Never changes the post-process default; never reads the
// enablement flag; the original transcription is preserved.
// ---------------------------------------------------------------------------

/// SEC-02: "local" must be a property of the URL, not of the provider's
/// `is_local_provider` flag (the Ollama base URL is user-editable). Only a
/// loopback host qualifies.
pub(crate) fn is_loopback_url(base_url: &str) -> bool {
    let without_scheme = base_url
        .trim()
        .strip_prefix("http://")
        .or_else(|| base_url.trim().strip_prefix("https://"))
        .unwrap_or(base_url.trim());
    let host_port = without_scheme.split('/').next().unwrap_or("");
    // Strip an IPv6 bracket form or a trailing :port.
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port)
    };
    let host = host.to_ascii_lowercase();
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

pub const REO_BLEND_PROMPT_ID: &str = "korero_reo_blend";

#[tauri::command]
#[specta::specta]
pub async fn tidy_history_entry_reo(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<crate::managers::history::HistoryEntry, String> {
    let settings = crate::settings::get_settings(&app);

    // The prompt is looked up by id even when post-processing is off.
    let prompt = settings
        .post_process_prompts
        .iter()
        .find(|p| p.id == REO_BLEND_PROMPT_ID)
        .cloned()
        .ok_or_else(|| {
            "The 'NZ English + te reo Māori' prompt is not present in settings".to_string()
        })?;

    // Loopback Ollama only (SEC-02).
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == "ollama")
        .cloned()
        .ok_or_else(|| "Ollama provider is not configured".to_string())?;
    if !is_loopback_url(&provider.base_url) {
        return Err(format!(
            "Tidy te reo only runs against a local (loopback) Ollama; the configured URL is {}",
            provider.base_url
        ));
    }
    let model = settings
        .post_process_models
        .get("ollama")
        .cloned()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| "No local Ollama model selected in Post-processing settings".to_string())?;
    if !crate::commands::ollama::is_reachable(&provider.base_url).await {
        return Err("Ollama is not running".to_string());
    }

    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {id} not found"))?;

    // Always from the ORIGINAL transcription, never from an earlier post-process.
    let processed_prompt = prompt
        .prompt
        .replace("${output}", &entry.transcription_text);
    let api_key = settings
        .post_process_api_keys
        .get("ollama")
        .cloned()
        .unwrap_or_default();
    let tidied = crate::llm_client::send_chat_completion(
        &provider,
        api_key,
        &model,
        processed_prompt,
        None,
        None,
    )
    .await?
    .ok_or_else(|| "The model returned no content".to_string())?;

    history_manager
        .update_post_processed_text(id, tidied)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tidy_reo_tests {
    use super::is_loopback_url;

    #[test]
    fn loopback_urls_are_local() {
        for u in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:11434/v1",
            "https://LOCALHOST/v1",
            "localhost:11434",
        ] {
            assert!(is_loopback_url(u), "{u} should be loopback");
        }
    }

    #[test]
    fn non_loopback_urls_are_not_local() {
        for u in [
            "http://192.168.1.20:11434/v1",
            "https://ollama.example.com/v1",
            "http://mybox.local:11434/v1",
            "http://localhost.evil.com/v1",
            "",
        ] {
            assert!(!is_loopback_url(u), "{u} must not count as loopback");
        }
    }
}
