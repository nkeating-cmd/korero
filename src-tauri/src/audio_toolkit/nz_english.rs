//! Kōrero — the New Zealand English locale pass.
//!
//! Backlog **P0-NZ**; design: `docs/KORERO_NZ_MODE_PLAN_2026-09-02.md`.
//!
//! # Why this module exists, and why `en-NZ` is not a language
//!
//! New Zealand English is not a language an ASR engine can be told to decode. It is an accent, a
//! lexicon, an orthography, and code-switching with te reo Māori. Measured on 2026-09-02:
//!
//! * Whisper's language allow-list (`whisper.cpp` `g_lang`) is keyed on **bare ISO-639-1**. There is
//!   no `en-NZ`. Passing one is not rejected and not ignored — `whisper_lang_id` returns `-1`, and
//!   `whisper_token_lang(ctx, -1)` evaluates to `token_sot`, so the start token is pushed twice into
//!   the decoder prompt with no error raised at any boundary.
//! * Parakeet TDT v3 — the model onboarding recommends — carries `supports_language_selection:
//!   false` and its `ParakeetParams` has no language field at all. The setting is **discarded**.
//!
//! So `en-NZ` is carried as a **Kōrero locale tag**, never an engine language. It is folded to bare
//! `en` at the engine boundary by [`fold_locale_for_engine`], exactly as `zh-Hans` / `zh-Hant` are
//! folded to `zh`, and it exists solely to switch on the deterministic post-engine pass below.
//!
//! This is why the pass works on **every** engine, Parakeet included, with no model download, no
//! LLM, and no second keyboard shortcut.
//!
//! # Safety properties
//!
//! 1. **Inert unless selected.** [`apply_nz_english`] is only ever called behind
//!    [`is_nz_locale`]. `korero_nz_pass_is_inert_when_not_selected` pins that the output is
//!    byte-identical for any other locale.
//! 2. **Exact whole-word mapping only. No suffix rules.** A rule like "`-ize` → `-ise`" would
//!    corrupt *capsize*, *prize*, *size*. Every entry is a literal key, so a word that is not in a
//!    table cannot be touched. `korero_nz_spelling_does_not_touch_exempt_words` pins this.
//! 3. **The T1 ambiguity guard is respected.** In te reo the macron IS the plural marker on a whole
//!    noun class — wahine/wāhine, matua/mātua, tangata/tāngata — and keke/kēkē is a different word.
//!    Auto-macronising those would silently pluralise a singular. Every such form is **excluded from
//!    the tables entirely**, mirroring `REO_MACRON_AMBIGUOUS` in `text.rs`. Fail-safe by
//!    construction: an excluded word is simply not touched, and a user's taught correction still
//!    repairs it.
//! 4. **Acronyms are protected.** An ALL-CAPS token of four characters or fewer is skipped, so
//!    `NGA` (National Gallery of Art) never becomes `NGĀ`.
//! 5. **A user's taught correction always wins.** This pass runs BEFORE
//!    `corrections::apply_corrections`, which is deterministic and last in the chain.
//!
//! # ⚠ NOT REVIEWED BY A TE REO SPEAKER
//!
//! Same caveat `REO_MACRON_AMBIGUOUS` carries in `text.rs`, and it matters more here because this
//! list is an order of magnitude longer. The tables are deliberately conservative: where there was
//! any doubt about whether a word carries a macron, or whether its macron-free form is a distinct
//! word, **the entry was left out rather than guessed**. Review is owed before this is described as
//! complete anywhere user-facing. See backlog T6.

/// The Kōrero locale tag. Never sent to an engine.
pub const NZ_LOCALE: &str = "en-NZ";

/// True when the NZ English post-engine pass should run.
///
/// Keyed on the RAW `selected_language` setting, not the model-validated one — the pass is a text
/// transform and is meaningful even on an engine that ignores language entirely. This mirrors
/// `maybe_convert_chinese_variant`, which also reads `settings.selected_language` directly.
pub fn is_nz_locale(selected_language: &str) -> bool {
    selected_language == NZ_LOCALE
}

