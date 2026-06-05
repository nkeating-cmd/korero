//! Kōrero (v1.4.0): Extended history commands for inline correction.
//!
//! update_post_processed_text — lets the user correct the LLM post-processed
//! output directly from the History panel, without re-running transcription.
//! The corrected text is persisted to SQLite and a HistoryUpdatePayload::Updated
//! event is emitted so the frontend state refreshes in real time.

use crate::managers::history::HistoryManager;
use std::sync::Arc;
use tauri::{AppHandle, State};

/// Update only the post-processed text for a history entry.
///
/// Used by the History panel's inline correction UI so the user can fix
/// LLM output mistakes without discarding the original Whisper transcription.
/// Internally delegates to HistoryManager::update_post_processed_text which
/// runs a targeted SQL UPDATE and emits HistoryUpdatePayload::Updated.
#[tauri::command]
#[specta::specta]
pub async fn update_post_processed_text(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
    text: String,
) -> Result<(), String> {
    history_manager
        .update_post_processed_text(id, text)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
