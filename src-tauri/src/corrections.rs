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

/// v1.22.0: loudness-normalise a mono buffer before ASR. Quiet / under-recorded
/// audio measurably hurts Whisper/Parakeet accuracy; this lifts it toward the
/// ~-20 dBFS level the model front-ends expect. SAFE BY DESIGN:
///   - **boost-only** — never attenuates already-loud audio,
///   - **peak-capped** — scales so the loudest sample stays below ~-0.4 dBFS
///     (never introduces clipping),
///   - **silence-guarded** — a near-silent buffer is left untouched (never
///     amplifies room tone / hiss into "speech").
/// A no-op on well-levelled or silent audio. Pure DSP, no model dependency.
/// `transcribe-rs` exposes no normalization, so this lives in our pipeline and
/// is applied once at the top of `TranscriptionManager::transcribe`.
pub fn normalize_for_asr(samples: &mut [f32]) {
    if samples.is_empty() {
        return;
    }
    // Linear amplitudes (dBFS -> linear = 10^(dB/20)).
    const TARGET_RMS: f32 = 0.1; // ~ -20 dBFS, the level ASR front-ends expect
    const PEAK_CEILING: f32 = 0.95; // ~ -0.4 dBFS, leave headroom / never clip
    const SILENCE_FLOOR: f32 = 0.005; // ~ -46 dBFS RMS; below this = do not boost

    let mut sum_sq = 0.0f64;
    let mut peak = 0.0f32;
    for &s in samples.iter() {
        sum_sq += (s as f64) * (s as f64);
        let a = s.abs();
        if a > peak {
            peak = a;
        }
    }
    let rms = (sum_sq / samples.len() as f64).sqrt() as f32;
    // Silence / invalid-peak guard.
    if rms < SILENCE_FLOOR || peak <= 0.0 {
        return;
    }
    // Boost-only: leave audio that is already at/above target alone.
    let mut gain = TARGET_RMS / rms;
    if gain <= 1.0 {
        return;
    }
    // Never push the loudest sample past the ceiling (clip-safe).
    gain = gain.min(PEAK_CEILING / peak);
    if gain <= 1.0 {
        return;
    }
    for s in samples.iter_mut() {
        *s *= gain;
    }
}

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

/// True when a term carries a te reo Māori macron.
///
/// Used only to prioritise the bias prompt (backlog T5): a macron-bearing term
/// is one the decoder cannot produce without help, so it is worth more of the
/// bounded prompt budget than an ASCII product name.
fn has_macron(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(
            c,
            '\u{101}' | '\u{100}'
                | '\u{113}' | '\u{112}'
                | '\u{12b}' | '\u{12a}'
                | '\u{14d}' | '\u{14c}'
                | '\u{16b}' | '\u{16a}'
        )
    })
}

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
    build_bias_prompt_shaped(custom_words, corrections, crate::settings::BiasPromptShape::List)
}

/// Kōrero (v1.40.0, M1f / backlog T5): the nonsense token used by the eval
/// harness's prompt-LEAKAGE probe. Deliberately not a word in any language.
pub const BIAS_PROBE_TOKEN: &str = "zxqvorbital kelmurdine";

/// Natural-language frame for `BiasPromptShape::Sentence`. Counted inside the
/// `BIAS_MAX_CHARS` budget so whisper.cpp's own truncation never drops a term.
const BIAS_SENTENCE_PREFIX: &str =
    "Kōrero, dictated in New Zealand English with te reo Māori. Words used: ";
const BIAS_SENTENCE_SUFFIX: &str = ".";

/// Kōrero (v1.40.0, M1f): `build_bias_prompt` with an explicit shape.
/// `List` is byte-identical to the v1.19.1 output (pinned by test).
pub fn build_bias_prompt_shaped(
    custom_words: &[String],
    corrections: &[TranscriptCorrection],
    shape: crate::settings::BiasPromptShape,
) -> Option<String> {
    use crate::settings::BiasPromptShape as S;
    match shape {
        S::Off => None,
        S::Probe => Some(BIAS_PROBE_TOKEN.to_string()),
        S::List => join_bias_terms(bias_terms(custom_words, corrections), 0),
        S::Sentence => {
            let frame = BIAS_SENTENCE_PREFIX.len() + BIAS_SENTENCE_SUFFIX.len();
            join_bias_terms(bias_terms(custom_words, corrections), frame)
                .map(|body| format!("{BIAS_SENTENCE_PREFIX}{body}{BIAS_SENTENCE_SUFFIX}"))
        }
    }
}

