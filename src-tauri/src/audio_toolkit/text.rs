use natural::phonetics::soundex;
use once_cell::sync::Lazy;
use regex::Regex;
use strsim::levenshtein;

/// Builds an n-gram string by cleaning and concatenating words
///
/// Strips punctuation from each word, lowercases, and joins without spaces.
/// This allows matching "Charge B" against "ChargeBee".
fn build_ngram(words: &[&str]) -> String {
    words
        .iter()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect::<Vec<_>>()
        .concat()
}

/// Korero (v1.27.0): curated ordinary-English vocabulary for the false-positive
/// veto in `apply_custom_words`.
///
/// Sorted ASCII-lowercase, queried with `binary_search` -- no new dependency and
/// no allocation. It holds the highest-frequency English words, every common
/// function word, and the apostrophe-free contraction forms that survive
/// punctuation stripping ("isnt", "dont", "thats").
///
/// Two deliberate exclusions:
///   * te reo Maori homographs (ana, ate, kite, mate, pai, ora, aroha, hui, iwi,
///     mana, roa, tapu, wai, whare, mahi, ...) -- listing them would veto te reo
///     repair, which is the main reason the custom-words feature exists here.
///   * bare initials and initialisms (b, q, id, iq, ai) -- their PRESENCE in an
///     utterance is the evidence that the speaker spelled out a name
///     ("Charge B", "project I Q"), so they must never count as ordinary English.
static COMMON_EN: &[&str] = &[
    "a", "able", "about", "above", "accept", "access", "according", "account", "across", "act",
    "action", "active", "activity", "actual", "actually", "add", "added", "addition",
    "additional", "address", "after", "afternoon", "again", "against", "age", "ago", "agree",
    "agreed", "ahead", "all", "allow", "allowed", "almost", "alone", "along", "already", "also",
    "although", "always", "among", "amount", "an", "and", "another", "answer", "any", "anyone",
    "anything", "apart", "apply", "approach", "approved", "april", "are", "area", "areas",
    "arent", "around", "arrange", "as", "ask", "asked", "assume", "at", "attach", "attend",
    "august", "author", "available", "avoid", "away", "back", "backlog", "bad", "balance",
    "bank", "bar", "base", "based", "basic", "basis", "be", "became", "because", "become",
    "been", "before", "begin", "beginning", "behalf", "behind", "being", "believe", "below",
    "benefit", "best", "better", "between", "beyond", "big", "bill", "billing", "bit", "block",
    "board", "body", "book", "both", "bottom", "box", "brand", "break", "brief", "bring",
    "brought", "budget", "build", "building", "built", "business", "but", "buy", "by", "call",
    "called", "came", "can", "cannot", "cant", "capacity", "capital", "car", "card", "care",
    "carry", "case", "cases", "cash", "catch", "cause", "centre", "certain", "chain", "chair",
    "challenge", "chance", "change", "changed", "changes", "channel", "chapter", "charge",
    "charges", "chart", "chat", "check", "checked", "child", "children", "choice", "choose",
    "chose", "city", "claim", "class", "clean", "clear", "clearly", "click", "client",
    "clients", "close", "closed", "cloud", "club", "code", "coffee", "cold", "colour", "come",
    "comes", "coming", "comment", "comments", "commit", "committee", "common", "company",
    "compare", "complete", "completed", "complex", "concern", "condition", "confirm", "connect",
    "consider", "contact", "contain", "content", "context", "continue", "contract", "control",
    "conversation", "copy", "core", "cork", "corner", "correct", "cost", "costs", "could",
    "couldnt", "council", "count", "country", "couple", "course", "court", "cover", "covered",
    "create", "created", "credit", "critical", "current", "currently", "customer", "customers",
    "cut", "cycle", "daily", "damage", "data", "date", "day", "days", "deal", "dealt",
    "december", "decide", "decision", "deep", "default", "defer", "definitely", "degree",
    "delay", "deliver", "delivery", "demand", "department", "depend", "describe", "design",
    "desk", "detail", "details", "develop", "device", "did", "didnt", "diet", "difference",
    "different", "difficult", "digital", "direct", "direction", "director", "discuss",
    "discussion", "display", "do", "document", "does", "doesnt", "doing", "dollar", "domain",
    "done", "dont", "door", "double", "down", "download", "draft", "drive", "driver", "drop",
    "dry", "due", "during", "duty", "each", "earlier", "early", "easier", "easy", "economic",
    "edge", "edit", "effect", "effective", "effort", "either", "element", "else", "email",
    "employee", "end", "energy", "engage", "engine", "english", "enough", "ensure", "enter",
    "entire", "entry", "equal", "equipment", "error", "escalate", "especially", "essential",
    "estimate", "even", "evening", "event", "ever", "every", "everyone", "everything",
    "evidence", "exact", "example", "excel", "except", "exchange", "execute", "exercise",
    "exist", "expect", "expense", "experience", "expert", "explain", "export", "extend",
    "extra", "face", "fact", "factor", "fail", "failed", "fair", "fall", "false", "family",
    "far", "fast", "father", "fault", "favour", "feature", "february", "feed", "feel", "few",
    "field", "figure", "file", "fill", "film", "final", "finally", "finance", "financial",
    "find", "fine", "finish", "fire", "firm", "first", "fiscal", "fit", "five", "fix", "fixed",
    "flag", "flat", "flight", "floor", "flow", "focus", "folder", "follow", "following", "food",
    "foot", "for", "force", "forecast", "form", "formal", "format", "forward", "found", "four",
    "frame", "free", "freight", "friday", "friend", "from", "front", "full", "function", "fund",
    "funding", "further", "future", "gain", "game", "gap", "gas", "gather", "gave", "general",
    "get", "give", "given", "glass", "global", "go", "goal", "goes", "going", "gold", "gone",
    "good", "got", "government", "grant", "graph", "great", "green", "grid", "ground", "group",
    "grow", "growth", "guard", "guess", "guest", "guide", "guy", "had", "hadnt", "half", "hand",
    "handle", "happen", "happy", "hard", "has", "hasnt", "have", "havent", "having", "he",
    "head", "health", "hear", "heard", "heart", "heat", "held", "help", "her", "here", "hero",
    "high", "hill", "him", "himself", "hire", "his", "history", "hit", "hold", "holiday",
    "home", "hope", "horse", "host", "hot", "hotel", "hour", "house", "how", "however", "huge",
    "human", "hundred", "i", "idea", "if", "im", "image", "impact", "implement", "import",
    "important", "improve", "in", "inbox", "include", "included", "income", "increase",
    "indeed", "index", "indicate", "industry", "information", "initial", "input", "inside",
    "insight", "instead", "insurance", "intend", "interest", "internal", "into", "introduce",
    "invoice", "involve", "is", "isnt", "issue", "issues", "it", "item", "items", "its",
    "itself", "ive", "january", "job", "join", "joint", "journey", "judge", "july", "jump",
    "june", "just", "keep", "kept", "key", "kick", "kid", "kill", "kind", "king", "kitchen",
    "knew", "know", "known", "label", "labour", "lack", "land", "language", "large", "last",
    "late", "later", "latest", "launch", "law", "lay", "lead", "leader", "leading", "learn",
    "least", "leave", "led", "left", "legal", "length", "less", "lesson", "let", "lets",
    "letter", "level", "licence", "lie", "life", "light", "like", "likely", "limit", "limited",
    "line", "link", "list", "listen", "little", "live", "load", "loan", "local", "location",
    "lock", "log", "logic", "long", "look", "loop", "lose", "loss", "lost", "lot", "love",
    "low", "lower", "lunch", "machine", "made", "mail", "main", "maintain", "major", "make",
    "making", "manage", "management", "manager", "many", "map", "march", "margin", "mark",
    "market", "master", "match", "material", "matter", "maybe", "mean", "means", "meant",
    "measure", "media", "medical", "medium", "meet", "meeting", "meetings", "member", "memory",
    "mention", "menu", "message", "met", "method", "middle", "might", "mile", "milestone",
    "military", "milk", "million", "mind", "mine", "minute", "miss", "missing", "mission",
    "mistake", "mix", "mobile", "model", "modern", "moment", "monday", "money", "monitor",
    "month", "months", "more", "morning", "most", "mother", "motion", "move", "movement",
    "movie", "much", "must", "my", "name", "national", "nature", "near", "nearly", "necessary",
    "need", "needs", "network", "never", "new", "news", "next", "nice", "night", "nine", "no",
    "node", "none", "nor", "normal", "north", "not", "note", "notes", "nothing", "notice",
    "november", "now", "number", "object", "objective", "obvious", "occur", "october", "of",
    "off", "offer", "office", "officer", "official", "often", "oil", "ok", "okay", "old", "on",
    "once", "one", "online", "only", "onto", "open", "operate", "operation", "opinion",
    "opportunity", "option", "or", "order", "organisation", "other", "others", "our", "out",
    "outcome", "outline", "output", "outside", "over", "overall", "owner", "pace", "pack",
    "package", "page", "paid", "pain", "paint", "pair", "panel", "paper", "parent", "park",
    "part", "particular", "partner", "party", "pass", "past", "path", "patient", "pattern",
    "pay", "payment", "peace", "people", "per", "percent", "perfect", "perform", "performance",
    "perhaps", "period", "permit", "person", "personal", "phase", "phone", "photo", "physical",
    "pick", "picture", "piece", "pilot", "pipeline", "place", "plain", "plan", "planning",
    "plans", "plant", "platform", "play", "please", "plus", "pocket", "point", "policy",
    "political", "poor", "pop", "popular", "port", "portal", "position", "positive", "possible",
    "post", "potential", "pound", "power", "practical", "practice", "prepare", "present",
    "president", "press", "pressure", "pretty", "prevent", "previous", "price", "primary",
    "print", "prior", "priority", "private", "probably", "problem", "procedure", "process",
    "produce", "product", "production", "professional", "profile", "profit", "program",
    "progress", "project", "projects", "promise", "proper", "property", "proposal", "propose",
    "protect", "provide", "provider", "public", "pull", "purchase", "purpose", "push", "put",
    "quality", "quarter", "question", "queue", "quick", "quickly", "quiet", "quite", "quote",
    "race", "radio", "raise", "range", "rate", "rather", "reach", "read", "ready", "real",
    "really", "reason", "receive", "recent", "recently", "record", "recover", "reduce", "refer",
    "reference", "reflect", "regard", "region", "register", "regular", "relate", "relationship",
    "release", "relevant", "remain", "remember", "remove", "render", "rent", "repair", "repeat",
    "replace", "reply", "report", "reports", "represent", "request", "require", "research",
    "reserve", "resource", "respect", "respond", "response", "responsible", "rest", "result",
    "results", "retail", "return", "revenue", "review", "revise", "reward", "rich", "ride",
    "right", "ring", "rise", "risk", "river", "road", "role", "roll", "room", "root", "round",
    "route", "row", "rule", "run", "running", "safe", "safety", "said", "sale", "sales", "same",
    "sample", "save", "saw", "say", "scale", "scene", "schedule", "scheme", "school", "science",
    "scope", "score", "screen", "search", "season", "seat", "second", "secret", "section",
    "sector", "secure", "security", "see", "seek", "seem", "seen", "select", "sell", "send",
    "senior", "sense", "sent", "sentence", "separate", "september", "series", "serious",
    "serve", "service", "session", "set", "setting", "settle", "seven", "several", "shall",
    "shape", "share", "sharp", "she", "sheet", "shift", "ship", "shop", "short", "should",
    "shouldnt", "show", "shown", "side", "sign", "signal", "significant", "similar", "simple",
    "simply", "since", "single", "sir", "sister", "sit", "site", "situation", "six", "size",
    "skill", "skip", "sky", "sleep", "slide", "slight", "slow", "small", "smart", "smile",
    "snow", "so", "social", "soft", "software", "solution", "solve", "some", "someone",
    "something", "sometimes", "son", "song", "soon", "sorry", "sort", "sound", "source",
    "south", "space", "speak", "special", "specific", "speech", "speed", "spend", "spent",
    "split", "sport", "spot", "spread", "spring", "staff", "stage", "stand", "standard", "star",
    "start", "state", "statement", "station", "status", "stay", "step", "stick", "still",
    "stock", "stop", "store", "story", "straight", "strategy", "street", "stress", "strong",
    "structure", "student", "study", "stuff", "style", "subject", "submit", "success", "such",
    "sudden", "suggest", "suitable", "summary", "summer", "sun", "supply", "support", "suppose",
    "sure", "surface", "survey", "system", "table", "take", "taken", "talk", "target", "task",
    "tasks", "tax", "teach", "team", "teams", "tech", "technical", "technology", "tell", "ten",
    "term", "terms", "test", "text", "than", "thank", "that", "thats", "the", "their", "them",
    "theme", "themselves", "then", "theory", "there", "therefore", "theres", "these", "they",
    "theyre", "thing", "things", "think", "third", "this", "those", "though", "thought",
    "thousand", "three", "through", "throughout", "thus", "ticket", "tie", "time", "times",
    "tiny", "title", "to", "today", "together", "token", "told", "tomorrow", "tone", "tonight",
    "too", "took", "tool", "tools", "top", "topic", "total", "touch", "tour", "toward", "town",
    "track", "trade", "traffic", "train", "training", "transfer", "transport", "travel",
    "treat", "tree", "trend", "trial", "trip", "trouble", "true", "trust", "truth", "try",
    "turn", "twenty", "twice", "two", "type", "typical", "under", "understand", "union", "unit",
    "unless", "until", "up", "update", "upon", "upper", "us", "use", "used", "useful", "user",
    "users", "using", "usual", "usually", "value", "van", "variety", "various", "vendor",
    "version", "very", "via", "video", "view", "views", "village", "visit", "voice", "volume",
    "vote", "wait", "walk", "wall", "want", "war", "warm", "warning", "was", "wasnt", "watch",
    "water", "wave", "way", "we", "wear", "web", "website", "week", "weekly", "weeks", "weight",
    "welcome", "well", "went", "were", "werent", "west", "weve", "what", "whatever", "whats",
    "when", "where", "whether", "which", "while", "white", "who", "whole", "whom", "whose",
    "why", "wide", "wife", "will", "win", "wind", "window", "wine", "winter", "wire", "wise",
    "wish", "with", "within", "without", "woman", "women", "wonder", "wont", "wood", "word",
    "words", "work", "worked", "worker", "working", "works", "world", "worry", "worth", "would",
    "wouldnt", "write", "writing", "written", "wrong", "yard", "year", "years", "yes",
    "yesterday", "yet", "you", "young", "your", "youre", "yourself", "youve",
];

