//! Kōrero (v1.15.0): user-taught transcription corrections.
//!
//! A small, deterministic layer that fixes KNOWN mis-transcriptions
//! (wrong → right pairs the user has saved) after the fuzzy custom-words
//! pass. Hooked into `TranscriptionManager::transcribe` via a build patch, so
//! every transcription path benefits: global dictation, Notes, Meetings live
//! segments, chunked re-transcription, and WAV imports.
//!
//! The same pairs are exposed as a prompt glossary (`glossary_block`) so the
//! post-processing LLM also fixes NEAR-MISS variants the exact pass can't.

use crate::settings::TranscriptCorrection;

/// Cap the glossary so a huge corrections list can't crowd out the prompt.
const GLOSSARY_MAX: usize = 50;

/// Apply every correction as a case-insensitive, word-bounded replacement.
/// Multi-word `wrong` phrases are supported. Invalid/empty pairs are skipped.
/// Replacement preserves the user's exact `right` casing.
pub fn apply_corrections(text: &str, corrections: &[TranscriptCorrection]) -> String {
    if corrections.is_empty() || text.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    // Maintain a lowercased copy for the cheap presence pre-check, recomputed
    // only when a replacement actually changes `out`.
    let mut lower = out.to_lowercase();
    for c in corrections {
        let wrong = c.wrong.trim();
        let right = c.right.trim();
        if wrong.is_empty() || right.is_empty() || wrong.eq_ignore_ascii_case(right) {
            continue;
        }
        // Efficiency (v1.19.2): skip the (relatively expensive) regex compile
        // entirely when the term isn't even present, case-insensitively. On a
        // meeting with many live segments and a long corrections list this
        // avoids thousands of needless compilations.
        if !lower.contains(&wrong.to_lowercase()) {
            continue;
        }
        // Word-bounded + case-insensitive.
        let pattern = format!(r"(?i)\b{}\b", regex::escape(wrong));
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                // NoExpand (v1.19.2 bug fix): treat `right` as a LITERAL
                // replacement. Without it, a `$` in the user's correction
                // (a price, a $variable) is interpreted as a capture-group
                // reference and silently mangles or empties the output.
                if let std::borrow::Cow::Owned(replaced) =
                    re.replace_all(&out, regex::NoExpand(right))
                {
                    out = replaced;
                    lower = out.to_lowercase();
                }
            }
            Err(e) => {
                log::warn!("Correction '{wrong}' produced an invalid pattern: {e}");
            }
        }
    }
    out
}

