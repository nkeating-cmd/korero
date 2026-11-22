//! Kōrero 1.50 — written forms: spoken numbers, money, dates, times and measures written the way
//! people write them ("one thousand two hundred" → `1,200`).
//!
//! Sibling of [`crate::audio_toolkit::dictation_format`] and [`crate::audio_toolkit::nz_english`],
//! with the same discipline: deterministic, offline, microseconds, runs on EVERY dictation
//! (both shortcuts), and returns the input **verbatim** when nothing matched.
//!
//! # The contract with the LLM layer
//!
//! This is the floor, not the ceiling. It converts only what the words make unambiguous. It never
//! adds a `$` the speaker did not say, never guesses an all-number date, and leaves any run of
//! number words it cannot read with confidence exactly as spoken. The LLM clean-up can still infer
//! the rest.
//!
//! # Conventions (New Zealand by default; `us` switches dates only)
//!
//! - One to nine stay as words in prose; 10 and above become figures. Units, money, percent,
//!   decimals, times, dates, versions and street numbers always take figures.
//! - Thousands are grouped with a comma from 1,000 (`1,200`), never in years, codes or phones.
//! - Round millions and billions keep the word: `1.5 million`, `$2 billion`.
//! - `$` means the dollar the speaker said; other currencies only when named (`US$`, `A$`, `€`).
//! - Dates are day month year with no suffix (`21 March 2026`); US style `March 21, 2026`.
//! - Times are `3:30 pm`, matching Kōrero's own meeting titles.
//! - NZ phone grouping: `021 456 7890`, `04 123 4567`, `0800 123 456`, `+64 21 456 7890`.
//! - Te reo Māori numbers (tahi, rua, toru…) are not English number words and are never touched.
//!
//! Plan and rule list: Kōrero project doc `KORERO_1.50_NUMBERS-PLAN_2026-11-22.md`.

// ---------------------------------------------------------------------------------------------
// Vocabulary
// ---------------------------------------------------------------------------------------------

const UNITS: &[(&str, u64)] = &[
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
];

const TEENS: &[(&str, u64)] = &[
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
];

const TENS: &[(&str, u64)] = &[
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];

const SCALES: &[(&str, u64)] = &[
    ("thousand", 1_000),
    ("million", 1_000_000),
    ("billion", 1_000_000_000),
];

/// Ordinal words with the cardinal value and kind they stand for.
const ORDINALS: &[(&str, u64)] = &[
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("fifth", 5),
    ("sixth", 6),
    ("seventh", 7),
    ("eighth", 8),
    ("ninth", 9),
    ("tenth", 10),
    ("eleventh", 11),
    ("twelfth", 12),
    ("thirteenth", 13),
    ("fourteenth", 14),
    ("fifteenth", 15),
    ("sixteenth", 16),
    ("seventeenth", 17),
    ("eighteenth", 18),
    ("nineteenth", 19),
    ("twentieth", 20),
    ("thirtieth", 30),
    ("fortieth", 40),
    ("fiftieth", 50),
    ("sixtieth", 60),
    ("seventieth", 70),
    ("eightieth", 80),
    ("ninetieth", 90),
    ("hundredth", 100),
    ("thousandth", 1_000),
    ("millionth", 1_000_000),
];

/// "the nineteen nineties" → 1990s.
const PLURAL_TENS: &[(&str, u64)] = &[
    ("twenties", 20),
    ("thirties", 30),
    ("forties", 40),
    ("fifties", 50),
    ("sixties", 60),
    ("seventies", 70),
    ("eighties", 80),
    ("nineties", 90),
    ("hundreds", 0),
];

const MONTHS: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Words before a number that force figures even below ten, without grouping concerns.
const DIGIT_PREV: &[&str] = &[
    "version",
    "page",
    "pages",
    "chapter",
    "section",
    "figure",
    "level",
    "gate",
    "platform",
    "terminal",
    "episode",
    "sprint",
    "milestone",
    "phase",
    "room",
    "slide",
    "tier",
    "release",
];

/// Words before a number that make it a code: figures, never grouped.
const CODE_PREV: &[&str] = &[
    "invoice",
    "ticket",
    "reference",
    "ref",
    "account",
    "flight",
    "extension",
    "ext",
    "postcode",
    "id",
    "pr",
    "issue",
    "bus",
    "route",
];

/// Within three words before a digit string ("my PIN is one nine eight four"), these make it a
/// code or number. A digit string without one stays as spoken: countdowns and mic checks.
const DIGIT_CUES: &[&str] = &[
    "invoice",
    "ticket",
    "order",
    "reference",
    "ref",
    "account",
    "flight",
    "pin",
    "code",
    "extension",
    "ext",
    "postcode",
    "dial",
    "call",
    "ring",
    "phone",
    "number",
    "id",
    "mobile",
    "cell",
    "landline",
    "on",
    "pr",
    "issue",
    "room",
    "bus",
    "route",
    "password",
    "passcode",
];

/// Words that may follow a complete date, year or clock time. Anything else (a noun: "In August
/// two people resigned") means the number belongs to what follows, not to the date.
const AFTER_OK: &[&str] = &[
    "at", "and", "or", "until", "till", "through", "is", "was", "will", "would", "works", "suits",
    "we", "i", "you", "they", "he", "she", "it", "in", "on", "for", "from", "by", "before",
    "after", "then", "so", "but", "if", "when", "next", "this", "that", "today", "tonight",
    "tomorrow", "sharp", "please", "okay", "ok", "latest", "ish", "onwards", "onward", "deadline",
    "the",
];

/// Words before a cardinal or year pair that make it a year.
const YEAR_PREV: &[&str] = &[
    "in", "since", "by", "until", "till", "from", "of", "during", "before", "after", "year",
    "circa", "early", "late", "mid", "summer", "winter", "autumn", "spring", "fy", "class", "act",
];

/// After "nine hundred three", these mean the speaker said two numbers ("900, 3 times a day").
const MULTIPLIERS: &[&str] = &[
    "times", "each", "per", "apiece", "lots", "of", "doses", "tablets",
];

/// "a hundred times", "a million thanks": hyperbole, left as words.
const HYPERBOLE: &[&str] = &[
    "times",
    "thanks",
    "ways",
    "reasons",
    "things",
    "questions",
    "miles",
    "apologies",
    "years",
];

/// Ordinals of ten and above become figures only after "the" or before one of these.
const ORD_NOUNS: &[&str] = &[
    "century",
    "birthday",
    "anniversary",
    "floor",
    "place",
    "percentile",
    "edition",
    "annual",
    "round",
    "term",
    "parliament",
    "amendment",
];

const STREET_TYPES: &[&str] = &[
    "Street",
    "St",
    "Road",
    "Rd",
    "Avenue",
    "Ave",
    "Lane",
    "Drive",
    "Terrace",
    "Tce",
    "Place",
    "Crescent",
    "Way",
    "Parade",
    "Highway",
    "Grove",
    "Close",
    "Court",
    "Quay",
    "Esplanade",
];

const TLDS: &[&str] = &[
    "nz", "com", "org", "net", "io", "ai", "au", "uk", "dev", "app", "edu", "gov", "info", "me",
    "tv",
];

/// Never the first label of a web address: "the dot com boom" is prose.
const URL_STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "of", "to", "in", "on", "at", "for", "with", "is", "it",
    "that", "this", "dot",
];

const EMAIL_CUES: &[&str] = &[
    "email", "emailed", "emailing", "e-mail", "address", "cc", "mail",
];

const AT_WORDS: &[&str] = &["at", "by", "from", "until", "till", "before", "after"];

const DAY_PARTS: &[&str] = &["morning", "afternoon", "evening"];

/// Units: spoken form (lower case, space-separated tokens) → symbol. Longest forms first.
const UNIT_FORMS: &[(&str, &str)] = &[
    ("kilometres per hour", "km/h"),
    ("kilometers per hour", "km/h"),
    ("kilometres an hour", "km/h"),
    ("kilometers an hour", "km/h"),
    ("k's an hour", "km/h"),
    ("ks an hour", "km/h"),
    ("k an hour", "km/h"),
    ("megabits per second", "Mbps"),
    ("gigabits per second", "Gbps"),
    ("kilowatt hours", "kWh"),
    ("kilowatt hour", "kWh"),
    ("square metres", "m²"),
    ("square meters", "m²"),
    ("square metre", "m²"),
    ("square meter", "m²"),
    ("square kilometres", "km²"),
    ("degrees celsius", "°C"),
    ("degree celsius", "°C"),
    ("degrees centigrade", "°C"),
    ("degrees fahrenheit", "°F"),
    ("degrees", "°"),
    ("degree", "°"),
    ("kph", "km/h"),
    ("kilometres", "km"),
    ("kilometers", "km"),
    ("kilometre", "km"),
    ("kilometer", "km"),
    ("k's", "km"),
    ("metres", "m"),
    ("meters", "m"),
    ("metre", "m"),
    ("centimetres", "cm"),
    ("centimeters", "cm"),
    ("centimetre", "cm"),
    ("millimetres", "mm"),
    ("millimeters", "mm"),
    ("millimetre", "mm"),
    ("kilograms", "kg"),
    ("kilogram", "kg"),
    ("kilos", "kg"),
    ("kilo", "kg"),
    ("grams", "g"),
    ("gram", "g"),
    ("milligrams", "mg"),
    ("milligram", "mg"),
    ("litres", "L"),
    ("liters", "L"),
    ("litre", "L"),
    ("liter", "L"),
    ("millilitres", "mL"),
    ("milliliters", "mL"),
    ("millilitre", "mL"),
    ("mils", "mL"),
    ("mls", "mL"),
    ("hectares", "ha"),
    ("hectare", "ha"),
    ("kilobytes", "kB"),
    ("megabytes", "MB"),
    ("gigabytes", "GB"),
    ("terabytes", "TB"),
    ("megabits", "Mb"),
    ("gigahertz", "GHz"),
    ("megahertz", "MHz"),
    ("kilowatts", "kW"),
    ("kilowatt", "kW"),
    ("megawatts", "MW"),
    ("megawatt", "MW"),
    ("watts", "W"),
];

/// Words after a number that take a decimal half ("two and a half hours") but no symbol.
const TIME_UNITS: &[&str] = &[
    "hours", "minutes", "seconds", "days", "weeks", "months", "years", "hour", "minute", "day",
    "week", "month", "year",
];

// ---------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Tok {
    /// Punctuation before the word (quotes, brackets).
    lead: String,
    /// The word itself, as written.
    core: String,
    /// Punctuation after the word (`,` `.` `?` `!` `)` …).
    trail: String,
    /// Whitespace after the token, kept so the output reflows nothing.
    sep: String,
    /// `core`, lower-cased.
    low: String,
}