/// Case- and punctuation-insensitive lookup key for one spoken token.
/// Mirrors `build_ngram`'s cleaning for a single word.
fn build_match_key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// True when `key` is an ordinary English word.
fn is_common_en(key: &str) -> bool {
    COMMON_EN.binary_search(&key).is_ok()
}

/// True when EVERY token of the n-gram is an ordinary English word -- i.e. what
/// the speaker said is already valid English and needs no "correction".
///
/// Note the direction: this asks whether the SPOKEN tokens are English, never
/// whether the custom word is. That keeps upstream's fuzzy recovery working
/// (`helo` -> `hello`, `wrold` -> `world`), because "helo" and "wrold" are not
/// English words.
fn is_ordinary_english_phrase(words: &[&str]) -> bool {
    words.iter().all(|w| {
        let k = build_match_key(w);
        !k.is_empty() && is_common_en(&k)
    })
}

/// Finds the best matching custom word for a candidate string
///
/// Uses Levenshtein distance and Soundex phonetic matching to find
/// the best match above the given threshold.
///
/// # Arguments
/// * `candidate` - The cleaned/lowercased candidate string to match
/// * `custom_words` - Original custom words (for returning the replacement)
/// * `custom_words_nospace` - Custom words with spaces removed, lowercased (for comparison)
/// * `threshold` - Maximum similarity score to accept
///
/// # Returns
/// The best matching custom word and its score, if any match was found
fn find_best_match<'a>(
    candidate: &str,
    custom_words: &'a [String],
    custom_words_nospace: &[String],
    threshold: f64,
) -> Option<(&'a String, f64)> {
    if candidate.is_empty() || candidate.len() > 50 {
        return None;
    }

    let mut best_match: Option<&String> = None;
    let mut best_score = f64::MAX;

    for (i, custom_word_nospace) in custom_words_nospace.iter().enumerate() {
        // Skip if lengths are too different (optimization + prevents over-matching)
        // Use percentage-based check: max 25% length difference (prevents n-grams from
        // matching significantly shorter custom words, e.g., "openaigpt" vs "openai")
        let len_diff = (candidate.len() as i32 - custom_word_nospace.len() as i32).abs() as f64;
        let max_len = candidate.len().max(custom_word_nospace.len()) as f64;
        let max_allowed_diff = (max_len * 0.25).max(2.0); // At least 2 chars difference allowed
        if len_diff > max_allowed_diff {
            continue;
        }

        // Korero (v1.27.0): the v1.15.1 prefix-extension guard used to sit here
        // and has been REMOVED. It required a strict prefix relationship with a
        // length difference of >= 2, so it protected only the bare word
        // ("project") while missing "projects" and every n-gram concatenation
        // ("projectis" is distance ONE from "projectiq"), and it simultaneously
        // blocked the legitimate abbreviation path "chargeb" -> ChargeBee. The
        // ordinary-English veto in apply_custom_words replaces it.

        // Calculate Levenshtein distance (normalized by length)
        let levenshtein_dist = levenshtein(candidate, custom_word_nospace);
        let max_len = candidate.len().max(custom_word_nospace.len()) as f64;
        let levenshtein_score = if max_len > 0.0 {
            levenshtein_dist as f64 / max_len
        } else {
            1.0
        };

        // Calculate phonetic similarity using Soundex
        //
        // Korero (v1.27.0): length-aware phonetic gate. Soundex emits only four
        // symbols (initial + 3 digits), so past a short word it stops discriminating
        // entirely -- "project", "projects", "projectiq" and "projectison" ALL encode
        // to P622. The v1.3.0 normalised floor cannot express that, because a
        // normalised score scales with length: at candidate length 11 a score of 0.40
        // is FOUR raw edits. Require an ABSOLUTE Levenshtein distance of <= 2 before
        // the phonetic match is allowed to earn the 0.3x discount below. This kills
        // the n-gram absorption cases ("projectison" = 3 edits, "projectinthe" = 4)
        // while keeping exact matches (distance 0, the macron-restoration path) and
        // genuine 1-2 character near-misses.
        //
        // Deliberately gates `phonetic_match` rather than `combined_score` so the
        // v1.3.0 floor patch's Replace text stays byte-identical -- C:\dev\korero is
        // never reset from upstream, and that entry's idempotency check is a plain
        // "is my Replace already in the file" substring test.
        const MAX_PHONETIC_EDITS: usize = 2;
        let phonetic_match =
            soundex(candidate, custom_word_nospace) && levenshtein_dist <= MAX_PHONETIC_EDITS;

        // Combine scores: favor phonetic matches, but also consider string similarity
        // Kōrero (v1.3.0): Levenshtein floor — only apply the 0.3× phonetic boost
        // when words are already close (score ≤ 0.40). Above this threshold the words
        // are too different to be the same word said differently; coincidental Soundex
        // matches (e.g. "have"/hapū → H100; "when i say"/WhānauOS → W520) no longer
        // produce false substitutions.
        let combined_score = if phonetic_match && levenshtein_score <= 0.40 {
            levenshtein_score * 0.3 // Give significant boost to phonetic matches
        } else {
            levenshtein_score
        };

        // Accept if the score is good enough (configurable threshold)
        if combined_score < threshold && combined_score < best_score {
            best_match = Some(&custom_words[i]);
            best_score = combined_score;
        }
    }

    best_match.map(|m| (m, best_score))
}

