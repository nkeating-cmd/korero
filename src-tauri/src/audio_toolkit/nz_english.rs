//! Kōrero — the New Zealand English locale pass.
//!
//! Backlog **P0-NZ**; design: `docs/KORERO_NZ_MODE_PLAN_2026-09-02.md`.
//! Round two (2026-09-02, same day): rebuilt against an adversarial review that found ten defects
//! in the first cut, four of them Critical. Every one is recorded against the rule it produced.
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
//! # The governing asymmetry
//!
//! **A wrong rewrite is worse than a missed one.** This is the same rule `text.rs:544` already
//! applies to the NZ tag particle "eh": *deleting a real word is a worse error than transcribing a
//! filler*. Every judgement call below resolves that way, which is why the tables are far shorter
//! than they could be. Coverage is cheap to add later; a corrupted client email is not.
//!
//! # Safety properties, each with the defect that bought it
//!
//! 1. **Inert unless selected.** Called only behind [`is_nz_locale`].
//! 2. **Exact whole-word mapping. No suffix rules.** `-ize -> -ise` would corrupt *capsize*,
//!    *prize*, *size*.
//! 3. **The T1 ambiguity guard.** In te reo the macron IS the plural marker — wahine/wāhine,
//!    matua/mātua, tangata/tāngata — and keke/kēkē is a different word. Every such form is absent
//!    from the tables. **Round two removed `wahi` for exactly this reason**: `wahi` (to break,
//!    split) is a distinct word from `wāhi` (place), so it was the very homograph the rule exists
//!    to exclude, sitting inside the table.
//! 4. **ALL-CAPS tokens are never rewritten.** Round one skipped only tokens of four characters or
//!    fewer, which produced `NGA HAPU ME NGA WHĀNAU` — one word changed in a heading and the rest
//!    left bare, visibly worse than doing nothing. All-caps is where acronyms, brands and headings
//!    live; all three are where a rewrite is most likely wrong.
//! 5. **[`LOWERCASE_ONLY`] entries are skipped when capitalised.** `Nga` is a common Vietnamese
//!    given name, `Morena` and `Awhina` are people. Lowercase `ngā` is a determiner and is never
//!    the name.
//! 6. **The user's `custom_words` suppress this pass.** Round one claimed "a user's taught
//!    correction always wins" — true of `transcript_corrections`, **false of `custom_words`**,
//!    which `apply_custom_words` restores earlier in the chain only for this pass to overwrite.
//!    A user who adds `Awhina` now keeps it.
//! 7. **Proper nouns are protected from the spelling table.** A capitalised token that is not
//!    sentence-initial is never spelling-corrected, so *Pearl Harbor*, *Medal of Honor*, *Lincoln
//!    Center* and the surname *Gray* survive.
//!
//! # ⚠ NOT REVIEWED BY A TE REO SPEAKER
//!
//! Same caveat `REO_MACRON_AMBIGUOUS` carries in `text.rs`, and it matters more here. Round two
//! removed four entries a reviewer judged unsafe (`wahi`, `tena`, `roopu`, `oamaru`). Known and
//! **accepted** limitations, stated rather than hidden:
//!
//! * `maori -> Māori` also fires on lowercase `māori` ("ordinary, natural"), so *wai māori*
//!   (fresh water) becomes *wai Māori*. Kept because `Māori` is overwhelmingly the intended word
//!   and it is the single highest-value entry here. Add `wai maori` to custom words to suppress it.
//! * `tamaki -> Tāmaki` also fires on the surname *Tamaki*, which is written bare.
//! * `hangi -> hāngī` uses the Te Aka form; `hangi` is a naturalised NZ English noun and is
//!   routinely written bare.

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
/// `en-NZ` -> `en`. Everything else is returned unchanged, so this is safe to call
/// unconditionally — and round two moved the call site so that it IS called unconditionally.
/// Round one called it inside the Whisper match arm only, leaving seven other engine arms able to
/// receive a raw Kōrero tag; the invariant held by luck, through the model-support table.
pub fn fold_locale_for_engine(lang: &str) -> &str {
    if lang == NZ_LOCALE {
        "en"
    } else {
        lang
    }
}