fn tokenize(text: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut chunk = String::new();
    let mut sep = String::new();
    let flush = |chunk: &mut String, sep: &mut String, toks: &mut Vec<Tok>| {
        if chunk.is_empty() && sep.is_empty() {
            return;
        }
        let chars: Vec<char> = chunk.chars().collect();
        let is_word = |c: &char| c.is_alphanumeric();
        let start = chars.iter().position(is_word).unwrap_or(chars.len());
        let end = chars
            .iter()
            .rposition(is_word)
            .map(|p| p + 1)
            .unwrap_or(start);
        let lead: String = chars[..start].iter().collect();
        let core: String = chars[start..end].iter().collect();
        let trail: String = chars[end..].iter().collect();
        toks.push(Tok {
            low: core.to_lowercase(),
            lead,
            core,
            trail,
            sep: std::mem::take(sep),
        });
        chunk.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            sep.push(c);
        } else {
            if !sep.is_empty() {
                flush(&mut chunk, &mut sep, &mut toks);
            }
            chunk.push(c);
        }
    }
    flush(&mut chunk, &mut sep, &mut toks);
    // A text that opens with whitespace produces an empty first token carrying it; keep it.
    toks
}

fn lookup(table: &[(&str, u64)], w: &str) -> Option<u64> {
    table.iter().find(|(k, _)| *k == w).map(|(_, v)| *v)
}

fn in_list(list: &[&str], w: &str) -> bool {
    list.contains(&w)
}

/// A word that can be part of a cardinal (not "oh", "and", "a", "point").
fn is_card_word(w: &str) -> bool {
    w == "zero"
        || w == "hundred"
        || lookup(UNITS, w).is_some()
        || lookup(TEENS, w).is_some()
        || lookup(TENS, w).is_some()
        || lookup(SCALES, w).is_some()
        || lookup(ORDINALS, w).is_some()
}

fn digit_value(w: &str) -> Option<u64> {
    match w {
        "zero" | "oh" | "nought" | "o" => Some(0),
        _ => lookup(UNITS, w),
    }
}

/// Splits "twenty-one" into its parts when every part is a number word.
fn num_parts(low: &str) -> Option<Vec<String>> {
    if low.is_empty() {
        return None;
    }
    let parts: Vec<&str> = low.split('-').collect();
    if parts.iter().all(|p| is_card_word(p)) {
        Some(parts.iter().map(|p| p.to_string()).collect())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Number grammar
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Start,
    Unit,
    Teen,
    Tens,
    Hundred,
    Scale,
    And,
}

#[derive(Debug, Clone, Copy, Default)]
struct CardInfo {
    ordinal: bool,
    /// "nine hundred three": a unit straight after "hundred"/scale without "and".
    bare_after_hundred: bool,
    /// The run ended on a scale word ("ten thousand").
    ends_on_scale: Option<u64>,
    has_scale: bool,
}

/// Parses a whole run of words as one cardinal (or ordinal on its last word). `None` when the
/// words are not one well-formed number, e.g. "twenty twenty" or "three thirty".
fn parse_card(ws: &[String]) -> Option<(u64, CardInfo)> {
    if ws.is_empty() {
        return None;
    }
    let mut info = CardInfo::default();
    if ws.len() == 1 && ws[0] == "zero" {
        return Some((0, info));
    }
    let mut total: u64 = 0;
    let mut cur: u64 = 0;
    let mut last = Kind::Start;
    let mut last_scale = u64::MAX;
    for (idx, raw) in ws.iter().enumerate() {
        let is_last = idx + 1 == ws.len();
        let (w, ord) = match lookup(ORDINALS, raw) {
            Some(v) if is_last => {
                info.ordinal = true;
                (ordinal_base(v), true)
            }
            Some(_) => return None,
            None => (raw.as_str(), false),
        };
        if w == "a" {
            if idx == 0 && ws.len() > 1 && (ws[1] == "hundred" || lookup(SCALES, &ws[1]).is_some())
            {
                cur = 1;
                last = Kind::Unit;
                continue;
            }
            return None;
        }
        if w == "and" {
            if matches!(last, Kind::Hundred | Kind::Scale) && !is_last {
                last = Kind::And;
                continue;
            }
            return None;
        }
        if let Some(v) = lookup(UNITS, w) {
            let ok = match last {
                Kind::Start | Kind::And | Kind::Hundred | Kind::Scale => true,
                Kind::Tens => cur.is_multiple_of(10),
                _ => false,
            };
            if !ok {
                return None;
            }
            if matches!(last, Kind::Hundred | Kind::Scale) {
                info.bare_after_hundred = true;
            }
            cur += v;
            last = Kind::Unit;
            continue;
        }
        if let Some(v) = lookup(TEENS, w) {
            if !matches!(last, Kind::Start | Kind::And | Kind::Hundred | Kind::Scale) {
                return None;
            }
            if matches!(last, Kind::Hundred | Kind::Scale) {
                info.bare_after_hundred = true;
            }
            cur += v;
            last = Kind::Teen;
            continue;
        }
        if let Some(v) = lookup(TENS, w) {
            if !matches!(last, Kind::Start | Kind::And | Kind::Hundred | Kind::Scale) {
                return None;
            }
            if matches!(last, Kind::Hundred | Kind::Scale) {
                info.bare_after_hundred = true;
            }
            cur += v;
            last = Kind::Tens;
            continue;
        }
        if w == "hundred" && last == Kind::Start && ord {
            // "the hundredth time".
            cur = 100;
            last = Kind::Hundred;
            continue;
        }
        if w == "hundred" {
            let ok = matches!(last, Kind::Unit | Kind::Teen | Kind::Tens)
                && cur > 0
                && cur < 100
                && (total == 0 || cur < 10);
            if !ok {
                return None;
            }
            cur *= 100;
            last = Kind::Hundred;
            continue;
        }
        if let Some(s) = lookup(SCALES, w) {
            let ok = matches!(last, Kind::Unit | Kind::Teen | Kind::Tens | Kind::Hundred)
                && cur > 0
                && s < last_scale;
            if !ok {
                return None;
            }
            total += cur * s;
            cur = 0;
            last_scale = s;
            last = Kind::Scale;
            info.has_scale = true;
            if is_last {
                info.ends_on_scale = Some(s);
            }
            continue;
        }
        return None;
    }
    if last == Kind::And || last == Kind::Start {
        return None;
    }
    Some((total + cur, info))
}

/// The cardinal word an ordinal value stands for, so `parse_card` can reuse its grammar.
fn ordinal_base(v: u64) -> &'static str {
    match v {
        100 => "hundred",
        1_000 => "thousand",
        1_000_000 => "million",
        _ => UNITS
            .iter()
            .chain(TEENS.iter())
            .chain(TENS.iter())
            .find(|(_, x)| *x == v)
            .map(|(k, _)| *k)
            .unwrap_or("zero"),
    }
}

/// "a hundredth"/"one hundredth": `parse_card` needs a multiplier before "hundred".
fn ordinal_suffix(v: u64) -> &'static str {
    let t = v % 100;
    if (11..=13).contains(&t) {
        return "th";
    }
    match v % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

/// "nineteen ninety nine", "twenty twenty six", "twenty oh five" → the year.
fn parse_year_pair(ws: &[String]) -> Option<u64> {
    if ws.len() < 2 {
        return None;
    }
    let a = lookup(TEENS, &ws[0]).or_else(|| if ws[0] == "twenty" { Some(20) } else { None })?;
    if a < 11 {
        return None;
    }
    let rest = &ws[1..];
    let b = if rest[0] == "oh" || rest[0] == "o" || rest[0] == "nought" {
        if rest.len() != 2 {
            return None;
        }
        lookup(UNITS, &rest[1])?
    } else if rest.len() == 1 {
        lookup(TEENS, &rest[0]).or_else(|| lookup(TENS, &rest[0]))?
    } else if rest.len() == 2 {
        let t = lookup(TENS, &rest[0])?;
        t + lookup(UNITS, &rest[1])?
    } else {
        return None;
    };
    let y = a * 100 + b;
    if (1100..=2099).contains(&y) {
        Some(y)
    } else {
        None
    }
}

/// "nineteen nineties" → "1990s".
fn parse_decade(ws: &[String]) -> Option<String> {
    if ws.len() != 2 {
        return None;
    }
    let a = lookup(TEENS, &ws[0]).or_else(|| if ws[0] == "twenty" { Some(20) } else { None })?;
    let b = lookup(PLURAL_TENS, &ws[1])?;
    if a < 11 {
        return None;
    }
    Some(format!("{}s", a * 100 + b))
}

fn group(v: u64) -> String {
    let s = v.to_string();
    if v < 1000 {
        return s;
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `1,500,000` → `1.5 million` when the value is round to two decimals of the scale.
fn group_large(v: u64) -> String {
    for (name, s) in [("billion", 1_000_000_000u64), ("million", 1_000_000u64)] {
        if v >= s && v.is_multiple_of(s / 100) {
            let whole = v / s;
            let frac = (v % s) / (s / 100);
            return if frac == 0 {
                format!("{} {}", group(whole), name)
            } else if frac.is_multiple_of(10) {
                format!("{}.{} {}", group(whole), frac / 10, name)
            } else {
                format!("{}.{:02} {}", group(whole), frac, name)
            };
        }
    }
    group(v)
}

// ---------------------------------------------------------------------------------------------
// The pass
// ---------------------------------------------------------------------------------------------

/// True when the dictation language writes English, the only language these rules know.
/// `en`, `en-NZ`, `en-AU`, `en-GB`, `en-US`; never "auto", which may be any language
/// (French "six cents" is 600, not `6c`).
pub fn is_english_writing(selected_language: &str) -> bool {
    selected_language == "en" || selected_language.starts_with("en-")
}

/// Options for [`apply_written_forms`].
#[derive(Debug, Clone, Default)]
pub struct WrittenFormsOptions {
    /// US date order (`March 21, 2026`) instead of NZ (`21 March 2026`).
    pub us_dates: bool,
    /// The user's own words and corrections: any token they cover is never changed.
    pub protected: Vec<String>,
}

enum Step {
    /// Replace tokens `i..=end` with `text`; optionally drop the previous emitted token (`the`,
    /// `minus`, `FY`, `v`) when its lower-cased word equals the given one.
    Replace {
        text: String,
        end: usize,
        drop_prev: Option<&'static str>,
    },
    /// Emit tokens `i..=end` unchanged and carry on after them.
    Skip { end: usize },
}

struct Piece {
    text: String,
    sep: String,
    low: String,
}

struct Pass<'a> {
    toks: &'a [Tok],
    protected: Vec<bool>,
    us: bool,
}

/// Applies the written-forms pass. Returns the input unchanged when nothing matched.
pub fn apply_written_forms(text: &str, opts: &WrittenFormsOptions) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    // Quick exit: no English number word, month or "dot"/"at" at all.
    let toks = tokenize(text);
    let protected = protect(&toks, &opts.protected);
    let pass = Pass {
        toks: &toks,
        protected,
        us: opts.us_dates,
    };
    let mut out: Vec<Piece> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        let step = if pass.protected[i] {
            None
        } else {
            pass.try_at(i)
        };
        match step {
            Some(Step::Replace {
                text,
                end,
                drop_prev,
            }) => {
                let mut lead = toks[i].lead.clone();
                if let Some(w) = drop_prev.filter(|_| i > 0 && !pass.protected[i - 1]) {
                    if let Some(p) = out.pop_if(|p| p.low == w) {
                        // Keep the dropped word's own opening punctuation and capitalised start.
                        let first_alpha = p.text.chars().position(|c| c.is_alphanumeric());
                        if let Some(pos) = first_alpha {
                            let mut pre: String = p.text.chars().take(pos).collect();
                            pre.push_str(&lead);
                            lead = pre;
                        }
                    }
                }
                let t = &toks[end];
                out.push(Piece {
                    text: format!("{lead}{text}{}", t.trail),
                    sep: t.sep.clone(),
                    low: String::new(),
                });
                i = end + 1;
            }
            Some(Step::Skip { end }) => {
                out.extend(toks[i..=end].iter().map(raw_piece));
                i = end + 1;
            }
            None => {
                out.push(raw_piece(&toks[i]));
                i += 1;
            }
        }
    }
    let result: String = out.iter().map(|p| format!("{}{}", p.text, p.sep)).collect();
    if result == text {
        text.to_string()
    } else {
        result
    }
}