/// Applies custom word corrections to transcribed text using fuzzy matching
///
/// This function corrects words in the input text by finding the best matches
/// from a list of custom words using a combination of:
/// - Levenshtein distance for string similarity
/// - Soundex phonetic matching for pronunciation similarity
/// - N-gram matching for multi-word speech artifacts (e.g., "Charge B" -> "ChargeBee")
///
/// # Arguments
/// * `text` - The input text to correct
/// * `custom_words` - List of custom words to match against
/// * `threshold` - Maximum similarity score to accept (0.0 = exact match, 1.0 = any match)
///
/// # Returns
/// The corrected text with custom words applied
pub fn apply_custom_words(text: &str, custom_words: &[String], threshold: f64) -> String {
    if custom_words.is_empty() {
        return text.to_string();
    }

    // Pre-compute lowercase versions to avoid repeated allocations
    let custom_words_lower: Vec<String> = custom_words.iter().map(|w| w.to_lowercase()).collect();

    // Pre-compute versions with spaces removed for n-gram comparison
    // Kōrero (v1.3.0): strip Māori macrons from custom words before comparison.
    // Whisper outputs macron-free ASCII ("hapu"); without this, the custom word
    // "hapū" would have Levenshtein distance 1 (u≠ū), score 0.25 > threshold 0.18,
    // so the correct substitution never fires. Nested fn is fine in Rust item scope.
    fn strip_macrons(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                'ā' | 'Ā' => 'a',
                'ē' | 'Ē' => 'e',
                'ī' | 'Ī' => 'i',
                'ō' | 'Ō' => 'o',
                'ū' | 'Ū' => 'u',
                _ => c,
            })
            .collect()
    }

    let custom_words_nospace: Vec<String> = custom_words_lower
        .iter()
        .map(|w| strip_macrons(&w.replace(' ', "")))
        .collect();

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let mut matched = false;

        // Try n-grams from longest (3) to shortest (1) - greedy matching
        for n in (1..=3).rev() {
            if i + n > words.len() {
                continue;
            }

            let ngram_words = &words[i..i + n];
            let ngram = build_ngram(ngram_words);

            if let Some((replacement, score)) =
                find_best_match(&ngram, custom_words, &custom_words_nospace, threshold)
            {
                // Korero (v1.27.0): ordinary-English veto (false-positive guard).
                //
                // An EXACT match (score 0.0) ALWAYS applies -- that is the te reo /
                // macron / casing path ("whanau" -> "whanau" with macrons is distance
                // 0 after the v1.3.0 strip_macrons patch) and it must not regress.
                //
                // An INEXACT match is only ever a mis-transcription repair, so it must
                // never overwrite text that is already valid English.
                if score > 0.0 {
                    // (a) The whole utterance is ordinary English, so the speaker
                    //     meant it: "project", "projects", "project is", "guest",
                    //     "cork". Nothing to repair.
                    if is_ordinary_english_phrase(ngram_words) {
                        continue;
                    }
                    // (b) A multi-token n-gram must never SWALLOW a trailing ordinary
                    //     English word on an inexact match, because that silently
                    //     DELETES what the speaker said. "Charge B is" falls back to
                    //     the 2-gram "Charge B" -> ChargeBee, and "is" survives.
                    if n >= 2 && is_common_en(&build_match_key(ngram_words[n - 1])) {
                        continue;
                    }
                }

                // Extract punctuation from first and last words of the n-gram
                let (prefix, _) = extract_punctuation(ngram_words[0]);
                let (_, suffix) = extract_punctuation(ngram_words[n - 1]);

                // Preserve case from first word
                let corrected = preserve_case_pattern(ngram_words[0], replacement);

                result.push(format!("{}{}{}", prefix, corrected, suffix));
                i += n;
                matched = true;
                break;
            }
        }

        if !matched {
            result.push(words[i].to_string());
            i += 1;
        }
    }

    result.join(" ")
}

