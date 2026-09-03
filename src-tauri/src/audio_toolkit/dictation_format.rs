//! Kōrero — the deterministic dictation formatting pass.
//!
//! UX round 2026-09-02. Sibling to [`crate::audio_toolkit::nz_english`], same shape and same
//! discipline: an offline, engine-independent transform that runs on EVERY dictation, including
//! the plain `ctrl+space` path where the LLM post-processor never fires.
//!
//! # The contract with the LLM layer
//!
//! Kōrero now has two formatting layers and they must not fight:
//!
//! | | this module | `korero_clean_transcript` and friends |
//! |---|---|---|
//! | runs | **always**, in-process, every shortcut | only on `transcribe_with_post_process` |
//! | cost | microseconds | seconds, and needs Ollama up |
//! | decides | only what a spoken cue makes **unambiguous** | anything needing judgement |
//! | may guess | **never** | yes, that is the point |
//!
//! **This layer is the floor, not the ceiling.** It structures what the speaker explicitly marked
//! — "bullet point", "first… second… third" — and refuses everything else. The LLM layer infers
//! the lists that were not cued. The prompts carry a matching instruction to *preserve* existing
//! bullets and breaks, so the ceiling cannot undo the floor.
//!
//! # What this deliberately does NOT do, and why
//!
//! **Sentence casing and terminal punctuation: not here.** Measured on Nic's own `history.db`
//! (2026-09-02), Parakeet already returns *"Can you set up the folder structure for this project
//! here and save context?"* — capitalised, punctuated, question mark intact. Adding a casing pass
//! would duplicate the model and risk fighting it. The engines solved this; the app should not
//! re-solve it.
//!
//! **Conjunctive lists: not here.** *"I need milk, eggs and bread"* is a sentence, not a list, and
//! nothing in the audio distinguishes it from one. Bulleting it would be a guess, and a guess that
//! mangles ordinary prose is worse than no feature — the same asymmetry that governs
//! [`crate::audio_toolkit::nz_english`] and the "eh" filler rule.
//!
//! **Unpunctuated enumerations: not here.** *"first do X second do Y"* has no reliable boundary —
//! deciding where item one ends is exactly the judgement this layer refuses to make. It is left to
//! the LLM, which can read the sense. Only the punctuated form is handled.
//!
//! # ⚠ CUE CALIBRATION IS PENDING REAL OUTPUT
//!
//! The cue tables below are written against how these models are *expected* to punctuate a spoken
//! list. **That has not been observed** — `history.db` holds no list dictation, so there is no
//! evidence of what Parakeet actually emits for "first, do X. Second, do Y." Until one real sample
//! exists, [`apply_dictation_format`] is conservative by construction: every rule requires an
//! explicit marker, and anything unmatched is returned **verbatim**. It cannot damage a transcript
//! it does not understand — it can only fail to help.

/// Markers that open an enumerated item when they begin a sentence.
///
/// Ordinal words only. Digits are excluded deliberately: *"1990 was a good year"* would match, and
/// a false bullet in the middle of prose is exactly the failure this module exists to avoid.
const ORDINALS: &[&str] = &[
    "first", "firstly", "second", "secondly", "third", "thirdly", "fourth", "fourthly", "fifth",
    "fifthly", "sixth", "seventh", "eighth", "ninth", "tenth", "next", "finally", "lastly",
];

/// Explicit spoken cues that force a new bullet, whatever else is happening.
const BULLET_CUES: &[&str] = &["bullet point", "new bullet", "next bullet", "bullet"];

/// Explicit spoken cues that force a line break without a bullet.
const BREAK_CUES: &[&str] = &["new paragraph", "new line", "line break"];

/// The bullet marker written into the transcript.
///
/// A hyphen, not `•`. Dictation reaches the target application through the **clipboard**
/// (`clipboard.rs::paste_via_clipboard`), so whatever is produced is pasted literally into Slack,
/// Word, an email, a terminal. `-` is valid Markdown, survives every plain-text field, and needs no
/// font support; `•` renders from a fallback face in a monospace editor and is not Markdown.
const BULLET: &str = "- ";

/// Minimum enumerated items before a run is treated as a list.
///
/// Two, not one. *"First I went to the shop and then I came home"* is prose that happens to open
/// with an ordinal; a single marker is never enough evidence.
const MIN_ITEMS: usize = 2;

/// True when the text already carries structure, in which case this pass must not touch it.
///
/// Re-running over an already-formatted transcript is how a bullet becomes `- - item`. Also covers
/// the case where the LLM layer ran first, or the user pasted formatted text into a Note.
fn already_structured(text: &str) -> bool {
    text.contains('\n')
        || text.trim_start().starts_with(BULLET)
        || text.trim_start().starts_with('•')
        || text.trim_start().starts_with('*')
}

