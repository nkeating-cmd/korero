//! Kōrero (v1.40.0, M4 step 2): curated te reo Māori macron data.
//!
//! Two TSV files are compiled in with `include_str!` and parsed once:
//!
//! * `ambiguous.tsv` — bare forms that are THEMSELVES valid, distinct words
//!   (overwhelmingly singular/plural pairs). This is the single source of truth
//!   for the T1 veto in `text.rs` and for the NZ-English lexicon's exclusion
//!   rule; previously the list lived as a private static in `text.rs` with a
//!   literal copy in a `nz_english.rs` test.
//! * `lexicon.tsv` — additional macron-restoration headwords (`common`) and
//!   official place names (`proper`). **Inert until the file header reads
//!   `# status: reviewed`** (a te reo speaker has read both files). The parser
//!   also drops, and logs once, any row whose bare key is ambiguous, is an
//!   ordinary English word, or collides with another row — exclusion by
//!   construction — and a test asserts the committed file produces zero drops.
//!
//! Hot-path cost: one `OnceLock` parse per process; lookups are a sorted-slice
//! binary search, the same shape as the static tables in `nz_english.rs`.

use std::sync::OnceLock;

const AMBIGUOUS_TSV: &str = include_str!("ambiguous.tsv");
const LEXICON_TSV: &str = include_str!("lexicon.tsv");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexiconRow {
    pub bare: String,
    pub canonical: String,
    pub proper: bool,
}

#[derive(Debug, Default)]
pub struct Lexicon {
    /// Sorted, lowercase, macron-free bare keys that must never be auto-macronised.
    ambiguous: Vec<String>,
    /// (bare, macronised) pairs from `ambiguous.tsv`, sorted by bare.
    ambiguous_pairs: Vec<(String, String)>,
    /// Sorted by bare key. Empty when the file is not `reviewed`.
    rows: Vec<LexiconRow>,
    /// Header status of `lexicon.tsv`.
    pub status: String,
    /// Rows dropped by the exclusion rules (bare key, reason).
    pub dropped: Vec<(String, &'static str)>,
}

fn header_status(tsv: &str) -> String {
    tsv.lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("# status:")
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "missing".to_string())
}

fn data_rows(tsv: &str) -> impl Iterator<Item = Vec<&str>> {
    tsv.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|cols| cols.first().map(|c| *c != "bare").unwrap_or(false))
}

fn strip_macrons_lower(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{101}' | '\u{100}' => 'a',
            '\u{113}' | '\u{112}' => 'e',
            '\u{12b}' | '\u{12a}' => 'i',
            '\u{14d}' | '\u{14c}' => 'o',
            '\u{16b}' | '\u{16a}' => 'u',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}

pub(crate) fn parse(
    ambiguous_tsv: &str,
    lexicon_tsv: &str,
    is_common_en: &dyn Fn(&str) -> bool,
) -> Lexicon {
    let mut ambiguous_pairs: Vec<(String, String)> = data_rows(ambiguous_tsv)
        .filter(|c| c.len() >= 2)
        .map(|c| (strip_macrons_lower(c[0]), c[1].trim().to_string()))
        .filter(|(b, m)| !b.is_empty() && !m.is_empty())
        .collect();
    ambiguous_pairs.sort();
    ambiguous_pairs.dedup();
    let mut ambiguous: Vec<String> = ambiguous_pairs.iter().map(|(b, _)| b.clone()).collect();
    ambiguous.sort();
    ambiguous.dedup();

    let status = header_status(lexicon_tsv);
    let mut rows: Vec<LexiconRow> = Vec::new();
    let mut dropped: Vec<(String, &'static str)> = Vec::new();
    if status == "reviewed" {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for cols in data_rows(lexicon_tsv) {
            if cols.len() < 4 {
                dropped.push((
                    cols.first().unwrap_or(&"").to_string(),
                    "row has fewer than 4 columns",
                ));
                continue;
            }
            let bare = strip_macrons_lower(cols[0]);
            let canonical = cols[1].trim().to_string();
            if bare.is_empty() || canonical.is_empty() {
                dropped.push((bare, "empty key or canonical"));
                continue;
            }
            if strip_macrons_lower(&canonical) != bare {
                dropped.push((bare, "canonical differs from bare by more than macrons"));
                continue;
            }
            if ambiguous.binary_search(&bare).is_ok() {
                dropped.push((bare, "bare form is on the ambiguity list"));
                continue;
            }
            if is_common_en(&bare) {
                dropped.push((bare, "bare form is an ordinary English word"));
                continue;
            }
            if !seen.insert(bare.clone()) {
                dropped.push((bare, "duplicate bare key"));
                continue;
            }
            let proper = cols[2].trim().eq_ignore_ascii_case("proper");
            rows.push(LexiconRow {
                bare,
                canonical,
                proper,
            });
        }
        rows.sort_by(|a, b| a.bare.cmp(&b.bare));
    }
    Lexicon {
        ambiguous,
        ambiguous_pairs,
        rows,
        status,
        dropped,
    }
}

fn global() -> &'static Lexicon {
    static LEX: OnceLock<Lexicon> = OnceLock::new();
    LEX.get_or_init(|| {
        let lex = parse(
            AMBIGUOUS_TSV,
            LEXICON_TSV,
            &crate::audio_toolkit::text::is_common_en,
        );
        for (key, why) in &lex.dropped {
            log::warn!("reo_lexicon: dropped '{key}': {why}");
        }
        if lex.status != "reviewed" {
            log::info!(
                "reo_lexicon: lexicon.tsv status is '{}' — extra rows inert",
                lex.status
            );
        }
        lex
    })
}