/// Preserves the case pattern of the original word when applying a replacement
fn preserve_case_pattern(original: &str, replacement: &str) -> String {
    if original.chars().all(|c| c.is_uppercase()) {
        replacement.to_uppercase()
    } else if original.chars().next().map_or(false, |c| c.is_uppercase()) {
        let mut chars: Vec<char> = replacement.chars().collect();
        if let Some(first_char) = chars.get_mut(0) {
            *first_char = first_char.to_uppercase().next().unwrap_or(*first_char);
        }
        chars.into_iter().collect()
    } else {
        replacement.to_string()
    }
}

/// Extracts punctuation prefix and suffix from a word
fn extract_punctuation(word: &str) -> (&str, &str) {
    let prefix_end = word.chars().take_while(|c| !c.is_alphanumeric()).count();
    let suffix_start = word
        .char_indices()
        .rev()
        .take_while(|(_, c)| !c.is_alphanumeric())
        .count();

    let prefix = if prefix_end > 0 {
        &word[..prefix_end]
    } else {
        ""
    };

    let suffix = if suffix_start > 0 {
        &word[word.len() - suffix_start..]
    } else {
        ""
    };

    (prefix, suffix)
}

/// Returns filler words appropriate for the given language code.
///
/// Some words like "um" and "ha" are real words in certain languages
/// (e.g., Portuguese "um" = "a/an", Spanish "ha" = "has"), so we only
/// include them as fillers for languages where they are truly fillers.
fn get_filler_words_for_language(lang: &str) -> &'static [&'static str] {
    let base_lang = lang.split(&['-', '_'][..]).next().unwrap_or(lang);

    match base_lang {
        "en" => &[
            "uh", "um", "uhm", "umm", "uhh", "uhhh", "ah", "hmm", "hm", "mmm", "mm", "mh", "eh",
            "ehh", "ha",
        ],
        "es" => &["ehm", "mmm", "hmm", "hm"],
        "pt" => &["ahm", "hmm", "mmm", "hm"],
        "fr" => &["euh", "hmm", "hm", "mmm"],
        "de" => &["äh", "ähm", "hmm", "hm", "mmm"],
        "it" => &["ehm", "hmm", "mmm", "hm"],
        "cs" => &["ehm", "hmm", "mmm", "hm"],
        "pl" => &["hmm", "mmm", "hm"],
        "tr" => &["hmm", "mmm", "hm"],
        "ru" => &["хм", "ммм", "hmm", "mmm"],
        "uk" => &["хм", "ммм", "hmm", "mmm"],
        "ar" => &["hmm", "mmm"],
        "ja" => &["hmm", "mmm"],
        "ko" => &["hmm", "mmm"],
        "vi" => &["hmm", "mmm", "hm"],
        "zh" => &["hmm", "mmm"],
        // Conservative universal fallback (no "um", "eh", "ha")
        _ => &[
            "uh", "uhm", "umm", "uhh", "uhhh", "ah", "hmm", "hm", "mmm", "mm", "mh", "ehh",
        ],
    }
}