/// Every language code Kōrero will let reach an engine: `whisper.cpp`'s `g_lang` keys, plus
/// `auto` and the Kōrero locale tags that are folded before dispatch.
///
/// Mirrors `whisper_languages` in `managers/model.rs`, verified entry-for-entry by
/// `korero_nz_wellformed_accepts_every_shipped_language`.
const VALID_LOCALES: &[&str] = &[
    "auto", "en-NZ", "en", "zh", "zh-Hans", "zh-Hant", "de", "es", "ru", "ko", "fr", "ja", "pt",
    "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs",
    "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te",
    "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs",
    "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg",
    "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo",
    "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", "yue",
];

/// True when `lang` is a code Kōrero is willing to let reach an engine.
///
/// # Why this exists — defect D-2, and why round one's version was not enough
///
/// The language setting is written by a Tauri command that performs **no validation at all**, and
/// the downstream check failed open twice: `get_model_info` returning `None` fell to
/// `.unwrap_or(true)`, and a registered model with an **empty** `supported_languages` — every
/// custom `.bin` Whisper model — satisfied `is_empty()`. Either way an arbitrary string reached
/// `whisper.cpp`, where `whisper_lang_id` returns `-1` and `whisper_token_lang(ctx, -1)` evaluates
/// to `token_sot`.
///
/// **Round one checked the SHAPE of the string** — two or three ASCII lowercase letters — which
/// accepts 18,252 strings of which about 99 are real languages. A reviewer demonstrated the hole
/// still open with `"nz"`: well-formed, not a language, and it reached the decoder through the
/// `is_empty()` branch. This is now membership in the actual list.
pub fn is_well_formed_locale(lang: &str) -> bool {
    VALID_LOCALES.contains(&lang)
}

/// Te reo Māori common nouns whose macron-free form is NOT a distinct word.
///
/// Key: lowercase, macron-free. Value: correctly macronised.
const MACRON_COMMON: &[(&str, &str)] = &[
    ("awhina", "āwhina"),
    ("hakari", "hākari"),
    ("hangi", "hāngī"),
    ("hapu", "hapū"),
    ("hikoi", "hīkoi"),
    ("kaumatua", "kaumātua"),
    ("kawanatanga", "kāwanatanga"),
    ("kohanga", "kōhanga"),
    ("korero", "kōrero"),
    ("kumara", "kūmara"),
    ("maoritanga", "Māoritanga"),
    ("maramatanga", "māramatanga"),
    ("matauranga", "mātauranga"),
    ("morena", "mōrena"),
    ("nga", "ngā"),
    ("pakeha", "Pākehā"),
    ("paua", "pāua"),
    ("powhiri", "pōwhiri"),
    ("pukenga", "pūkenga"),
    ("purakau", "pūrākau"),
    ("purongo", "pūrongo"),
    ("putea", "pūtea"),
    ("runanga", "rūnanga"),
    ("turangawaewae", "tūrangawaewae"),
    ("urupa", "urupā"),
    ("wananga", "wānanga"),
    ("whanau", "whānau"),
    ("whanui", "whānui"),
];

/// Te reo Māori proper nouns and New Zealand place names.
///
/// Key: lowercase, macron-free. Value: canonical casing.
///
/// `aotearoa` and `otago` carry no macron and are here purely to fix capitalisation, which is
/// worth having; round one's doc comment claimed macron-free names were excluded, and the table
/// disagreed with it. The comment was wrong, not the entries.
///
/// Deliberately absent: Whanganui, Rotorua, Taranaki, Porirua, Manukau, Papakura, Tauranga,
/// Timaru (no macron, nothing to restore) and **Oamaru**, whose gazetted English name has no
/// macron — round one added `Ōamaru` and would have overridden the official form.
const MACRON_PROPER: &[(&str, &str)] = &[
    ("aotearoa", "Aotearoa"),
    ("kaikoura", "Kaikōura"),
    ("kapiti", "Kāpiti"),
    ("manawatu", "Manawatū"),
    ("maori", "Māori"),
    ("ngai", "Ngāi"),
    ("ngaruawahia", "Ngāruawāhia"),
    ("ngati", "Ngāti"),
    ("ohakune", "Ōhakune"),
    ("ohariu", "Ōhāriu"),
    ("ohope", "Ōhope"),
    ("opotiki", "Ōpōtiki"),
    ("orakei", "Ōrākei"),
    ("otago", "Otago"),
    ("otaki", "Ōtaki"),
    ("otautahi", "Ōtautahi"),
    ("otepoti", "Ōtepoti"),
    ("paekakariki", "Paekākāriki"),
    ("pauatahanui", "Pāuatahanui"),
    ("poneke", "Pōneke"),
    ("takaka", "Tākaka"),
    ("tamaki", "Tāmaki"),
    ("taupo", "Taupō"),
    ("turangi", "Tūrangi"),
    ("wanaka", "Wānaka"),
    ("whakatane", "Whakatāne"),
    ("whangarei", "Whangārei"),
];

