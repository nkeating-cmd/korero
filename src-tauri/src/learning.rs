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
//!   user actually made, never to change text by itself. (If the user does make that edit
//!   twice, it is learnt like any other: it is their correction.)
//! - **Names only.** The corrected side must look like a name (a capital inside it, a macron,
//!   or a new capital in the middle of a sentence), which leaves out rewording ("costs" →
//!   "cost", "their" → "there").
//! - **A single word is almost never a rule by itself.** A rule for one word changes every use
//!   of it, and a word Kōrero heard may be a real word or another person's name ("minister" →
//!   "Ministry", "Tim" → "Tom", "kite" → "Kyt"). So a single word takes the unchanged word
//!   beside it as context and is learnt as a phrase. It is also counted as a *suggestion*:
//!   after three corrections Kōrero offers, once, to always write it, and the user decides.
//!   The one exception is a word whose corrected form has a capital inside it and different
//!   letters (FENZ from "fens", GitHub): that becomes a rule after three corrections.
//! - **A macron alone is never applied automatically.** "marama" and "mārama" are different
//!   words, and no list of such pairs is complete, so a macron-only change is a suggestion at
//!   most (and not even that for the pairs Kōrero knows are different words).
//! - **Seen twice.** One edit is watched; the same edit again makes a phrase a rule.
//! - **Never ordinary words alone.** If every word Kōrero wrote is ordinary English, nothing
//!   is learnt. A te reo word whose macron-free form is a different word (tangata, tāngata)
//!   is never learnt, nor is a possessive or plural of the same word ("Chris" → "Chris's").
//! - **Switches itself off.** If the user changes a learnt correction back, the rule is off
//!   and stays off.
//! - **Local and dictation only.** Learnt pairs are not taught corrections: they are stored
//!   on this computer, never reach meetings, notes or imports, and are never given to an AI as
//!   a list, until the user chooses "Use everywhere". (A dictation Kōrero has corrected is
//!   cleaned up like any other when the user asks for a clean-up.)
//!
//! This file is pure (no app state, no I/O) so every rule is unit-tested and fuzzed.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A phrase becomes a rule when its replacement has been made this many times.
pub const ACTIVATE_AT: u32 = 2;
/// A single word needs one more: its rule changes every use of that word.
pub const ACTIVATE_AT_ONE_WORD: u32 = 3;
/// The most pairs kept.
pub const MAX_PAIRS: usize = 200;
/// The most different corrections taken from one edit: an edit that changes more is a rewrite.
const MAX_PAIRS_PER_EDIT: usize = 3;
/// The most pairs kept from one edit (one correction can give a word and several phrases).
const MAX_FOUND_PER_EDIT: usize = 8;
/// The longest run of words on either side of one replacement.
const MAX_SPAN_WORDS: usize = 4;
/// Texts longer than this (in words) are not compared.
const MAX_WORDS: usize = 600;
/// The longest corrected side kept, in characters.
const MAX_MEANT_CHARS: usize = 60;

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
    /// A single word counted so Kōrero can offer it: never applied until the user says so.
    #[serde(default)]
    pub suggest_only: bool,
}

/// A suggestion is offered when its correction has been made this many times.
pub const SUGGEST_AT: u32 = 3;

impl LearnedPair {
    /// How many times this replacement must be made before it is a rule.
    pub fn needed(&self) -> u32 {
        if self.heard.contains(' ') {
            ACTIVATE_AT
        } else {
            ACTIVATE_AT_ONE_WORD
        }
    }

    /// True when Kōrero makes this replacement on dictation.
    pub fn active(&self) -> bool {
        !self.off && !self.suggest_only && self.seen >= self.needed()
    }
}

/// What one edit taught.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, Type)]
pub struct LearnOutcome {
    /// Pairs that became rules with this edit.
    pub learnt: Vec<LearnedPair>,
    /// Pairs counted but not yet rules.
    pub watching: u32,
    /// Rules switched off because the user changed them back.
    pub switched_off: Vec<LearnedPair>,
    /// Single words corrected often enough, with this edit, to offer as a taught correction.
    #[serde(default)]
    pub suggested: Vec<LearnedPair>,
}

/// One replacement found in an edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Lower case.
    pub heard: String,
    pub meant: String,
    /// A single word: counted towards a suggestion, never a rule.
    pub suggest_only: bool,
}

/// The two word lists the rules lean on, passed in so this file stays pure.
pub struct Lexicon<'a> {
    /// True for a lower-case ordinary English word.
    pub common: &'a dyn Fn(&str) -> bool,
    /// True for a lower-case, macron-free te reo word that is a different word from its
    /// macronised form (the singular of a plural marked by a long vowel).
    pub ambiguous: &'a dyn Fn(&str) -> bool,
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

fn without_macrons(s: &str) -> String {
    s.chars().map(fold_macron).collect()
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

/// True when two phrases have the same sound key: the strictest sound test.
fn sounds_the_same(a: &str, b: &str) -> bool {
    let (ka, kb) = (phrase_key(a), phrase_key(b));
    !ka.is_empty() && ka == kb
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
    // The letters must be close too, so two words that share only their consonants do not pass.
    let (la, lb) = (letters(a), letters(b));
    let longest = la.len().max(lb.len());
    longest > 0 && (levenshtein(&la, &lb) as f64) / (longest as f64) <= 0.6
}

/// True when a word (without its edge punctuation) is plain enough to learn from or to
/// replace: letters, with an apostrophe or hyphen inside at most. That leaves out addresses
/// ("nick@kite.nz"), web names ("kite.co.nz"), figures and anything joined by other marks.
fn plain_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, '\'' | '’' | '-'))
        && word.chars().any(|c| c.is_alphabetic())
}

fn ends_sentence(token: &str) -> bool {
    token
        .trim_end_matches(|c: char| matches!(c, '"' | '\'' | ')' | '”' | '’'))
        .ends_with(['.', '!', '?'])
}

fn first_upper(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase())
}

