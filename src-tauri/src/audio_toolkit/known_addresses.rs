//! Kōrero 1.51 — known addresses: a misheard email address is replaced with the one the user
//! told Kōrero about ("nick@kite.co.nz" → "nic@kyt.nz").
//!
//! The speech model mishears names it has never seen. The written-forms pass
//! (`written_forms.rs`) formats what it hears but cannot know that "kite" was "Kyt". This module
//! knows the user's own addresses (Settings → Advanced → "Your email addresses", and every address
//! the user taught from History) and replaces an address that **sounds the same** with the real
//! one.
//!
//! # Rules (from the 26 Nov 2026 red-team review)
//!
//! - **Emails only.** A bare domain is never replaced: "kit.nz" and "cat.org.nz" are real places.
//! - **Both halves must sound alike**: the local part ("nick" ~ "nic") and the domain's first label
//!   ("kite" ~ "kyt"), by [`sound_key`]: the consonant skeleton, the first vowel (y counts as i)
//!   and the number of syllables. Consonants alone merged kite, kit, kate, cat, cut and coat.
//! - **Suffix:** the same suffix wins first; otherwise only `co.nz` ↔ `nz` (the speech model was
//!   seen inserting "co"). "org", "govt", "ac" and "school" are spoken, so a different one is a
//!   different address.
//! - **Exactly one match**, or nothing changes. An address the user already knows is never
//!   touched.
//! - **Dictation only.** Meetings, notes and imports are other people's words; a client's real
//!   address must never become the user's own. The caller decides (`actions.rs`).

/// The suffixes the speech model confuses: "kite dot nz" heard as "kite.co.nz".
const SWAPPABLE: [(&str, &str); 2] = [("co.nz", "nz"), ("nz", "co.nz")];

/// A deterministic English sound-alike key: (consonant skeleton, first vowel class, syllables).
///
/// `nick`, `nic`, `nik` → ("nk", 'i', 1); `kite`, `kyt`, `kyte` → ("kt", 'i', 1);
/// `kate` → ("kt", 'a', 1); `coat` → ("kt", 'o', 1); `kitty` → ("kt", 'i', 2).
pub fn sound_key(word: &str) -> (String, char, usize) {
    let w = word.to_lowercase();
    let mut w: String = w.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if w.is_empty() {
        return (String::new(), '-', 0);
    }
    // First vowel and syllables are read from the spelling before the consonant rewrite.
    let first_vowel = {
        let chars: Vec<char> = w.chars().collect();
        let found = if is_vowel(chars[0]) {
            Some(chars[0])
        } else {
            chars[1..]
                .iter()
                .copied()
                .find(|c| is_vowel(*c) || *c == 'y')
        };
        match found {
            Some('y') => 'i',
            Some(v) => v,
            None => '-',
        }
    };
    let syllables = {
        let s = if w.len() > 3 && w.ends_with('e') {
            &w[..w.len() - 1]
        } else {
            &w[..]
        };
        let mut n = 0;
        let mut in_vowel = false;
        for c in s.chars() {
            let v = is_vowel(c) || c == 'y';
            if v && !in_vowel {
                n += 1;
            }
            in_vowel = v;
        }
        n.max(1)
    };
    // Consonant skeleton.
    if w.starts_with('x') {
        w.replace_range(..1, "s"); // xero ~ zero
    }
    let mut w = w.replace("ph", "f").replace("ck", "k");
    let chars: Vec<char> = w.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if *c == 'c' {
            let soft = chars
                .get(i + 1)
                .map(|n| matches!(n, 'e' | 'i' | 'y'))
                .unwrap_or(false);
            out.push(if soft { 's' } else { 'k' });
        } else {
            out.push(*c);
        }
    }
    w = out.replace('q', "k").replace('x', "ks").replace('z', "s");
    if w.starts_with("kn") {
        w.remove(0);
    }
    w = w.replace("wr", "r").replace("gh", "");
    if w.is_empty() {
        return (String::new(), first_vowel, syllables);
    }
    let mut skel = String::new();
    let mut chars = w.chars();
    let first = chars.next().unwrap();
    skel.push(first);
    for c in chars {
        if !is_vowel(c) && c != 'y' {
            skel.push(c);
        }
    }
    let mut dedup = String::new();
    for c in skel.chars() {
        if !dedup.ends_with(c) {
            dedup.push(c);
        }
    }
    (dedup, first_vowel, syllables)
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

/// An email address split into (local, first label of the domain, the rest of the domain).
fn split_email(s: &str) -> Option<(String, String, String)> {
    let s = s.to_lowercase();
    let (local, domain) = s.split_once('@')?;
    if local.is_empty() || domain.contains('@') {
        return None;
    }
    let (first, rest) = domain.split_once('.')?;
    let ok_label =
        |l: &str| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let ok_local = local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    if !ok_local || !ok_label(first) || rest.split('.').any(|l| !ok_label(l)) {
        return None;
    }
    Some((local.to_string(), first.to_string(), rest.to_string()))
}