fn raw_piece(t: &Tok) -> Piece {
    Piece {
        text: format!("{}{}{}", t.lead, t.core, t.trail),
        sep: t.sep.clone(),
        low: if t.lead.is_empty() && t.trail.is_empty() {
            t.low.clone()
        } else {
            String::new()
        },
    }
}

/// Marks every token covered by one of the user's protected phrases.
fn protect(toks: &[Tok], phrases: &[String]) -> Vec<bool> {
    let mut mask = vec![false; toks.len()];
    for phrase in phrases {
        let words: Vec<String> = phrase
            .split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|w| !w.is_empty())
            .collect();
        if words.is_empty() || words.len() > toks.len() {
            continue;
        }
        for s in 0..=toks.len() - words.len() {
            if words.iter().enumerate().all(|(k, w)| toks[s + k].low == *w) {
                for m in mask.iter_mut().skip(s).take(words.len()) {
                    *m = true;
                }
            }
        }
    }
    mask
}

/// A collected run of number words.
struct Run {
    words: Vec<String>,
    /// Last token index of the run.
    end: usize,
}

#[derive(Debug, Clone)]
enum Qty {
    Percent,
    /// Symbol, and whether it attaches with no space (`20°C`).
    Unit(&'static str, bool),
    /// Currency symbol before the number.
    Money(&'static str),
    /// `50c`.
    Cents,
}

impl<'a> Pass<'a> {
    fn low(&self, i: usize) -> &str {
        self.toks.get(i).map(|t| t.low.as_str()).unwrap_or("")
    }

    /// True when token `i` exists, has no punctuation around it and is not protected: it may be
    /// read together with the token before it.
    fn clean(&self, i: usize) -> bool {
        self.toks
            .get(i)
            .map(|t| t.lead.is_empty() && !self.protected[i])
            .unwrap_or(false)
    }

    /// True when the run can continue from token `i` to `i + 1` (no punctuation between them).
    fn joins(&self, i: usize) -> bool {
        self.toks
            .get(i)
            .map(|t| t.trail.is_empty())
            .unwrap_or(false)
            && self.clean(i + 1)
    }

    fn prev_low(&self, i: usize) -> &str {
        if i == 0 || !self.toks[i - 1].trail.is_empty() {
            return "";
        }
        self.low(i - 1)
    }

    fn try_at(&self, i: usize) -> Option<Step> {
        let t = &self.toks[i];
        if t.core.is_empty() {
            return None;
        }
        if let Some(s) = self.fixed_phrase(i) {
            return Some(s);
        }
        if let Some(s) = self.web_address(i) {
            return Some(s);
        }
        if let Some(s) = self.intl_phone(i) {
            return Some(s);
        }
        if let Some(s) = self.clock_phrase(i) {
            return Some(s);
        }
        if let Some(s) = self.month_first_date(i) {
            return Some(s);
        }
        if let Some(s) = self.dollar_and_cents_a(i) {
            return Some(s);
        }
        self.number_run(i)
    }

    // ------------------------------------------------------------------ fixed phrases

    fn fixed_phrase(&self, i: usize) -> Option<Step> {
        const PHRASES: &[(&[&str], &str)] = &[
            (&["fifty", "fifty"], "50/50"),
            (&["twenty", "four", "seven"], "24/7"),
            (&["twenty-four", "seven"], "24/7"),
        ];
        // Idioms that must stay words.
        const IDIOMS: &[&[&str]] = &[
            &["forty", "winks"],
            &["nineteen", "to", "the", "dozen"],
            &["eleventh", "hour"],
            &["twenty", "questions"],
        ];
        for idiom in IDIOMS {
            if self.matches_seq(i, idiom) {
                return Some(Step::Skip {
                    end: i + idiom.len() - 1,
                });
            }
        }
        if self.prev_low(i) == "catch" {
            if let Some(run) = self.collect(i) {
                return Some(Step::Skip { end: run.end });
            }
        }
        for (seq, out) in PHRASES {
            if self.matches_seq(i, seq) {
                let end = i + seq.len() - 1;
                // Not when the run carries on ("fifty fifty five").
                if self.joins(end) && num_parts(self.low(end + 1)).is_some() {
                    return None;
                }
                return Some(Step::Replace {
                    text: out.to_string(),
                    end,
                    drop_prev: None,
                });
            }
        }
        None
    }

    fn matches_seq(&self, i: usize, seq: &[&str]) -> bool {
        if i + seq.len() > self.toks.len() {
            return false;
        }
        seq.iter()
            .enumerate()
            .all(|(k, w)| self.low(i + k) == *w && (k + 1 == seq.len() || self.joins(i + k)))
            && !self.protected[i]
    }

    // ------------------------------------------------------------------ email and web

    fn web_address(&self, i: usize) -> Option<Step> {
        let lo = self.low(i);
        // Email: <local> at <label> (dot <label>)+, only after an email cue word.
        if self.joins(i) && self.low(i + 1) == "at" && is_label(lo) && !in_list(URL_STOP, lo) {
            let cue = (1..=3).any(|k| i >= k && in_list(EMAIL_CUES, self.low(i - k)));
            if cue {
                if let Some((domain, end)) = self.domain(i + 2) {
                    return Some(Step::Replace {
                        text: format!("{lo}@{domain}"),
                        end,
                        drop_prev: None,
                    });
                }
            }
        }
        // www dot …
        if lo == "www" || (lo == "w" && self.matches_seq(i, &["w", "w", "w"])) {
            let start = if lo == "www" { i } else { i + 2 };
            if self.joins(start) && self.low(start + 1) == "dot" {
                if let Some((domain, end)) = self.domain(start + 2) {
                    return Some(Step::Replace {
                        text: format!("www.{domain}"),
                        end,
                        drop_prev: None,
                    });
                }
            }
        }
        if !in_list(URL_STOP, lo) {
            if let Some((domain, end)) = self.domain(i) {
                return Some(Step::Replace {
                    text: domain,
                    end,
                    drop_prev: None,
                });
            }
        }
        None
    }

    /// `kyt dot co dot nz slash insights` → `kyt.co.nz/insights`.
    fn domain(&self, i: usize) -> Option<(String, usize)> {
        let first = self.low(i);
        // The first label starts with a letter: "21c dot nz" is not an address.
        if !is_label(first)
            || !first
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false)
            || is_card_word(first)
            || !self.clean(i)
        {
            return None;
        }
        let mut labels = vec![first.to_string()];
        let mut k = i;
        while self.joins(k)
            && self.low(k + 1) == "dot"
            && self.joins(k + 1)
            && is_label(self.low(k + 2))
        {
            labels.push(self.low(k + 2).to_string());
            k += 2;
        }
        if labels.len() < 2 || !in_list(TLDS, labels.last().unwrap()) {
            return None;
        }
        let mut out = labels.join(".");
        while self.joins(k)
            && self.low(k + 1) == "slash"
            && self.joins(k + 1)
            && is_label(self.low(k + 2))
        {
            out.push('/');
            out.push_str(self.low(k + 2));
            k += 2;
        }
        Some((out, k))
    }

    // ------------------------------------------------------------------ phones

    /// "plus sixty four two one …" → `+64 21 …`.
    fn intl_phone(&self, i: usize) -> Option<Step> {
        if self.low(i) != "plus" || !self.joins(i) {
            return None;
        }
        let cc = self.collect(i + 1)?;
        // The run may run straight into the digits ("sixty four two one four …").
        let (code, digits_start) =
            if cc.words.len() >= 2 && cc.words[0] == "sixty" && cc.words[1] == "four" {
                (64, 2)
            } else {
                return None;
            };
        let rest = &cc.words[digits_start..];
        let digits = digit_string(rest)?;
        if digits.len() < 7 {
            return None;
        }
        let digits = digits.trim_start_matches('0').to_string();
        if digits.len() < 7 {
            // All zeros or too short once the trunk zero is gone: not a phone number.
            return Some(Step::Skip { end: cc.end });
        }
        let (area, local) = if digits.starts_with('2') {
            digits.split_at(2)
        } else {
            digits.split_at(1)
        };
        Some(Step::Replace {
            text: format!("+{code} {area} {}", group_digits(local)),
            end: cc.end,
            drop_prev: None,
        })
    }

    // ------------------------------------------------------------------ clock phrases

    /// "half past three", "quarter to five", "twenty past six" with am/pm, a part of the day, or
    /// "at"/"by"/… before it.
    fn clock_phrase(&self, i: usize) -> Option<Step> {
        let lo = self.low(i);
        let (mins, rel_at) = if lo == "half" || lo == "quarter" {
            if !self.joins(i) {
                return None;
            }
            (if lo == "half" { 30 } else { 15 }, i + 1)
        } else {
            let run = self.collect(i)?;
            let (v, info) = parse_card(&run.words)?;
            if info.ordinal || !matches!(v, 5 | 10 | 20 | 25) || !self.joins(run.end) {
                return None;
            }
            (v, run.end + 1)
        };
        let rel = self.low(rel_at);
        if rel != "past" && rel != "to" {
            return None;
        }
        if lo == "half" && rel == "to" {
            return None;
        }
        if !self.joins(rel_at) {
            return None;
        }
        let hr = self.collect(rel_at + 1)?;
        if hr.words.len() != 1 {
            return None;
        }
        let (h, info) = parse_card(&hr.words)?;
        if info.ordinal || !(1..=12).contains(&h) {
            return None;
        }
        let (h, m) = if rel == "past" {
            (h, mins)
        } else {
            (if h == 1 { 12 } else { h - 1 }, 60 - mins)
        };
        let base = format!("{h}:{m:02}");
        if let Some((ampm, end)) = self.ampm(hr.end) {
            return Some(Step::Replace {
                text: format!("{base} {ampm}"),
                end,
                drop_prev: None,
            });
        }
        if self.day_part_follows(hr.end) || in_list(AT_WORDS, self.prev_low(i)) {
            return Some(Step::Replace {
                text: base,
                end: hr.end,
                drop_prev: None,
            });
        }
        None
    }

    /// am/pm after token `i`: returns the normalised form and the token it ends on.
    fn ampm(&self, i: usize) -> Option<(&'static str, usize)> {
        if !self.joins(i) {
            return None;
        }
        let t = &self.toks[i + 1];
        let w: String = t.low.chars().filter(|c| *c != '.').collect();
        match w.as_str() {
            "am" => Some(("am", i + 1)),
            "pm" => Some(("pm", i + 1)),
            _ => None,
        }
    }

    fn day_part_follows(&self, i: usize) -> bool {
        (self.joins(i)
            && self.low(i + 1) == "in"
            && self.joins(i + 1)
            && self.low(i + 2) == "the"
            && self
                .toks
                .get(i + 3)
                .map(|t| in_list(DAY_PARTS, &t.low))
                .unwrap_or(false))
            || (self.joins(i) && self.low(i + 1) == "at" && self.low(i + 2) == "night")
            || (self.joins(i) && self.low(i + 1) == "tonight")
    }

    // ------------------------------------------------------------------ dates

    fn month_at(&self, i: usize) -> Option<&'static str> {
        let t = self.toks.get(i)?;
        if !t.lead.is_empty() || self.protected[i] {
            return None;
        }
        let m = MONTHS.iter().find(|m| m.to_lowercase() == t.low)?;
        // "may" and "march" are also ordinary words: only the capitalised form is a month.
        if (t.low == "may" || t.low == "march")
            && !t
                .core
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
        {
            return None;
        }
        Some(m)
    }