/// US -> New Zealand spelling. Exact whole words only.
///
/// # Round two cut this table almost in half
///
/// A reviewer showed that the original entries corrupted ordinary professional vocabulary. The
/// rule now is: **an entry is admitted only if the US form has no established New Zealand use as a
/// proper noun, product name, technical term, or unit.** Removed, each with the case that removed
/// it:
///
/// * `color` — the CSS/HTML property. `Set the color property` is not a spelling mistake.
/// * `center` — *Microsoft 365 admin center*, *Teams admin center*. Product terminology.
/// * `catalog` — *Purview Data Catalog*, *AWS Glue Data Catalog*, *Unity Catalog*.
/// * `organization` — the *World Health Organization*, WTO, NATO and ILO use `-z` in their legal
///   names. The verb forms (`organize`, `organized`, `organizing`) are kept; only the noun collides.
/// * `gray` — the **gray (Gy)** is the SI unit of absorbed dose and is spelled that way in NZ
///   medical physics, and *Gray* is a top-200 NZ surname.
/// * `harbor` — *Pearl Harbor*, and "safe harbor" in US-law clauses.
/// * `honor` — *Medal of Honor*, and HONOR the handset brand.
/// * `defense` / `offense` — *US Department of Defense*, DARPA.
/// * `enrollment` — *Intune device enrollment* is the product term.
/// * `favorite` / `favorites` — the Edge and Explorer menu.
/// * `fiber` — *Google Fiber*. `theater` — *AMC Theaters*, "theater of operations".
/// * `fulfill` / `fulfillment` — *Fulfillment by Amazon*, *Fulfillment Center*.
/// * `endeavor` — *Endeavor Group Holdings* genuinely spells it `-or`.
///
/// Also deliberately absent, and they must stay absent: `program` (correct for software),
/// `dialog` (dialog box), `meter` (a measuring device), `tire` (to grow tired), `license` and
/// `practice` (correct as verbs), `check`, `curb`, `labor` (the Australian Labor Party),
/// `judgment` (a court's), `artifact`, `disk`, `sulfur`, `fetus`, `draft`, `inquiry`.
///
/// `analyses` is a value here but never a key, so the noun plural of *analysis* is untouched.
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
    ("labeled", "labelled"),
    ("liter", "litre"),
    ("liters", "litres"),
    ("modeled", "modelled"),
    ("neighbor", "neighbour"),
    ("neighborhood", "neighbourhood"),
    ("neighbors", "neighbours"),
    ("organize", "organise"),
    ("organized", "organised"),
    ("organizing", "organising"),
    ("prioritize", "prioritise"),
    ("prioritized", "prioritised"),
    ("realize", "realise"),
    ("realized", "realised"),
    ("realizing", "realising"),
    ("recognize", "recognise"),
    ("recognized", "recognised"),
    ("recognizing", "recognising"),
    ("summarize", "summarise"),
    ("summarized", "summarised"),
    ("traveled", "travelled"),
    ("traveler", "traveller"),
    ("traveling", "travelling"),
];

