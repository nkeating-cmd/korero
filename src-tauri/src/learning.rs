//! Kōrero 1.52 (B14): learning mishearings from the user's own corrections.
//!
//! When the user corrects a dictation in History, the words they replaced are compared with
//! the words they put in their place. A replacement that looks like a mishearing of a name
//! ("foreign emergency" → "Fire and Emergency") is counted. The second time the same
//! replacement is made, Kōrero starts making it itself, on dictation only.
//!
//! What keeps this safe (red team, 26 Nov 2026):
//! - **Sound alone never rewrites.** "for an emergency" sounds closer to "Fire and Emergency"
//!   than "foreign emergency" does, so a sound match is only ever used to judge an edit the
//!   user actually made, never to change text by itself.
//! - **Names only.** The corrected side must look like a name (a capital inside it, a macron,
//!   or a new capital in the middle of a sentence), which leaves out rewording ("costs" →
//!   "cost", "their" → "there").
//! - **Seen twice.** One edit is watched; the same edit again makes it a rule.
//! - **Switches itself off.** If the user changes a learnt correction back, the rule is off
//!   and stays off.
//! - **Local and dictation only.** Learnt pairs are not taught corrections: they never reach
//!   meetings, notes, imports or any AI prompt until the user chooses "Use everywhere".
//!
//! This file is pure (no app state, no I/O) so every rule is unit-tested and fuzzed.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A replacement becomes a rule when it has been made this many times.
pub const ACTIVATE_AT: u32 = 2;
/// The most pairs kept. Watched pairs go first when the list is full.
pub const MAX_PAIRS: usize = 200;
/// The most pairs taken from one edit: an edit that changes more is a rewrite.
const MAX_PAIRS_PER_EDIT: usize = 3;
/// The longest run of words on either side of one replacement.
const MAX_SPAN_WORDS: usize = 4;
/// Texts longer than this (in words) are not compared.
const MAX_WORDS: usize = 600;

/// One replacement the user has made in History.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Type)]
pub struct LearnedPair {
    /// What Kōrero wrote: lower case, single spaces.
    pub heard: String,
    /// What the user put in its place, as they typed it.
    pub meant: String,
    /// How many separate edits made this replacement.
    #[serde(default)]
    pub seen: u32,
    /// Switched off, by the user or because they changed it back. Never learnt again.
    #[serde(default)]
    pub off: bool,
    /// The day it was last seen (yyyy-mm-dd), for the list in Settings.
    #[serde(default)]
    pub last_seen: String,
}

impl LearnedPair {
    /// True when Kōrero makes this replacement on dictation.
    pub fn active(&self) -> bool {
        !self.off && self.seen >= ACTIVATE_AT
    }
}

/// What one edit taught.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, Type)]
pub struct LearnOutcome {
    /// Pairs that became rules with this edit.
    pub learnt: Vec<LearnedPair>,
    /// Pairs seen for the first time (watched, not yet rules).
    pub watching: u32,
    /// Rules switched off because the user changed them back.
    pub switched_off: Vec<LearnedPair>,
}

/// A word without the punctuation around it.
fn core(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric())
}

fn fold_macron(c: char) -> char {
    match c {
        'ā' | 'Ā' => 'a',
        'ē' | 'Ē' => 'e',
        'ī' | 'Ī' => 'i',
        'ō' | 'Ō' => 'o',
        'ū' | 'Ū' => 'u',
        other => other,
    }
}

fn has_macron(s: &str) -> bool {
    s.chars().any(|c| fold_macron(c) != c)
}

/// Lower-case ASCII letters of a phrase, macrons folded, everything else dropped.
fn letters(phrase: &str) -> Vec<char> {
    phrase
        .chars()
        .map(fold_macron)
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_ascii_lowercase())
        .collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// A rough sound key for one word: its consonants, with letters that sound alike merged.
