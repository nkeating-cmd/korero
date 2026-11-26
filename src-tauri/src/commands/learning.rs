//! Kōrero 1.52 (B14): the commands behind learning from History edits. The rules live in
//! `crate::learning`, which is pure; this file only reads and writes settings.

use crate::learning::{self, LearnOutcome, LearnedPair};
use crate::settings::{get_settings, write_settings};
use tauri::AppHandle;

/// Called when the user saves a correction in Dictation history. `original` is the dictation
/// as heard, `before` the text they started from, `after` what they saved. Nothing is kept
/// but the replaced words themselves, a count and the day. Never logs any text.
#[tauri::command]
#[specta::specta]
pub fn learn_from_edit(
    app: AppHandle,
    original: String,
    before: String,
    after: String,
) -> Result<LearnOutcome, String> {
    let mut settings = get_settings(&app);
    if !settings.learning_enabled {
        return Ok(LearnOutcome::default());
    }
    let taught: Vec<String> = settings
        .transcript_corrections
        .iter()
        .map(|c| c.wrong.clone())
        .collect();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let was = settings.learned_pairs.clone();
    let outcome = learning::observe(
        &mut settings.learned_pairs,
        &original,
        &before,
        &after,
        &taught,
        &today,
        &|w| crate::audio_toolkit::text::is_common_en(w),
    );
    if settings.learned_pairs != was {
        write_settings(&app, settings);
    }
    log::info!(
        "learning: {} learnt, {} watching, {} switched off",
        outcome.learnt.len(),
        outcome.watching,
        outcome.switched_off.len()
    );
    Ok(outcome)
}

/// The Settings page's edits to the list (switch off, switch back on, forget). Returns the
/// list as stored.
#[tauri::command]
#[specta::specta]
pub fn update_learned_pairs(
    app: AppHandle,
    pairs: Vec<LearnedPair>,
) -> Result<Vec<LearnedPair>, String> {
    let clean = learning::sanitise(pairs);
    let mut settings = get_settings(&app);
    settings.learned_pairs = clean.clone();
    write_settings(&app, settings);
    Ok(clean)
}

/// The "Learn from my corrections" switch. Off: nothing is learnt and no learnt rule is
/// applied; the list is kept so switching back on restores it.
#[tauri::command]
#[specta::specta]
pub fn change_learning_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.learning_enabled = enabled;
    write_settings(&app, settings);
    Ok(())
}