    /// A day of the month at token `i` ("twenty first", "twenty one"): value and end.
    fn day_at(&self, i: usize) -> Option<(u64, usize)> {
        let run = self.collect(i)?;
        let (v, _) = parse_card(&run.words)?;
        if (1..=31).contains(&v) {
            Some((v, run.end))
        } else {
            None
        }
    }

    /// A year after token `i` (optionally after a comma on token `i`).
    fn year_after(&self, i: usize) -> Option<(u64, usize)> {
        let t = self.toks.get(i)?;
        if !(t.trail.is_empty() || t.trail == ",") || !self.clean(i + 1) {
            return None;
        }
        let run = self.collect(i + 1)?;
        if let Some(y) = parse_year_pair(&run.words) {
            return Some((y, run.end));
        }
        let (v, info) = parse_card(&run.words)?;
        if !info.ordinal && (1100..=2099).contains(&v) && self.ends_ok(run.end) {
            return Some((v, run.end));
        }
        None
    }

    /// True when nothing after token `end` continues the phrase: punctuation, the end, or a word
    /// from `AFTER_OK`.
    fn ends_ok(&self, end: usize) -> bool {
        self.ends_ok_in(end, AFTER_OK)
    }

    fn ends_ok_in(&self, end: usize, ok: &[&str]) -> bool {
        match self.toks.get(end) {
            None => true,
            Some(t) if !t.trail.is_empty() => true,
            Some(_) => end + 1 >= self.toks.len() || in_list(ok, self.low(end + 1)),
        }
    }

    fn fmt_date(&self, d: u64, m: &str, y: Option<u64>) -> String {
        match (self.us, y) {
            (false, Some(y)) => format!("{d} {m} {y}"),
            (false, None) => format!("{d} {m}"),
            (true, Some(y)) => format!("{m} {d}, {y}"),
            (true, None) => format!("{m} {d}"),
        }
    }

    /// "March twenty first", "March the twenty first, twenty twenty six".
    fn month_first_date(&self, i: usize) -> Option<Step> {
        let m = self.month_at(i)?;
        if !self.joins(i) {
            return None;
        }
        let mut k = i + 1;
        if self.low(k) == "the" {
            if !self.joins(k) {
                return None;
            }
            k += 1;
        }
        // "May the fourth be with you", "May one of you…": at the start of a sentence these
        // are verbs. Only after a date word.
        if matches!(m, "May" | "March")
            && !in_list(
                &[
                    "on", "by", "from", "until", "till", "in", "since", "before", "after", "of",
                    "early", "late", "mid", "next", "last", "this", "due", "dated",
                ],
                self.prev_low(i),
            )
        {
            return None;
        }
        let (d, end) = self.day_at(k)?;
        let (y, end) = match self.year_after(end) {
            Some((y, e)) => (Some(y), e),
            None => (None, end),
        };
        if !self.ends_ok(end) {
            return None;
        }
        Some(Step::Replace {
            text: self.fmt_date(d, m, y),
            end,
            drop_prev: None,
        })
    }

    /// Day first: "the twenty first of March", "twenty one March 2026". `end` is the day's end.
    fn day_first_date(&self, d: u64, end: usize) -> Option<(String, usize)> {
        if !(1..=31).contains(&d) || !self.joins(end) {
            return None;
        }
        let mut k = end + 1;
        if self.low(k) == "of" {
            if !self.joins(k) {
                return None;
            }
            k += 1;
        }
        let m = self.month_at(k)?;
        let (y, end) = match self.year_after(k) {
            Some((y, e)) => (Some(y), e),
            None => (None, k),
        };
        if !self.ends_ok(end) {
            return None;
        }
        Some((self.fmt_date(d, m, y), end))
    }

    // ------------------------------------------------------------------ money oddities

    /// "a dollar fifty" → `$1.50`.
    fn dollar_and_cents_a(&self, i: usize) -> Option<Step> {
        if self.low(i) != "a" || !self.joins(i) || self.low(i + 1) != "dollar" || !self.joins(i + 1)
        {
            return None;
        }
        let run = self.collect(i + 2)?;
        let (c, info) = parse_card(&run.words)?;
        if info.ordinal || !(1..=99).contains(&c) || !self.money_tail_ok(run.end) {
            return None;
        }
        Some(Step::Replace {
            text: format!("$1.{c:02}"),
            end: run.end,
            drop_prev: None,
        })
    }

    /// Bare cents after "dollars" are read only where nothing else follows them.
    fn money_tail_ok(&self, end: usize) -> bool {
        let t = &self.toks[end];
        if !t.trail.is_empty() || end + 1 >= self.toks.len() {
            return true;
        }
        in_list(
            &[
                "each",
                "an",
                "a",
                "per",
                "for",
                "and",
                "plus",
                "including",
                "please",
                "thanks",
            ],
            self.low(end + 1),
        )
    }

    // ------------------------------------------------------------------ runs

    /// Collects the run of number words starting at `i`.
    fn collect(&self, i: usize) -> Option<Run> {
        if !self.clean_start(i) {
            return None;
        }
        let mut words: Vec<String> = Vec::new();
        let mut ends: Vec<usize> = Vec::new();
        let mut k = i;
        loop {
            let lo = self.low(k);
            let next = self.low(k + 1);
            let next_is_num = num_parts(next).is_some();
            let last_word = words.last().map(|s| s.as_str()).unwrap_or("");
            let accepted = if let Some(parts) = num_parts(lo) {
                words.extend(parts.iter().cloned());
                true
            } else if lo == "a"
                && k == i
                && matches!(next, "hundred" | "thousand" | "million" | "billion")
            {
                words.push("a".into());
                true
            } else if lo == "and"
                && k > i
                && (last_word == "hundred" || lookup(SCALES, last_word).is_some())
                && next_is_num
            {
                words.push("and".into());
                true
            } else if lo == "point"
                && ((k > i && (is_card_word(last_word) || last_word == "oh"))
                    || (k == i && digit_value(next).is_some() && next != "o"))
            {
                words.push("point".into());
                true
            } else if matches!(lo, "oh" | "nought" | "o")
                && (k > i || next_is_num || digit_value(next).is_some() || next == "point")
            {
                words.push("oh".into());
                true
            } else if matches!(lo, "double" | "triple") && digit_value(next).is_some() {
                words.push(lo.to_string());
                true
            } else if k > i && lookup(PLURAL_TENS, lo).is_some() {
                // "nineteen nineties": the decade word closes the run.
                words.push(lo.to_string());
                ends.resize(words.len(), k);
                break;
            } else {
                false
            };
            if !accepted {
                break;
            }
            ends.resize(words.len(), k);
            if !self.joins(k) {
                break;
            }
            k += 1;
        }
        // Trim connective words the run cannot end on.
        while let Some(w) = words.last() {
            if matches!(
                w.as_str(),
                "and" | "point" | "double" | "triple" | "a" | "oh"
            ) && words.len() > 1
            {
                if w == "oh" && words.iter().filter(|x| *x != "oh").count() > 0 && words.len() >= 2
                {
                    // "oh" may legitimately end a digit run ("one two oh"), keep it.
                    break;
                }
                words.pop();
                ends.pop();
            } else {
                break;
            }
        }
        if words.is_empty()
            || words
                .iter()
                .all(|w| matches!(w.as_str(), "oh" | "a" | "and" | "point"))
        {
            return None;
        }
        let end = *ends.last()?;
        Some(Run { words, end })
    }

    fn clean_start(&self, i: usize) -> bool {
        self.toks
            .get(i)
            .map(|t| !self.protected[i] && !t.core.is_empty())
            .unwrap_or(false)
    }