/// Deliberately coarse. It is only ever used to ask "could this edit be a mishearing?".
fn word_key(word: &str) -> Vec<char> {
    let w = letters(word);
    let n = w.len();
    let vowel = |c: char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
    let mut out: Vec<char> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let c = w[i];
        let next = w.get(i + 1).copied();
        let (sound, step): (Option<char>, usize) = match (c, next) {
            ('p', Some('h')) => (Some('f'), 2),
            ('g', Some('h')) => (None, 2),
            ('g', Some('n')) => (Some('n'), 2),
            ('k', Some('n')) if i == 0 => (Some('n'), 2),
            ('w', Some('r')) if i == 0 => (Some('r'), 2),
            ('c', Some('k')) => (Some('k'), 2),
            ('c', Some('h')) | ('s', Some('h')) => (Some('x'), 2),
            ('t', Some('h')) => (Some('t'), 2),
            ('q', Some('u')) => (Some('k'), 2),
            ('c', Some(e)) if matches!(e, 'e' | 'i' | 'y') => (Some('s'), 1),
            ('g', Some(e)) if matches!(e, 'e' | 'i' | 'y') => (Some('j'), 1),
            ('c', _) | ('q', _) | ('g', _) => (Some('k'), 1),
            ('x', _) => {
                out.push('k');
                (Some('s'), 1)
            }
            ('z', _) => (Some('s'), 1),
            ('d', _) => (Some('t'), 1),
            ('b', _) => (Some('p'), 1),
            ('v', _) => (Some('f'), 1),
            ('h', _) | ('w', _) => (None, 1),
            (v, _) if vowel(v) => (None, 1),
            (other, _) => (Some(other), 1),
        };
        if let Some(s) = sound {
            if out.last() != Some(&s) {
                out.push(s);
            }
        }
        i += step;
    }
    out
}

fn phrase_key(phrase: &str) -> Vec<char> {
    phrase.split_whitespace().flat_map(word_key).collect()
}

/// True when two phrases could be the same words heard differently.
pub fn sounds_alike(a: &str, b: &str) -> bool {
    let (ka, kb) = (phrase_key(a), phrase_key(b));
    if ka.is_empty() || kb.is_empty() {
        return false;
    }
    let longest = ka.len().max(kb.len());
    let d = levenshtein(&ka, &kb);
    let keys_close = if longest <= 2 {
        d == 0
    } else {
        d <= 3 && (d as f64) / (longest as f64) <= 0.34
    };
    if !keys_close {
        return false;
    }
    // The letters must be close too, so two words that share only their consonants
    // ("cost" and "cast" are fine; "pact" and "epoxied" are not) do not pass.
    let (la, lb) = (letters(a), letters(b));
    let longest = la.len().max(lb.len());
    longest > 0 && (levenshtein(&la, &lb) as f64) / (longest as f64) <= 0.6
}

/// True when `meant` reads as a name: a capital inside it, a macron, or a capital the user
/// added in the middle of a sentence. A capital that only starts a sentence is not enough.
fn name_like(heard_as_written: &str, meant: &str, at_sentence_start: bool) -> bool {
    if has_macron(meant) {
        return true;
    }
    let mut chars = meant.chars();
    let first = chars.next();
    if chars.any(|c| c.is_uppercase()) {
        return true;
    }
    let heard_first_lower = heard_as_written
        .chars()
        .next()
        .is_some_and(|c| c.is_lowercase());
    first.is_some_and(|c| c.is_uppercase()) && !at_sentence_start && heard_first_lower
}

/// Words that never take part: addresses, links and anything with a figure in it (those
/// belong to the address and number passes).
fn excluded(token: &str) -> bool {
    token.contains('@')
        || token.contains("://")
        || token.chars().any(|c| c.is_numeric())
        || core(token).is_empty()
}

fn ends_sentence(token: &str) -> bool {
    token
        .trim_end_matches(|c: char| matches!(c, '"' | '\'' | ')' | '”' | '’'))
        .ends_with(['.', '!', '?', ':'])
}