/// Bounded, comma-joined term list. `reserved` is subtracted from the char
/// budget so a surrounding frame cannot push the prompt past `BIAS_MAX_CHARS`.
fn join_bias_terms(terms: Vec<String>, reserved: usize) -> Option<String> {
    if terms.is_empty() {
        return None;
    }
    let budget = BIAS_MAX_CHARS.saturating_sub(reserved);
    // Bound: corrections-first ordering means the most valuable terms survive
    // the cap rather than being dropped by Whisper's own truncation.
    let mut out = String::new();
    for t in terms.into_iter().take(BIAS_MAX_TERMS) {
        let sep = if out.is_empty() { "" } else { ", " };
        if out.len() + sep.len() + t.len() > budget {
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

/// The ordered, de-duplicated term list behind every prompt shape.
fn bias_terms(custom_words: &[String], corrections: &[TranscriptCorrection]) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut terms: Vec<String> = Vec::new();

    // 1. Corrections' RIGHT side first (highest signal).
    for c in corrections {
        let right = c.right.trim();
        if !right.is_empty() && seen.insert(right.to_lowercase()) {
            terms.push(right.to_string());
        }
    }
    // 2. Then the custom-words list, macron-bearing terms FIRST.
    //
    // Kōrero (backlog T5, 2026-08-26): the cap below drops from the TAIL, and
    // `default_custom_words()` historically listed ten tooling terms
    // (Monday.com, Copilot, M365, ...) before every te reo entry — so for a user
    // with many taught corrections, te reo was the first thing to fall off the
    // end of the budget. A term carrying a macron is by definition one the
    // decoder cannot produce unaided; an ASCII product name usually survives
    // without biasing. Priority follows that asymmetry rather than whatever
    // order the list happens to be in. Relative order is stable within each
    // group, so a user's own ordering is otherwise respected.
    let (macron_terms, plain_terms): (Vec<&String>, Vec<&String>) =
        custom_words.iter().partition(|w| has_macron(w));
    for w in macron_terms.into_iter().chain(plain_terms) {
        let w = w.trim();
        if !w.is_empty() && seen.insert(w.to_lowercase()) {
            terms.push(w.to_string());
        }
    }
    terms
}

/// Kōrero (v1.40.0, M3 echo guard): whisper.cpp's documented failure mode is
/// to EMIT the `initial_prompt` at the start of the transcript. Strip a leading
/// run of at least `MIN_ECHO_TOKENS` consecutive prompt tokens from `text`.
/// A transcript that legitimately starts with ONE custom word ("Kōrero is…")
/// is below the threshold and untouched. Returns the input unchanged when the
/// prompt is `None`, empty, or does not lead the text.
pub const MIN_ECHO_TOKENS: usize = 3;

pub fn strip_prompt_echo(text: &str, prompt: Option<&str>) -> String {
    let Some(prompt) = prompt else {
        return text.to_string();
    };
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric() || *c == '\'')
            .collect::<String>()
            .to_lowercase()
    };
    let prompt_tokens: Vec<String> = prompt
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(norm)
        .filter(|t| !t.is_empty())
        .collect();
    if prompt_tokens.len() < MIN_ECHO_TOKENS {
        return text.to_string();
    }
    let text_tokens: Vec<(usize, &str)> = text
        .split_whitespace()
        .map(|w| (w.as_ptr() as usize - text.as_ptr() as usize, w))
        .collect();
    // Count how many leading text tokens match the prompt token sequence.
    let mut matched = 0usize;
    for (i, (_, w)) in text_tokens.iter().enumerate() {
        match prompt_tokens.get(i) {
            Some(p) if *p == norm(w) => matched += 1,
            _ => break,
        }
    }
    if matched < MIN_ECHO_TOKENS {
        return text.to_string();
    }
    match text_tokens.get(matched) {
        Some((offset, _)) => text[*offset..].to_string(),
        None => String::new(),
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
    fn bias_prompt_list_is_byte_identical_to_shaped_list() {
        let custom = vec!["whānau".to_string(), "Copilot".to_string(), "kōrero".to_string()];
        let corrections = vec![corr("fanau", "whānau"), corr("wakapapa", "whakapapa")];
        assert_eq!(
            build_bias_prompt(&custom, &corrections),
            build_bias_prompt_shaped(&custom, &corrections, crate::settings::BiasPromptShape::List)
        );
        assert_eq!(
            build_bias_prompt(&custom, &corrections).as_deref(),
            Some("whānau, whakapapa, kōrero, Copilot")
        );
    }

    #[test]
    fn bias_prompt_shape_sentence_frames_terms_within_budget() {
        let big: Vec<String> = (0..200).map(|i| format!("kupu{i:03}")).collect();
        let p = build_bias_prompt_shaped(&big, &[], crate::settings::BiasPromptShape::Sentence).unwrap();
        assert!(p.starts_with(BIAS_SENTENCE_PREFIX));
        assert!(p.ends_with(BIAS_SENTENCE_SUFFIX));
        assert!(p.len() <= BIAS_MAX_CHARS, "sentence prompt {} chars exceeds budget", p.len());
        assert!(p.contains("kupu000"));
    }

    #[test]
    fn bias_prompt_shape_off_and_probe() {
        let custom = vec!["whānau".to_string()];
        assert!(build_bias_prompt_shaped(&custom, &[], crate::settings::BiasPromptShape::Off).is_none());
        assert_eq!(
            build_bias_prompt_shaped(&custom, &[], crate::settings::BiasPromptShape::Probe).as_deref(),
            Some(BIAS_PROBE_TOKEN)
        );
    }

    #[test]
    fn strip_prompt_echo_fires_on_echoed_list() {
        let prompt = "whānau, kōrero, hapū, Copilot";
        let text = "whānau kōrero hapū Copilot kia ora everyone";
        assert_eq!(strip_prompt_echo(text, Some(prompt)), "kia ora everyone");
    }

    #[test]
    fn strip_prompt_echo_fires_on_echoed_sentence_frame() {
        let prompt = format!("{BIAS_SENTENCE_PREFIX}whānau, kōrero{BIAS_SENTENCE_SUFFIX}");
        let text = "Kōrero, dictated in New Zealand English with te reo Māori. Words used: whānau, kōrero. Tēnā koutou";
        assert_eq!(strip_prompt_echo(text, Some(&prompt)), "Tēnā koutou");
    }

    #[test]
    fn strip_prompt_echo_leaves_single_custom_word_start() {
        let prompt = "Kōrero, whānau, hapū";
        let text = "Kōrero is a dictation app";
        assert_eq!(strip_prompt_echo(text, Some(prompt)), text);
        assert_eq!(strip_prompt_echo(text, None), text);
    }

    #[test]
    fn bias_prompt_none_when_empty() {
        assert!(build_bias_prompt(&[], &[]).is_none());
        assert!(build_bias_prompt(&["   ".to_string()], &[corr("", "")]).is_none());
    }

    #[test]
    fn bias_prompt_puts_macron_terms_before_ascii_ones() {
        // Backlog T5: the budget is spent from the front, so the terms the
        // decoder cannot produce unaided must not sit behind product names.
        let custom = vec![
            "Monday.com".to_string(),
            "Copilot".to_string(),
            "wh\u{101}nau".to_string(),
            "GST".to_string(),
            "hap\u{16b}".to_string(),
        ];
        let p = build_bias_prompt(&custom, &[]).unwrap();
        let macron_at = p.find("wh\u{101}nau").expect("macron term present");
        let ascii_at = p.find("Monday.com").expect("ascii term present");
        assert!(
            macron_at < ascii_at,
            "macron-bearing terms must lead the prompt: {p}"
        );
        // Relative order WITHIN each group is preserved.
        assert!(p.find("wh\u{101}nau").unwrap() < p.find("hap\u{16b}").unwrap(), "{p}");
        assert!(p.find("Monday.com").unwrap() < p.find("Copilot").unwrap(), "{p}");
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