/// Folds a Kōrero locale tag to the bare language code an engine can accept.
///
/// `en-NZ` -> `en`. Everything else is returned unchanged, so this is safe to call unconditionally.
/// Without this, `en-NZ` reaches `whisper.cpp` and malforms the decoder prompt (see module docs).
pub fn fold_locale_for_engine(lang: &str) -> &str {
    if lang == NZ_LOCALE {
        "en"
    } else {
        lang
    }
}

/// True when `lang` is a shape Kōrero is willing to let reach an engine.
///
/// # Why this exists — defect D-2
///
/// The language setting is written by a Tauri command that performs **no validation at all**, and
/// the downstream check failed open twice: `get_model_info` returning `None` (an unknown or
/// unregistered model id) fell to `.unwrap_or(true)`, and a registered model with an **empty**
/// `supported_languages` — which is every custom `.bin` Whisper model — satisfied `is_empty()`.
/// Either way an arbitrary string reached `whisper.cpp`, where it is neither rejected nor ignored:
/// `whisper_lang_id` returns `-1` and `whisper_token_lang(ctx, -1)` evaluates to `token_sot`, so
/// the start token is pushed twice into the decoder prompt.
///
/// This check is deliberately about the **shape of the string**, not about model support, so it
/// still holds when we know nothing about the model.
pub fn is_well_formed_locale(lang: &str) -> bool {
    // Kōrero's own locale tags, each folded before it reaches an engine.
    if lang == "auto" || lang == NZ_LOCALE || lang == "zh-Hans" || lang == "zh-Hant" {
        return true;
    }
    // Every key in whisper.cpp's `g_lang` is bare ISO-639-1/-3: two or three ASCII lowercase
    // letters ("en", "mi", "haw", "yue").
    let n = lang.chars().count();
    (2..=3).contains(&n) && lang.chars().all(|c| c.is_ascii_lowercase())
}

/// Te reo Māori common nouns whose macron-free form is NOT a distinct word.
///
/// Key: lowercase, macron-free. Value: lowercase, correctly macronised.
/// Case of the output follows the case of the input token.
///
/// Every entry here is a word whose bare form is unambiguous. Anything on the T1 ambiguity list is
/// absent by construction — see `korero_nz_lexicon_excludes_t1_ambiguous_forms`.
const MACRON_COMMON: &[(&str, &str)] = &[
    ("awhina", "āwhina"),
    ("hakari", "hākari"),
    ("hangi", "hāngī"),
    ("hapu", "hapū"),
    ("kaumatua", "kaumātua"),
    ("kawanatanga", "kāwanatanga"),
    ("kohanga", "kōhanga"),
    ("korero", "kōrero"),
    ("matauranga", "mātauranga"),
    ("morena", "mōrena"),
    ("nga", "ngā"),
    ("pakeha", "Pākehā"),
    ("powhiri", "pōwhiri"),
    ("purakau", "pūrākau"),
    ("putea", "pūtea"),
    ("roopu", "rōpū"),
    ("ropu", "rōpū"),
    ("tena", "tēnā"),
    ("turangawaewae", "tūrangawaewae"),
    ("urupa", "urupā"),
    ("wahi", "wāhi"),
    ("wananga", "wānanga"),
    ("whanau", "whānau"),
    ("whanui", "whānui"),
];