fn first_lower(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_lowercase())
}

/// True when `meant` reads as a name the transcriber got wrong.
fn name_like(heard_written: &str, meant: &str, at_sentence_start: bool) -> bool {
    if has_macron(meant) {
        return true;
    }
    // A capital inside it: FENZ, GitHub, Fire and Emergency.
    if meant.chars().skip(1).any(|c| c.is_uppercase()) {
        return true;
    }
    if !first_upper(meant) {
        return false;
    }
    // The user added a capital the transcriber did not write.
    if first_lower(heard_written) {
        return true;
    }
    // Both capitalised ("Nick" → "Nic"): only mid-sentence, and only when they sound the same,
    // so a change of meaning ("Tuesday" → "Thursday") is not learnt.
    !at_sentence_start && sounds_the_same(heard_written, meant)
}

/// True when one side is the other plus a possessive or plural ending.
fn same_word_with_an_ending(heard: &str, meant: &str) -> bool {
    let (h, m) = (heard.to_lowercase(), meant.to_lowercase());
    let ending = |long: &str, short: &str| {
        long.strip_prefix(short)
            .is_some_and(|rest| matches!(rest, "s" | "es" | "'s" | "’s" | "'" | "’"))
    };
    ending(&m, &h) || ending(&h, &m)
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
fn replaced_spans(full_before: &[&str], full_after: &[&str]) -> Vec<Span> {
    // Set aside the words both texts start and end with, so one correction in the middle is
    // one span even when a word inside it ("and") also appears nearby.
    let same = |x: &str, y: &str| core(x) == core(y);
    let lead = full_before
        .iter()
        .zip(full_after)
        .take_while(|(x, y)| same(x, y))
        .count();
    let tail = full_before[lead..]
        .iter()
        .rev()
        .zip(full_after[lead..].iter().rev())
        .take_while(|(x, y)| same(x, y))
        .count();
    let before = &full_before[lead..full_before.len() - tail];
    let after = &full_after[lead..full_after.len() - tail];
    let (n, m) = (before.len(), after.len());
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
                spans.push(Span {
                    b0: b0 + lead,
                    b1: i + lead,
                    a0: a0 + lead,
                    a1: j + lead,
                });
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
    tokens.iter().map(|t| core(t)).collect::<Vec<_>>().join(" ")
}

/// True when the only change between two runs of words is the capital on the first letter of
/// some of them ("the bank" → "the Bank"): sentence case or emphasis, not a name.
fn only_first_letters_differ(heard: &[&str], meant: &[&str]) -> bool {
    heard.len() == meant.len()
        && heard.iter().zip(meant).all(|(h, m)| {
            let (h, m) = (core(h), core(m));
            h.to_lowercase() == m.to_lowercase() && h.chars().skip(1).eq(m.chars().skip(1))
        })
}

/// True when the corrected form has a capital inside it and is not just the heard word in
/// capitals ("aid" → "AID" is emphasis as often as an acronym).
fn distinct_inner_capital(heard: &str, meant: &str) -> bool {
    meant.chars().skip(1).any(|c| c.is_uppercase())
        && !(meant.to_lowercase() == heard && meant.chars().all(|c| !c.is_lowercase()))
}

/// The checks every pair must pass, whatever its kind.
fn acceptable(heard_written: &str, meant: &str, heard: &str) -> bool {
    heard.chars().count() >= 3
        && meant.chars().count() <= MAX_MEANT_CHARS
        && heard_written != meant
        && sounds_alike(heard, meant)
}

/// The replacements in an edit that look like mishearings of a name, heard in lower case.
pub fn edit_pairs(before: &str, after: &str, lex: &Lexicon) -> Vec<Found> {
    let b: Vec<&str> = before.split_whitespace().collect();
    let a: Vec<&str> = after.split_whitespace().collect();
    if b.is_empty() || a.is_empty() || b.len() > MAX_WORDS || a.len() > MAX_WORDS {
        return Vec::new();
    }
    // Punctuation between the words ("foreign, emergency") is not one phrase.
    let joined = |tokens: &[&str]| {
        tokens.iter().enumerate().all(|(k, t)| {
            (k + 1 == tokens.len() || t.ends_with(core(t))) && (k == 0 || t.starts_with(core(t)))
        })
    };
    let spans = replaced_spans(&b, &a);
    let mut out: Vec<Found> = Vec::new();
    // The different corrections made in this edit (the same word fixed four times is one).
    let mut changes: Vec<(String, String)> = Vec::new();
    for s in &spans {
        let (mut b0, mut b1, mut a0, mut a1) = (s.b0, s.b1, s.a0, s.a1);
        if b1 - b0 > MAX_SPAN_WORDS || a1 - a0 > MAX_SPAN_WORDS {
            continue;
        }
        if only_first_letters_differ(&b[b0..b1], &a[a0..a1]) {
            continue;
        }
        let at_start = a0 == 0 || ends_sentence(a[a0 - 1]);
        let changed_heard = join_cores(&b[b0..b1]);
        let changed_meant = join_cores(&a[a0..a1]);
        if !changed_heard
            .split(' ')
            .chain(changed_meant.split(' '))
            .all(plain_word)
        {
            continue;
        }
        if !name_like(&changed_heard, &changed_meant, at_start) {
            continue;
        }
        // A possessive or plural of the same word ("Chris" → "Chris's") is grammar.
        if same_word_with_an_ending(&changed_heard, &changed_meant) {
            continue;
        }
        let changed_lower = changed_heard.to_lowercase();
        // Only the macrons changed. Two such forms can be different words (marama, mārama),
        // so this is never applied automatically; known different words are not even offered.
        let macron_only = has_macron(&changed_meant)
            && changed_lower == without_macrons(&changed_meant.to_lowercase());
        if macron_only && changed_lower.split(' ').any(|w| (lex.ambiguous)(w)) {
            continue;
        }
        let before_this_span = out.len();
        let mut push = |heard: String, meant: String, suggest_only: bool| {
            if !out.iter().any(|f| f.heard == heard) {
                out.push(Found {
                    heard,
                    meant,
                    suggest_only,
                });
            }
        };

        if b1 - b0 == 1 && a1 - a0 == 1 {
            // One word for one word. A rule for one word changes every use of it.
            if !acceptable(&changed_heard, &changed_meant, &changed_lower) {
                continue;
            }
            // An ordinary word only put in capitals ("the NEW plan") is emphasis.
            if (lex.common)(&changed_lower) && changed_meant.to_lowercase() == changed_lower {
                continue;
            }
            // Left alone elsewhere in the text just corrected: it is a real word too.
            if count_heard(after, &[changed_lower.clone()], &changed_meant) > 0 {
                continue;
            }
            let bare_rule = !macron_only
                && !(lex.common)(&changed_lower)
                && distinct_inner_capital(&changed_lower, &changed_meant);
            push(changed_lower.clone(), changed_meant.clone(), !bare_rule);
            if bare_rule || macron_only {
                note_change(&mut changes, &changed_lower, &changed_meant);
                continue;
            }
            // Otherwise it is also learnt as a phrase, with the unchanged word beside it.
        } else if macron_only {
            continue;
        }

        if b1 - b0 == 1 {
            // One word is too broad to be a rule by itself: it would rewrite every "foreign",
            // every "minister", every "Tim". Take the unchanged word beside it as context.
            if b1 < b.len() && a1 < a.len() && core(b[b1]) == core(a[a1]) {
                b1 += 1;
                a1 += 1;
            } else if b0 > 0 && a0 > 0 && core(b[b0 - 1]) == core(a[a0 - 1]) {
                b0 -= 1;
                a0 -= 1;
            } else {
                if out.len() > before_this_span {
                    note_change(&mut changes, &changed_lower, &changed_meant);
                }
                continue;
            }
        }
        let heard_words: Vec<&str> = b[b0..b1].iter().map(|t| core(t)).collect();
        let meant_words: Vec<&str> = a[a0..a1].iter().map(|t| core(t)).collect();
        let phrase_ok = heard_words.iter().chain(meant_words.iter()).all(|w| plain_word(w))
            && joined(&b[b0..b1])
            && joined(&a[a0..a1]);
        let heard_written = heard_words.join(" ");
        let meant = meant_words.join(" ");
        let heard = heard_written.to_lowercase();
        // Every word Kōrero wrote is ordinary English: the user reworded, or the name is
        // also an ordinary word ("mark said", "who is"). Too risky to make a rule.
        if phrase_ok
            && acceptable(&heard_written, &meant, &heard)
            && !heard.split(' ').all(|w| (lex.common)(w))
        {
            push(heard, meant, false);
        }
        if out.len() > before_this_span {
            note_change(&mut changes, &changed_lower, &changed_meant);
        }
    }
    if changes.len() > MAX_PAIRS_PER_EDIT {
        // More than a few different replacements in one edit is a rewrite: learn nothing.
        return Vec::new();
    }
    out.truncate(MAX_FOUND_PER_EDIT);
    out
}

fn note_change(changes: &mut Vec<(String, String)>, heard: &str, meant: &str) {
    if !changes.iter().any(|(h, m)| h == heard && m == meant) {
        changes.push((heard.to_string(), meant.to_string()));
    }
}

/// A word of the text with where its core (the word without edge punctuation) sits.
struct Tok<'a> {
    core: &'a str,
    core_start: usize,
    core_end: usize,
    start: usize,
    end: usize,
}

