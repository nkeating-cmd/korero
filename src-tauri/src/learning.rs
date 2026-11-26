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
//! - **Seen twice, or three times for a single word.** One edit is watched. A phrase becomes a
//!   rule the second time; a single word, which would change every use of that word, the third.
//! - **Never an ordinary word.** If every word Kōrero wrote is ordinary English, or the word
//!   is still used unchanged in the text the user just corrected, nothing is learnt. A te reo
//!   word whose macron-free form is a different word (tangata, tāngata) is never learnt.
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
/// The most pairs taken from one edit: an edit that changes more is a rewrite.
const MAX_PAIRS_PER_EDIT: usize = 3;
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
}

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
        !self.off && self.seen >= self.needed()
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

/// The replacements in an edit that look like mishearings of a name: (heard, meant), heard in
/// lower case.
pub fn edit_pairs(before: &str, after: &str, lex: &Lexicon) -> Vec<(String, String)> {
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
        if only_first_letters_differ(&b[b0..b1], &a[a0..a1]) {
            continue;
        }
        let at_start = a0 == 0 || ends_sentence(a[a0 - 1]);
        let changed_heard = join_cores(&b[b0..b1]);
        let changed_meant = join_cores(&a[a0..a1]);
        if !name_like(&changed_heard, &changed_meant, at_start) {
            continue;
        }
        // One word replaced by several ("foreign" → "Fire and"), or an ordinary word
        // replaced, is too broad to be a rule by itself: it would rewrite every "foreign".
        // Take the unchanged word beside it as context.
        if b1 - b0 == 1 {
            let heard_word = core(b[b0]).to_lowercase();
            if a1 - a0 != 1 || (lex.common)(&heard_word) {
                if b1 < b.len() && a1 < a.len() && core(b[b1]) == core(a[a1]) {
                    b1 += 1;
                    a1 += 1;
                } else if b0 > 0 && a0 > 0 && core(b[b0 - 1]) == core(a[a0 - 1]) {
                    b0 -= 1;
                    a0 -= 1;
                } else {
                    continue;
                }
            }
        }
        let heard_words: Vec<&str> = b[b0..b1].iter().map(|t| core(t)).collect();
        let meant_words: Vec<&str> = a[a0..a1].iter().map(|t| core(t)).collect();
        if !heard_words.iter().chain(meant_words.iter()).all(|w| plain_word(w)) {
            continue;
        }
        // Punctuation between the words ("foreign, emergency") is not one phrase.
        let joined = |tokens: &[&str]| {
            tokens.iter().enumerate().all(|(k, t)| {
                (k + 1 == tokens.len() || t.ends_with(core(t)))
                    && (k == 0 || t.starts_with(core(t)))
            })
        };
        if !joined(&b[b0..b1]) || !joined(&a[a0..a1]) {
            continue;
        }
        let heard_written = heard_words.join(" ");
        let meant = meant_words.join(" ");
        let heard = heard_written.to_lowercase();
        if heard.chars().count() < 3
            || meant.chars().count() > MAX_MEANT_CHARS
            || heard_written == meant
        {
            continue;
        }
        // Every word Kōrero wrote is ordinary English: the user reworded, or the name is
        // also an ordinary word ("mark said", "who is"). Too risky to make a rule.
        if heard.split(' ').all(|w| (lex.common)(w)) {
            continue;
        }
        // A te reo singular is not a misspelt plural (tangata, tāngata).
        if heard == without_macrons(&meant.to_lowercase())
            && heard.split(' ').any(|w| (lex.ambiguous)(w))
        {
            continue;
        }
        if !sounds_alike(&heard, &meant) {
            continue;
        }
        // The user left the same words alone elsewhere in the text they just corrected
        // ("we may go with Mae"): they are real words too.
        let heard_list: Vec<String> = heard.split(' ').map(str::to_string).collect();
        if count_heard(after, &heard_list, &meant) > 0 {
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

    for (heard, meant) in edit_pairs(before, after, lex) {
        if taught.iter().any(|t| t.trim().to_lowercase() == heard) {
            continue;
        }
        match pairs.iter_mut().find(|p| p.heard == heard) {
            // Switched off, or already a rule: one different edit does not undo either.
            Some(p) if p.off => {}
            Some(p) if p.active() && p.meant != meant => {}
            Some(p) if p.meant == meant => {
                let was_active = p.active();
                p.seen = p.seen.saturating_add(1);
                p.last_seen = today.to_string();
                if !was_active && p.active() {
                    outcome.learnt.push(p.clone());
                } else if !p.active() {
                    outcome.watching += 1;
                }
            }
            Some(p) => {
                // Still being watched, and corrected to something else: count again.
                p.meant = meant;
                p.seen = 1;
                p.last_seen = today.to_string();
                outcome.watching += 1;
            }
            None => {
                if pairs.len() >= MAX_PAIRS {
                    // Full: make room by dropping the oldest pair that is only watched. If
                    // every pair is a rule or switched off, this one is not kept.
                    match pairs.iter().position(|p| !p.active() && !p.off) {
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

/// A list from the Settings page, made safe to store: the same limits the learner keeps
/// (plain words, a heard side of at least three letters, a bounded corrected side), heard in
/// lower case, duplicates dropped, bounded.
pub fn sanitise(pairs: Vec<LearnedPair>) -> Vec<LearnedPair> {
    let mut out: Vec<LearnedPair> = Vec::new();
    for mut p in pairs {
        let heard: Vec<String> = p.heard.split_whitespace().map(|w| w.to_lowercase()).collect();
        let meant: Vec<&str> = p.meant.split_whitespace().collect();
        if heard.is_empty()
            || meant.is_empty()
            || heard.len() > MAX_SPAN_WORDS + 1
            || meant.len() > MAX_SPAN_WORDS + 1
            || !heard.iter().all(|w| plain_word(w))
            || !meant.iter().all(|w| plain_word(w))
        {
            continue;
        }
        p.heard = heard.join(" ");
        p.meant = meant.join(" ");
        if p.heard.chars().count() < 3 || p.meant.chars().count() > MAX_MEANT_CHARS {
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

    fn pairs_of(before: &str, after: &str) -> Vec<(String, String)> {
        edit_pairs(before, after, &lex())
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
        assert_eq!(pairs_of("we met the fens team", "we met the FENZ team"), one("fens", "FENZ"));
        assert_eq!(pairs_of("send it to kite today", "send it to Kyt today"), one("kite", "Kyt"));
        assert_eq!(pairs_of("our whanau is here", "our whānau is here"), one("whanau", "whānau"));
        assert_eq!(pairs_of("push it to github now", "push it to GitHub now"), one("github", "GitHub"));
        // Already capitalised by the transcriber, corrected to a name that sounds the same.
        assert_eq!(pairs_of("we saw Nick there", "we saw Nic there"), one("nick", "Nic"));
        assert_eq!(pairs_of("ask Steven about it", "ask Stephen about it"), one("steven", "Stephen"));
        // A lower-case word after a full stop is not a sentence start ("e.g. kite called").
        assert_eq!(pairs_of("e.g. kite called", "e.g. Kyt called"), one("kite", "Kyt"));
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
            assert_eq!(pairs_of(b, a), none(), "{b:?} -> {a:?}");
        }
    }

    /// Every input the independent review (26 Nov 2026) used to break the first version.
    #[test]
    fn review_findings_stay_fixed() {
        for (b, a) in [
            // M4: a name that is also an ordinary word, used both ways in one dictation.
            ("we may go with may", "we may go with Mae"),
            // M4: context made only of ordinary words.
            ("who is on call", "WHO is on call"),
            ("tell us about it", "tell US about it"),
            ("then mark said yes", "then Marc said yes"),
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
        ] {
            assert_eq!(pairs_of(b, a), none(), "{b:?} -> {a:?}");
        }
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
        let mut pairs = vec![pair("kite", "Kyt", 3)];
        let out = obs(
            &mut pairs,
            "kite sent a kite",
            "Kyt sent a Kyt",
            "Kyt sent a kite",
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
        let mut pairs = vec![pair("kite", "Kyt", 5)];
        let b = "send it to kite today";
        let out = obs(&mut pairs, b, b, "send it to Kyte today");
        assert_eq!(out, LearnOutcome::default());
        assert_eq!((pairs[0].meant.as_str(), pairs[0].seen), ("Kyt", 5));
        // A pair that is only watched does start again.
        let mut pairs = vec![pair("kite", "Kyt", 1)];
        obs(&mut pairs, b, b, "send it to Kyte today");
        assert_eq!((pairs[0].meant.as_str(), pairs[0].seen), ("Kyte", 1));
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
        assert_eq!(out.watching, 1);
        assert_eq!(pairs.len(), MAX_PAIRS);
        assert!(pairs.iter().any(|p| p.heard == "word0 x"), "the rule was kept");
        assert!(pairs.iter().any(|p| p.heard == "kite"));
        // Every pair a rule: the new one is not kept, and nothing says it was.
        let mut pairs: Vec<LearnedPair> = (0..MAX_PAIRS)
            .map(|i| pair(&format!("word{i} x"), &format!("Word{i} X"), 2))
            .collect();
        let out = obs(&mut pairs, b, b, "send it to Kyt today");
        assert_eq!(out, LearnOutcome::default());
        assert!(!pairs.iter().any(|p| p.heard == "kite"));
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
    fn sanitise_keeps_the_learners_own_limits() {
        let out = sanitise(vec![
            pair("  Foreign   Emergency ", " Fire and  Emergency ", 2),
            pair("foreign emergency", "Other", 2),
            pair("", "X", 2),
            pair("x", "", 2),
            pair("a", "B", 5),
            pair("long", &"y".repeat(61), 2),
            pair("nick@kite.nz", "Nic", 2),
            pair("kite.co.nz", "Kyt", 2),
            pair(&"w ".repeat(40), "Kyt", 2),
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
            assert!(first.len() <= MAX_PAIRS_PER_EDIT);
            for (h, m) in &first {
                assert!(h.chars().count() >= 3 && !m.is_empty());
                assert_eq!(*h, h.to_lowercase());
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