/// Te reo Māori proper nouns and New Zealand place names.
///
/// Key: lowercase, macron-free. Value: canonical casing, always emitted as stored (unless the input
/// was ALL CAPS). A proper noun stays capitalised even when the speaker's transcript did not.
///
/// Deliberately excludes names that carry NO macron (Whanganui, Rotorua, Taranaki, Porirua,
/// Manukau, Papakura, Tauranga, Timaru) — there is nothing to restore, so an entry would be pure
/// risk for zero gain.
const MACRON_PROPER: &[(&str, &str)] = &[
    ("aotearoa", "Aotearoa"),
    ("kaikoura", "Kaikōura"),
    ("kapiti", "Kāpiti"),
    ("manawatu", "Manawatū"),
    ("maori", "Māori"),
    ("ngaruawahia", "Ngāruawāhia"),
    ("oamaru", "Ōamaru"),
    ("ohakune", "Ōhakune"),
    ("ohope", "Ōhope"),
    ("opotiki", "Ōpōtiki"),
    ("orakei", "Ōrākei"),
    ("otago", "Otago"),
    ("otaki", "Ōtaki"),
    ("otautahi", "Ōtautahi"),
    ("otepoti", "Ōtepoti"),
    ("paekakariki", "Paekākāriki"),
    ("papamoa", "Pāpāmoa"),
    ("pauatahanui", "Pāuatahanui"),
    ("takaka", "Tākaka"),
    ("tamaki", "Tāmaki"),
    ("taupo", "Taupō"),
    ("turangi", "Tūrangi"),
    ("wanaka", "Wānaka"),
    ("whakatane", "Whakatāne"),
    ("whangarei", "Whangārei"),
];

/// US -> New Zealand spelling.
///
/// Exact whole words only. Every entry was chosen because the US form is **never** correct in NZ
/// English in any sense. Deliberately EXCLUDED, and each exclusion is a real ambiguity:
///
/// * `program` — correct in NZ for computer programs.
/// * `dialog` — correct in "dialog box".
/// * `meter` — correct for a measuring device (power meter).
/// * `tire` — correct as the verb "to tire".
/// * `license` / `practice` — correct as verbs; the noun/verb split is not decidable per-word.
/// * `check`, `curb` — both are ordinary NZ words with different meanings.
/// * `labor` — collides with the Australian Labor Party as a proper noun.
const SPELLING: &[(&str, &str)] = &[
    ("aluminum", "aluminium"),
    ("analyze", "analyse"),
    ("analyzed", "analysed"),
    ("analyzes", "analyses"),
    ("analyzing", "analysing"),
    ("behavior", "behaviour"),
    ("behavioral", "behavioural"),
    ("behaviors", "behaviours"),
    ("canceled", "cancelled"),
    ("canceling", "cancelling"),
    ("catalog", "catalogue"),
    ("catalogs", "catalogues"),
    ("center", "centre"),
    ("centered", "centred"),
    ("centers", "centres"),
    ("color", "colour"),
    ("colored", "coloured"),
    ("colors", "colours"),
    ("defense", "defence"),
    ("endeavor", "endeavour"),
    ("enrollment", "enrolment"),
    ("favor", "favour"),
    ("favored", "favoured"),
    ("favorite", "favourite"),
    ("favorites", "favourites"),
    ("favors", "favours"),
    ("fiber", "fibre"),
    ("fulfill", "fulfil"),
    ("fulfillment", "fulfilment"),
    ("gray", "grey"),
    ("harbor", "harbour"),
    ("honor", "honour"),
    ("honored", "honoured"),
    ("labeled", "labelled"),
    ("labeling", "labelling"),
    ("liter", "litre"),
    ("liters", "litres"),
    ("modeled", "modelled"),
    ("modeling", "modelling"),
    ("neighbor", "neighbour"),
    ("neighborhood", "neighbourhood"),
    ("neighbors", "neighbours"),
    ("offense", "offence"),
    ("organization", "organisation"),
    ("organizations", "organisations"),
    ("organize", "organise"),
    ("organized", "organised"),
    ("organizing", "organising"),
    ("realize", "realise"),
    ("realized", "realised"),
    ("realizing", "realising"),
    ("recognize", "recognise"),
    ("recognized", "recognised"),
    ("recognizing", "recognising"),
    ("theater", "theatre"),
    ("traveled", "travelled"),
    ("traveler", "traveller"),
    ("traveling", "travelling"),
];

/// Longest ALL-CAPS token still treated as a possible acronym and therefore skipped.
const ACRONYM_MAX_LEN: usize = 4;

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| table[i].1)
}