fn toks(text: &str) -> Vec<Tok<'_>> {
    let base = text.as_ptr() as usize;
    text.split_whitespace()
        .map(|t| {
            let start = t.as_ptr() as usize - base;
            let c = core(t);
            let core_start = if c.is_empty() {
                start
            } else {
                c.as_ptr() as usize - base
            };
            Tok {
                core: c,
                core_start,
                core_end: core_start + c.len(),
                start,
                end: start + t.len(),
            }
        })
        .collect()
}

/// True when `words` (lower case) are the words at `i`, as one unbroken phrase.
fn phrase_at(toks: &[Tok], i: usize, words: &[String]) -> bool {
    let n = words.len();
    if n == 0 || i + n > toks.len() {
        return false;
    }
    (0..n).all(|k| {
        let t = &toks[i + k];
        plain_word(t.core)
            && t.core.to_lowercase() == words[k]
            && (k + 1 == n || t.core_end == t.end)
            && (k == 0 || t.core_start == t.start)
    })
}

/// The text between the first word's start and the last word's end.
fn written_at<'a>(text: &'a str, toks: &[Tok], i: usize, n: usize) -> &'a str {
    &text[toks[i].core_start..toks[i + n - 1].core_end]
}

/// Equal, or equal but for the case of the first letter (a rule applied at the start of a
/// sentence).
fn same_but_first_case(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (a.chars(), b.chars());
    match (x.next(), y.next()) {
        (Some(p), Some(q)) => p.to_lowercase().eq(q.to_lowercase()) && x.eq(y),
        (None, None) => true,
        _ => false,
    }
}

/// How many times the heard words stand in `text` in a form the rule would change.
fn count_heard(text: &str, heard: &[String], meant: &str) -> usize {
    let t = toks(text);
    (0..t.len())
        .filter(|&i| {
            phrase_at(&t, i, heard)
                && !same_but_first_case(written_at(text, &t, i, heard.len()), meant)
        })
        .count()
}

/// How many times the corrected form stands in `text`.
fn count_meant(text: &str, meant: &str) -> usize {
    let words: Vec<String> = meant.split(' ').map(|w| w.to_lowercase()).collect();
    let t = toks(text);
    (0..t.len())
        .filter(|&i| {
            phrase_at(&t, i, &words)
                && same_but_first_case(written_at(text, &t, i, words.len()), meant)
        })
        .count()
}