static MULTI_SPACE_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s{2,}").unwrap());

/// Collapses repeated words (3+ repetitions) to a single instance.
/// E.g., "wh wh wh wh" -> "wh", "I I I I" -> "I"
fn collapse_stutters(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let word = words[i];
        let word_lower = word.to_lowercase();

        if word_lower.chars().all(|c| c.is_alphabetic()) {
            // Count consecutive repetitions (case-insensitive)
            let mut count = 1;
            while i + count < words.len() && words[i + count].to_lowercase() == word_lower {
                count += 1;
            }

            // If 3+ repetitions, collapse to single instance
            if count >= 3 {
                result.push(word);
                i += count;
            } else {
                result.push(word);
                i += 1;
            }
        } else {
            result.push(word);
            i += 1;
        }
    }

    result.join(" ")
}

/// Filters transcription output by removing filler words and stutter artifacts.
///
/// This function cleans up raw transcription text by:
/// 1. Removing filler words based on the app language (or custom list)
/// 2. Collapsing repeated word stutters (e.g., "wh wh wh" -> "wh")
/// 3. Cleaning up excess whitespace
///
/// # Arguments
/// * `text` - The raw transcription text to filter
/// * `lang` - The app language code (e.g., "en", "pt-BR") used to select filler words
/// * `custom_filler_words` - Optional user-provided filler word list. `Some(vec)` overrides
///   language defaults; `Some(empty vec)` disables filtering; `None` uses language defaults.
///
/// # Returns
/// The filtered text with filler words and stutters removed
pub fn filter_transcription_output(
    text: &str,
    lang: &str,
    custom_filler_words: &Option<Vec<String>>,
) -> String {
    let mut filtered = text.to_string();

    // Build filler patterns from custom list or language defaults
    let patterns: Vec<Regex> = match custom_filler_words {
        Some(words) => words
            .iter()
            .filter_map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).ok())
            .collect(),
        None => get_filler_words_for_language(lang)
            .iter()
            .map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).unwrap())
            .collect(),
    };

    // Remove filler words
    for pattern in &patterns {
        filtered = pattern.replace_all(&filtered, "").to_string();
    }

    // Collapse repeated 1-2 letter words (stutter artifacts like "wh wh wh wh")
    filtered = collapse_stutters(&filtered);

    // Clean up multiple spaces to single space
    filtered = MULTI_SPACE_PATTERN.replace_all(&filtered, " ").to_string();

    // Trim leading/trailing whitespace
    filtered.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_custom_words_exact_match() {
        let text = "hello world";
        let custom_words = vec!["Hello".to_string(), "World".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_apply_custom_words_fuzzy_match() {
        let text = "helo wrold";
        let custom_words = vec!["hello".to_string(), "world".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_preserve_case_pattern() {
        assert_eq!(preserve_case_pattern("HELLO", "world"), "WORLD");
        assert_eq!(preserve_case_pattern("Hello", "world"), "World");
        assert_eq!(preserve_case_pattern("hello", "WORLD"), "WORLD");
    }

    #[test]
    fn test_extract_punctuation() {
        assert_eq!(extract_punctuation("hello"), ("", ""));
        assert_eq!(extract_punctuation("!hello?"), ("!", "?"));
        assert_eq!(extract_punctuation("...hello..."), ("...", "..."));
    }

    #[test]
    fn test_empty_custom_words() {
        let text = "hello world";
        let custom_words = vec![];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_filter_filler_words() {
        let text = "So uhm I was thinking uh about this";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "So I was thinking about this");
    }

    #[test]
    fn test_filter_filler_words_case_insensitive() {
        let text = "UHM this is UH a test";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "this is a test");
    }

    #[test]
    fn test_filter_filler_words_with_punctuation() {
        let text = "Well, uhm, I think, uh. that's right";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Well, I think, that's right");
    }

    #[test]
    fn test_filter_cleans_whitespace() {
        let text = "Hello    world   test";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Hello world test");
    }

    #[test]
    fn test_filter_trims() {
        let text = "  Hello world  ";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_filter_combined() {
        let text = "  Uhm, so I was, uh, thinking about this  ";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "so I was, thinking about this");
    }

    #[test]
    fn test_filter_preserves_valid_text() {
        let text = "This is a completely normal sentence.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "This is a completely normal sentence.");
    }

    #[test]
    fn test_filter_stutter_collapse() {
        let text = "w wh wh wh wh wh wh wh wh wh why";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "w wh why");
    }

    #[test]
    fn test_filter_stutter_short_words() {
        let text = "I I I I think so so so so";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "I think so");
    }

    #[test]
    fn test_filter_stutter_longer_words() {
        let text = "Check data doc doc doc doc documentation.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Check data doc documentation.");
    }

    #[test]
    fn test_filter_stutter_mixed_case() {
        let text = "No NO no NO no";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "No");
    }

    #[test]
    fn test_filter_stutter_preserves_two_repetitions() {
        let text = "no no is fine";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "no no is fine");
    }

    #[test]
    fn test_filter_english_removes_um() {
        let text = "um I think um this is good";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "I think this is good");
    }

    #[test]
    fn test_filter_portuguese_preserves_um() {
        // "um" means "a/an" in Portuguese
        let text = "um gato bonito";
        let result = filter_transcription_output(text, "pt", &None);
        assert_eq!(result, "um gato bonito");
    }

    #[test]
    fn test_filter_spanish_preserves_ha() {
        // "ha" means "has" in Spanish
        let text = "ha sido un buen día";
        let result = filter_transcription_output(text, "es", &None);
        assert_eq!(result, "ha sido un buen día");
    }

    #[test]
    fn test_filter_language_code_with_region() {
        // "pt-BR" should normalize to "pt"
        let text = "um gato bonito";
        let result = filter_transcription_output(text, "pt-BR", &None);
        assert_eq!(result, "um gato bonito");
    }

    #[test]
    fn test_filter_custom_filler_words_override() {
        let custom = Some(vec!["okay".to_string(), "right".to_string()]);
        let text = "okay so I think right this works";
        let result = filter_transcription_output(text, "en", &custom);
        assert_eq!(result, "so I think this works");
    }

    #[test]
    fn test_filter_custom_filler_words_empty_disables() {
        let custom = Some(vec![]);
        let text = "So uhm I was thinking uh about this";
        let result = filter_transcription_output(text, "en", &custom);
        // No filler words removed since custom list is empty
        assert_eq!(result, "So uhm I was thinking uh about this");
    }

    #[test]
    fn test_filter_unknown_language_uses_fallback() {
        let text = "uh I think uhm this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "I think this works");
    }

    #[test]
    fn test_filter_fallback_does_not_remove_um() {
        // Fallback (unknown language) should not remove "um" since it's a real word in some languages
        let text = "um I think this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "um I think this works");
    }

    #[test]
    fn test_apply_custom_words_ngram_two_words() {
        let text = "il cui nome è Charge B, che permette";
        let custom_words = vec!["ChargeBee".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("ChargeBee,"));
        assert!(!result.contains("Charge B"));
    }

    #[test]
    fn test_apply_custom_words_ngram_three_words() {
        let text = "use Chat G P T for this";
        let custom_words = vec!["ChatGPT".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("ChatGPT"));
    }

    #[test]
    fn test_apply_custom_words_prefers_longer_ngram() {
        let text = "Open AI GPT model";
        let custom_words = vec!["OpenAI".to_string(), "GPT".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "OpenAI GPT model");
    }

    #[test]
    fn test_apply_custom_words_ngram_preserves_case() {
        let text = "CHARGE B is great";
        let custom_words = vec!["ChargeBee".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("CHARGEBEE"));
    }

    #[test]
    fn test_apply_custom_words_ngram_with_spaces_in_custom() {
        // Custom word with space should also match against split words
        let text = "using Mac Book Pro";
        let custom_words = vec!["MacBook Pro".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("MacBook"));
    }

    #[test]
    fn test_apply_custom_words_trailing_number_not_doubled() {
        // Verify that trailing non-alpha chars (like numbers) aren't double-counted
        // between build_ngram stripping them and extract_punctuation capturing them
        let text = "use GPT4 for this";
        let custom_words = vec!["GPT-4".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        // Should NOT produce "GPT-44" (double-counting the trailing 4)
        assert!(
            !result.contains("GPT-44"),
            "got double-counted result: {}",
            result
        );
    }

    // ===== Korero (v1.27.0): ordinary-English veto + length-aware phonetic gate =====

    /// A realistic NZ custom-word list: brand names that extend ordinary English
    /// words, plus the shipped te reo seed entries.
    fn nz_words() -> Vec<String> {
        [
            "ProjectIQ",
            "ChargeBee",
            "whakapapa",
            "whānau",
            "kōrero",
            "hapū",
            "mahi",
            "iwi",
            "tangata",
            "GST",
            "Cowork",
            "Aotearoa",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// The SHIPPED default (settings.rs default_word_correction_threshold), not
    /// upstream's loose 0.5 -- these regressions only reproduce at 0.18.
    const PROD_THRESHOLD: f64 = 0.18;

    fn korero(text: &str) -> String {
        apply_custom_words(text, &nz_words(), PROD_THRESHOLD)
    }

    #[test]
    fn korero_ordinary_english_is_never_rewritten() {
        // Every one of these is corrupted on the shipped build, and most of them
        // also LOSE one or two words to the greedy longest-first n-gram loop.
        assert_eq!(korero("the project is on track"), "the project is on track");
        assert_eq!(
            korero("the projects are all late"),
            "the projects are all late"
        );
        assert_eq!(korero("kick the project off"), "kick the project off");
        assert_eq!(korero("the guest list"), "the guest list");
        assert_eq!(korero("pull the cork out"), "pull the cork out");

        // Further sentences from the same family.
        assert_eq!(korero("project"), "project");
        assert_eq!(
            korero("I will start the project in the morning"),
            "I will start the project in the morning"
        );
        assert_eq!(
            korero("that project is a big one"),
            "that project is a big one"
        );
        assert_eq!(korero("the project as a whole"), "the project as a whole");
        assert_eq!(
            korero("we should project it out"),
            "we should project it out"
        );
        // Regression fence on the v1.3.0 normalised floor ("have" vs "hapu").
        assert_eq!(korero("we have a plan"), "we have a plan");
    }

    #[test]
    fn korero_custom_words_are_still_corrected() {
        // Multi-token spell-outs must still resolve, and must NOT eat the next word.
        assert_eq!(korero("project I Q needs a look"), "ProjectIQ needs a look");
        assert_eq!(korero("project ID is missing"), "ProjectIQ is missing");
        assert_eq!(
            korero("Charge B is the biller"),
            "ChargeBee is the biller"
        );
        // Te reo must survive untouched.
        assert_eq!(korero("we did the mahi"), "we did the mahi");
        assert_eq!(korero("add GST to that"), "add GST to that");
    }

    #[test]
    fn korero_exact_matches_bypass_the_veto() {
        // Macron restoration is an EXACT match (distance 0) after the v1.3.0
        // strip_macrons patch, so the veto must never touch it.
        assert_eq!(korero("whanau"), "whānau");
        assert_eq!(korero("hapu"), "hapū");
        assert_eq!(korero("our whanau meeting"), "our whānau meeting");
        assert_eq!(
            korero("hapu and iwi consultation"),
            "hapū and iwi consultation"
        );
        // "cowork" is English-looking but matches exactly, so casing still applies.
        assert_eq!(korero("open cowork now"), "open Cowork now");
    }

    #[test]
    fn korero_soundex_boost_requires_two_or_fewer_edits() {
        let custom = vec!["ProjectIQ".to_string()];
        let keys = vec!["projectiq".to_string()];

        // "projectison" ("project is on") is 3 edits away but Soundex-identical
        // (P622 on both sides). The absolute-distance gate must deny the boost,
        // leaving the raw score 3/11 = 0.27 above the 0.18 threshold.
        assert!(find_best_match("projectison", &custom, &keys, PROD_THRESHOLD).is_none());
        // 1 edit still earns the boost -- the gate must not be over-tight. It is
        // the ordinary-English veto, not this gate, that stops "project is".
        assert!(find_best_match("projectis", &custom, &keys, PROD_THRESHOLD).is_some());
        // Exact match is distance 0 and always survives.
        assert!(find_best_match("projectiq", &custom, &keys, PROD_THRESHOLD).is_some());
        // The near-miss band the boost exists to serve is 1-2 raw edits.
        assert!(levenshtein("fakapapa", "whakapapa") <= 2);
    }

    #[test]
    fn korero_ordinary_english_predicate() {
        assert!(is_ordinary_english_phrase(&["project"]));
        assert!(is_ordinary_english_phrase(&["project", "is", "on"]));
        assert!(is_ordinary_english_phrase(&["Project", "Is."]));
        assert!(!is_ordinary_english_phrase(&["Charge", "B"])); // "b" is an initial
        assert!(!is_ordinary_english_phrase(&["project", "ID"])); // "id" is an initialism
        assert!(!is_ordinary_english_phrase(&["whanau"])); // not English
        assert!(!is_ordinary_english_phrase(&[""])); // empty key never counts
    }

    #[test]
    fn korero_common_en_list_invariants() {
        assert!(
            COMMON_EN.windows(2).all(|w| w[0] < w[1]),
            "COMMON_EN must be sorted ascending and duplicate-free for binary_search"
        );
        for w in COMMON_EN.iter() {
            assert!(
                w.chars().all(|c| c.is_ascii_lowercase()),
                "COMMON_EN entries must be ASCII lowercase: {}",
                w
            );
            assert!(
                w.len() >= 2 || *w == "a" || *w == "i",
                "stray single-letter entry in COMMON_EN: {}",
                w
            );
        }
        // Te reo homographs must never be treated as ordinary English, or te reo
        // repair would be vetoed.
        for w in [
            "ana", "ate", "kite", "mate", "pai", "ora", "aroha", "hui", "iwi", "mana", "roa",
            "tapu", "wai", "whare", "mahi", "kai", "whanau", "hapu", "korero", "tangata",
        ]
        .iter()
        {
            assert!(!is_common_en(w), "te reo word must not be in COMMON_EN: {}", w);
        }
        // Initials and initialisms are the EVIDENCE of a spelled-out name.
        for w in ["b", "q", "id", "iq", "ai"].iter() {
            assert!(
                !is_common_en(w),
                "initialism must not be in COMMON_EN: {}",
                w
            );
        }
    }
}