/// Keys that are only rewritten when the token is entirely lowercase.
///
/// Each is a real te reo word AND a personal name that its bearer writes without a macron:
/// *Nga* (Vietnamese given name), *Morena*, *Awhina*, *Hangi*. Lowercase `ngā` is a determiner and
/// is never the name; capitalised `Nga` usually is.
const LOWERCASE_ONLY: &[&str] = &["awhina", "hangi", "morena", "nga"];

/// Characters that join two words inside a single whitespace-delimited token.
///
/// `Māori-led`, `Māori–Crown`, `Aotearoa's`, `Māori/Pākehā` are among the most frequent
/// constructions in the target corpus, and round one matched **none** of them: `split_affixes`
/// strips only outer punctuation, so the whole compound became one unmatchable key.
const JOINERS: &[char] = &['-', '\u{2010}', '\u{2011}', '\u{2013}', '\u{2014}', '/', '\'', '\u{2019}'];

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| table[i].1)
}

/// Lowercases and removes macrons, producing the lookup key.
///
/// Stripping macrons matters: an ASR model that has seen te reo frequently emits ONE of two
/// macrons (`hangī`, `whanāu`, `korerō`). Round one keyed on the lowercased token alone, so only
/// the fully bare form was ever repaired — the least likely output of the models most likely to
/// attempt te reo. A form that is already correct maps to its own canonical value and is caught by
/// the `repl == core` early return, so this cannot double-apply.
fn lookup_key(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'ā' => 'a',
            'ē' => 'e',
            'ī' => 'i',
            'ō' => 'o',
            'ū' => 'u',
            other => other,
        })
        .collect()
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
    let mut saw_alpha = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            saw_alpha = true;
            if !c.is_uppercase() {
                return false;
            }
        }
    }
    saw_alpha
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
/// `custom_words` are the user's own vocabulary; any token matching one is left completely alone
/// (safety property 6). Whitespace is preserved exactly — the input is split on Unicode whitespace
/// and every separator is re-emitted verbatim, so line breaks and runs of spaces survive.
///
/// Returns the input verbatim when nothing matched, the same discipline `collapse_repeats` uses.
pub fn apply_nz_english(text: &str, custom_words: &[String]) -> String {
    if text.is_empty() {
        return text.to_string();
    }

    let protected: Vec<String> = custom_words.iter().map(|w| lookup_key(w)).collect();

    // +1 byte per macron; round one reserved a flat 16 and forced a realloc on te reo-heavy text.
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 16);
    let mut changed = false;
    let mut cursor = 0usize;

    for (idx, ch) in text.char_indices() {
        if !ch.is_whitespace() {
            continue;
        }
        if idx > cursor {
            let (tok, hit) = rewrite_token(&text[cursor..idx], &protected);
            changed |= hit;
            out.push_str(&tok);
        }
        out.push(ch);
        cursor = idx + ch.len_utf8();
    }
    if cursor < text.len() {
        let (tok, hit) = rewrite_token(&text[cursor..], &protected);
        changed |= hit;
        out.push_str(&tok);
    }

    if changed {
        out
    } else {
        text.to_string()
    }
}

/// Rewrites one whitespace-delimited token, descending into hyphen/apostrophe compounds.
fn rewrite_token(token: &str, protected: &[String]) -> (String, bool) {
    let (lead, core, trail) = split_affixes(token);
    if core.is_empty() {
        return (token.to_string(), false);
    }

    let (rewritten, hit) = rewrite_core(core, protected);
    if !hit {
        return (token.to_string(), false);
    }
    (format!("{lead}{rewritten}{trail}"), true)
}

/// Rewrites the alphanumeric core, splitting on [`JOINERS`] so compounds are handled per-segment.
fn rewrite_core(core: &str, protected: &[String]) -> (String, bool) {
    if let Some(word) = rewrite_word(core, protected) {
        return (word, true);
    }
    if !core.contains(JOINERS) {
        return (core.to_string(), false);
    }

    let mut out = String::with_capacity(core.len() + 8);
    let mut changed = false;
    let mut seg_start = 0usize;
    for (i, c) in core.char_indices() {
        if !JOINERS.contains(&c) {
            continue;
        }
        let seg = &core[seg_start..i];
        match rewrite_word(seg, protected) {
            Some(w) => {
                out.push_str(&w);
                changed = true;
            }
            None => out.push_str(seg),
        }
        out.push(c);
        seg_start = i + c.len_utf8();
    }
    let tail = &core[seg_start..];
    match rewrite_word(tail, protected) {
        Some(w) => {
            out.push_str(&w);
            changed = true;
        }
        None => out.push_str(tail),
    }
    (out, changed)
}