/// Apply rules to a dictation. Whole words only, case ignored, the longest rule first, and
/// each stretch of text changed at most once (one rule's result is never fed to another).
/// A word inside an address, a web name or a hyphenated word is never touched.
pub fn apply(text: &str, rules: &[(String, String)]) -> String {
    let mut prepared: Vec<(Vec<String>, &str)> = rules
        .iter()
        .map(|(h, m)| {
            (
                h.split_whitespace().map(|w| w.to_lowercase()).collect::<Vec<_>>(),
                m.as_str(),
            )
        })
        .filter(|(h, m)| !h.is_empty() && !m.trim().is_empty())
        .collect();
    if prepared.is_empty() || text.is_empty() {
        return text.to_string();
    }
    prepared.sort_by(|x, y| y.0.len().cmp(&x.0.len()));
    let t = toks(text);
    let mut out = String::with_capacity(text.len() + 16);
    let mut copied = 0;
    let mut i = 0;
    while i < t.len() {
        let hit = prepared.iter().find(|(h, _)| phrase_at(&t, i, h));
        let Some((heard, meant)) = hit else {
            i += 1;
            continue;
        };
        let n = heard.len();
        let written = written_at(text, &t, i, n);
        out.push_str(&text[copied..t[i].core_start]);
        if first_upper(written) && first_lower(meant) {
            // Keep the sentence's capital ("Nga mihi" → "Ngā mihi").
            let mut c = meant.chars();
            if let Some(f) = c.next() {
                out.extend(f.to_uppercase());
                out.push_str(c.as_str());
            }
        } else {
            out.push_str(meant);
        }
        copied = t[i + n - 1].core_end;
        i += n;
    }
    out.push_str(&text[copied..]);
    out
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
    lex: &Lexicon,
) -> LearnOutcome {
    let mut outcome = LearnOutcome::default();

    // A rule Kōrero applied to this dictation, changed back by the user: the corrected form
    // went and the words as heard came back in its place. Deleting the sentence, or
    // rewording it some other way, is not changing it back.
    for p in pairs.iter_mut().filter(|p| p.active()) {
        let heard: Vec<String> = p.heard.split(' ').map(str::to_string).collect();
        if count_heard(original, &heard, &p.meant) > 0
            && count_meant(after, &p.meant) < count_meant(before, &p.meant)
            && count_heard(after, &heard, &p.meant) > count_heard(before, &heard, &p.meant)
        {
            p.off = true;
            outcome.switched_off.push(p.clone());
        }
    }

    for found in edit_pairs(before, after, lex) {
        let Found {
            heard,
            meant,
            suggest_only,
        } = found;
        if taught.iter().any(|t| t.trim().to_lowercase() == heard) {
            continue;
        }
        match pairs.iter_mut().find(|p| p.heard == heard) {
            // Switched off, or already a rule: one different edit does not undo either.
            Some(p) if p.off => {}
            Some(p) if p.active() && !same_but_first_case(&p.meant, &meant) => {}
            Some(p) if same_but_first_case(&p.meant, &meant) => {
                // Corrected once mid-sentence ("ngā") and once at the start of one ("Ngā"):
                // the same correction. The mid-sentence form is the word's own.
                if first_lower(&meant) {
                    p.meant = meant;
                }
                let was_active = p.active();
                p.seen = p.seen.saturating_add(1);
                p.last_seen = today.to_string();
                if p.suggest_only {
                    if p.seen == SUGGEST_AT {
                        outcome.suggested.push(p.clone());
                    } else if p.seen < SUGGEST_AT {
                        outcome.watching += 1;
                    }
                } else if !was_active && p.active() {
                    outcome.learnt.push(p.clone());
                } else if !p.active() {
                    outcome.watching += 1;
                }
            }
            Some(p) => {
                // Still being watched, and corrected to something else: count again.
                p.meant = meant;
                p.seen = 1;
                p.suggest_only = suggest_only;
                p.last_seen = today.to_string();
                outcome.watching += 1;
            }
            None => {
                if pairs.len() >= MAX_PAIRS {
                    // Full: make room by dropping the least-corrected pair that is only watched. If
                    // every pair is a rule or switched off, this one is not kept.
                    let least = pairs
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| !p.active() && !p.off)
                        .min_by_key(|(_, p)| p.seen)
                        .map(|(k, _)| k);
                    match least {
                        Some(drop) => {
                            pairs.remove(drop);
                        }
                        None => continue,
                    }
                }
                pairs.push(LearnedPair {
                    heard,
                    meant,
                    seen: 1,
                    off: false,
                    last_seen: today.to_string(),
                    suggest_only,
                });
                outcome.watching += 1;
            }
        }
    }
    outcome
}

/// The rules Kōrero applies on dictation: (heard, meant).
pub fn active_rules(pairs: &[LearnedPair]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter(|p| p.active())
        .map(|p| (p.heard.clone(), p.meant.clone()))
        .collect()
}