/// Normalises one user-entered address: trimmed, lower case, and a valid email. `None` otherwise.
pub fn normalise_address(s: &str) -> Option<String> {
    let t = s
        .trim()
        .trim_matches(|c: char| c == '<' || c == '>')
        .to_lowercase();
    split_email(&t).map(|_| t)
}

/// The local part's sound key, from its letters ("nic.k" and "nick" alike; digits ignored), plus
/// what tells people's names apart where the domain key is loose: the first vowel group
/// ("joan" is not "jon", "bean" is not "ben") and a silent final "e" ("mike" is not "mick",
/// "dane" is not "dan"). Independent review, 1.51.
fn local_key(local: &str) -> ((String, char, usize), String, bool) {
    let letters: Vec<char> = local
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let start = letters
        .iter()
        .position(|c| is_vowel(*c))
        .unwrap_or(letters.len());
    let group: String = letters[start..]
        .iter()
        .take_while(|c| is_vowel(**c))
        .collect();
    let n = letters.len();
    let silent_e =
        n >= 3 && letters[n - 1] == 'e' && !is_vowel(letters[n - 2]) && is_vowel(letters[n - 3]);
    (sound_key(local), group, silent_e)
}

/// True when `candidate` sounds like `known` with the given suffix rule.
fn sounds_like(
    c: &(String, String, String),
    k: &(String, String, String),
    exact_suffix: bool,
) -> bool {
    let suffix_ok = if exact_suffix {
        c.2 == k.2
    } else {
        SWAPPABLE.iter().any(|(a, b)| c.2 == *a && k.2 == *b)
    };
    if !suffix_ok {
        return false;
    }
    // The same domain with a different name is someone else at that place ("nick@kyt.nz" beside
    // "nic@kyt.nz"): only a misheard domain says the whole address was misheard. A same-domain
    // mishearing is taught from History instead, as an exact correction.
    if c.1 == k.1 && c.2 == k.2 {
        return false;
    }
    // Speech mishears letters, not numbers or marks: "1nic@kyt.nz", "nic_k@kite.nz" and
    // "_nick@kite.nz" are someone else.
    let marks = |p: &(String, String, String)| -> String {
        p.0.chars()
            .filter(|c| !c.is_ascii_alphabetic())
            .chain(p.1.chars().filter(char::is_ascii_digit))
            .collect()
    };
    if marks(c) != marks(k) {
        return false;
    }
    let digits = |p: &(String, String, String)| -> String {
        p.0.chars()
            .chain(p.1.chars())
            .filter(char::is_ascii_digit)
            .collect()
    };
    if digits(c) != digits(k) {
        return false;
    }
    let (lc, lk) = (local_key(&c.0), local_key(&k.0));
    let (dc, dk) = (sound_key(&c.1), sound_key(&k.1));
    // Keys of one letter carry too little to match on ("jo", "x").
    lc.0 .0.len() >= 2 && dc.0.len() >= 2 && lc == lk && dc == dk
}

/// The known address a misheard `candidate` stands for, if exactly one sounds like it.
pub fn match_known(candidate: &str, known: &[String]) -> Option<String> {
    let c = split_email(candidate)?;
    let known: Vec<(String, (String, String, String))> = known
        .iter()
        .filter_map(|k| normalise_address(k).and_then(|n| split_email(&n).map(|p| (n, p))))
        .collect();
    let lower = candidate.to_lowercase();
    if known.iter().any(|(n, _)| *n == lower) {
        return None;
    }
    for exact in [true, false] {
        let hits: Vec<&String> = known
            .iter()
            .filter(|(_, k)| sounds_like(&c, k, exact))
            .map(|(n, _)| n)
            .collect();
        match hits.len() {
            0 => continue,
            1 => return Some(hits[0].clone()),
            _ => return None, // ambiguous: leave it as heard
        }
    }
    None
}

/// Replaces every misheard email address in `text` with the known one it sounds like. Returns the
/// text and how many were replaced. Text without an "@" is returned untouched.
pub fn snap_known_addresses(text: &str, known: &[String]) -> (String, usize) {
    snap_addresses(text, known, &[])
}