/// True when `bare` (lowercase, macron-free) is a valid distinct word that must
/// never be auto-macronised. The T1 veto.
pub fn is_ambiguous_bare(bare: &str) -> bool {
    global()
        .ambiguous
        .binary_search_by(|k| k.as_str().cmp(bare))
        .is_ok()
}

/// All ambiguous bare keys, sorted (for tests and tooling).
pub fn ambiguous_keys() -> &'static [String] {
    &global().ambiguous
}

/// All (bare, macronised) ambiguity pairs, sorted by bare (for tests and tooling).
pub fn ambiguous_pairs() -> &'static [(String, String)] {
    &global().ambiguous_pairs
}

/// Lexicon lookup by bare key. Returns `(canonical, is_proper)` only when the
/// file is `reviewed`; otherwise always `None`.
pub fn lookup(bare: &str) -> Option<(&'static str, bool)> {
    let lex = global();
    lex.rows
        .binary_search_by(|r| r.bare.as_str().cmp(bare))
        .ok()
        .map(|i| (lex.rows[i].canonical.as_str(), lex.rows[i].proper))
}

/// Header status of the compiled-in `lexicon.tsv` (`draft` / `reviewed`).
pub fn status() -> &'static str {
    &global().status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_en(_: &str) -> bool {
        false
    }

    #[test]
    fn reo_lexicon_ambiguous_covers_every_t1_pair() {
        for w in [
            "wahine", "matua", "tangata", "keke", "tupuna", "tipuna", "teina", "tuakana", "tuahine",
        ] {
            assert!(is_ambiguous_bare(w), "{w} must be on the ambiguity list");
        }
        assert!(!is_ambiguous_bare("whanau"));
        assert!(!is_ambiguous_bare("korero"));
    }

    #[test]
    fn reo_lexicon_ambiguous_keys_sorted_and_bare() {
        let keys = ambiguous_keys();
        assert!(
            keys.windows(2).all(|w| w[0] < w[1]),
            "ambiguous keys must be sorted and unique"
        );
        for k in keys {
            assert_eq!(
                k,
                &strip_macrons_lower(k),
                "key '{k}' must be lowercase and macron-free"
            );
        }
        assert!(keys.len() >= 15);
    }

    #[test]
    fn reo_lexicon_committed_file_produces_zero_drops() {
        // Parse the committed file as if reviewed, so the exclusion rules are exercised on it.
        let forced = LEXICON_TSV.replacen("# status: draft", "# status: reviewed", 1);
        let lex = parse(
            AMBIGUOUS_TSV,
            &forced,
            &crate::audio_toolkit::text::is_common_en,
        );
        assert!(
            lex.dropped.is_empty(),
            "committed lexicon.tsv has rows the parser would drop: {:?}",
            lex.dropped
        );
    }

    #[test]
    fn reo_lexicon_draft_status_makes_rows_inert() {
        let lex = parse(
            "bare\tmacronised\treason\tsource\nwahine\twāhine\tT1\tx\n",
            "# status: draft\nbare\tcanonical\tkind\tsource\nwhanau\twhānau\tcommon\tx\n",
            &no_en,
        );
        assert_eq!(lex.status, "draft");
        assert!(lex.rows.is_empty());
        let lex = parse(
            "bare\tmacronised\treason\tsource\nwahine\twāhine\tT1\tx\n",
            "# status: reviewed\nbare\tcanonical\tkind\tsource\nwhanau\twhānau\tcommon\tx\n",
            &no_en,
        );
        assert_eq!(lex.rows.len(), 1);
        assert_eq!(lex.rows[0].canonical, "whānau");
    }

    #[test]
    fn reo_lexicon_exclusion_by_construction() {
        let lex = parse(
            "bare\tmacronised\treason\tsource\nwahine\twāhine\tT1\tx\n",
            "# status: reviewed\nbare\tcanonical\tkind\tsource\n\
             wahine\twāhine\tcommon\tx\n\
             mate\tmāte\tcommon\tx\n\
             whanau\twhānau\tcommon\tx\n\
             whanau\twhānau\tcommon\ty\n\
             kai\tkāhi\tcommon\tx\n\
             short\n",
            &|w| w == "mate",
        );
        assert_eq!(
            lex.rows.len(),
            1,
            "only the clean whānau row survives: {:?}",
            lex.dropped
        );
        let reasons: Vec<&str> = lex.dropped.iter().map(|(_, r)| *r).collect();
        assert!(reasons.contains(&"bare form is on the ambiguity list"));
        assert!(reasons.contains(&"bare form is an ordinary English word"));
        assert!(reasons.contains(&"duplicate bare key"));
        assert!(reasons.contains(&"canonical differs from bare by more than macrons"));
        assert!(reasons.contains(&"row has fewer than 4 columns"));
    }

    #[test]
    fn reo_lexicon_lookup_inert_while_draft() {
        // The committed file is draft: lookup must answer None for anything.
        assert_eq!(status(), "draft");
        assert!(lookup("whanau").is_none());
    }
}
