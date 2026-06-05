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
    for c in corrections {
        let wrong = c.wrong.trim();
        let right = c.right.trim();
        if wrong.is_empty() || right.is_empty() || wrong.eq_ignore_ascii_case(right) {
            continue;
        }
        // Word-bounded + case-insensitive. The corrections list is small and
        // this runs once per transcription, so per-call compilation is fine.
        let pattern = format!(r"(?i)\b{}\b", regex::escape(wrong));
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                out = re.replace_all(&out, right).into_owned();
            }
            Err(e) => {
                log::warn!("Correction '{wrong}' produced an invalid pattern: {e}");
            }
        }
    }
    out
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