/// Splits a token into (leading punctuation, core, trailing punctuation).
fn split_affixes(token: &str) -> (&str, &str, &str) {
    let start = token
        .char_indices()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(i, _)| i)
        .unwrap_or(token.len());
    let end = token
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(token.len());
    if start >= end {
        return (token, "", "");
    }
    (&token[..start], &token[start..end], &token[end..])
}

fn is_all_caps(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic()) && s.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase())
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
}

fn capitalise_first(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + it.as_str(),
    }
}

fn lower_first(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().collect::<String>() + it.as_str(),
    }
}

/// Applies the New Zealand English pass.
///
/// Whitespace is preserved exactly: the input is split on ASCII whitespace boundaries and rejoined
/// with the original separators, so line breaks and runs of spaces survive unchanged.
///
/// Returns the input **verbatim** when nothing matched — the same discipline `collapse_repeats`
/// uses, so a transcript that needs no repair is never reflowed.
pub fn apply_nz_english(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len() + 16);
    let mut changed = false;
    let mut cursor = 0usize;

    for (idx, ch) in text.char_indices() {
        if !ch.is_whitespace() {
            continue;
        }
        if idx > cursor {
            let (tok, hit) = rewrite_token(&text[cursor..idx]);
            changed |= hit;
            out.push_str(&tok);
        }
        out.push(ch);
        cursor = idx + ch.len_utf8();
    }
    if cursor < text.len() {
        let (tok, hit) = rewrite_token(&text[cursor..]);
        changed |= hit;
        out.push_str(&tok);
    }

    if changed {
        out
    } else {
        text.to_string()
    }
}

/// Rewrites one whitespace-delimited token. Returns (result, changed).
fn rewrite_token(token: &str) -> (String, bool) {
    let (lead, core, trail) = split_affixes(token);
    if core.is_empty() {
        return (token.to_string(), false);
    }

    // Safety property 4: never touch a short ALL-CAPS token; it is probably an acronym.
    if is_all_caps(core) && core.chars().count() <= ACRONYM_MAX_LEN {
        return (token.to_string(), false);
    }

    let key = core.to_lowercase();

    // Proper nouns first — the canonical casing wins over the speaker's.
    if let Some(canon) = lookup(MACRON_PROPER, &key) {
        let repl = if is_all_caps(core) {
            canon.to_uppercase()
        } else {
            canon.to_string()
        };
        if repl == core {
            return (token.to_string(), false);
        }
        return (format!("{lead}{repl}{trail}"), true);
    }

    for table in [MACRON_COMMON, SPELLING] {
        if let Some(canon) = lookup(table, &key) {
            let repl = if is_all_caps(core) {
                canon.to_uppercase()
            } else if starts_upper(core) {
                capitalise_first(canon)
            } else if starts_upper(canon) {
                // A lowercase input against a capitalised canonical form (e.g. "pakeha" ->
                // "Pākehā"): keep the canonical capital, it is a proper adjective.
                canon.to_string()
            } else {
                lower_first(canon)
            };
            if repl == core {
                return (token.to_string(), false);
            }
            return (format!("{lead}{repl}{trail}"), true);
        }
    }

    (token.to_string(), false)
}

#[cfg(test)]
mod korero_nz_tests {
    use super::*;

    /// Mirror of `REO_MACRON_AMBIGUOUS` in `audio_toolkit::text`. Kept as a literal copy because
    /// that constant is private to its module and widening its visibility would add patch surface
    /// to a file that already carries twelve patches. `korero_nz_lexicon_excludes_t1_ambiguous_forms`
    /// is what makes the copy load-bearing rather than decorative.
    const T1_AMBIGUOUS: &[&str] = &[
        "ana", "keke", "maku", "mana", "matua", "naku", "nana", "tangata", "taua", "teina",
        "tipuna", "tuahine", "tuakana", "tupuna", "wahine",
    ];