/// One replaced run: positions in the before and after word lists.
struct Span {
    b0: usize,
    b1: usize,
    a0: usize,
    a1: usize,
}

/// The runs of words that were replaced by other words (insertions and deletions alone are
/// not replacements). Words are compared without their punctuation, case included.
fn replaced_spans(before: &[&str], after: &[&str]) -> Vec<Span> {
    let (n, m) = (before.len(), after.len());
    // Longest common subsequence, by table.
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if core(before[i]) == core(after[j]) {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut spans = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut b0, mut a0) = (0, 0);
    let mut open = false;
    let mut close = |open: &mut bool, b0: usize, i: usize, a0: usize, j: usize| {
        if *open {
            if i > b0 && j > a0 {
                spans.push(Span { b0, b1: i, a0, a1: j });
            }
            *open = false;
        }
    };
    while i < n || j < m {
        if i < n && j < m && core(before[i]) == core(after[j]) {
            close(&mut open, b0, i, a0, j);
            i += 1;
            j += 1;
            continue;
        }
        if !open {
            open = true;
            b0 = i;
            a0 = j;
        }
        if j >= m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1]) {
            i += 1;
        } else {
            j += 1;
        }
    }
    close(&mut open, b0, i, a0, j);
    spans
}

fn join_cores(tokens: &[&str]) -> String {
    tokens
        .iter()
        .map(|t| core(t))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The replacements in an edit that look like mishearings of a name: (heard, meant), heard in
/// lower case. `is_common` says whether a lower-case word is ordinary English.
pub fn edit_pairs(
    before: &str,
    after: &str,
    is_common: &dyn Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let b: Vec<&str> = before.split_whitespace().collect();
    let a: Vec<&str> = after.split_whitespace().collect();
    if b.is_empty() || a.is_empty() || b.len() > MAX_WORDS || a.len() > MAX_WORDS {
        return Vec::new();
    }
    let spans = replaced_spans(&b, &a);
    let mut out: Vec<(String, String)> = Vec::new();
    for s in &spans {
        let (mut b0, mut b1, mut a0, mut a1) = (s.b0, s.b1, s.a0, s.a1);
        if b1 - b0 > MAX_SPAN_WORDS || a1 - a0 > MAX_SPAN_WORDS {
            continue;
        }
        // One word replaced by several ("foreign" → "Fire and"), or an ordinary word
        // replaced ("mark" → "Marc"), is too broad to be a rule by itself: it would rewrite
        // every "foreign". Take the unchanged word beside it as context.
        let one_word = b1 - b0 == 1;
        let heard_word = core(b[b0]).to_lowercase();
        let needs_context = one_word && (a1 - a0 != 1 || is_common(&heard_word));
        if needs_context {
            if b1 < b.len() && a1 < a.len() && core(b[b1]) == core(a[a1]) && !excluded(b[b1]) {
                b1 += 1;
                a1 += 1;
            } else if b0 > 0 && a0 > 0 && core(b[b0 - 1]) == core(a[a0 - 1]) && !excluded(b[b0 - 1])
            {
                b0 -= 1;
                a0 -= 1;
            } else {
                continue;
            }
        }
        if b[b0..b1].iter().chain(a[a0..a1].iter()).any(|t| excluded(t)) {
            continue;
        }
        let heard_written = join_cores(&b[b0..b1]);
        let meant = join_cores(&a[a0..a1]);
        let heard = heard_written.to_lowercase();
        if heard.chars().count() < 3 || meant.chars().count() > 60 || heard_written == meant {
            continue;
        }
        // A change of nothing but the first letter's case is sentence punctuation, not a name.
        if heard == meant.to_lowercase() {
            let rest_same = heard_written.chars().skip(1).eq(meant.chars().skip(1));
            if rest_same {
                continue;
            }
        }
        // Several ordinary words replaced, all of them ordinary: the user reworded. (One
        // ordinary word replaced by a name is kept, with its neighbour as context, above.)
        if !needs_context && heard.split(' ').all(|w| is_common(w)) {
            continue;
        }
        let at_start = a0 == 0 || ends_sentence(a[a0 - 1]);
        if !name_like(&heard_written, &meant, at_start) {
            continue;
        }
        if !sounds_alike(&heard, &meant) {
            continue;
        }
        if !out.iter().any(|(h, _)| *h == heard) {
            out.push((heard, meant));
        }
    }
    if out.len() > MAX_PAIRS_PER_EDIT {
        // More than a few replacements in one edit is a rewrite: learn nothing from it.
        return Vec::new();
    }
    out
}

/// True when `needle` (lower case) is in `haystack` as whole words, case ignored.
fn contains_words(haystack: &str, needle: &str) -> bool {
    let hay: Vec<String> = haystack
        .split_whitespace()
        .map(|t| core(t).to_lowercase())
        .collect();
    let need: Vec<String> = needle
        .split_whitespace()
        .map(|t| core(t).to_lowercase())
        .collect();
    if need.is_empty() || need.len() > hay.len() {
        return false;
    }
    hay.windows(need.len()).any(|w| w == need.as_slice())
}

/// Record what one History edit taught. `original` is the dictation as heard, `before` the
/// text the user started editing, `after` what they saved. `taught` are the heard sides of
/// corrections the user already taught by hand (those are left to them). Returns what
/// changed; `pairs` is updated in place.
pub fn observe(
    pairs: &mut Vec<LearnedPair>,
    original: &str,
    before: &str,
    after: &str,
    taught: &[String],
    today: &str,
    is_common: &dyn Fn(&str) -> bool,
) -> LearnOutcome {
    let mut outcome = LearnOutcome::default();

    // A rule Kōrero applied to this dictation, changed back by the user: switch it off.
    for p in pairs.iter_mut().filter(|p| p.active()) {
        if contains_words(original, &p.heard)
            && contains_words(before, &p.meant)
            && !contains_words(after, &p.meant)
        {
            p.off = true;
            outcome.switched_off.push(p.clone());
        }
    }

    for (heard, meant) in edit_pairs(before, after, is_common) {
        if taught.iter().any(|t| t.trim().to_lowercase() == heard) {
            continue;
        }
        match pairs.iter_mut().find(|p| p.heard == heard) {
            Some(p) if p.off => {}
            Some(p) if p.meant == meant => {
                let was_active = p.active();
                p.seen = p.seen.saturating_add(1);
                p.last_seen = today.to_string();
                if !was_active && p.active() {
                    outcome.learnt.push(p.clone());
                }
            }
            Some(p) => {
                // The same words corrected to something else: start counting again.
                p.meant = meant;
                p.seen = 1;
                p.last_seen = today.to_string();
                outcome.watching += 1;
            }
            None => {
                pairs.push(LearnedPair {
                    heard,
                    meant,
                    seen: 1,
                    off: false,
                    last_seen: today.to_string(),
                });
                outcome.watching += 1;
            }
        }
    }

    // Keep the list bounded: watched pairs go first, oldest first.
    while pairs.len() > MAX_PAIRS {
        let drop = pairs
            .iter()
            .position(|p| !p.active() && !p.off)
            .or_else(|| pairs.iter().position(|p| !p.active()))
            .unwrap_or(0);
        pairs.remove(drop);
    }
    outcome
}

/// The rules Kōrero applies on dictation: (heard, meant).
pub fn active_rules(pairs: &[LearnedPair]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter(|p| p.active() && !p.heard.trim().is_empty() && !p.meant.trim().is_empty())
        .map(|p| (p.heard.clone(), p.meant.clone()))
        .collect()
}

/// A list from the Settings page, made safe to store: trimmed, heard in lower case, empty
/// and duplicate rows dropped, bounded.
pub fn sanitise(pairs: Vec<LearnedPair>) -> Vec<LearnedPair> {
    let mut out: Vec<LearnedPair> = Vec::new();
    for mut p in pairs {
        p.heard = p
            .heard
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        p.meant = p.meant.split_whitespace().collect::<Vec<_>>().join(" ");
        if p.heard.is_empty() || p.meant.is_empty() || p.meant.chars().count() > 60 {
            continue;
        }
        if out.iter().any(|q| q.heard == p.heard) {
            continue;
        }
        out.push(p);
        if out.len() == MAX_PAIRS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small stand-in for the app's ordinary-English list.
    fn common(w: &str) -> bool {
        const WORDS: &[&str] = &[
            "a", "an", "and", "the", "for", "to", "of", "in", "is", "it", "we", "i", "their",
            "there", "cost", "costs", "mark", "said", "front", "friend", "first", "find", "fire",
            "meeting", "with", "today", "on", "at", "was", "price", "will", "be", "our", "this",
        ];
        WORDS.contains(&w)
    }

    fn pairs_of(before: &str, after: &str) -> Vec<(String, String)> {
        edit_pairs(before, after, &common)
    }

    fn one(h: &str, m: &str) -> Vec<(String, String)> {
        vec![(h.to_string(), m.to_string())]
    }

    #[test]
    fn the_case_that_started_it() {
        assert_eq!(
            pairs_of(
                "I am working with foreign emergency on the plan.",
                "I am working with Fire and Emergency on the plan."
            ),
            one("foreign emergency", "Fire and Emergency")
        );
        // Typed without the second capital: the unchanged word comes along as context, so
        // the rule is never the bare word "foreign".
        assert_eq!(
            pairs_of(
                "working with foreign emergency today",
                "working with Fire and emergency today"
            ),
            one("foreign emergency", "Fire and emergency")
        );
    }

    #[test]
    fn names_and_acronyms_are_learnt() {
        assert_eq!(
            pairs_of("we met the fens team", "we met the FENZ team"),
            one("fens", "FENZ")
        );
        assert_eq!(
            pairs_of("send it to kite today", "send it to Kyt today"),
            one("kite", "Kyt")
        );
        assert_eq!(
            pairs_of("our far no is here", "our whānau is here"),
            Vec::<(String, String)>::new(),
            "too far apart in sound to be a mishearing"
        );
        assert_eq!(
            pairs_of("our whanau is here", "our whānau is here"),
            one("whanau", "whānau")
        );
        // An ordinary word replaced by a name takes its neighbour as context.
        assert_eq!(
            pairs_of("then mark said yes", "then Marc said yes"),
            one("mark said", "Marc said")
        );
    }

    #[test]
    fn rewording_is_not_learnt() {
        for (b, a) in [
            ("the costs are high", "the cost is high"),
            ("it is over their", "it is over there"),
            ("we had a meeting", "we had a workshop"),
            ("the price will be fine", "the pricing will be fine"),
            ("see you tuesday then", "see you thursday then"),
            ("See you Tuesday then", "See you Thursday then"),
            ("this is good", "This is good"),
            ("we agreed. their plan works", "we agreed. There plan works"),
            ("a plan for an emergency", "a plan for an emergency."),
            ("we talked about it", "we talked about it and more"),
            ("we talked about it at length", "we talked at length"),
            ("completely different words here", "Nothing Alike At All"),
        ] {
            assert_eq!(pairs_of(b, a), Vec::<(String, String)>::new(), "{b:?} -> {a:?}");
        }
    }

    #[test]
    fn sound_alone_is_judged_only_on_real_edits() {
        // The red team's table: these all sound like "Fire and Emergency". None of them is
        // ever touched unless the user made that edit, and an edit of ordinary words to a
        // name still has to pass the other rules.
        assert!(sounds_alike("foreign emergency", "Fire and Emergency"));
        assert!(sounds_alike("for an emergency", "Fire and Emergency"));
        assert!(sounds_alike("family emergency", "Fire and Emergency"), "coarse on purpose");
        assert!(!sounds_alike("budget meeting", "Fire and Emergency"));
        assert!(!sounds_alike("costs", "Pricing"));
        assert!(sounds_alike("nick", "Nic"));
        assert!(sounds_alike("fens", "FENZ"));
        assert!(!sounds_alike("", "FENZ"));
        assert!(!sounds_alike("a", "I"));
    }

    #[test]
    fn addresses_links_and_figures_are_left_alone() {
        for (b, a) in [
            ("email nick@kite.co.nz now", "email Nic@Kyt.nz now"),
            ("see https://kite.nz today", "see https://Kyt.nz today"),
            ("it was 3 pm", "it was 3 PM"),
        ] {
            assert_eq!(pairs_of(b, a), Vec::<(String, String)>::new(), "{b:?}");
        }
    }

    #[test]
    fn a_big_rewrite_teaches_nothing() {
        let b = "so kite and fens and nick and tamaki went";
        let a = "so Kyt and FENZ and Nic and Tāmaki went";
        assert_eq!(pairs_of(b, a), Vec::<(String, String)>::new());
    }

    fn obs(pairs: &mut Vec<LearnedPair>, orig: &str, b: &str, a: &str) -> LearnOutcome {
        observe(pairs, orig, b, a, &[], "2026-11-26", &common)
    }

    #[test]
    fn seen_twice_makes_a_rule() {
        let mut pairs = Vec::new();
        let b = "working with foreign emergency";
        let a = "working with Fire and Emergency";
        let first = obs(&mut pairs, b, b, a);
        assert_eq!((first.watching, first.learnt.len()), (1, 0));
        assert!(active_rules(&pairs).is_empty(), "one edit is only watched");
        let second = obs(&mut pairs, b, b, a);
        assert_eq!(second.learnt.len(), 1);
        assert_eq!(
            active_rules(&pairs),
            one("foreign emergency", "Fire and Emergency")
        );
        // A third time changes nothing and announces nothing.
        let third = obs(&mut pairs, b, b, a);
        assert!(third.learnt.is_empty());
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].seen, 3);
    }

    #[test]
    fn two_in_one_edit_count_once() {
        let mut pairs = Vec::new();
        let b = "the fens team and the fens office";
        let a = "the FENZ team and the FENZ office";
        obs(&mut pairs, b, b, a);
        assert_eq!(pairs[0].seen, 1);
        assert!(active_rules(&pairs).is_empty());
    }

    #[test]
    fn changing_it_back_switches_it_off_for_good() {
        let mut pairs = vec![LearnedPair {
            heard: "foreign emergency".into(),
            meant: "Fire and Emergency".into(),
            seen: 2,
            off: false,
            last_seen: String::new(),
        }];
        // Kōrero applied the rule; this time the user really did mean the words as heard.
        let out = obs(
            &mut pairs,
            "a foreign emergency was declared",
            "a Fire and Emergency was declared",
            "a foreign emergency was declared",
        );
        assert_eq!(out.switched_off.len(), 1);
        assert!(pairs[0].off);
        assert!(active_rules(&pairs).is_empty());
        // Making the old edit again does not bring it back.
        let b = "working with foreign emergency";
        obs(&mut pairs, b, b, "working with Fire and Emergency");
        obs(&mut pairs, b, b, "working with Fire and Emergency");
        assert!(active_rules(&pairs).is_empty());
    }

    #[test]
    fn an_unrelated_edit_does_not_switch_a_rule_off() {
        let mut pairs = vec![LearnedPair {
            heard: "fens".into(),
            meant: "FENZ".into(),
            seen: 2,
            off: false,
            last_seen: String::new(),
        }];
        let out = obs(
            &mut pairs,
            "the fens team met on monday",
            "the FENZ team met on monday",
            "the FENZ team met on Tuesday",
        );
        assert!(out.switched_off.is_empty());
        assert!(pairs[0].active());
    }

    #[test]
    fn a_different_correction_starts_the_count_again() {
        let mut pairs = Vec::new();
        let b = "send it to kite today";
        obs(&mut pairs, b, b, "send it to Kyt today");
        obs(&mut pairs, b, b, "send it to Kyte today");
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].meant.as_str(), pairs[0].seen), ("Kyte", 1));
        assert!(active_rules(&pairs).is_empty());
    }

    #[test]
    fn hand_taught_corrections_are_left_to_the_user() {
        let mut pairs = Vec::new();
        let b = "the fens team";
        let out = observe(
            &mut pairs,
            b,
            b,
            "the FENZ team",
            &["Fens".to_string()],
            "2026-11-26",
            &common,
        );
        assert_eq!(out, LearnOutcome::default());
        assert!(pairs.is_empty());
    }

    #[test]
    fn the_list_is_bounded_and_rules_outlive_watched_pairs() {
        let mut pairs: Vec<LearnedPair> = (0..MAX_PAIRS)
            .map(|i| LearnedPair {
                heard: format!("word{i}"),
                meant: format!("Word{i}"),
                seen: if i == 0 { 2 } else { 1 },
                off: false,
                last_seen: String::new(),
            })
            .collect();
        let b = "send it to kite today";
        obs(&mut pairs, b, b, "send it to Kyt today");
        assert_eq!(pairs.len(), MAX_PAIRS);
        assert!(pairs.iter().any(|p| p.heard == "word0"), "the rule was kept");
        assert!(pairs.iter().any(|p| p.heard == "kite"));
    }

    #[test]
    fn sanitise_cleans_what_the_page_sends() {
        let p = |h: &str, m: &str| LearnedPair {
            heard: h.into(),
            meant: m.into(),
            seen: 2,
            off: false,
            last_seen: String::new(),
        };
        let out = sanitise(vec![
            p("  Foreign   Emergency ", " Fire and  Emergency "),
            p("foreign emergency", "Other"),
            p("", "X"),
            p("x", ""),
            p("long", &"y".repeat(61)),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].heard, "foreign emergency");
        assert_eq!(out[0].meant, "Fire and Emergency");
    }

    #[test]
    fn old_settings_without_the_new_fields_still_load() {
        let p: LearnedPair = serde_json::from_str(r#"{"heard":"fens","meant":"FENZ"}"#).unwrap();
        assert_eq!((p.seen, p.off), (0, false));
        assert!(!p.active());
    }

    /// No input panics, and the same input always gives the same answer. Multi-byte spaces
    /// and punctuation are in the alphabet on purpose (L-020).
    #[test]
    fn fuzz_never_panics_and_is_stable() {
        const BITS: &[&str] = &[
            "foreign", "emergency", "Fire", "and", "Emergency", "fens", "FENZ", "kite", "Kyt",
            "the", "a", "\u{a0}", "\u{3000}", " ", "  ", ".", ",", "!", "whānau", "whanau", "é",
            "nick@kite.nz", "3", "$5", "\n", "\t", "'", "\"", "—", "日本", "🙂", "A", "I", "",
        ];
        let mut seed: u64 = 0x5eed_1234_abcd_ef01;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let mut text = |next: &mut dyn FnMut() -> usize| {
            let n = next() % 12;
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(BITS[next() % BITS.len()]);
                if next() % 3 != 0 {
                    s.push(' ');
                }
            }
            s
        };
        let mut pairs = Vec::new();
        for _ in 0..40_000 {
            let (o, b, a) = (text(&mut next), text(&mut next), text(&mut next));
            let first = edit_pairs(&b, &a, &common);
            assert_eq!(first, edit_pairs(&b, &a, &common));
            assert!(first.len() <= MAX_PAIRS_PER_EDIT);
            for (h, m) in &first {
                assert!(!h.is_empty() && !m.is_empty());
                assert_eq!(*h, h.to_lowercase());
            }
            let _ = sounds_alike(&b, &a);
            obs(&mut pairs, &o, &b, &a);
            assert!(pairs.len() <= MAX_PAIRS);
        }
    }
}