    /// The quantity named after token `end` (percent, unit, money), if any.
    fn qty_after(&self, end: usize) -> Option<(Qty, usize)> {
        if !self.joins(end) {
            return None;
        }
        let k = end + 1;
        let lo = self.low(k);
        if lo == "percent" || lo == "%" {
            return Some((Qty::Percent, k));
        }
        if lo == "per" && self.joins(k) && self.low(k + 1) == "cent" {
            return Some((Qty::Percent, k + 1));
        }
        // Currency, optionally qualified.
        let quals: &[(&[&str], &'static str)] = &[
            (&["us"], "US$"),
            (&["american"], "US$"),
            (&["aussie"], "A$"),
            (&["australian"], "A$"),
            (&["nz"], "NZ$"),
            (&["kiwi"], "NZ$"),
            (&["new", "zealand"], "NZ$"),
            (&["canadian"], "C$"),
            (&["singapore"], "S$"),
            (&["hong", "kong"], "HK$"),
        ];
        for (q, sym) in quals {
            if self.matches_seq(k, q) {
                let after = k + q.len() - 1;
                if self.joins(after) && matches!(self.low(after + 1), "dollars" | "dollar") {
                    return Some((Qty::Money(sym), after + 1));
                }
            }
        }
        match lo {
            "dollars" | "dollar" | "bucks" => return Some((Qty::Money("$"), k)),
            "euros" | "euro" => return Some((Qty::Money("€"), k)),
            "yen" => return Some((Qty::Money("¥"), k)),
            "quid" => return Some((Qty::Money("£"), k)),
            "cents" | "cent" => return Some((Qty::Cents, k)),
            _ => {}
        }
        if self.matches_seq(k, &["japanese", "yen"]) {
            return Some((Qty::Money("¥"), k + 1));
        }
        if self.matches_seq(k, &["british", "pounds"]) {
            return Some((Qty::Money("£"), k + 1));
        }
        if self.matches_seq(k, &["pounds", "sterling"]) {
            return Some((Qty::Money("£"), k + 1));
        }
        for (form, sym) in UNIT_FORMS {
            let seq: Vec<&str> = form.split(' ').collect();
            if self.matches_seq(k, &seq) {
                // Plain "degrees" is also a qualification ("two degrees in law"): only as a
                // temperature at the end of a phrase or before a weather word.
                if *sym == "°"
                    && !self.ends_ok_in(
                        k,
                        &[
                            "today",
                            "tonight",
                            "tomorrow",
                            "outside",
                            "overnight",
                            "inside",
                            "warmer",
                            "colder",
                            "hotter",
                            "cooler",
                        ],
                    )
                {
                    return None;
                }
                let attach = sym.starts_with('°');
                return Some((Qty::Unit(sym, attach), k + seq.len() - 1));
            }
        }
        None
    }

    fn fmt_qty(&self, num: &str, q: &Qty) -> String {
        match q {
            Qty::Percent => format!("{num}%"),
            Qty::Unit(sym, true) => format!("{num}{sym}"),
            Qty::Unit(sym, false) => format!("{num} {sym}"),
            Qty::Money(sym) => format!("{sym}{num}"),
            Qty::Cents => format!("{num}c"),
        }
    }

    /// Cents after a dollar amount: "and fifty cents", "fifty cents", or bare "fifty" at the end.
    fn cents_after(&self, end: usize) -> Option<(u64, usize)> {
        if !self.joins(end) {
            return None;
        }
        let mut k = end + 1;
        if self.low(k) == "and" {
            if !self.joins(k) {
                return None;
            }
            k += 1;
        }
        let run = self.collect(k)?;
        let (c, info) = parse_card(&run.words)?;
        if info.ordinal || !(1..=99).contains(&c) {
            return None;
        }
        if self.joins(run.end) && matches!(self.low(run.end + 1), "cents" | "cent") {
            return Some((c, run.end + 1));
        }
        if self.money_tail_ok(run.end) {
            return Some((c, run.end));
        }
        None
    }

    /// The main entry for a run of number words at `i`.
    fn number_run(&self, i: usize) -> Option<Step> {
        let run = self.collect(i)?;
        let ws = &run.words;
        let prev = self.prev_low(i);

        // Words before it that stay or go.
        let neg = matches!(prev, "minus" | "negative") && !self.protected[i - 1];
        let vprefix = (prev == "v") && i > 0 && self.toks[i - 1].trail.is_empty();
        let fy = prev == "fy";

        // 1. Digit strings: phones, codes, PINs.
        if ws
            .iter()
            .all(|w| digit_value(w).is_some() || matches!(w.as_str(), "double" | "triple"))
        {
            if let Some(d) = digit_string(ws) {
                // A cue word within three words before ("my PIN is …", "call me on …"), or a
                // phone-shaped string (trunk zero, 8+ digits). Without one, a digit string is a
                // countdown or a mic check ("five four three two one", "one two one two").
                let id = (1..=3).any(|k| i >= k && in_list(DIGIT_CUES, self.low(i - k)))
                    || in_list(DIGIT_PREV, prev);
                let phone_shaped = d.starts_with('0') && d.len() >= 8;
                if (id || phone_shaped) && d.len() >= 3 && ws.len() >= 3 {
                    let text = if d.starts_with('0') && d.len() >= 8 {
                        nz_phone(&d)
                    } else {
                        d
                    };
                    return Some(Step::Replace {
                        text,
                        end: run.end,
                        drop_prev: None,
                    });
                }
            }
            if ws.len() >= 2
                && ws
                    .iter()
                    .any(|w| matches!(w.as_str(), "oh" | "double" | "triple"))
            {
                return Some(Step::Skip { end: run.end });
            }
        }

        // 2. Decimals and versions.
        if ws.iter().any(|w| w == "point") {
            // "At one point two of them left": an idiom, not 1.2.
            if ws.len() >= 2 && ws[0] == "one" && ws[1] == "point" && matches!(prev, "at" | "some")
            {
                return Some(Step::Skip { end: run.end });
            }
            let version_ctx = matches!(
                prev,
                "version"
                    | "v"
                    | "release"
                    | "build"
                    | "firmware"
                    | "python"
                    | "node"
                    | "ios"
                    | "android"
                    | "windows"
            );
            let Some((num, scale)) = decimal_string(ws, version_ctx) else {
                return Some(Step::Skip { end: run.end });
            };
            // A bare "point five" only with what it measures ("point five mils").
            if ws[0] == "point" && self.qty_after(run.end).is_none() {
                return Some(Step::Skip { end: run.end });
            }
            let mut num = match scale {
                Some(s) => format!("{num} {s}"),
                None => num,
            };
            if vprefix {
                return Some(Step::Replace {
                    text: format!("{}{num}", self.toks[i - 1].core),
                    end: run.end,
                    drop_prev: Some("v"),
                });
            }
            if let Some((q, qend)) = self.qty_after(run.end) {
                if neg {
                    num = format!("-{num}");
                }
                return Some(Step::Replace {
                    text: self.fmt_qty(&num, &q),
                    end: qend,
                    drop_prev: if neg { Some(prev_static(prev)) } else { None },
                });
            }
            return Some(Step::Replace {
                text: num,
                end: run.end,
                drop_prev: None,
            });
        }

        // 3. Decades and year pairs.
        if let Some(dec) = parse_decade(ws) {
            return Some(Step::Replace {
                text: dec,
                end: run.end,
                drop_prev: None,
            });
        }
        if let Some(y) = parse_year_pair(ws) {
            let clock_like = y < 1300 && y % 100 < 60 && in_list(AT_WORDS, prev);
            let time_unit_next = self.joins(run.end) && in_list(TIME_UNITS, self.low(run.end + 1));
            if !clock_like && !time_unit_next && self.year_context(i, run.end) {
                return Some(Step::Replace {
                    text: y.to_string(),
                    end: run.end,
                    drop_prev: None,
                });
            }
            // A clock time read as a pair ("eleven fifteen am", "before twelve thirty") is
            // handled below.
            if !clock_like
                && self.ampm(run.end).is_none()
                && !(self.joins(run.end) && self.low(run.end + 1) == "hours")
            {
                return Some(Step::Skip { end: run.end });
            }
        }

        // 4. Clock times: "three thirty pm", "at nine fifteen", "fourteen hundred hours".
        if let Some(s) = self.clock_run(i, &run) {
            return Some(s);
        }

        // 5. One well-formed number.
        let Some((v, info)) = parse_card(ws) else {
            // Two numbers side by side ("twelve fifteen", "two three"): leave as spoken.
            return Some(Step::Skip { end: run.end });
        };

        if info.bare_after_hundred
            && self.joins(run.end)
            && in_list(MULTIPLIERS, self.low(run.end + 1))
        {
            return Some(Step::Skip { end: run.end });
        }
        if ws[0] == "a" && self.joins(run.end) && in_list(HYPERBOLE, self.low(run.end + 1)) {
            return Some(Step::Skip { end: run.end });
        }

        // Ordinals. "thirty second ad", "the ninety second rule": tens + "second" is a duration.
        if info.ordinal
            && ws.len() >= 2
            && ws.last().map(|w| w == "second").unwrap_or(false)
            && !(self.joins(run.end) && in_list(ORD_NOUNS, self.low(run.end + 1)))
        {
            if let Some((text, end)) = self.day_first_date(v, run.end) {
                return Some(Step::Replace {
                    text,
                    end,
                    drop_prev: Some("the"),
                });
            }
            return Some(Step::Skip { end: run.end });
        }
        if info.ordinal {
            if let Some((text, end)) = self.day_first_date(v, run.end) {
                return Some(Step::Replace {
                    text,
                    end,
                    drop_prev: Some("the"),
                });
            }
            let noun = self.joins(run.end) && in_list(ORD_NOUNS, self.low(run.end + 1));
            // After "the", only when a noun follows: "the 21st century", never "of the eleventh".
            let the_noun = prev == "the"
                && self.joins(run.end)
                && !matches!(self.low(run.end + 1), "of" | "and" | "or" | "to");
            // "on the twenty third": a day of the month.
            let on_the_day = prev == "the"
                && (1..=31).contains(&v)
                && i >= 2
                && self.toks[i - 2].trail.is_empty()
                && in_list(
                    &[
                        "on", "by", "from", "until", "till", "before", "after", "since",
                    ],
                    self.low(i - 2),
                );
            if v >= 10 && (the_noun || noun || on_the_day) {
                return Some(Step::Replace {
                    text: format!("{}{}", group(v), ordinal_suffix(v)),
                    end: run.end,
                    drop_prev: None,
                });
            }
            return Some(Step::Skip { end: run.end });
        }

        // Day-first date with a cardinal day ("twenty one March").
        if (1..=31).contains(&v) && self.joins(run.end) && self.month_at(run.end + 1).is_some() {
            if let Some((text, end)) = self.day_first_date(v, run.end) {
                return Some(Step::Replace {
                    text,
                    end,
                    drop_prev: Some("the"),
                });
            }
        }

        // FY26, v2.
        if fy && (10..=99).contains(&v) {
            return Some(Step::Replace {
                text: format!("{}{v}", self.toks[i - 1].core),
                end: run.end,
                drop_prev: Some("fy"),
            });
        }
        if vprefix {
            return Some(Step::Replace {
                text: format!("{}{v}", self.toks[i - 1].core),
                end: run.end,
                drop_prev: Some("v"),
            });
        }

        let mut num = group_large(v);
        let mut end = run.end;

        // "and a half".
        if self.matches_seq_after(run.end, &["and", "a", "half"]) {
            let half_end = run.end + 3;
            let next = self.low(half_end + 1);
            if self.joins(half_end) && matches!(next, "million" | "billion") {
                num = format!("{v}.5 {next}");
                end = half_end + 1;
            } else if self.qty_after(half_end).is_some()
                || (self.joins(half_end) && in_list(TIME_UNITS, next))
            {
                num = format!("{v}.5");
                end = half_end;
            }
        }

        // "five grand", "fifty k" (with dollars).
        if end == run.end && self.joins(run.end) {
            let next = self.low(run.end + 1);
            if next == "grand" && (self.money_tail_ok(run.end + 1) || self.ends_ok(run.end + 1)) {
                return Some(Step::Replace {
                    text: format!("${}", group_large(v * 1000)),
                    end: run.end + 1,
                    drop_prev: None,
                });
            }
            if next == "k"
                && !info.has_scale
                && self.joins(run.end + 1)
                && matches!(self.low(run.end + 2), "dollars" | "dollar")
            {
                return Some(Step::Replace {
                    text: format!("${}", group_large(v * 1000)),
                    end: run.end + 2,
                    drop_prev: None,
                });
            }
            if matches!(next, "k" | "ks") {
                // "fifty k": thousand or kilometres; leave as spoken.
                return Some(Step::Skip { end: run.end });
            }
            if next == "nil" {
                return Some(Step::Replace {
                    text: format!("{v}–0"),
                    end: run.end + 1,
                    drop_prev: None,
                });
            }
        }

        // Ranges and ratios: "five to ten percent", "five to ten thousand dollars", "three to one ratio".
        if end == run.end {
            if let Some(s) = self.range_from(i, v, &info, run.end) {
                return Some(s);
            }
        }

        // "two or three hundred dollars", "five, six hundred dollars": the first number belongs
        // to the amount too, so neither is converted alone.
        if i > 0 && self.qty_after(end).is_some() {
            let p = &self.toks[i - 1];
            // Figures count too, so a second pass reads the text the same way.
            let numberish = |w: &str| {
                num_parts(w).is_some() || (!w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
            };
            let num_before_comma = p.trail == "," && numberish(&p.low);
            let num_before_or = matches!(prev, "or" | "and")
                && i >= 2
                && self.toks[i - 2].trail.is_empty()
                && numberish(self.low(i - 2));
            if num_before_comma || num_before_or {
                return Some(Step::Skip { end: run.end });
            }
        }

        // Quantities. "Total minus five dollars" is a subtraction: a sign only on measures.
        if let Some((q, qend)) = self.qty_after(end) {
            let neg = neg && matches!(q, Qty::Unit(..) | Qty::Percent);
            if let Qty::Money(_) = q {
                if !num.contains('.') && !num.contains(' ') {
                    if let Some((c, cend)) = self.cents_after(qend) {
                        let text = self.fmt_qty(&format!("{num}.{c:02}"), &q);
                        return Some(Step::Replace {
                            text: if neg { format!("-{text}") } else { text },
                            end: cend,
                            drop_prev: if neg { Some(prev_static(prev)) } else { None },
                        });
                    }
                }
            }
            if let Qty::Cents = q {
                if v >= 100 {
                    return Some(Step::Skip { end: run.end });
                }
            }
            let mut text = self.fmt_qty(&num, &q);
            if neg {
                text = format!("-{text}");
            }
            return Some(Step::Replace {
                text,
                end: qend,
                drop_prev: if neg { Some(prev_static(prev)) } else { None },
            });
        }
        if end != run.end {
            // "two and a half hours".
            return Some(Step::Replace {
                text: num,
                end,
                drop_prev: None,
            });
        }

        // Years: a cardinal-form year must end its phrase ("in two thousand and five, we"),
        // never "since twelve hundred people signed".
        // "In two thousand twenty five revenue grew": a spoken year this century after a year
        // word is a year even mid-phrase; a round "two thousand" is not.
        let this_century = (2001..=2099).contains(&v)
            && ws.iter().any(|w| w == "thousand")
            && matches!(prev, "in" | "since" | "until" | "till" | "during");
        if (1100..=2099).contains(&v)
            && self.year_context(i, run.end)
            && (self.ends_ok(run.end) || this_century)
        {
            return Some(Step::Replace {
                text: v.to_string(),
                end: run.end,
                drop_prev: None,
            });
        }
        // "twenty to four", "ten past six" without am/pm: a time, not two numbers.
        if self.joins(run.end)
            && matches!(self.low(run.end + 1), "to" | "past")
            && self.joins(run.end + 1)
        {
            if let Some(h) = self.collect(run.end + 2) {
                if let Some((hv, hi)) = parse_card(&h.words) {
                    if !hi.ordinal && (1..=12).contains(&hv) && h.words.len() == 1 {
                        // Both numbers stay as spoken, so a second pass sees the same words.
                        return Some(Step::Skip { end: h.end });
                    }
                }
            }
        }
        // Codes ("invoice 4012"); "number one" stays an idiom.
        if in_list(CODE_PREV, prev) && v >= 10 {
            return Some(Step::Replace {
                text: v.to_string(),
                end: run.end,
                drop_prev: None,
            });
        }
        // Small numbers stay words unless something forces figures.
        if v < 10 {
            let next_forces = self.joins(run.end) && self.low(run.end + 1) == "percentage";
            // "page four", but not "this page one more time".
            let label = in_list(DIGIT_PREV, prev)
                && !(self.joins(run.end)
                    && in_list(
                        &[
                            "more", "thing", "things", "of", "another", "other", "last", "or",
                            "and", "time",
                        ],
                        self.low(run.end + 1),
                    ));
            if label || next_forces || self.street_follows(run.end) {
                return Some(Step::Replace {
                    text: v.to_string(),
                    end: run.end,
                    drop_prev: None,
                });
            }
            return Some(Step::Skip { end: run.end });
        }
        Some(Step::Replace {
            text: num,
            end: run.end,
            drop_prev: None,
        })
    }

    fn matches_seq_after(&self, end: usize, seq: &[&str]) -> bool {
        self.joins(end) && self.matches_seq(end + 1, seq)
    }

    fn year_context(&self, i: usize, end: usize) -> bool {
        let prev = self.prev_low(i);
        in_list(YEAR_PREV, prev)
            || (i > 0 && self.month_at(i - 1).is_some())
            || (self.joins(end) && matches!(self.low(end + 1), "onwards" | "onward"))
    }

    fn street_follows(&self, end: usize) -> bool {
        if !self.joins(end) {
            return false;
        }
        let first = &self.toks[end + 1];
        if !first
            .core
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
        {
            return false;
        }
        (end + 2..=end + 4).any(|k| {
            k < self.toks.len()
                && STREET_TYPES.contains(&self.toks[k].core.as_str())
                && (end + 1..k).all(|j| self.toks[j].trail.is_empty())
        })
    }

    /// Clock times read from one run.
    fn clock_run(&self, i: usize, run: &Run) -> Option<Step> {
        let ws = &run.words;
        // "fourteen hundred hours", "oh nine hundred hours".
        if self.joins(run.end) && self.low(run.end + 1) == "hours" {
            // "fifteen hundred hours of flying" is a duration.
            if !self.ends_ok(run.end + 1) {
                return None;
            }
            let (h, m) = military(ws)?;
            return Some(Step::Replace {
                text: format!("{h:02}:{m:02}"),
                end: run.end + 1,
                drop_prev: None,
            });
        }
        let (h, m) = hour_minute(ws)?;
        let at = in_list(AT_WORDS, self.prev_low(i));
        if let Some((ampm, end)) = self.ampm(run.end) {
            let text = match m {
                Some(m) => format!("{h}:{m:02} {ampm}"),
                None => format!("{h} {ampm}"),
            };
            return Some(Step::Replace {
                text,
                end,
                drop_prev: None,
            });
        }
        if let Some(m) = m {
            // "at four fifty a kilo", "at four thirty five of us": not a time.
            if (at && self.ends_ok(run.end)) || self.day_part_follows(run.end) {
                return Some(Step::Replace {
                    text: format!("{h}:{m:02}"),
                    end: run.end,
                    drop_prev: None,
                });
            }
        }
        None
    }

    fn range_from(&self, i: usize, a: u64, a_info: &CardInfo, end: usize) -> Option<Step> {
        let conj = self.low(end + 1);
        if !self.joins(end) || !matches!(conj, "to" | "or") || !self.joins(end + 1) {
            return None;
        }
        let b_run = self.collect(end + 2)?;
        let (b, b_info) = parse_card(&b_run.words)?;
        if b_info.ordinal || a_info.ordinal {
            return None;
        }
        // Ratio.
        if conj == "to"
            && self.joins(b_run.end)
            && matches!(self.low(b_run.end + 1), "ratio" | "odds")
        {
            return Some(Step::Replace {
                text: format!("{a}:{b}"),
                end: b_run.end,
                drop_prev: None,
            });
        }
        let (q, qend) = self.qty_after(b_run.end)?;
        if matches!(q, Qty::Cents) {
            return None;
        }
        // "five to ten thousand": the scale carries back.
        let mut a = a;
        let b_scale = b_info.ends_on_scale.or_else(|| {
            (b_run.words.last().map(|w| w == "hundred").unwrap_or(false)).then_some(100)
        });
        if let Some(s) = b_scale {
            if !a_info.has_scale && a < b / s {
                a *= s;
            }
        }
        if a >= b {
            return None;
        }
        let _ = i;
        if conj == "or" {
            // "two or three hundred dollars" → "$200 or $300".
            let one = |v: u64| self.fmt_qty(&group_large(v), &q);
            return Some(Step::Replace {
                text: format!("{} or {}", one(a), one(b)),
                end: qend,
                drop_prev: None,
            });
        }
        let text = match q {
            Qty::Money(sym) => format!("{sym}{}–{sym}{}", group_large(a), group_large(b)),
            Qty::Percent => format!("{}–{}%", group_large(a), group_large(b)),
            Qty::Unit(sym, true) => format!("{}–{}{sym}", group_large(a), group_large(b)),
            Qty::Unit(sym, false) => format!("{}–{} {sym}", group_large(a), group_large(b)),
            Qty::Cents => unreachable!(),
        };
        Some(Step::Replace {
            text,
            end: qend,
            drop_prev: None,
        })
    }
}

fn prev_static(w: &str) -> &'static str {
    match w {
        "minus" => "minus",
        "negative" => "negative",
        _ => "",
    }
}

fn is_label(w: &str) -> bool {
    !w.is_empty()
        && w.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// "oh two one double four" → "02144". `None` if any word is not a digit word.
fn digit_string(ws: &[String]) -> Option<String> {
    let mut out = String::new();
    let mut repeat = 1;
    for w in ws {
        match w.as_str() {
            "double" => repeat = 2,
            "triple" => repeat = 3,
            _ => {
                let d = digit_value(w)?;
                for _ in 0..repeat {
                    out.push_str(&d.to_string());
                }
                repeat = 1;
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn group_digits(d: &str) -> String {
    match d.len() {
        6 => format!("{} {}", &d[..3], &d[3..]),
        7 => format!("{} {}", &d[..3], &d[3..]),
        8 => format!("{} {}", &d[..4], &d[4..]),
        _ => d.to_string(),
    }
}

/// NZ phone grouping for a digit string starting with 0.
fn nz_phone(d: &str) -> String {
    if d.starts_with("0800") || d.starts_with("0508") || d.starts_with("0900") {
        return format!("{} {}", &d[..4], group_digits(&d[4..]));
    }
    if d.starts_with("02") && (9..=11).contains(&d.len()) {
        return format!("{} {}", &d[..3], group_digits(&d[3..]));
    }
    if d.len() == 9 && matches!(d.as_bytes()[1], b'3' | b'4' | b'6' | b'7' | b'9') {
        return format!("{} {} {}", &d[..2], &d[2..5], &d[5..]);
    }
    d.to_string()
}

/// "three point one four" → "3.14"; "one point four nine point oh" → "1.49.0"; "one point five
/// million" → ("1.5", Some("million")).
fn decimal_string(ws: &[String], version_ctx: bool) -> Option<(String, Option<String>)> {
    let mut ws: Vec<String> = ws.to_vec();
    let mut scale = None;
    if let Some(last) = ws.last() {
        if matches!(last.as_str(), "million" | "billion") {
            scale = ws.pop();
        }
    }
    let segs: Vec<&[String]> = ws.split(|w| w == "point").collect();
    if segs.len() < 2 || segs.iter().skip(1).any(|s| s.is_empty()) {
        return None;
    }
    if segs.len() > 2 && !version_ctx && segs.len() < 3 {
        return None;
    }
    let left = if segs[0].is_empty() || (segs[0].len() == 1 && digit_value(&segs[0][0]) == Some(0))
    {
        "0".to_string()
    } else {
        let (v, info) = parse_card(segs[0])?;
        if info.ordinal {
            return None;
        }
        v.to_string()
    };
    let mut out = left;
    for seg in &segs[1..] {
        out.push('.');
        if let Some(d) = digit_string(seg) {
            out.push_str(&d);
        } else if version_ctx || segs.len() > 2 {
            let (v, info) = parse_card(seg)?;
            if info.ordinal {
                return None;
            }
            out.push_str(&v.to_string());
        } else {
            return None;
        }
    }
    if scale.is_some() && segs.len() > 2 {
        return None;
    }
    Some((out, scale))
}

/// Hour and optional minute from a run: "three" → (3, None); "three thirty" → (3, Some(30));
/// "eleven oh five" → (11, Some(5)).
fn hour_minute(ws: &[String]) -> Option<(u64, Option<u64>)> {
    let h = lookup(UNITS, &ws[0]).or_else(|| lookup(TEENS, &ws[0]).filter(|v| *v <= 12))?;
    if !(1..=12).contains(&h) {
        return None;
    }
    let rest = &ws[1..];
    if rest.is_empty() {
        return Some((h, None));
    }
    let m = if rest[0] == "oh" {
        if rest.len() != 2 {
            return None;
        }
        lookup(UNITS, &rest[1])?
    } else if rest.len() == 1 {
        lookup(TEENS, &rest[0]).or_else(|| lookup(TENS, &rest[0]))?
    } else if rest.len() == 2 {
        lookup(TENS, &rest[0])? + lookup(UNITS, &rest[1])?
    } else {
        return None;
    };
    if m > 59 {
        return None;
    }
    Some((h, Some(m)))
}

/// "fourteen hundred" → (14, 0); "oh nine hundred" → (9, 0); "fourteen thirty" → (14, 30).
fn military(ws: &[String]) -> Option<(u64, u64)> {
    let (h, rest): (u64, &[String]) = if ws[0] == "oh" {
        (lookup(UNITS, ws.get(1)?)?, &ws[2..])
    } else if ws[0] == "twenty"
        && ws.len() >= 2
        && lookup(UNITS, &ws[1]).map(|v| v <= 3).unwrap_or(false)
    {
        (20 + lookup(UNITS, &ws[1])?, &ws[2..])
    } else {
        (
            lookup(TEENS, &ws[0]).or(if ws[0] == "twenty" { Some(20) } else { None })?,
            &ws[1..],
        )
    };
    if h > 23 {
        return None;
    }
    if rest.len() == 1 && rest[0] == "hundred" {
        return Some((h, 0));
    }
    if ws[0] == "oh" || h < 13 {
        return None;
    }
    let (_, m) = hour_minute(
        &[String::from("one")]
            .iter()
            .cloned()
            .chain(rest.iter().cloned())
            .collect::<Vec<_>>(),
    )?;
    Some((h, m?))
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn nz(s: &str) -> String {
        apply_written_forms(s, &WrittenFormsOptions::default())
    }

    fn us(s: &str) -> String {
        apply_written_forms(
            s,
            &WrittenFormsOptions {
                us_dates: true,
                ..Default::default()
            },
        )
    }

    /// Every pair: (spoken, written). Invented text only.
    fn check(cases: &[(&str, &str)]) {
        let mut bad = Vec::new();
        for (input, want) in cases {
            let got = nz(input);
            if got != *want {
                bad.push(format!("\n  in:   {input}\n  want: {want}\n  got:  {got}"));
            }
        }
        assert!(
            bad.is_empty(),
            "{} case(s) failed:{}",
            bad.len(),
            bad.join("")
        );
    }

    /// Must come back byte-identical.
    fn unchanged(cases: &[&str]) {
        let mut bad = Vec::new();
        for input in cases {
            let got = nz(input);
            if got != *input {
                bad.push(format!("\n  in:  {input}\n  got: {got}"));
            }
        }
        assert!(
            bad.is_empty(),
            "{} case(s) changed:{}",
            bad.len(),
            bad.join("")
        );
    }

    #[test]
    fn english_only() {
        for l in ["en", "en-NZ", "en-US", "en-AU", "en-GB"] {
            assert!(is_english_writing(l), "{l}");
        }
        for l in ["auto", "fr", "es", "mi", "ja", "", "eng"] {
            assert!(!is_english_writing(l), "{l}");
        }
    }

    #[test]
    fn the_screenshot_case() {
        check(&[(
            "you can spend about one thousand two hundred you can end up with a mess",
            "you can spend about 1,200 you can end up with a mess",
        )]);
    }

    #[test]
    fn cardinals() {
        check(&[
            ("We invited twelve people.", "We invited 12 people."),
            ("about forty-two of them", "about 42 of them"),
            ("about forty two of them", "about 42 of them"),
            ("roughly forty five thousand users", "roughly 45,000 users"),
            ("two hundred and five submissions", "205 submissions"),
            ("a hundred people came", "100 people came"),
            ("A thousand people came", "1,000 people came"),
            ("one hundred people", "100 people"),
            ("fifteen hundred people", "1,500 people"),
            (
                "one million two hundred and thirty four thousand rows",
                "1,234,000 rows",
            ),
            ("one million five hundred thousand rows", "1.5 million rows"),
            ("two billion requests", "2 billion requests"),
            (
                "three million two hundred and fifty thousand",
                "3.25 million",
            ),
            ("Twenty people agreed.", "20 people agreed."),
            ("We had ninety nine problems", "We had 99 problems"),
            ("it took eleven weeks", "it took 11 weeks"),
        ]);
    }

    #[test]
    fn small_numbers_stay_words() {
        unchanged(&[
            "We have three options.",
            "one of the best",
            "one of those days",
            "on the one hand",
            "one or two things",
            "a couple of things",
            "a few people",
            "the one and only",
            "one-on-one meeting",
            "I said nine to five.",
            "zero",
            "two three",
        ]);
    }

    #[test]
    fn small_numbers_take_figures_in_context() {
        check(&[
            ("see page four", "see page 4"),
            ("version two is out", "version 2 is out"),
            ("on level three", "on level 3"),
            ("five Wellington Road", "5 Wellington Road"),
            ("forty two Smith Street", "42 Smith Street"),
            ("phase two starts", "phase 2 starts"),
        ]);
    }

    #[test]
    fn ambiguity_guard() {
        unchanged(&[
            "nine hundred three times a day",
            "twelve fifteen",
            "three twenty dollar notes",
            "I've told you a hundred times",
            "a million thanks",
            "forty winks",
            "the eleventh hour",
            "catch twenty two",
            "twenty twenty vision",
            "Number one priority is the release.",
            "It's twenty to four.",
            "It's about forty k from here.",
        ]);
        check(&[("nine hundred and three times a day", "903 times a day")]);
    }

    #[test]
    fn ordinals() {
        check(&[
            ("the twenty first century", "the 21st century"),
            ("his fortieth birthday", "his 40th birthday"),
            ("the hundredth time", "the 100th time"),
            ("in the twenty second round", "in the 22nd round"),
            ("the eleventh floor", "the 11th floor"),
            ("Meet on the twenty third.", "Meet on the 23rd."),
            (
                "We're meeting on the fourteenth.",
                "We're meeting on the 14th.",
            ),
        ]);
        unchanged(&[
            "First, open the file.",
            "he came second",
            "the third option",
            "a thirty second clip",
            "every twentieth person",
        ]);
    }

    #[test]
    fn decimals_and_fractions() {
        check(&[
            ("three point five metres", "3.5 m"),
            ("it rose three point one four", "it rose 3.14"),
            ("zero point two", "0.2"),
            ("nought point two", "0.2"),
            ("one point two five", "1.25"),
            ("two and a half kilometres", "2.5 km"),
            ("two and a half hours", "2.5 hours"),
            ("one and a half million dollars", "$1.5 million"),
            ("one point five million", "1.5 million"),
            ("a fifty fifty split", "a 50/50 split"),
        ]);
        unchanged(&["half the team", "a third of them", "three quarters of us"]);
    }

    #[test]
    fn money() {
        check(&[
            ("it cost twelve dollars fifty", "it cost $12.50"),
            ("it cost twelve dollars and fifty cents", "it cost $12.50"),
            ("five dollars", "$5"),
            ("one dollar", "$1"),
            ("a dollar fifty", "$1.50"),
            ("fifty cents", "50c"),
            ("two million dollars", "$2 million"),
            ("one thousand two hundred dollars", "$1,200"),
            ("fifty US dollars", "US$50"),
            ("twenty Australian dollars", "A$20"),
            ("thirty New Zealand dollars", "NZ$30"),
            ("ten euros", "€10"),
            ("five grand", "$5,000"),
            ("fifty k dollars", "$50,000"),
            ("fifty dollars an hour", "$50 an hour"),
            ("twelve dollars fifty each", "$12.50 each"),
        ]);
        check(&[("a two dollar shop", "a $2 shop")]);
        unchanged(&["it weighs five pounds"]);
    }

    #[test]
    fn no_dollar_sign_is_ever_guessed() {
        check(&[("it costs one thousand two hundred", "it costs 1,200")]);
        unchanged(&["fifty k", "the grand total"]);
    }

    #[test]
    fn percent() {
        check(&[
            ("three point five percent", "3.5%"),
            ("five percent", "5%"),
            ("five per cent", "5%"),
            ("a hundred percent", "100%"),
            ("five to ten percent", "5–10%"),
            ("three percentage points", "3 percentage points"),
        ]);
    }

    #[test]
    fn measures() {
        check(&[
            ("two litres", "2 L"),
            ("five hundred mils", "500 mL"),
            ("twenty degrees", "20°"),
            ("twenty degrees Celsius", "20°C"),
            ("minus three degrees", "-3°"),
            ("a hundred kilometres an hour", "100 km/h"),
            ("three kilos", "3 kg"),
            ("sixteen gigabytes", "16 GB"),
            ("five to ten kilometres", "5–10 km"),
            ("forty five minutes", "45 minutes"),
            ("fifty square metres", "50 m²"),
        ]);
        unchanged(&["five minutes", "it weighs five pounds"]);
    }

    #[test]
    fn dates() {
        check(&[
            ("the twenty first of March", "21 March"),
            (
                "The twenty first of March is a Saturday",
                "21 March is a Saturday",
            ),
            ("on March twenty first", "on 21 March"),
            ("on March the twenty first", "on 21 March"),
            ("on the third of March", "on 3 March"),
            ("twenty one March twenty twenty six", "21 March 2026"),
            (
                "by the fifth of November, twenty twenty six",
                "by 5 November 2026",
            ),
            ("on the first of may", "on the first of may"),
            ("on the first of May", "on 1 May"),
        ]);
        unchanged(&[
            "we may march on",
            "the fifth of the eleventh",
            "Tuesday the third",
        ]);
        assert_eq!(
            us("on March twenty first, twenty twenty six"),
            "on March 21, 2026"
        );
        assert_eq!(us("the twenty first of March"), "March 21");
    }

    #[test]
    fn years() {
        check(&[
            ("in twenty twenty six", "in 2026"),
            ("since nineteen ninety nine", "since 1999"),
            ("in two thousand and five", "in 2005"),
            ("from twenty oh five onwards", "from 2005 onwards"),
            ("the nineteen nineties", "the 1990s"),
            ("FY twenty six", "FY26"),
            ("about two thousand and five people", "about 2,005 people"),
        ]);
        check(&[(
            "under the Privacy Act twenty twenty",
            "under the Privacy Act 2020",
        )]);
        unchanged(&[
            "the nineties",
            "twenty twenty",
            "We have twenty twenty six goals.",
        ]);
    }

    #[test]
    fn times() {
        check(&[
            ("three thirty pm", "3:30 pm"),
            ("at three thirty", "at 3:30"),
            ("meet at nine fifteen", "meet at 9:15"),
            ("ten am", "10 am"),
            ("ten a.m.", "10 am."),
            ("three pm", "3 pm"),
            ("eleven oh five am", "11:05 am"),
            ("half past three in the afternoon", "3:30 in the afternoon"),
            ("at half past three", "at 3:30"),
            ("quarter past nine am", "9:15 am"),
            ("by quarter to five", "by 4:45"),
            ("at twenty past six", "at 6:20"),
            ("fourteen hundred hours", "14:00"),
            ("oh nine hundred hours", "09:00"),
            ("ten o'clock", "10 o'clock"),
        ]);
        unchanged(&[
            "three o'clock",
            "half past",
            "noon",
            "midnight",
            "I am here",
        ]);
        check(&[("two hundred hours of footage", "200 hours of footage")]);
    }

    #[test]
    fn ranges_scores_ratios() {
        check(&[
            ("five to ten thousand dollars", "$5,000–$10,000"),
            ("ten to twenty people", "10 to 20 people"),
            ("three nil", "3–0"),
            ("a three to one ratio", "a 3:1 ratio"),
            ("open twenty four seven", "open 24/7"),
        ]);
    }

    #[test]
    fn phones_and_codes() {
        check(&[
            (
                "call me on oh two one four five six seven eight nine oh",
                "call me on 021 456 7890",
            ),
            ("oh four one two three four five six seven", "04 123 4567"),
            ("oh eight oh oh one two three four five six", "0800 123 456"),
            (
                "plus sixty four two one four five six seven eight nine oh",
                "+64 21 456 7890",
            ),
            ("invoice one two three four", "invoice 1234"),
            ("the code is four four double seven", "the code is 4477"),
            ("dial one one one", "dial 111"),
            ("invoice four thousand and twelve", "invoice 4012"),
        ]);
        unchanged(&["one two three four", "one two three go"]);
    }

    #[test]
    fn versions_and_web() {
        check(&[
            ("version two point one", "version 2.1"),
            ("v two", "v2"),
            ("version one point four nine point oh", "version 1.49.0"),
            ("email nic at kyt dot nz", "email nic@kyt.nz"),
            (
                "my address is nic at kyt dot co dot nz",
                "my address is nic@kyt.co.nz",
            ),
            ("see kyt dot nz slash insights", "see kyt.nz/insights"),
            ("go to w w w dot kyt dot nz", "go to www.kyt.nz"),
        ]);
        unchanged(&["look at the dot com boom", "the dot product"]);
    }

    #[test]
    fn never_touches() {
        unchanged(&[
            "tahi, rua, toru, whā",
            "Kia ora koutou",
            "we spent $1,200 on 3 things",
            "it was 2026",
            "",
            "   ",
        ]);
        let opts = WrittenFormsOptions {
            protected: vec!["Nine Inch Nails".into()],
            ..Default::default()
        };
        assert_eq!(
            apply_written_forms("I like Nine Inch Nails a lot", &opts),
            "I like Nine Inch Nails a lot"
        );
    }

    /// Every defect the independent review reproduced on 22 Nov 2026, kept as a regression set.
    #[test]
    fn review_findings_stay_fixed() {
        // Critical 1: no panic on an all-zero phone number.
        unchanged(&[
            "plus sixty four oh oh oh oh oh oh oh",
            "plus sixty four zero zero zero zero zero zero zero",
        ]);
        // Critical 2: a month followed by a count is not a date.
        unchanged(&[
            "In August two people resigned.",
            "In June one of the directors left.",
            "In March the three of us went to Rotorua.",
            "Back in May the first thing we did was hire.",
            "May one of you send me the notes?",
            "May the fourth be with you.",
        ]);
        check(&[
            (
                "In June two thousand people marched on Parliament.",
                "In June 2,000 people marched on Parliament.",
            ),
            (
                "On March fifth, two thousand people came.",
                "On 5 March, 2,000 people came.",
            ),
            (
                "on the eleventh of March the three of us",
                "on 11 March the three of us",
            ),
        ]);
        // Critical 3: clock times after by/before/from/until.
        check(&[
            (
                "Can we meet before twelve thirty?",
                "Can we meet before 12:30?",
            ),
            ("by eleven thirty tomorrow", "by 11:30 tomorrow"),
            (
                "from eleven fifteen until twelve forty five",
                "from 11:15 until 12:45",
            ),
            ("by two thirty", "by 2:30"),
        ]);
        // Critical 4: doses and idioms with "point".
        check(&[(
            "Give her point five mils of paracetamol.",
            "Give her 0.5 mL of paracetamol.",
        )]);
        unchanged(&[
            "At one point two of them walked out.",
            "at some point we should",
        ]);
        // Critical 5: no invented $ or units.
        unchanged(&[
            "They won three grand slams.",
            "two grand finals in a row",
            "I've got two degrees in law.",
            "a two degree difference",
            "We had three gigs last weekend.",
        ]);
        check(&[
            (
                "twelve hundred hours of community service",
                "1,200 hours of community service",
            ),
            ("fifteen hundred hours of flying", "1,500 hours of flying"),
            ("It's twenty degrees today.", "It's 20° today."),
            ("it pays five grand a month", "it pays $5,000 a month"),
        ]);
        // Critical 6: other changes of meaning.
        check(&[
            (
                "It's about two or three hundred dollars.",
                "It's about $200 or $300.",
            ),
            ("Total minus five dollars.", "Total minus $5."),
            (
                "They're selling at four fifty a kilo.",
                "They're selling at four fifty a kilo.",
            ),
            ("Python three point twelve.", "Python 3.12."),
        ]);
        unchanged(&[
            "I'll be there in twenty thirty minutes.",
            "then at four thirty five of us left",
            "Look at three thirty people waiting.",
            "Run the thirty second ad",
            "The ninety second rule.",
            "It was five, six hundred dollars.",
        ]);
        // Major 7: counts after year words keep their comma.
        check(&[
            (
                "A crowd of twelve hundred turned up.",
                "A crowd of 1,200 turned up.",
            ),
            (
                "Since twelve hundred people signed",
                "Since 1,200 people signed",
            ),
            ("By March fifteen hundred staff", "By March 1,500 staff"),
            ("Order twelve thousand units.", "Order 12,000 units."),
        ]);
        // Major 8: ordinary nouns before small numbers.
        unchanged(&[
            "At this stage one thing is clear.",
            "This season two players were injured.",
            "We need to build one more house.",
            "on the table one more time",
        ]);
        // Major 9: countdowns and mic checks.
        unchanged(&[
            "Five four three two one, lift off.",
            "One two one two, testing.",
        ]);
        // Second round (22 Nov): qualifications, lap times, years mid-phrase.
        unchanged(&[
            "She has two degrees and a diploma.",
            "He's running around three forty this lap.",
        ]);
        check(&[(
            "In two thousand twenty five revenue grew.",
            "In 2025 revenue grew.",
        )]);
        // Major 10: half-converted money.
        check(&[("It cost two dollars and fifty.", "It cost $2.50.")]);
        // Protected words are never dropped from before a number.
        let opts = WrittenFormsOptions {
            protected: vec!["minus".into()],
            ..Default::default()
        };
        assert_eq!(
            apply_written_forms("minus three degrees", &opts),
            "minus 3°"
        );
    }

    #[test]
    fn keeps_spacing_and_punctuation() {
        check(&[
            ("(twelve people)", "(12 people)"),
            ("\"twelve\" people", "\"12\" people"),
            ("twelve,  thirteen", "12,  13"),
            ("Twelve people.\nThirteen more.", "12 people.\n13 more."),
            ("it was twelve dollars fifty.", "it was $12.50."),
        ]);
    }

    #[test]
    fn idempotent() {
        for input in [
            "it cost twelve dollars fifty on the twenty first of March twenty twenty six at three thirty pm",
            "call oh two one four five six seven eight nine oh about one thousand two hundred",
            "five to ten percent of two and a half kilometres",
        ] {
            let once = nz(input);
            assert_eq!(nz(&once), once, "second pass changed: {once}");
        }
    }

    #[test]
    fn fast_enough() {
        let text = "We spent about one thousand two hundred dollars on the twenty first of March, \
                    met at three thirty pm, and called oh two one four five six seven eight nine oh. "
            .repeat(4);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = nz(&text);
        }
        let per = start.elapsed() / 100;
        // Generous for a debug build on a slow CI runner; release is far quicker.
        assert!(per.as_millis() < 20, "took {per:?} per call");
    }
}