/// The list after an edit on the Settings page. The page can switch a pair off or on and
/// forget one; it cannot add a pair, change what one says or raise its count. So the result
/// is the stored pairs the page kept, each with the page's `off`.
pub fn page_edit(stored: &[LearnedPair], from_page: &[LearnedPair]) -> Vec<LearnedPair> {
    stored
        .iter()
        .filter_map(|p| {
            from_page
                .iter()
                .find(|q| q.heard == p.heard && q.meant == p.meant)
                .map(|q| LearnedPair {
                    off: q.off,
                    ..p.clone()
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small stand-in for the app's ordinary-English list. The real list is exercised in
    /// `commands::learning::tests`.
    fn common(w: &str) -> bool {
        const WORDS: &[&str] = &[
            "a", "an", "and", "the", "for", "to", "of", "in", "is", "it", "we", "i", "their",
            "there", "cost", "costs", "mark", "said", "front", "friend", "first", "find", "fire",
            "meeting", "with", "today", "on", "at", "was", "price", "will", "be", "our", "this",
            "who", "us", "about", "bank", "team", "new", "plan", "not", "ok", "go", "so",
        ];
        WORDS.contains(&w)
    }
    fn ambiguous(w: &str) -> bool {
        ["tangata", "wahine", "matua"].contains(&w)
    }
    fn lex() -> Lexicon<'static> {
        Lexicon {
            common: &common,
            ambiguous: &ambiguous,
        }
    }

    /// The rules an edit would teach (suggestions left out).
    fn pairs_of(before: &str, after: &str) -> Vec<(String, String)> {
        edit_pairs(before, after, &lex())
            .into_iter()
            .filter(|f| !f.suggest_only)
            .map(|f| (f.heard, f.meant))
            .collect()
    }
    /// The single words an edit would count towards a suggestion.
    fn offers_of(before: &str, after: &str) -> Vec<(String, String)> {
        edit_pairs(before, after, &lex())
            .into_iter()
            .filter(|f| f.suggest_only)
            .map(|f| (f.heard, f.meant))
            .collect()
    }
    fn none() -> Vec<(String, String)> {
        Vec::new()
    }
    fn one(h: &str, m: &str) -> Vec<(String, String)> {
        vec![(h.to_string(), m.to_string())]
    }
    fn pair(h: &str, m: &str, seen: u32) -> LearnedPair {
        LearnedPair {
            heard: h.into(),
            meant: m.into(),
            seen,
            off: false,
            last_seen: String::new(),
            suggest_only: false,
        }
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
        // A capital inside the corrected word, and different letters: a rule by itself.
        assert_eq!(pairs_of("we met the fens team", "we met the FENZ team"), one("fens", "FENZ"));
        assert_eq!(offers_of("we met the fens team", "we met the FENZ team"), none());
        assert_eq!(pairs_of("push it to github now", "push it to GitHub now"), one("github", "GitHub"));
        // Any other single word is learnt with the word beside it, and counted as an offer.
        assert_eq!(pairs_of("send it to kite today", "send it to Kyt today"), one("kite today", "Kyt today"));
        assert_eq!(offers_of("send it to kite today", "send it to Kyt today"), one("kite", "Kyt"));
        assert_eq!(pairs_of("send it to kite", "send it to Kyt"), one("to kite", "to Kyt"));
        // Already capitalised by the transcriber, corrected to a name that sounds the same.
        assert_eq!(pairs_of("we saw Nick there", "we saw Nic there"), one("nick there", "Nic there"));
        assert_eq!(offers_of("we saw Nick there", "we saw Nic there"), one("nick", "Nic"));
        assert_eq!(pairs_of("ask Steven about it", "ask Stephen about it"), one("steven about", "Stephen about"));
        // A lower-case word after a full stop is not a sentence start ("e.g. kite called").
        assert_eq!(pairs_of("e.g. kite called", "e.g. Kyt called"), one("kite called", "Kyt called"));
        // No unchanged word beside it: still counted as an offer.
        assert_eq!(pairs_of("thanks, nick.", "thanks, Nic."), none());
        assert_eq!(offers_of("thanks, nick.", "thanks, Nic."), one("nick", "Nic"));
        // An ordinary word that is also a name is never a rule, but can be offered.
        assert_eq!(pairs_of("then mark said yes", "then Marc said yes"), none());
        assert_eq!(offers_of("then mark said yes", "then Marc said yes"), one("mark", "Marc"));
        // A phrase fixed once and missed once in the same dictation is still learnt.
        assert_eq!(
            pairs_of(
                "met foreign emergency and then foreign emergency again",
                "met Fire and Emergency and then foreign emergency again"
            ),
            one("foreign emergency", "Fire and Emergency")
        );
    }

    #[test]
    fn a_macron_alone_is_offered_and_never_applied_by_itself() {
        // marama (moon) and mārama (clear) are different words; no list of such pairs is
        // complete, so nothing with only a macron changed becomes a rule.
        for (b, a, h, m) in [
            ("our whanau is here", "our whānau is here", "whanau", "whānau"),
            ("te marama hou", "te mārama hou", "marama", "mārama"),
            ("the maori people", "the Māori people", "maori", "Māori"),
            ("thanks nga mihi", "thanks ngā mihi", "nga", "ngā"),
        ] {
            assert_eq!(pairs_of(b, a), none(), "{b:?}");
            assert_eq!(offers_of(b, a), one(h, m), "{b:?}");
        }
        // Pairs Kōrero knows are different words are not even offered.
        assert_eq!(edit_pairs("he tangata pai", "he tāngata pai", &lex()), Vec::new());
        assert_eq!(edit_pairs("te wahine nei", "te wāhine nei", &lex()), Vec::new());
        assert_eq!(edit_pairs("nga matua katoa", "ngā mātua katoa", &lex()), Vec::new());
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
            ("we agreed. Their plan works", "we agreed. There plan works"),
            ("a plan for an emergency", "a plan for an emergency."),
            ("we talked about it", "we talked about it and more"),
            ("we talked about it at length", "we talked at length"),
            ("completely different words here", "Nothing Alike At All"),
            ("our far no is here", "our whānau is here"),
            // A name the sentence starts with: its capital could be the sentence's.
            ("Nick called", "Nic called"),
        ] {
            assert_eq!(edit_pairs(b, a, &lex()), Vec::new(), "{b:?} -> {a:?}");
        }
    }

    /// Every input the independent review (26 Nov 2026) used to break the first version.
    #[test]
    fn review_findings_stay_fixed() {
        for (b, a) in [
            // M4: context made only of ordinary words.
            ("who is on call", "WHO is on call"),
            ("tell us about it", "tell US about it"),
            // M5: te reo singulars are not misspelt plurals.
            ("he tangata pai", "he tāngata pai"),
            ("te wahine nei", "te wāhine nei"),
            // Minor: capitals on ordinary words, emphasis, web names, punctuation.
            ("we met at the bank", "we met at the Bank"),
            ("thanks to the team", "thanks to the Team"),
            ("the new plan", "the NEW plan"),
            ("this is not ok", "this is NOT ok"),
            ("see kite.co.nz today", "see Kyt.co.nz today"),
            ("ask kite—and go", "ask Kyt—and go"),
            ("and/or both", "And/Or both"),
            ("foreign, emergency crews", "Fire and, Emergency crews"),
            // Second pass: possessives and plurals are grammar, not names.
            ("ask Chris view", "ask Chris's view"),
            ("it is Fire and Emergency plan", "it is Fire and Emergency's plan"),
            ("the Jones family", "the Joneses family"),
        ] {
            assert_eq!(edit_pairs(b, a, &lex()), Vec::new(), "{b:?} -> {a:?}");
        }
        // A word that is a real word, another person's name, or only put in capitals is
        // never a rule by itself. With its neighbour it is narrow enough to be the user's call.
        for (b, a, h, m) in [
            ("ask may about it", "ask Mae about it", "may about", "Mae about"),
            ("call Tim tomorrow", "call Tom tomorrow", "tim tomorrow", "Tom tomorrow"),
            ("see the minister said no", "see the Ministry said no", "minister said", "Ministry said"),
            ("an urgent matter", "an URGENT matter", "urgent matter", "URGENT matter"),
            ("an aid package", "an AID package", "aid package", "AID package"),
        ] {
            assert_eq!(pairs_of(b, a), one(h, m), "{b:?} -> {a:?}");
        }
        // A single word left alone elsewhere in the same correction is a real word too.
        assert_eq!(edit_pairs("we may go with may", "we may go with Mae", &lex()), Vec::new());
        assert_eq!(
            edit_pairs("the fens team and the fens office", "the FENZ team and the fens office", &lex()),
            Vec::new()
        );
    }

    #[test]
    fn sound_alone_is_judged_only_on_real_edits() {
        // The red team's table: these all sound like "Fire and Emergency". None of them is
        // ever touched unless the user made that edit.
        assert!(sounds_alike("foreign emergency", "Fire and Emergency"));
        assert!(sounds_alike("for an emergency", "Fire and Emergency"));
        assert!(sounds_alike("family emergency", "Fire and Emergency"), "coarse on purpose");
        assert!(!sounds_alike("budget meeting", "Fire and Emergency"));
        assert!(!sounds_alike("costs", "Pricing"));
        assert!(sounds_alike("nick", "Nic"));
        assert!(sounds_alike("fens", "FENZ"));
        assert!(!sounds_alike("", "FENZ"));
        assert!(!sounds_alike("a", "I"));
        assert!(sounds_the_same("Nick", "Nic"));
        assert!(!sounds_the_same("Tuesday", "Thursday"));
    }

    #[test]
    fn addresses_links_and_figures_are_left_alone() {
        for (b, a) in [
            ("email nick@kite.co.nz now", "email Nic@Kyt.nz now"),
            ("see https://kite.nz today", "see https://Kyt.nz today"),
            ("it was 3 pm", "it was 3 PM"),
            ("version v2 shipped", "version V2 shipped"),
        ] {
            assert_eq!(pairs_of(b, a), none(), "{b:?}");
        }
    }

    #[test]
    fn a_big_rewrite_teaches_nothing() {
        let b = "so kite and fens and nick and tamaki went";
        let a = "so Kyt and FENZ and Nic and Tāmaki went";
        assert_eq!(pairs_of(b, a), none());
    }

    fn obs(pairs: &mut Vec<LearnedPair>, orig: &str, b: &str, a: &str) -> LearnOutcome {
        observe(pairs, orig, b, a, &[], "2026-11-26", &lex())
    }

    #[test]
    fn a_phrase_seen_twice_is_a_rule() {
        let mut pairs = Vec::new();
        let b = "working with foreign emergency";
        let a = "working with Fire and Emergency";
        let first = obs(&mut pairs, b, b, a);
        assert_eq!((first.watching, first.learnt.len()), (1, 0));
        assert!(active_rules(&pairs).is_empty(), "one edit is only watched");
        let second = obs(&mut pairs, b, b, a);
        assert_eq!((second.watching, second.learnt.len()), (0, 1));
        assert_eq!(active_rules(&pairs), one("foreign emergency", "Fire and Emergency"));
        // A third time changes nothing and announces nothing.
        let third = obs(&mut pairs, b, b, a);
        assert_eq!(third, LearnOutcome::default());
        assert_eq!((pairs.len(), pairs[0].seen), (1, 3));
    }

    #[test]
    fn a_single_word_needs_three() {
        let mut pairs = Vec::new();
        let b = "we met the fens team";
        let a = "we met the FENZ team";
        assert_eq!(obs(&mut pairs, b, b, a).watching, 1);
        assert_eq!(obs(&mut pairs, b, b, a).watching, 1);
        assert!(active_rules(&pairs).is_empty(), "twice is not enough for one word");
        assert_eq!(obs(&mut pairs, b, b, a).learnt.len(), 1);
        assert_eq!(active_rules(&pairs), one("fens", "FENZ"));
    }

    #[test]
    fn a_single_word_is_offered_after_three_and_never_applied() {
        let mut pairs = Vec::new();
        // Corrected in three different sentences: no phrase repeats, the word is counted.
        for (b, a) in [
            ("ask kite now", "ask Kyt now"),
            ("tell kite about it", "tell Kyt about it"),
        ] {
            let out = obs(&mut pairs, b, b, a);
            assert!(out.suggested.is_empty() && out.learnt.is_empty());
            assert!(out.watching > 0);
        }
        let out = obs(&mut pairs, "from kite again", "from kite again", "from Kyt again");
        assert_eq!(out.suggested.len(), 1);
        assert_eq!((out.suggested[0].heard.as_str(), out.suggested[0].meant.as_str()), ("kite", "Kyt"));
        assert!(active_rules(&pairs).is_empty(), "an offer is never applied");
        // Offered once: a fourth correction does not offer again.
        let out = obs(&mut pairs, "with kite soon", "with kite soon", "with Kyt soon");
        assert!(out.suggested.is_empty());
        // Dismissed (switched off), it is not counted again.
        pairs.iter_mut().find(|p| p.heard == "kite").unwrap().off = true;
        let seen = pairs.iter().find(|p| p.heard == "kite").unwrap().seen;
        obs(&mut pairs, "near kite then", "near kite then", "near Kyt then");
        assert_eq!(pairs.iter().find(|p| p.heard == "kite").unwrap().seen, seen);
    }

    #[test]
    fn the_same_word_fixed_many_times_in_one_dictation_is_one_correction() {
        // Fourth pass: a long dictation naming one organisation four times taught nothing.
        assert_eq!(
            pairs_of(
                "the fens a the fens b the fens c the fens d",
                "the FENZ a the FENZ b the FENZ c the FENZ d"
            ),
            one("fens", "FENZ")
        );
        assert_eq!(
            offers_of(
                "ask kite a ask kite b ask kite c ask kite d",
                "ask Kyt a ask Kyt b ask Kyt c ask Kyt d"
            ),
            one("kite", "Kyt")
        );
        assert_eq!(
            offers_of(
                "te marama a te marama b te marama c te marama d",
                "te mārama a te mārama b te mārama c te mārama d"
            ),
            one("marama", "mārama")
        );
    }

    #[test]
    fn a_sentence_capital_is_the_same_correction() {
        let mut pairs = Vec::new();
        obs(&mut pairs, "nga mihi nui", "nga mihi nui", "Ngā mihi nui");
        obs(&mut pairs, "thanks nga mihi", "thanks nga mihi", "thanks ngā mihi");
        let out = obs(&mut pairs, "nga mihi", "nga mihi", "Ngā mihi");
        let p = pairs.iter().find(|p| p.heard == "nga").unwrap();
        assert_eq!((p.meant.as_str(), p.seen, p.suggest_only), ("ngā", 3, true));
        assert_eq!(out.suggested.len(), 1);
    }

    #[test]
    fn two_in_one_edit_count_once() {
        let mut pairs = Vec::new();
        let b = "the foreign emergency crew and foreign emergency staff";
        let a = "the Fire and Emergency crew and Fire and Emergency staff";
        obs(&mut pairs, b, b, a);
        assert_eq!(pairs[0].seen, 1);
        assert!(active_rules(&pairs).is_empty());
    }

    #[test]
    fn changing_it_back_switches_it_off_for_good() {
        let mut pairs = vec![pair("foreign emergency", "Fire and Emergency", 2)];
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
    fn one_of_two_changed_back_is_enough() {
        let mut pairs = vec![pair("fens", "FENZ", 3)];
        let out = obs(
            &mut pairs,
            "fens sent a fens",
            "FENZ sent a FENZ",
            "FENZ sent a fens",
        );
        assert_eq!(out.switched_off.len(), 1);
    }

    #[test]
    fn other_edits_do_not_switch_a_rule_off() {
        let orig = "I met foreign emergency. Then lunch.";
        let before = "I met Fire and Emergency. Then lunch.";
        for after in [
            "Then lunch.",
            "I met FENZ today. Then lunch.",
            "I met Fire & Emergency today. Then lunch.",
            "It is Fire and Emergency's plan. Then lunch.",
            "I met Fire and Emergency. Then dinner.",
        ] {
            let mut pairs = vec![pair("foreign emergency", "Fire and Emergency", 2)];
            let out = obs(&mut pairs, orig, before, after);
            assert!(out.switched_off.is_empty(), "{after:?}");
            assert!(pairs[0].active(), "{after:?}");
        }
    }

    #[test]
    fn a_rule_is_not_undone_by_one_different_edit() {
        let mut pairs = vec![pair("kite today", "Kyt today", 5)];
        let b = "send it to kite today";
        let out = obs(&mut pairs, b, b, "send it to Kyte today");
        assert!(out.learnt.is_empty() && out.switched_off.is_empty());
        assert_eq!((pairs[0].meant.as_str(), pairs[0].seen), ("Kyt today", 5));
        // A pair that is only watched does start again.
        let mut pairs = vec![pair("kite today", "Kyt today", 1)];
        obs(&mut pairs, b, b, "send it to Kyte today");
        assert_eq!((pairs[0].meant.as_str(), pairs[0].seen), ("Kyte today", 1));
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
            &lex(),
        );
        assert_eq!(out, LearnOutcome::default());
        assert!(pairs.is_empty());
    }

    #[test]
    fn a_full_list_drops_a_watched_pair_and_never_claims_what_it_did_not_keep() {
        let mut pairs: Vec<LearnedPair> = (0..MAX_PAIRS)
            .map(|i| pair(&format!("word{i} x"), &format!("Word{i} X"), if i == 0 { 2 } else { 1 }))
            .collect();
        let b = "send it to kite today";
        let out = obs(&mut pairs, b, b, "send it to Kyt today");
        assert_eq!(out.watching, 2, "the phrase and the word");
        assert_eq!(pairs.len(), MAX_PAIRS);
        assert!(pairs.iter().any(|p| p.heard == "word0 x"), "the rule was kept");
        assert!(pairs.iter().any(|p| p.heard == "kite today"));
        // A pair corrected often is not the one dropped.
        let mut pairs: Vec<LearnedPair> = (0..MAX_PAIRS)
            .map(|i| pair(&format!("word{i} x"), &format!("Word{i} X"), 2))
            .collect();
        pairs[0] = LearnedPair {
            suggest_only: true,
            ..pair("nick", "Nic", 50)
        };
        pairs[1].seen = 1;
        obs(&mut pairs, b, b, "send it to Kyt today");
        assert!(pairs.iter().any(|p| p.heard == "nick"), "the well-counted offer was kept");
        // Every pair a rule: the new one is not kept, and nothing says it was.
        let mut pairs: Vec<LearnedPair> = (0..MAX_PAIRS)
            .map(|i| pair(&format!("word{i} x"), &format!("Word{i} X"), 2))
            .collect();
        let out = obs(&mut pairs, b, b, "send it to Kyt today");
        assert_eq!(out, LearnOutcome::default());
        assert!(!pairs.iter().any(|p| p.heard.starts_with("kite")));
    }

    fn rules() -> Vec<(String, String)> {
        vec![
            ("foreign emergency".to_string(), "Fire and Emergency".to_string()),
            ("kite".to_string(), "Kyt".to_string()),
            ("github".to_string(), "GitHub".to_string()),
            ("nga".to_string(), "ngā".to_string()),
        ]
    }

    #[test]
    fn rules_replace_whole_words_and_nothing_else() {
        let r = rules();
        assert_eq!(
            apply("Working with Foreign Emergency, and for an emergency.", &r),
            "Working with Fire and Emergency, and for an emergency."
        );
        assert_eq!(apply("foreign emergencies", &r), "foreign emergencies");
        assert_eq!(apply("nothing to do", &r), "nothing to do");
        assert_eq!(apply("kite", &[]), "kite");
        assert_eq!(apply("", &r), "");
        // Case-only rules apply, and a sentence keeps its capital.
        assert_eq!(apply("push to github. Github is up", &r), "push to GitHub. GitHub is up");
        assert_eq!(apply("Nga mihi, nga mihi", &r), "Ngā mihi, ngā mihi");
        // Spacing the transcriber wrote differently still matches; other spacing is kept.
        assert_eq!(apply("the  foreign\nemergency  crew", &r), "the  Fire and Emergency  crew");
        // Never part of a longer word, a possessive, a hyphenated word or a dash run.
        for s in ["kite-flying", "kite's tail", "kite—red", "kites", "skite"] {
            assert_eq!(apply(s, &r), s);
        }
        assert_eq!(apply("(kite) \"kite\" kite.", &r), "(Kyt) \"Kyt\" Kyt.");
    }

    #[test]
    fn rules_never_edit_inside_an_address() {
        let r = rules();
        assert_eq!(
            apply("email nick@kite.co.nz about the kite", &r),
            "email nick@kite.co.nz about the Kyt"
        );
        for s in ["see kite.co.nz", "see www.kite.co.nz today", "https://kite.nz/kite"] {
            assert_eq!(apply(s, &r), s);
        }
    }

    #[test]
    fn the_longest_rule_wins_and_results_are_not_fed_back() {
        let r = vec![
            ("foreign".to_string(), "Fore".to_string()),
            ("the foreign emergency".to_string(), "Fire and Emergency".to_string()),
            ("kite".to_string(), "Kyt team".to_string()),
            ("kyt team".to_string(), "Kite Team Ltd".to_string()),
        ];
        assert_eq!(apply("call the foreign emergency", &r), "call Fire and Emergency");
        assert_eq!(apply("ask kite now", &r), "ask Kyt team now");
        assert_eq!(apply("costs $5", &[("costs".to_string(), "$1 Co".to_string())]), "$1 Co $5");
    }

    #[test]
    fn the_settings_page_can_only_switch_off_or_forget() {
        let stored = vec![
            pair("foreign emergency", "Fire and Emergency", 2),
            pair("fens", "FENZ", 3),
            pair("kite today", "Kyt today", 1),
        ];
        let mut switched = stored[0].clone();
        switched.off = true;
        switched.seen = 4_000_000;
        switched.last_seen = "never".into();
        let from_page = vec![
            switched,
            // Forgotten: "fens" is not sent back. Invented or altered pairs are dropped.
            pair("the", "Thee", 9),
            pair("kite today", "Kite Today", 9),
        ];
        let out = page_edit(&stored, &from_page);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].heard, "foreign emergency");
        assert!(out[0].off);
        assert_eq!((out[0].seen, out[0].last_seen.as_str()), (2, ""));
    }

    #[test]
    fn old_settings_without_the_new_fields_still_load() {
        let p: LearnedPair = serde_json::from_str(r#"{"heard":"fens","meant":"FENZ"}"#).unwrap();
        assert_eq!((p.seen, p.off), (0, false));
        assert!(!p.active());
    }

    /// No input panics, the same input always gives the same answer, and applying rules a
    /// second time changes nothing more. Multi-byte spaces and punctuation are in the
    /// alphabet on purpose (L-020).
    #[test]
    fn fuzz_never_panics_and_is_stable() {
        const BITS: &[&str] = &[
            "foreign", "emergency", "Fire", "and", "Emergency", "fens", "FENZ", "kite", "Kyt",
            "the", "a", "\u{a0}", "\u{3000}", " ", "  ", ".", ",", "!", "whānau", "whanau", "é",
            "nick@kite.nz", "3", "$5", "\n", "\t", "'", "\"", "—", "日本", "🙂", "A", "I", "",
            "-", "kite.co.nz", "’", "Nga", "nga", "github",
        ];
        let mut seed: u64 = 0x5eed_1234_abcd_ef01;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let text = |next: &mut dyn FnMut() -> usize| {
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
        let r = rules();
        let mut pairs = Vec::new();
        for _ in 0..40_000 {
            let (o, b, a) = (text(&mut next), text(&mut next), text(&mut next));
            let first = edit_pairs(&b, &a, &lex());
            assert_eq!(first, edit_pairs(&b, &a, &lex()));
            assert!(first.len() <= MAX_FOUND_PER_EDIT);
            for f in &first {
                let (h, m) = (&f.heard, &f.meant);
                assert!(h.chars().count() >= 3 && !m.is_empty());
                assert_eq!(*h, h.to_lowercase());
                assert!(f.suggest_only || h.contains(' ') || m.chars().skip(1).any(|c| c.is_uppercase()));
                assert!(h.split(' ').chain(m.split(' ')).all(plain_word), "{h:?} {m:?}");
            }
            let _ = sounds_alike(&b, &a);
            let once = apply(&b, &r);
            assert_eq!(apply(&once, &r), once, "a second pass changed {b:?}");
            obs(&mut pairs, &o, &b, &a);
            assert!(pairs.len() <= MAX_PAIRS);
        }
    }
}