/// As [`snap_known_addresses`], plus the exact address corrections the user taught (`taught`:
/// heard → means, from History's "Remember this address?"). A taught pair wins over sound and
/// applies even to a same-domain mishearing, which sound never fixes. Dictation only, like the
/// rest of this module.
pub fn snap_addresses(
    text: &str,
    known: &[String],
    taught: &[(String, String)],
) -> (String, usize) {
    let taught: Vec<(String, String)> = taught
        .iter()
        .filter_map(|(w, r)| Some((normalise_address(w)?, r.trim().to_string())))
        .filter(|(_, r)| !r.is_empty())
        .collect();
    if (known.is_empty() && taught.is_empty()) || !text.contains('@') {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut n = 0;
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String, n: &mut usize| {
        if word.is_empty() {
            return;
        }
        let chars: Vec<char> = word.chars().collect();
        let start = chars
            .iter()
            .position(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
            .unwrap_or(chars.len());
        let end = chars
            .iter()
            .rposition(|c| c.is_ascii_alphanumeric())
            .map(|p| p + 1)
            .unwrap_or(start);
        let core: String = chars[start..end].iter().collect();
        // A letter glued to the address ("āmike@zero.com") makes it another address; Chinese
        // and Japanese, written without spaces, may touch it ("宛先はnick@…").
        let glued = |c: Option<&char>| c.map(|c| c.is_alphanumeric() && (*c as u32) < 0x2E80);
        let touching = glued(start.checked_sub(1).and_then(|p| chars.get(p))) == Some(true)
            || glued(chars.get(end)) == Some(true);
        if core.contains('@') && !touching {
            let lower = core.to_lowercase();
            let exact = taught
                .iter()
                .find(|(w, _)| *w == lower)
                .map(|(_, r)| r.clone());
            if let Some(real) = exact.or_else(|| match_known(&core, known)) {
                let lead: String = chars[..start].iter().collect();
                let trail: String = chars[end..].iter().collect();
                out.push_str(&lead);
                out.push_str(&real);
                out.push_str(&trail);
                *n += 1;
                word.clear();
                return;
            }
        }
        out.push_str(word);
        word.clear();
    };
    for ch in text.chars() {
        if ch.is_whitespace() {
            flush(&mut word, &mut out, &mut n);
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    flush(&mut word, &mut out, &mut n);
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        [
            "nic@kyt.nz",
            "nic@rumbo.nz",
            "sam@gmail.com",
            "mike@xero.com",
            "jo@spark.co.nz",
            "kate@mbie.govt.nz",
            "tim@xtra.co.nz",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn snap(s: &str) -> String {
        snap_known_addresses(s, &known()).0
    }

    #[test]
    fn keys() {
        assert_eq!(sound_key("nick"), sound_key("nic"));
        assert_eq!(sound_key("nik"), sound_key("nic"));
        assert_eq!(sound_key("kite"), sound_key("kyt"));
        assert_eq!(sound_key("kyte"), sound_key("kyt"));
        assert_eq!(sound_key("zero"), sound_key("xero"));
        assert_ne!(sound_key("kate"), sound_key("kyt"));
        assert_ne!(sound_key("cat"), sound_key("kyt"));
        assert_ne!(sound_key("coat"), sound_key("kyt"));
        assert_ne!(sound_key("cut"), sound_key("kyt"));
        assert_ne!(sound_key("kitty"), sound_key("kyt"));
        assert_ne!(sound_key("sim"), sound_key("sam"));
        assert_ne!(sound_key("mac"), sound_key("mike"));
        assert_ne!(sound_key("tom"), sound_key("tim"));
        // "nike" and "nic" share a key (long and short i); the whole address must match too.
    }

    /// The mishearings seen or expected for Nic's address all come back to it.
    #[test]
    fn real_mishearings_snap() {
        for heard in [
            "nick@kite.co.nz",
            "nick@kite.nz",
            "nic@kite.nz",
            "nik@kyte.co.nz",
            "Nick@Kite.co.nz",
        ] {
            assert_eq!(snap(heard), "nic@kyt.nz", "{heard}");
        }
        assert_eq!(
            snap("Email me at nick@kite.co.nz, thanks."),
            "Email me at nic@kyt.nz, thanks."
        );
        assert_eq!(snap("mike@zero.com"), "mike@xero.com");
        // Japanese punctuation around an address (independent review, 1.51).
        assert_eq!(snap("宛先はnick@kite.co.nz、次"), "宛先はnic@kyt.nz、次");
        assert_eq!(snap("ānick@kite.co.nz"), "ānick@kite.co.nz");
        // Marks are not misheard (independent review, second pass).
        for heard in [
            "_nick@kite.co.nz",
            "nic_k@kite.nz",
            "n.i.c@kite.nz",
            "nick+x@kite.nz",
        ] {
            assert_eq!(snap(heard), heard, "{heard}");
        }
    }

    /// The red team's false snaps (26 Nov) must stay as heard.
    #[test]
    fn false_snaps_stay() {
        for heard in [
            "nike@cat.org.nz",
            "1nic@kyt.nz",
            "nic2@kyt.nz",
            "nic@kyt2.nz",
            "sim@gmail.com",
            "mac@xero.com",
            "tom@xtra.co.nz",
            "kate@kyt.nz",
            "neck@coat.nz",
            "nick@kitty.co.nz",
            "jo@sparks.co.nz",
            "nick@kite.org.nz",
            "nick@kite.com",
            "kite.co.nz",
            "kit.nz",
            "see kyt.nz for more",
            "joe@spark.co.nz",
        ] {
            assert_eq!(snap(heard), heard, "{heard}");
        }
    }

    #[test]
    fn taught_corrections_apply_exactly() {
        let taught = vec![("Nick@kyt.nz".to_string(), "nic@kyt.nz".to_string())];
        // Same domain: sound never snaps it, a taught pair does.
        assert_eq!(snap("Mail nick@kyt.nz."), "Mail nick@kyt.nz.");
        assert_eq!(
            snap_addresses("Mail nick@kyt.nz.", &known(), &taught),
            ("Mail nic@kyt.nz.".to_string(), 1)
        );
        // With no known addresses at all.
        assert_eq!(
            snap_addresses("to NICK@KYT.NZ", &[], &taught).0,
            "to nic@kyt.nz"
        );
        // Only that exact address.
        assert_eq!(
            snap_addresses("to nick@kyt.co.nz", &[], &taught).0,
            "to nick@kyt.co.nz"
        );
    }

    /// The independent review's false snaps (26 Nov): other people, not mishearings.
    #[test]
    fn other_people_are_not_the_user() {
        for (known, heard) in [
            ("mike@gmail.com", "mick@gmail.com"),
            ("ann@gmail.com", "anne@gmail.com"),
            ("jess@gmail.com", "jesse@gmail.com"),
            ("jon@gmail.com", "joan@gmail.com"),
            ("ben@gmail.com", "bean@gmail.com"),
            ("dan@xtra.co.nz", "dane@xtra.co.nz"),
            ("kate@mbie.govt.nz", "kat@mbie.govt.nz"),
            ("kate@mbie.govt.nz", "cate@mbie.govt.nz"),
            ("nic@kyt.nz", "nick@kyt.nz"),
            ("nic@kyt.nz", "nic.k@kyt.nz"),
            // A misheard domain too, but a different name.
            ("mike@xero.com", "mick@zero.com"),
            ("jon@xero.com", "joan@zero.com"),
            ("dan@kyt.nz", "dane@kite.nz"),
        ] {
            let known = vec![known.to_string()];
            assert_eq!(
                snap_known_addresses(heard, &known).0,
                heard,
                "{known:?} {heard}"
            );
        }
    }

    #[test]
    fn known_and_ambiguous_are_left_alone() {
        assert_eq!(snap("nic@kyt.nz"), "nic@kyt.nz");
        // Two known addresses that differ only by suffix: an exact-suffix hit wins.
        let two: Vec<String> = vec!["nic@kyt.nz".into(), "nic@kyt.co.nz".into()];
        assert_eq!(snap_known_addresses("nick@kite.nz", &two).0, "nic@kyt.nz");
        assert_eq!(
            snap_known_addresses("nick@kite.co.nz", &two).0,
            "nic@kyt.co.nz"
        );
        // Two that sound the same: ambiguous, nothing changes.
        let alike: Vec<String> = vec!["nic@kyt.nz".into(), "nik@kite.nz".into()];
        assert_eq!(
            snap_known_addresses("nick@kyte.nz", &alike).0,
            "nick@kyte.nz"
        );
    }

    #[test]
    fn nothing_to_do() {
        assert_eq!(
            snap_known_addresses("nick@kite.co.nz", &[]).0,
            "nick@kite.co.nz"
        );
        assert_eq!(snap("no address here"), "no address here");
        assert_eq!(snap(""), "");
        assert_eq!(snap("  spaced\n\nlines  "), "  spaced\n\nlines  ");
        assert_eq!(snap("@ @@ a@ @b x@y"), "@ @@ a@ @b x@y");
        let once = snap("Send to nick@kite.co.nz and nic@kyt.nz.");
        assert_eq!(once, "Send to nic@kyt.nz and nic@kyt.nz.");
        assert_eq!(snap(&once), once);
    }

    #[test]
    fn normalise() {
        assert_eq!(
            normalise_address("  Nic@Kyt.NZ ").as_deref(),
            Some("nic@kyt.nz")
        );
        assert_eq!(
            normalise_address("<nic@kyt.nz>").as_deref(),
            Some("nic@kyt.nz")
        );
        assert_eq!(normalise_address("kyt.nz"), None);
        assert_eq!(normalise_address("nic@"), None);
        assert_eq!(normalise_address("a@b@c.nz"), None);
        assert_eq!(normalise_address("nic@kyt"), None);
    }
}