/// Splits into sentences on `.`, `?` and `!`.
///
/// ⚠ A `.` only ends a sentence when neither neighbour is a digit, so `$3.50`, `v1.19` and `0.18`
/// are never split. This is the same guard `meeting::collapse_repeats` documents at `meeting.rs`,
/// where its absence once turned "$3.50" into "$3. 50" on every import.
fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    for i in 0..chars.len() {
        let c = chars[i];
        if c != '.' && c != '?' && c != '!' {
            continue;
        }
        if c == '.' {
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
            if prev_digit && next_digit {
                continue;
            }
        }
        let piece: String = chars[start..=i].iter().collect();
        if !piece.trim().is_empty() {
            out.push(piece.trim().to_string());
        }
        start = i + 1;
    }
    if start < chars.len() {
        let tail: String = chars[start..].iter().collect();
        if !tail.trim().is_empty() {
            out.push(tail.trim().to_string());
        }
    }
    out
}

/// The ordinal opening a sentence, if any. Case-insensitive, must be the first word.
fn leading_ordinal(sentence: &str) -> Option<&'static str> {
    let first: String = sentence
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .to_lowercase();
    ORDINALS.iter().copied().find(|o| *o == first)
}

/// Strips the ordinal and any following comma from an enumerated item.
///
/// *"First, do the thing."* becomes *"do the thing."* — the marker was scaffolding for the ear and
/// the bullet now carries that meaning. The item is NOT re-capitalised: doing so would fight the
/// engine's own casing, and this module deliberately leaves casing alone (see module docs).
fn strip_ordinal(sentence: &str) -> String {
    let trimmed = sentence.trim_start();
    let after: String = trimmed
        .chars()
        .skip_while(|c| c.is_alphabetic())
        .collect::<String>();
    after
        .trim_start()
        .trim_start_matches(',')
        .trim_start()
        .to_string()
}

/// Applies the deterministic dictation formatting pass.
///
/// Returns the input **verbatim** when nothing matched — the discipline `collapse_repeats` and
/// `apply_nz_english` both use, so a transcript that needs no structure is never reflowed.
pub fn apply_dictation_format(text: &str) -> String {
    if text.trim().is_empty() || already_structured(text) {
        return text.to_string();
    }

    // 1. Explicit cues win outright: the speaker said the word "bullet".
    if let Some(formatted) = apply_explicit_cues(text) {
        return formatted;
    }

    // 2. Ordinal enumeration, punctuated form only.
    let sentences = split_sentences(text);
    if sentences.len() < MIN_ITEMS {
        return text.to_string();
    }
    let marked: Vec<bool> = sentences.iter().map(|s| leading_ordinal(s).is_some()).collect();
    let count = marked.iter().filter(|m| **m).count();
    if count < MIN_ITEMS {
        return text.to_string();
    }

    // The run must be contiguous and reach the end. A stray "Finally," in the middle of prose is
    // not a list, and half-bulleting a paragraph is worse than leaving it alone.
    let first_marked = marked.iter().position(|m| *m).unwrap();
    if !marked[first_marked..].iter().all(|m| *m) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len() + count * 4);
    for (i, sentence) in sentences.iter().enumerate() {
        if i < first_marked {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(sentence);
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(BULLET);
            out.push_str(&strip_ordinal(sentence));
        }
    }
    out
}

/// Handles the explicit spoken cues. `None` means no cue was present.
fn apply_explicit_cues(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let has_bullet = BULLET_CUES.iter().any(|c| lower.contains(c));
    let has_break = BREAK_CUES.iter().any(|c| lower.contains(c));
    if !has_bullet && !has_break {
        return None;
    }

    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    let mut changed = false;

    'outer: loop {
        let lower_rest = rest.to_lowercase();
        // Longest cue first so "new bullet" is not eaten by "bullet".
        let mut best: Option<(usize, usize, bool)> = None;
        for (cue, is_bullet) in BULLET_CUES
            .iter()
            .map(|c| (*c, true))
            .chain(BREAK_CUES.iter().map(|c| (*c, false)))
        {
            if let Some(pos) = lower_rest.find(cue) {
                let better = match best {
                    None => true,
                    Some((bp, bl, _)) => pos < bp || (pos == bp && cue.len() > bl),
                };
                if better {
                    best = Some((pos, cue.len(), is_bullet));
                }
            }
        }
        match best {
            None => break 'outer,
            Some((pos, len, is_bullet)) => {
                out.push_str(rest[..pos].trim_end());
                out.push('\n');
                if is_bullet {
                    out.push_str(BULLET);
                }
                rest = rest[pos + len..].trim_start().trim_start_matches(',').trim_start();
                changed = true;
            }
        }
    }
    if !changed {
        return None;
    }
    out.push_str(rest);
    Some(out.trim_start_matches('\n').to_string())
}

#[cfg(test)]
mod korero_fmt_tests {
    use super::*;

