//! Kōrero fork (v1.12.0): Notes dictation commands.
//!
//! Powers the in-app Notes page. A manual dictate button records audio, then
//! transcribes it (optionally running the post-processing prompt) and RETURNS
//! the text to the frontend so it can be inserted into the note canvas. This is
//! deliberately different from the global shortcut flow, which pastes into the
//! focused app and writes a History entry — here the text is simply handed
//! back, nothing is pasted and nothing is persisted server-side.
//!
//! A dedicated recording binding id keeps this path from ever colliding with
//! the "transcribe" / "transcribe_with_post_process" shortcut recordings.

use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::managers::audio::AudioRecordingManager;
use crate::managers::transcription::TranscriptionManager;

/// Recording binding id reserved for the Notes page.
const NOTE_BINDING: &str = "korero_note_dictation";

/// Begin a note dictation. Refuses if any recording is already active so the
/// note button and the global shortcuts can never interrupt each other.
#[tauri::command]
#[specta::specta]
pub async fn note_start_dictation(
    recording_manager: State<'_, Arc<AudioRecordingManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
) -> Result<(), String> {
    if recording_manager.is_recording() {
        return Err("A recording is already in progress.".to_string());
    }
    // v1.14.4: the global dictation shortcuts are guarded against an active
    // meeting (actions.rs), but this button takes a different path — without
    // this check it would open a SECOND stream on the same microphone and
    // fight the meeting capture for the device.
    if crate::meeting::is_meeting_active() {
        return Err(
            "A meeting is being recorded — stop it before dictating a note.".to_string(),
        );
    }
    // Pre-warm the model so transcription is ready the moment dictation stops.
    transcription_manager.initiate_model_load();
    recording_manager.try_start_recording(NOTE_BINDING)
}

/// Stop the note dictation, transcribe the captured audio, optionally run the
/// active post-processing prompt, and return the resulting text. Returns an
/// empty string when there was no audio.
#[tauri::command]
#[specta::specta]
pub async fn note_stop_dictation(
    app: AppHandle,
    recording_manager: State<'_, Arc<AudioRecordingManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    post_process: bool,
) -> Result<String, String> {
    let samples = match recording_manager.stop_recording(NOTE_BINDING) {
        Some(s) => s,
        None => return Ok(String::new()),
    };
    if samples.is_empty() {
        return Ok(String::new());
    }

    // Transcription is CPU-bound and blocking; run it off the async worker so
    // we don't stall the runtime while a long note is processed.
    let tm = transcription_manager.inner().clone();
    let raw = tauri::async_runtime::spawn_blocking(move || tm.transcribe(samples))
        .await
        .map_err(|e| format!("Transcription task failed: {e}"))?
        .map_err(|e| e.to_string())?;

    if raw.trim().is_empty() {
        return Ok(String::new());
    }

    // Reuse the shared text pipeline (Chinese-variant conversion + optional
    // post-processing). No paste, no History write — just hand the text back.
    let processed = crate::actions::process_transcription_output(&app, &raw, post_process).await;
    Ok(processed.final_text)
}

/// Discard the active note recording (if any) without transcribing it.
#[tauri::command]
#[specta::specta]
pub async fn note_cancel_dictation(
    recording_manager: State<'_, Arc<AudioRecordingManager>>,
) -> Result<(), String> {
    let _ = recording_manager.stop_recording(NOTE_BINDING);
    Ok(())
}

/// Kōrero (v1.14.3): post-process the WHOLE note with the active provider, a
/// caller-chosen prompt, and an optional model override (defaults to the
/// model configured for the provider under Post Process). Powers both
/// "Transcribe + clean up" (which now cleans the entire note, not just the
/// new snippet) and the re-runnable "Process note" action.
///
/// Prompts that contain the `${output}` placeholder (the dictation-pipeline
/// convention) get the note substituted INTO the prompt as the user message;
/// otherwise the prompt is the system message and the note the user message,
/// matching the meeting post-process flow.
#[tauri::command]
#[specta::specta]
pub async fn note_post_process(
    app: AppHandle,
    text: String,
    prompt: String,
    model: Option<String>,
) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| {
            "No post-processing provider is configured. Set one under Post Process.".to_string()
        })?;
    let configured_model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    let model = match model {
        Some(m) if !m.trim().is_empty() => m,
        _ => configured_model,
    };
    if model.trim().is_empty() {
        return Err(format!(
            "No model is configured for provider '{}'. Set one under Post Process.",
            provider.id
        ));
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    // Cap the note so it can't blow the model's context window.
    const MAX_CHARS: usize = 48_000;
    let text = if text.chars().count() > MAX_CHARS {
        let kept: String = text.chars().take(MAX_CHARS).collect();
        format!("{kept}\n\n[Note truncated to fit the model's context window.]")
    } else {
        text
    };

    let prompt = prompt.trim().to_string();
    let default_prompt = "Clean up this dictated note: fix punctuation, sentence breaks, and \
                          obvious mis-hearings; keep the meaning, structure, and tone; use NZ \
                          English spelling. Return ONLY the revised note text.";
    let (system, user) = if prompt.contains("${output}") {
        // Dictation-style prompt: the note goes INTO the prompt template.
        (None, prompt.replace("${output}", &text))
    } else if prompt.is_empty() {
        (Some(default_prompt.to_string()), text)
    } else {
        (Some(prompt), text)
    };

    // v1.15.0: teach the model the user's known mis-transcriptions so it
    // fixes near-miss variants the deterministic pass can't catch.
    let system = match (
        system,
        crate::corrections::glossary_block(&settings.transcript_corrections),
    ) {
        (Some(s), Some(g)) => Some(format!("{s}{g}")),
        (None, Some(g)) => Some(format!("You are revising a transcribed note.{g}")),
        (s, None) => s,
    };

    let answer = crate::llm_client::send_chat_completion_with_schema(
        &provider, api_key, &model, user, system, None, None, None,
    )
    .await?;
    answer.ok_or_else(|| "The model returned no output.".to_string())
}
