//! Kōrero 1.52 (B14): the commands behind learning from History edits. The rules live in
//! `crate::learning`, which is pure; this file only reads and writes settings.

use crate::learning::{self, LearnOutcome, LearnedPair, Lexicon};
use crate::settings::{get_settings, write_settings};
use tauri::AppHandle;

/// The app's own word lists: the ordinary-English list the custom-word matcher uses, and the
/// te reo words whose macron-free form is a different word.
fn real_lexicon() -> Lexicon<'static> {
    Lexicon {
        common: &|w| crate::audio_toolkit::text::is_common_en(w),
        ambiguous: &|w| crate::audio_toolkit::reo_lexicon::is_ambiguous_bare(w),
    }
}

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
        &real_lexicon(),
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

#[cfg(test)]
mod tests {
    use super::real_lexicon;
    use crate::learning::edit_pairs;

    fn pairs_of(before: &str, after: &str) -> Vec<(String, String)> {
        edit_pairs(before, after, &real_lexicon())
    }

    /// The rules against the app's real word lists (the tests in `learning.rs` use small
    /// stand-ins): what is learnt, and what the independent review showed must not be.
    #[test]
    fn the_real_word_lists_keep_ordinary_words_and_te_reo_safe() {
        let one = |h: &str, m: &str| vec![(h.to_string(), m.to_string())];
        assert_eq!(
            pairs_of("working with foreign emergency today", "working with Fire and Emergency today"),
            one("foreign emergency", "Fire and Emergency")
        );
        assert_eq!(pairs_of("we met the fens crew", "we met the FENZ crew"), one("fens", "FENZ"));
        for (b, a) in [
            ("who is on call", "WHO is on call"),
            ("tell us about it", "tell US about it"),
            ("ask it for help", "ask IT for help"),
            ("we met at the bank", "we met at the Bank"),
            ("we may go with may", "we may go with Mae"),
            ("he tangata pai", "he tāngata pai"),
            ("te wahine nei", "te wāhine nei"),
            ("nga matua katoa", "nga mātua katoa"),
            ("he kaka tera", "he kākā tera"),
            ("the costs are high", "the cost is high"),
            ("it is over their", "it is over there"),
        ] {
            assert_eq!(pairs_of(b, a), Vec::new(), "{b:?} -> {a:?}");
        }
    }

    /// Learnt pairs are not taught corrections: the glossary given to the notes model is
    /// built from `transcript_corrections` alone, so a learnt pair is in no prompt.
    #[test]
    fn learnt_pairs_are_in_no_glossary() {
        let mut s = crate::settings::get_default_settings();
        s.learned_pairs = vec![crate::learning::LearnedPair {
            heard: "foreign emergency".into(),
            meant: "Fire and Emergency".into(),
            seen: 5,
            off: false,
            last_seen: String::new(),
        }];
        let glossary = crate::corrections::glossary_block(&s.transcript_corrections);
        assert!(!glossary.unwrap_or_default().contains("Emergency"));
        assert!(s.learning_enabled, "learning is on by default");
    }
}