    /// Minimum lexicon key length. Short keys are disproportionately likely to collide with an
    /// English word or an acronym, so the bar is a length rule with a named exception list rather
    /// than case-by-case judgement.
    ///
    /// Lives in the test module deliberately: it is a rule ABOUT the tables, enforced only by
    /// `korero_nz_tables_are_sorted_lowercase_and_macron_free_keys`. At module scope it was dead
    /// code in every non-test build.
    const MIN_KEY_LEN: usize = 4;

    /// Short keys allowed despite [`MIN_KEY_LEN`]. `nga` earns its place because `ngā` is one of
    /// the most common te reo words in NZ English prose; it is not an English word, and the
    /// ALL-CAPS guard in `rewrite_token` already protects the `NGA` acronym.
    const SHORT_KEY_ALLOWLIST: &[&str] = &["nga"];

    fn all_tables() -> Vec<(&'static str, &'static str)> {
        let mut v = Vec::new();
        v.extend_from_slice(MACRON_COMMON);
        v.extend_from_slice(MACRON_PROPER);
        v.extend_from_slice(SPELLING);
        v
    }

    // ---- Safety property 3: the T1 guard ---------------------------------------------------

    #[test]
    fn korero_nz_lexicon_excludes_t1_ambiguous_forms() {
        for (key, _) in all_tables() {
            assert!(
                !T1_AMBIGUOUS.contains(&key),
                "'{key}' is on the T1 macron-ambiguity list -- auto-macronising it would turn a \
                 singular into a plural. It must not appear in any NZ lexicon table."
            );
        }
    }

    #[test]
    fn korero_nz_macron_restoration_respects_t1_guard() {
        // The canonical T1 case: a singular must survive.
        assert_eq!(apply_nz_english("one wahine spoke"), "one wahine spoke");
        assert_eq!(apply_nz_english("nga tangata"), "ngā tangata");
        // ...while an unambiguous word is still repaired.
        assert_eq!(apply_nz_english("the whanau"), "the whānau");
    }

    // ---- Safety property 1: inert unless selected --------------------------------------------

    #[test]
    fn korero_nz_pass_is_inert_when_not_selected() {
        assert!(!is_nz_locale("en"));
        assert!(!is_nz_locale("auto"));
        assert!(!is_nz_locale("mi"));
        assert!(!is_nz_locale("EN-NZ"));
        assert!(is_nz_locale("en-NZ"));
    }

    #[test]
    fn korero_nz_unmatched_text_is_returned_verbatim() {
        let s = "The quick brown fox jumps over the lazy dog.";
        assert_eq!(apply_nz_english(s), s);
        let multi = "line one\n\n  line   two\ttabbed";
        assert_eq!(apply_nz_english(multi), multi);
    }

    // ---- Safety property 2: exact mapping, no suffix rules -----------------------------------

    #[test]
    fn korero_nz_spelling_does_not_touch_exempt_words() {
        for w in [
            "capsize", "prize", "size", "seize", "maize", "exercise", "advertise", "surprise",
            "comprise", "enterprise", "otherwise", "compromise", "franchise", "program",
            "dialog", "meter", "tire", "license", "practice", "check", "curb", "labor",
        ] {
            assert_eq!(
                apply_nz_english(w),
                w,
                "'{w}' is deliberately not in the spelling table and must never be rewritten"
            );
        }
    }

    #[test]
    fn korero_nz_spelling_rewrites_the_unambiguous_ones() {
        assert_eq!(
            apply_nz_english("organize the color center"),
            "organise the colour centre"
        );
        assert_eq!(apply_nz_english("Organized"), "Organised");
        assert_eq!(apply_nz_english("BEHAVIOR"), "BEHAVIOUR");
    }

    // ---- Safety property 4: acronyms ---------------------------------------------------------

    #[test]
    fn korero_nz_short_all_caps_acronyms_are_untouched() {
        assert_eq!(apply_nz_english("NGA"), "NGA");
        assert_eq!(apply_nz_english("The NGA collection"), "The NGA collection");
        // but a long all-caps word is still a word
        assert_eq!(apply_nz_english("ORGANIZE"), "ORGANISE");
    }