/// v1.19.2 BUG FIX: persist the taught-corrections list to the backend.
///
/// Root cause of "corrections don't affect transcription": the frontend saved
/// via `updateSetting("transcript_corrections", …)`, but settingsStore had NO
/// updater for that key — so the edit only changed local React state, logged
/// "No handler for setting", and never reached the Rust settings file. The
/// transcription path (`apply_corrections`, `build_bias_prompt`,
/// `glossary_block`) reads the BACKEND list, so corrections were effectively
/// inert and were lost on restart. This command is the persistence path (same
/// shape as `update_custom_words`); settingsStore now routes the key here.
#[tauri::command]
#[specta::specta]
pub fn update_transcript_corrections(
    app: tauri::AppHandle,
    corrections: Vec<TranscriptCorrection>,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.transcript_corrections = corrections;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// A prompt-ready glossary of the corrections, for injection into
/// post-processing system prompts. None when there's nothing to add.
pub fn glossary_block(corrections: &[TranscriptCorrection]) -> Option<String> {
    let pairs: Vec<String> = corrections
        .iter()
        .filter(|c| !c.wrong.trim().is_empty() && !c.right.trim().is_empty())
        .take(GLOSSARY_MAX)
        .map(|c| format!("- \"{}\" should be \"{}\"", c.wrong.trim(), c.right.trim()))
        .collect();
    if pairs.is_empty() {
        return None;
    }
    Some(format!(
        "\n\nKnown transcription mistakes to fix wherever they appear (including close variants):\n{}",
        pairs.join("\n")
    ))
}

/// Cap the decode-time bias prompt. whisper.cpp truncates `initial_prompt` to
/// roughly its last `n_text_ctx/2` (~224) tokens; ~700 chars stays comfortably
/// inside that even on the smaller models, so the highest-signal terms are
/// never silently dropped by truncation.
const BIAS_MAX_TERMS: usize = 64;
const BIAS_MAX_CHARS: usize = 700;

/// v1.19.1: build the decode-time CONTEXT-BIASING prompt for the Whisper engine
/// — the local equivalent of Deepgram/AssemblyAI "keyterm prompting". Seeds the
/// decoder with the vocabulary the user actually cares about so the RIGHT
/// spelling is produced at SOURCE, instead of only being fixed post-hoc by
/// `apply_corrections`. Terms, in priority order:
///   1. the `right` side of every taught correction — highest signal, because
///      the user explicitly told us these are the correct forms (this closes the
///      loop: a taught "whakapapa" now also biases the model toward it), then
///   2. the custom-words list,
/// de-duplicated case-insensitively and bounded by term-count and total length
/// (the previous `custom_words.join(", ")` was UNBOUNDED and a large list would
/// silently overflow — and be truncated by — Whisper's prompt window). Returns
/// `None` when there is nothing to bias with.
pub fn build_bias_prompt(
    custom_words: &[String],
    corrections: &[TranscriptCorrection],
) -> Option<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut terms: Vec<String> = Vec::new();

    // 1. Corrections' RIGHT side first (highest signal).
    for c in corrections {
        let right = c.right.trim();
        if !right.is_empty() && seen.insert(right.to_lowercase()) {
            terms.push(right.to_string());
        }
    }
    // 2. Then the custom-words list.
    for w in custom_words {
        let w = w.trim();
        if !w.is_empty() && seen.insert(w.to_lowercase()) {
            terms.push(w.to_string());
        }
    }
    if terms.is_empty() {
        return None;
    }

    // Bound: corrections-first ordering means the most valuable terms survive
    // the cap rather than being dropped by Whisper's own truncation.
    let mut out = String::new();
    for t in terms.into_iter().take(BIAS_MAX_TERMS) {
        let sep = if out.is_empty() { "" } else { ", " };
        if out.len() + sep.len() + t.len() > BIAS_MAX_CHARS {
            break;
        }
        out.push_str(sep);
        out.push_str(&t);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corr(wrong: &str, right: &str) -> TranscriptCorrection {
        TranscriptCorrection {
            wrong: wrong.to_string(),
            right: right.to_string(),
        }
    }

    #[test]
    fn bias_prompt_none_when_empty() {
        assert!(build_bias_prompt(&[], &[]).is_none());
        assert!(build_bias_prompt(&["   ".to_string()], &[corr("", "")]).is_none());
    }

    #[test]
    fn bias_prompt_corrections_first_and_deduped() {
        let custom = vec!["whakapapa".to_string(), "Aotearoa".to_string()];
        let corrections = vec![corr("fakapapa", "whakapapa"), corr("curry row", "kōrero")];
        let p = build_bias_prompt(&custom, &corrections).unwrap();
        // Correction right-terms lead; the duplicate "whakapapa" appears once.
        assert!(p.starts_with("whakapapa, kōrero"), "got: {p}");
        assert!(p.contains("Aotearoa"));
        assert_eq!(p.to_lowercase().matches("whakapapa").count(), 1);
    }

    #[test]
    fn bias_prompt_is_bounded() {
        let big: Vec<String> = (0..500).map(|i| format!("term{i:05}")).collect();
        let p = build_bias_prompt(&big, &[]).unwrap();
        assert!(p.len() <= BIAS_MAX_CHARS, "len was {}", p.len());
    }

    #[test]
    fn corrections_dollar_right_side_is_literal() {
        // A `$` in the replacement must NOT be treated as a capture reference.
        let c = vec![corr("five dollars", "$5")];
        assert_eq!(apply_corrections("it cost five dollars today", &c), "it cost $5 today");
    }

    #[test]
    fn corrections_case_insensitive_and_word_bounded() {
        let c = vec![corr("fakapapa", "whakapapa")];
        assert_eq!(apply_corrections("My Fakapapa is", &c), "My whakapapa is");
        // Word-bounded: a substring inside a larger word is left alone.
        assert_eq!(apply_corrections("fakapapas", &c), "fakapapas");
    }

    #[test]
    fn corrections_noop_when_absent_or_empty() {
        let c = vec![corr("xyz", "abc")];
        assert_eq!(apply_corrections("nothing here", &c), "nothing here");
        assert_eq!(apply_corrections("text", &[]), "text");
    }
}