    // ---- the safety property: unmatched text is untouched --------------------------------

    #[test]
    fn korero_fmt_returns_prose_verbatim() {
        for s in [
            "Can you set up the folder structure for this project here and save context?",
            "I need milk, eggs and bread.",
            "First I went to the shop and then I came home.",
            "",
            "   ",
            "Meet me at 3.30 and bring the $3.50 float.",
        ] {
            assert_eq!(apply_dictation_format(s), s, "must not touch {s:?}");
        }
    }

    #[test]
    fn korero_fmt_never_touches_already_structured_text() {
        for s in [
            "- one\n- two",
            "line one\nline two",
            "* already a bullet",
        ] {
            assert_eq!(apply_dictation_format(s), s);
        }
    }

    #[test]
    fn korero_fmt_is_idempotent() {
        for s in [
            "First, do the thing. Second, do the other thing.",
            "Bullet point call the plumber. Bullet point pay the invoice.",
            "ordinary prose with no cues at all",
        ] {
            let once = apply_dictation_format(s);
            assert_eq!(apply_dictation_format(&once), once, "not idempotent for {s:?}");
        }
    }

    // ---- ordinals ------------------------------------------------------------------------

    #[test]
    fn korero_fmt_bullets_a_punctuated_enumeration() {
        assert_eq!(
            apply_dictation_format("First, call the plumber. Second, pay the invoice."),
            "- call the plumber.\n- pay the invoice."
        );
    }

    #[test]
    fn korero_fmt_keeps_a_lead_in_sentence_above_the_list() {
        assert_eq!(
            apply_dictation_format("Here is the plan. First, call the plumber. Second, pay it."),
            "Here is the plan.\n- call the plumber.\n- pay it."
        );
    }

    /// A single ordinal is prose. This is the rule that stops the feature eating paragraphs.
    #[test]
    fn korero_fmt_one_ordinal_is_not_a_list() {
        let s = "First I went to the shop. Then I came home.";
        assert_eq!(apply_dictation_format(s), s);
    }

    /// A stray "Finally," mid-paragraph must not half-bullet the text.
    #[test]
    fn korero_fmt_non_contiguous_markers_are_left_alone() {
        let s = "First, we talked. The meeting ran long. Finally, we agreed.";
        assert_eq!(apply_dictation_format(s), s);
    }

    // ---- explicit cues -------------------------------------------------------------------

    #[test]
    fn korero_fmt_explicit_bullet_cue() {
        assert_eq!(
            apply_dictation_format("Bullet point call the plumber. Bullet point pay the invoice."),
            "- call the plumber.\n- pay the invoice."
        );
    }

    #[test]
    fn korero_fmt_new_paragraph_cue_breaks_without_a_bullet() {
        let out = apply_dictation_format("That is the first thought. New paragraph here is another.");
        assert!(out.contains('\n'), "expected a break, got {out:?}");
        assert!(!out.contains("- "), "paragraph cue must not add a bullet: {out:?}");
    }

    /// "new bullet" must not be consumed by the shorter "bullet".
    #[test]
    fn korero_fmt_longest_cue_wins() {
        let out = apply_dictation_format("Do this. New bullet do that.");
        assert!(out.contains("- do that"), "got {out:?}");
        assert!(!out.contains("new "), "the cue itself must be removed: {out:?}");
    }

    // ---- the decimal guard ----------------------------------------------------------------

    #[test]
    fn korero_fmt_decimals_and_versions_never_split() {
        let s = "First, the float is $3.50. Second, we are on v1.19 now.";
        let out = apply_dictation_format(s);
        assert!(out.contains("$3.50"), "decimal split: {out:?}");
        assert!(out.contains("v1.19"), "version split: {out:?}");
        assert_eq!(out.lines().count(), 2, "expected exactly two bullets: {out:?}");
    }

    // ---- table hygiene ---------------------------------------------------------------------

    #[test]
    fn korero_fmt_cue_tables_are_lowercase_and_deduped() {
        for (name, table) in [("ORDINALS", ORDINALS), ("BULLET_CUES", BULLET_CUES), ("BREAK_CUES", BREAK_CUES)] {
            for entry in table {
                assert_eq!(*entry, entry.to_lowercase(), "{name} entry {entry:?} must be lowercase");
                assert!(!entry.is_empty(), "{name} has an empty entry");
            }
            let mut sorted: Vec<&str> = table.to_vec();
            sorted.sort_unstable();
            let before = sorted.len();
            sorted.dedup();
            assert_eq!(sorted.len(), before, "{name} has a duplicate");
        }
    }

    /// Digits must never open a bullet: "1990 was a good year" is not a list item.
    #[test]
    fn korero_fmt_digits_are_not_ordinals() {
        assert!(leading_ordinal("1990 was a good year.").is_none());
        assert!(leading_ordinal("2. do the thing").is_none());
    }
}