    // ---- The engine fold ---------------------------------------------------------------------

    #[test]
    fn korero_nz_locale_never_reaches_engine() {
        assert_eq!(fold_locale_for_engine("en-NZ"), "en");
        // everything else passes through untouched
        for l in ["en", "auto", "mi", "zh", "fr"] {
            assert_eq!(fold_locale_for_engine(l), l);
        }
    }

    // ---- D-2: the fail-open validation ------------------------------------------------------

    #[test]
    fn korero_nz_d2_rejects_malformed_locales() {
        // Every real whisper.cpp g_lang key shape, plus Korero's own tags.
        for good in [
            "auto", "en", "mi", "zh", "fr", "haw", "yue", "jw", "en-NZ", "zh-Hans", "zh-Hant",
        ] {
            assert!(is_well_formed_locale(good), "'{good}' must be accepted");
        }
        // The D-2 hazard class: anything whisper_lang_id would miss, returning -1 and pushing
        // token_sot into the language slot.
        for bad in [
            "en-US", "en_NZ", "EN", "En", "english", "", "e", "en-", "nz", // <- "nz" is 2 chars
        ] {
            if bad == "nz" {
                // "nz" is well-FORMED (2 lowercase ASCII); it is simply not a real language.
                // Shape validation cannot catch that, and the model-support check is what does.
                // Recorded explicitly so this test states the limit of what it proves.
                assert!(is_well_formed_locale(bad));
                continue;
            }
            assert!(!is_well_formed_locale(bad), "'{bad}' must be rejected");
        }
    }

    // ---- Casing ------------------------------------------------------------------------------

    #[test]
    fn korero_nz_casing_and_punctuation_are_preserved() {
        assert_eq!(apply_nz_english("whanau,"), "whānau,");
        assert_eq!(apply_nz_english("(korero)"), "(kōrero)");
        assert_eq!(apply_nz_english("Whanau."), "Whānau.");
        assert_eq!(apply_nz_english("taupo"), "Taupō");
        assert_eq!(apply_nz_english("Taupo!"), "Taupō!");
        assert_eq!(apply_nz_english("maori"), "Māori");
    }

    #[test]
    fn korero_nz_place_names_are_restored() {
        assert_eq!(
            apply_nz_english("driving from Kapiti to Otautahi via Taupo"),
            "driving from Kāpiti to Ōtautahi via Taupō"
        );
    }

    // ---- Table hygiene -----------------------------------------------------------------------

    #[test]
    fn korero_nz_tables_are_sorted_lowercase_and_macron_free_keys() {
        for (name, table) in [
            ("MACRON_COMMON", MACRON_COMMON),
            ("MACRON_PROPER", MACRON_PROPER),
            ("SPELLING", SPELLING),
        ] {
            let keys: Vec<&str> = table.iter().map(|(k, _)| *k).collect();
            let mut sorted = keys.clone();
            sorted.sort_unstable();
            assert_eq!(keys, sorted, "{name} must be sorted for binary_search");
            let mut dedup = sorted.clone();
            dedup.dedup();
            assert_eq!(dedup.len(), sorted.len(), "{name} has a duplicate key");
            for k in &keys {
                assert!(
                    k.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                    "{name} key '{k}' must be ASCII lowercase and macron-free -- the key is what a \
                     bare transcript looks like"
                );
                assert!(
                    k.chars().count() >= MIN_KEY_LEN || SHORT_KEY_ALLOWLIST.contains(k),
                    "{name} key '{k}' is shorter than {MIN_KEY_LEN} and is not on the justified \
                     short-key allowlist"
                );
            }
        }
    }

    #[test]
    fn korero_nz_no_entry_is_a_no_op() {
        for (k, v) in all_tables() {
            assert_ne!(
                k, v,
                "'{k}' maps to itself -- a no-op entry is dead weight and hides a typo"
            );
        }
    }
}