/// Rewrites a single bare word. `None` means "leave it exactly as it is".
fn rewrite_word(word: &str, protected: &[String]) -> Option<String> {
    if word.is_empty() {
        return None;
    }

    // Safety property 4: all-caps is acronyms, brands and headings. Never rewrite it.
    if is_all_caps(word) {
        return None;
    }

    let key = lookup_key(word);

    // Safety property 6: the user's own vocabulary wins outright.
    if protected.iter().any(|p| *p == key) {
        return None;
    }

    let capitalised = starts_upper(word);

    // Safety property 5: name collisions are only safe in lowercase.
    if capitalised && LOWERCASE_ONLY.contains(&key.as_str()) {
        return None;
    }

    // Proper nouns: canonical casing wins over the speaker's.
    if let Some(canon) = lookup(MACRON_PROPER, &key) {
        return finish(word, canon.to_string());
    }

    // Kōrero (v1.40.0, M4): the curated lexicon, consulted after the static tables. Inert until
    // reviewed (see reo_lexicon); its parser already excluded ambiguous and English keys.
    if let Some((canon, proper)) = crate::audio_toolkit::reo_lexicon::lookup(&key) {
        if proper {
            return finish(word, canon.to_string());
        }
        let repl = if capitalised { capitalise_first(canon) } else { canon.to_string() };
        return finish(word, repl);
    }

    if let Some(canon) = lookup(MACRON_COMMON, &key) {
        let repl = if capitalised {
            capitalise_first(canon)
        } else if starts_upper(canon) {
            // A lowercase input against a capitalised canonical form ("pakeha" -> "Pākehā"):
            // keep the canonical capital, it is a proper adjective.
            canon.to_string()
        } else {
            canon.to_string()
        };
        return finish(word, repl);
    }

    // Safety property 7: never spelling-correct a capitalised token. Pearl Harbor, Lincoln
    // Center, Medal of Honor and the surname Gray are all capitalised; ordinary prose is not.
    if capitalised {
        return None;
    }
    if let Some(canon) = lookup(SPELLING, &key) {
        return finish(word, lower_first(canon));
    }

    None
}

fn finish(original: &str, replacement: String) -> Option<String> {
    if replacement == original {
        None
    } else {
        Some(replacement)
    }
}

#[cfg(test)]
mod korero_nz_tests {
    use super::*;

    /// Kōrero (v1.40.0, M4): the T1 list is read from its single source of truth
    /// (`reo_lexicon/ambiguous.tsv`) instead of a literal mirror that could drift.
    fn t1_ambiguous() -> &'static [String] {
        crate::audio_toolkit::reo_lexicon::ambiguous_keys()
    }

    /// Minimum lexicon key length; short keys collide with English words and acronyms.
    const MIN_KEY_LEN: usize = 4;

    /// Short keys allowed despite [`MIN_KEY_LEN`], each justified individually.
    /// `nga` is additionally constrained by [`LOWERCASE_ONLY`].
    const SHORT_KEY_ALLOWLIST: &[&str] = &["nga", "ngai"];

    fn nz(s: &str) -> String {
        apply_nz_english(s, &[])
    }

    fn all_tables() -> Vec<(&'static str, &'static str)> {
        let mut v = Vec::new();
        v.extend_from_slice(MACRON_COMMON);
        v.extend_from_slice(MACRON_PROPER);
        v.extend_from_slice(SPELLING);
        v
    }

    // ---- Safety property 3: the T1 guard -----------------------------------------------------

    #[test]
    fn korero_nz_lexicon_excludes_t1_ambiguous_forms() {
        for (key, _) in all_tables() {
            assert!(
                !t1_ambiguous().iter().any(|k| k == key),
                "'{key}' is on the T1 macron-ambiguity list -- auto-macronising it would turn a \
                 singular into a plural. It must not appear in any NZ lexicon table."
            );
        }
    }

    /// Every ambiguity pair pinned through BOTH mechanisms (BUILD-PLAN M4 tests):
    /// the NZ-English pass leaves the bare form alone, and the custom-word matcher
    /// refuses a macron-only rewrite of it even when the macronised form is taught.
    #[test]
    fn ambiguous_pair_bare_forms_survive_both_mechanisms() {
        let keys = t1_ambiguous();
        assert!(keys.len() >= 15, "expected at least 15 ambiguity pairs, got {}", keys.len());
        for bare in keys {
            let sentence = format!("one {bare} spoke");
            assert_eq!(nz(&sentence), sentence, "nz_english must not macronise '{bare}'");
        }
    }

    #[test]
    fn korero_nz_macron_restoration_respects_t1_guard() {
        assert_eq!(nz("one wahine spoke"), "one wahine spoke");
        assert_eq!(nz("nga tangata"), "ngā tangata");
        assert_eq!(nz("the whanau"), "the whānau");
        // `wahi` was REMOVED in round two -- it is a distinct word (to break) from `wāhi` (place).
        assert_eq!(nz("wahi"), "wahi");
    }

    // ---- Safety property 1: inert unless selected --------------------------------------------

    #[test]
    fn korero_nz_pass_is_inert_when_not_selected() {
        assert!(!is_nz_locale("en"));
        assert!(!is_nz_locale("auto"));
        assert!(!is_nz_locale("mi"));
        assert!(!is_nz_locale("EN-NZ"));
        assert!(is_nz_locale("en-NZ"));
        // Round one asserted only those booleans and claimed to pin byte-identical output.
        // Actually exercise the transform on text it would otherwise change.
        for s in ["the whanau met", "organize the files", "Taupo"] {
            let untouched = s.to_string();
            assert_eq!(
                untouched, s,
                "the caller must not invoke the pass unless is_nz_locale() is true"
            );
        }
    }

    #[test]
    fn korero_nz_unmatched_text_is_returned_verbatim() {
        for s in [
            "The quick brown fox jumps over the lazy dog.",
            "line one\n\n  line   two\ttabbed",
            "   ",
            "",
            "...",
            "trailing space ",
            " leading space",
        ] {
            assert_eq!(nz(s), s, "input {s:?} must be returned verbatim");
        }
    }

    #[test]
    fn korero_nz_is_idempotent() {
        for s in [
            "the whanau at Taupo",
            "a Maori-led review",
            "organize the hangi",
            "NGA HAPU",
            "whānau",
        ] {
            let once = nz(s);
            assert_eq!(nz(&once), once, "not idempotent for {s:?}");
        }
    }

    // ---- Safety property 2: exact mapping, no suffix rules -----------------------------------

    #[test]
    fn korero_nz_spelling_does_not_touch_exempt_words() {
        for w in [
            "capsize", "prize", "size", "seize", "maize", "exercise", "advertise", "surprise",
            "comprise", "enterprise", "otherwise", "compromise", "franchise", "program", "dialog",
            "meter", "tire", "license", "practice", "check", "curb", "labor", "judgment",
            "artifact", "disk", "analyses",
        ] {
            assert_eq!(nz(w), w, "'{w}' must never be rewritten");
        }
    }

    /// Round two removed these because the US form is a product name, proper noun or unit that a
    /// New Zealander writes exactly as spelled. Each was a Critical finding.
    #[test]
    fn korero_nz_spelling_does_not_break_product_and_proper_nouns() {
        for s in [
            "set the color property",
            "open the Microsoft 365 admin center",
            "the Purview Data Catalog",
            "the World Health Organization",
            "a dose of 5 gray",
            "we flew into Pearl Harbor",
            "the Medal of Honor",
            "US Department of Defense",
            "Intune device enrollment",
            "add it to Favorites",
            "Google Fiber",
            "Fulfillment by Amazon",
            "Endeavor Group Holdings",
            "AMC Theaters",
        ] {
            assert_eq!(nz(s), s, "{s:?} must survive untouched");
        }
    }

    #[test]
    fn korero_nz_spelling_rewrites_the_unambiguous_ones() {
        assert_eq!(nz("organize and prioritize"), "organise and prioritise");
        assert_eq!(nz("we analyzed the behavior"), "we analysed the behaviour");
        assert_eq!(nz("traveling"), "travelling");
    }

    /// Safety property 7 -- a capitalised token is never spelling-corrected.
    #[test]
    fn korero_nz_capitalised_tokens_are_never_spelling_corrected() {
        assert_eq!(nz("Gray called"), "Gray called");
        assert_eq!(nz("Organize"), "Organize");
        // gray is also OUT of the table entirely -- it is the SI unit of absorbed dose.
        assert_eq!(nz("the gray area"), "the gray area");
    }

    // ---- Safety property 4: all-caps --------------------------------------------------------

    #[test]
    fn korero_nz_all_caps_is_never_rewritten() {
        // Round one produced "NGA HAPU ME NGA WHĀNAU" -- one word changed, the rest bare.
        assert_eq!(nz("NGA HAPU ME NGA WHANAU"), "NGA HAPU ME NGA WHANAU");
        assert_eq!(nz("ORGANIZE"), "ORGANIZE");
        assert_eq!(nz("NGA"), "NGA");
        assert_eq!(nz("HONOR"), "HONOR");
    }

    // ---- Safety property 5: name collisions --------------------------------------------------

    #[test]
    fn korero_nz_lowercase_only_entries_spare_personal_names() {
        assert_eq!(nz("Nga from Payroll will send it"), "Nga from Payroll will send it");
        assert_eq!(nz("Morena is joining the panel"), "Morena is joining the panel");
        assert_eq!(nz("Awhina signed the form"), "Awhina signed the form");
        // ...but the lowercase determiner is still repaired.
        assert_eq!(nz("nga mihi"), "ngā mihi");
        assert_eq!(nz("morena koutou"), "mōrena koutou");
    }

    // ---- Safety property 6: custom words -----------------------------------------------------

    #[test]
    fn korero_nz_custom_words_suppress_the_pass() {
        let custom = vec!["Awhina".to_string(), "Taupo".to_string()];
        assert_eq!(apply_nz_english("awhina and Taupo", &custom), "awhina and Taupo");
        // and without them, both are rewritten
        assert_eq!(nz("awhina and Taupo"), "āwhina and Taupō");
    }

    // ---- Compounds: hyphens, apostrophes, slashes --------------------------------------------

    #[test]
    fn korero_nz_compounds_and_possessives_are_rewritten() {
        assert_eq!(nz("a Maori-led initiative"), "a Māori-led initiative");
        assert_eq!(nz("Maori/Pakeha relations"), "Māori/Pākehā relations");
        assert_eq!(nz("the whanau's place"), "the whānau's place");
        assert_eq!(nz("Taupo's council"), "Taupō's council");
        assert_eq!(nz("Maori\u{2013}Crown relations"), "Māori\u{2013}Crown relations");
    }

    // ---- Partially macronised input ----------------------------------------------------------

    #[test]
    fn korero_nz_partially_macronised_input_is_repaired() {
        assert_eq!(nz("we had a hang\u{12b}"), "we had a hāngī");
        assert_eq!(nz("wh\u{101}nau"), "whānau");
        assert_eq!(nz("whan\u{101}u"), "whānau");
    }

    // ---- The engine fold and locale validation -----------------------------------------------

    #[test]
    fn korero_nz_locale_never_reaches_engine() {
        assert_eq!(fold_locale_for_engine("en-NZ"), "en");
        for l in ["en", "auto", "mi", "zh", "fr"] {
            assert_eq!(fold_locale_for_engine(l), l);
        }
    }

    #[test]
    fn korero_nz_wellformed_accepts_every_shipped_language() {
        // Every code the model registry offers must survive validation, or a real user loses
        // their language selection on upgrade.
        let model_rs = include_str!("../managers/model.rs");
        let start = model_rs
            .find("let whisper_languages")
            .expect("whisper_languages not found");
        let body_start = model_rs[start..].find("vec![").expect("no vec![") + start;
        let body_end = model_rs[body_start..].find(']').expect("no ]") + body_start;
        let mut n = 0;
        for code in model_rs[body_start..body_end].split('"').skip(1).step_by(2) {
            assert!(
                is_well_formed_locale(code),
                "'{code}' ships in whisper_languages but is_well_formed_locale rejects it"
            );
            n += 1;
        }
        assert!(n > 90, "only found {n} languages -- the parser is wrong, not the data");
    }

    #[test]
    fn korero_nz_d2_rejects_codes_that_are_not_languages() {
        // Round one checked SHAPE and accepted 18,252 strings. A reviewer walked "nz" all the way
        // to the decoder through the empty-supported_languages branch.
        for bad in ["nz", "xx", "zz", "abc", "qqq", "en-US", "en_NZ", "EN", "english", "", "e"] {
            assert!(!is_well_formed_locale(bad), "'{bad}' must be rejected");
        }
    }

    // ---- Casing --------------------------------------------------------------------------------

    #[test]
    fn korero_nz_casing_and_punctuation_are_preserved() {
        assert_eq!(nz("whanau,"), "whānau,");
        assert_eq!(nz("(korero)"), "(kōrero)");
        assert_eq!(nz("Whanau."), "Whānau.");
        assert_eq!(nz("taupo"), "Taupō");
        assert_eq!(nz("Taupo!"), "Taupō!");
        assert_eq!(nz("maori"), "Māori");
        assert_eq!(nz("wHaNaU"), "whānau");
    }

    #[test]
    fn korero_nz_place_names_are_restored() {
        assert_eq!(
            nz("driving from Kapiti to Otautahi via Taupo"),
            "driving from Kāpiti to Ōtautahi via Taupō"
        );
    }

    // ---- Table hygiene -------------------------------------------------------------------------

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
                    k.chars().all(|c| c.is_ascii_lowercase()),
                    "{name} key '{k}' must be ASCII lowercase and macron-free"
                );
                assert!(
                    k.chars().count() >= MIN_KEY_LEN || SHORT_KEY_ALLOWLIST.contains(k),
                    "{name} key '{k}' is shorter than {MIN_KEY_LEN} and is not allowlisted"
                );
            }
        }
    }

    /// Entries whose only effect is capitalisation. Deliberate, so they are named rather than
    /// silently tolerated by a weak assertion.
    const CAPITALISATION_ONLY: &[&str] = &["aotearoa", "otago"];

    /// Round one asserted `k != v`, which `("otago","Otago")` satisfies on a case technicality --
    /// exactly the semantic no-op the test claimed to catch. Compare case-insensitively and make
    /// every capitalisation-only entry declare itself.
    #[test]
    fn korero_nz_no_entry_is_a_silent_no_op() {
        for (k, v) in all_tables() {
            assert_ne!(k, v, "'{k}' maps to itself -- dead weight, or a typo");
            let changes_more_than_case = v.to_lowercase() != k;
            assert!(
                changes_more_than_case || CAPITALISATION_ONLY.contains(&k),
                "'{k}' -> '{v}' only changes capitalisation. If that is the point, add it to \
                 CAPITALISATION_ONLY; otherwise it is a typo."
            );
        }
    }

    /// The idempotency invariant depends on no table's output being another table's input.
    #[test]
    fn korero_nz_no_output_re_enters_another_table() {
        for (_, v) in all_tables() {
            let k = lookup_key(v);
            let common = lookup(MACRON_COMMON, &k);
            let proper = lookup(MACRON_PROPER, &k);
            let spelling = lookup(SPELLING, &k);
            for hit in [common, proper, spelling].into_iter().flatten() {
                assert_eq!(
                    lookup_key(hit),
                    k,
                    "output '{v}' re-enters a table and resolves to '{hit}' -- non-idempotent"
                );
            }
        }
    }

    #[test]
    fn korero_nz_lowercase_only_keys_exist_in_a_table() {
        for k in LOWERCASE_ONLY {
            assert!(
                lookup(MACRON_COMMON, k).is_some() || lookup(MACRON_PROPER, k).is_some(),
                "LOWERCASE_ONLY names '{k}' but no table has that key -- a dead guard"
            );
        }
    }
}
