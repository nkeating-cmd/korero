//! Kōrero 1.44: meeting notes for meetings too long to read in one pass.
//!
//! WHY. Up to 1.43 the notes model read the first 48 000 characters of a
//! transcript and nothing after it. On the test laptop that was 8 of 35
//! transcribed meetings (the longest about 79 000 characters), and because the
//! flagged moments were appended AFTER the transcript, a long meeting also lost
//! every flag the user had set.
//!
//! WHAT. A transcript longer than `SINGLE_PASS_MAX` is split, on line (turn)
//! boundaries, into parts of at most `PART_MAX` characters. Each part gets
//! working notes from a fixed extraction prompt; a final pass applies the
//! user's own notes prompt to those working notes. Under `SINGLE_PASS_MAX`
//! nothing changes: one call, as before.
//!
//! The helpers are pure (no I/O) so they can be unit-tested. `write_in_parts`
//! makes the model calls; `meeting.rs` passes it the app-specific pieces
//! (freeing the speech model, progress events, the streamed final pass), and
//! the live test below drives the same function against a real Ollama.

use std::future::Future;

/// The first line of the notes input when the user flagged moments. The
/// frontend (`notesInput` in `src/components/meetings/model.ts`) writes this
/// block FIRST, then a blank line, then the transcript. Keep the two in step:
/// `flags_intro_matches_frontend` checks the TypeScript copy.
pub const FLAGS_INTRO: &str = "[The person who recorded this flagged these moments as important. Give each one its own line in the notes.]";

/// Up to this many characters, notes are written in one pass, exactly as before 1.44.
pub const SINGLE_PASS_MAX: usize = 48_000;
/// The largest part of a long meeting, in characters. With the part prompt and
/// the flags this stays inside the context 1.43 already sizes for one pass.
pub const PART_MAX: usize = 40_000;
/// A long meeting is refused beyond this many parts (~320 000 characters,
/// about four times the longest meeting seen).
pub const MAX_PARTS: usize = 8;
/// The last line of a part is repeated at the start of the next one, so a
/// decision that spans the cut is seen whole, but only if it is this short.
const OVERLAP_MAX: usize = 2_000;

/// The extraction prompt for one part of a long meeting.
pub const PART_PROMPT: &str = "You are reading one part of a long meeting transcript that was split into \
parts because it is too long to read at once. Write working notes for THIS part only, so they can be \
merged with the notes from the other parts. Use these headings, and leave out any heading with nothing \
under it:\n\
Decisions\n\
Actions (each with its owner and any date)\n\
Key points (keep names, numbers and dates exactly as said)\n\
Open questions\n\
Flagged (only for flagged moments listed above the transcript that happen in THIS part; one line each)\n\
Do not add an introduction or a conclusion. Do not guess at what happened in other parts.";

/// Appended to the user's notes prompt for the final pass over the working notes.
pub fn combine_instruction(parts: usize, has_flags: bool) -> String {
    let mut s = format!(
        "\n\nThis meeting was too long to read at once, so it was read in {parts} parts, in order. \
         What follows is working notes from each part, not the transcript. Write the notes for the \
         WHOLE meeting from them, following the instructions above: merge items that repeat across \
         parts, keep every decision and every action with its owner, and keep the order of events. \
         Do not mention the parts."
    );
    if has_flags {
        s.push_str(" Give each flagged moment its own line.");
    }
    s
}

/// True when notes for `text` must be read in parts: it is over the one-pass
/// limit and its transcript (without the flags) does not fit in one part.
/// A meeting pushed over the limit only by its flags stays one pass.
pub fn needs_parts(text: &str) -> bool {
    if text.chars().count() <= SINGLE_PASS_MAX {
        return false;
    }
    let (flags, transcript) = split_flags(text);
    transcript.chars().count() > part_budget(flags)
}

/// The characters of transcript in one part: `PART_MAX` less the flags block,
/// which every part repeats, but never under half of `PART_MAX`.
pub fn part_budget(flags: Option<&str>) -> usize {
    let f = flags.map(|f| f.chars().count()).unwrap_or(0);
    PART_MAX.saturating_sub(f).max(PART_MAX / 2)
}

/// Split the notes input into (flags block, transcript). The flags block is
/// present only when the input starts with `FLAGS_INTRO`; it runs to the first
/// blank line.
pub fn split_flags(text: &str) -> (Option<&str>, &str) {
    if let Some(rest) = text.strip_prefix(FLAGS_INTRO) {
        match rest.find("\n\n") {
            Some(i) => {
                let end = FLAGS_INTRO.len() + i;
                (Some(&text[..end]), &text[end + 2..])
            }
            // Flags but no transcript after them: nothing to split.
            None => (Some(text), ""),
        }
    } else {
        (None, text)
    }
}

/// Cut one over-long line into pieces of at most `max` characters, at the last
/// sentence end (". ", "? ", "! ") or, failing that, the last space before the
/// limit. A line with no space in reach is cut at the limit.
fn cut_long_line(line: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: &str = line;
    while rest.chars().count() > max {
        // Byte index of the char at position `max`.
        let limit = rest
            .char_indices()
            .nth(max)
            .map(|(b, _)| b)
            .unwrap_or(rest.len());
        let window = &rest[..limit];
        let cut = [". ", "? ", "! "]
            .iter()
            .filter_map(|p| window.rfind(p).map(|i| i + p.len()))
            .max()
            .filter(|&i| i > limit / 2)
            .or_else(|| window.rfind(' ').map(|i| i + 1).filter(|&i| i > limit / 2))
            .unwrap_or(limit);
        out.push(rest[..cut].trim_end().to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// Split a transcript into parts of at most `max` characters, on line
/// boundaries (one line is one speaker turn). Lines longer than `max` (one
/// person speaking for a long time is a single line) are cut at sentence ends.
/// The last line of each part is repeated at the start of the next when it is
/// at most `OVERLAP_MAX` characters. Every line of the input is in at least one
/// part, in order.
pub fn split_parts(transcript: &str, max: usize) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    for line in transcript.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.chars().count() > max {
            pieces.extend(cut_long_line(line, max));
        } else {
            pieces.push(line.to_string());
        }
    }

    let mut parts: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_len = 0usize; // chars, including the newlines between lines
    for piece in &pieces {
        let n = piece.chars().count();
        let added = if cur.is_empty() { n } else { n + 1 };
        if !cur.is_empty() && cur_len + added > max {
            parts.push(cur.join("\n"));
            let carry = *cur.last().expect("non-empty");
            let carry_len = carry.chars().count();
            cur.clear();
            cur_len = 0;
            // Overlap only if the carried line and the next one still fit.
            if carry_len <= OVERLAP_MAX && carry_len + 1 + n <= max {
                cur.push(carry);
                cur_len = carry_len;
            }
        }
        cur_len += if cur.is_empty() { n } else { n + 1 };
        cur.push(piece);
    }
    if !cur.is_empty() {
        parts.push(cur.join("\n"));
    }
    parts
}

/// Write notes for a meeting longer than one pass: working notes for each
/// part (not streamed), then the user's prompt over those notes (streamed by
/// `final_pass`). Returns the notes and the system prompt of the final pass
/// (for `strip_llm_preamble`).
///
/// * `before_calls(largest_prompt_chars)` runs once, before the first call.
/// * `progress(stage, part, of)` reports "part" before each part and
///   "combine" before the final pass.
/// * For Ollama the model is warmed once at the context of the largest part,
///   so every call in the job reuses it instead of reloading (~20 s each).
#[allow(clippy::too_many_arguments)]
pub async fn write_in_parts<B, P, F, Fut>(
    provider: &crate::settings::PostProcessProvider,
    api_key: &str,
    model: &str,
    system: &str,
    glossary: &str,
    text: &str,
    before_calls: B,
    progress: P,
    final_pass: F,
) -> Result<(String, String), String>
where
    B: FnOnce(usize),
    P: Fn(&str, u32, u32),
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let (flags, transcript) = split_flags(text);
    let parts = split_parts(transcript, part_budget(flags));
    if parts.len() > MAX_PARTS {
        return Err(too_long_message(text.chars().count()));
    }
    let n = parts.len();
    let part_system = format!("{PART_PROMPT}{glossary}");
    let users: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| part_user(flags, p, i, n))
        .collect();
    let largest =
        users.iter().map(|u| u.chars().count()).max().unwrap_or(0) + part_system.chars().count();
    before_calls(largest);
    if provider.id == "ollama" {
        let tokens = users
            .iter()
            .map(|u| crate::ollama_chat::estimate_tokens(u))
            .max()
            .unwrap_or(0)
            .saturating_add(crate::ollama_chat::estimate_tokens(&part_system));
        crate::ollama_chat::warm(
            &provider.base_url,
            model,
            tokens,
            crate::ollama_chat::CallKind::Meeting.num_predict(),
        )
        .await;
    }
    log::info!(
        "meeting notes: {} chars read in {n} parts",
        text.chars().count()
    );

    let mut notes: Vec<String> = Vec::with_capacity(n);
    for (i, user) in users.into_iter().enumerate() {
        progress("part", (i + 1) as u32, n as u32);
        let out = crate::llm_client::send_chat_completion_meeting(
            provider,
            api_key.to_string(),
            model,
            user,
            Some(part_system.clone()),
        )
        .await
        .map_err(|e| format!("Part {} of {n} failed: {e}", i + 1))?;
        let out = crate::meeting::strip_llm_preamble(&out, &part_system);
        if out.trim().is_empty() {
            return Err(format!(
                "The model returned nothing for part {} of {n}.",
                i + 1
            ));
        }
        notes.push(out);
    }

    progress("combine", n as u32, n as u32);
    let combine_system = format!(
        "{system}{}{glossary}",
        combine_instruction(n, flags.is_some())
    );
    let answer = final_pass(combine_user(flags, &notes), combine_system.clone()).await?;
    Ok((answer, combine_system))
}

/// The user message for one part.
pub fn part_user(flags: Option<&str>, part: &str, index: usize, total: usize) -> String {
    let head = format!("Part {} of {} of the meeting transcript:", index + 1, total);
    match flags {
        Some(f) => format!("{f}\n\n{head}\n\n{part}"),
        None => format!("{head}\n\n{part}"),
    }
}

/// The user message for the final pass: the flags (if any), then each part's
/// working notes under its own heading.
pub fn combine_user(flags: Option<&str>, part_notes: &[String]) -> String {
    let n = part_notes.len();
    let body = part_notes
        .iter()
        .enumerate()
        .map(|(i, t)| format!("### Working notes, part {} of {n}\n\n{}", i + 1, t.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    match flags {
        Some(f) => format!("{f}\n\n{body}"),
        None => body,
    }
}

/// The message when a meeting is too long even for parts.
pub fn too_long_message(chars: usize) -> String {
    format!(
        "This meeting is too long to write notes for ({} characters; the limit is about {}). \
         Use the trim markers to choose the part that matters, then try again.",
        group_thousands(chars),
        group_thousands(PART_MAX * MAX_PARTS)
    )
}

/// The line appended to an Ask answer when the question was asked of only the
/// first part of a long transcript. Floors the percentage.
pub fn ask_coverage_note(kept: usize, total: usize) -> String {
    let pct = (kept.min(total) * 100).checked_div(total).unwrap_or(100);
    format!(
        "\n\n*This answer is based on the first {pct}% of the transcript: the meeting is longer than \
         the {}-character limit for questions. To ask about a later part, trim the saved meeting to \
         that part and ask again.*",
        group_thousands(kept)
    )
}

/// The line appended to a one-pass result (a refine, an Audio Brief draft)
/// made from only the first part of a long text. Floors the percentage.
pub fn partial_note(kept: usize, total: usize) -> String {
    let pct = (kept.min(total) * 100).checked_div(total).unwrap_or(100);
    format!(
        "\n\n*This was written from the first {pct}% of the text: it is longer than the \
         {}-character limit.*",
        group_thousands(kept)
    )
}

fn group_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fictional transcript of `turns` alternating turns, each about `len` chars.
    fn fixture(turns: usize, len: usize) -> String {
        (0..turns)
            .map(|i| {
                let who = if i % 2 == 0 { "You" } else { "Others" };
                let words =
                    format!("turn {i:04} ") + &"lorem ipsum dolor sit amet. ".repeat(len / 28 + 1);
                format!(
                    "{who}: {}",
                    words.chars().take(len).collect::<String>().trim_end()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Lines of the parts with the overlap removed, in order.
    fn unoverlapped(parts: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in parts {
            for (j, l) in p.lines().enumerate() {
                if j == 0 && out.last().map(|x| x == l).unwrap_or(false) {
                    continue; // the carried line
                }
                out.push(l.to_string());
            }
        }
        out
    }

    #[test]
    fn notes_split_covers_all() {
        // About the size of the longest real meeting (79 284 characters).
        let t = fixture(400, 197);
        assert!(t.chars().count() > 79_000, "{}", t.chars().count());
        let parts = split_parts(&t, PART_MAX);
        // ~81k with a one-turn overlap: two or three parts, never more.
        assert!((2..=3).contains(&parts.len()), "{} parts", parts.len());
        for p in &parts {
            assert!(p.chars().count() <= PART_MAX, "{}", p.chars().count());
        }
        let original: Vec<String> = t.lines().map(str::to_string).collect();
        assert_eq!(unoverlapped(&parts), original, "every turn, in order, once");
    }

    #[test]
    fn notes_split_on_turn_boundaries_only() {
        let t = fixture(400, 197);
        let lines: std::collections::HashSet<&str> = t.lines().collect();
        for p in split_parts(&t, PART_MAX) {
            for l in p.lines() {
                assert!(lines.contains(l), "a part contains a cut turn: {l:.40}");
            }
        }
    }

    #[test]
    fn notes_split_overlaps_one_short_turn() {
        let t = fixture(400, 197);
        let parts = split_parts(&t, PART_MAX);
        let last_of_first = parts[0].lines().last().unwrap();
        assert_eq!(parts[1].lines().next().unwrap(), last_of_first);
    }

    #[test]
    fn notes_split_cuts_a_monologue_at_sentences() {
        // One speaker, one line of ~90k characters (combine() merges a speaker's turns).
        let line = format!(
            "You: {}",
            "This is one sentence of a long talk. ".repeat(2_500)
        );
        let parts = split_parts(&line, PART_MAX);
        assert!(parts.len() >= 3, "{}", parts.len());
        for p in &parts {
            assert!(p.chars().count() <= PART_MAX);
        }
        // Cuts land after a full stop, so no part starts mid-word.
        for p in &parts[1..] {
            assert!(p.starts_with("This is one sentence"), "{:.40}", p);
        }
        let words: usize = parts.iter().map(|p| p.matches("sentence").count()).sum();
        assert_eq!(
            words, 2_500,
            "nothing lost or doubled (no overlap on long lines)"
        );
    }

    #[test]
    fn notes_split_under_limit_is_one_part() {
        let t = fixture(10, 100);
        assert_eq!(split_parts(&t, PART_MAX), vec![t]);
    }

    #[test]
    fn notes_split_handles_non_latin_text() {
        // Char counts, not bytes: macrons and CJK must not panic or overflow.
        let t = (0..3000)
            .map(|i| format!("Others: kōrero whānau 会議 {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let parts = split_parts(&t, 10_000);
        assert!(parts.len() > 1);
        for p in &parts {
            assert!(p.chars().count() <= 10_000);
        }
    }

    #[test]
    fn flags_split_from_the_front() {
        let input = format!("{FLAGS_INTRO}\n- 01:02:03 (budget): \"we agreed\"\n- 01:10:00\n\nYou: hello\nOthers: hi");
        let (f, body) = split_flags(&input);
        assert_eq!(f.unwrap().lines().count(), 3);
        assert_eq!(body, "You: hello\nOthers: hi");
        let (none, same) = split_flags("You: hello");
        assert!(none.is_none());
        assert_eq!(same, "You: hello");
    }

    #[test]
    fn flags_reach_every_part_and_the_combine() {
        let flags = format!("{FLAGS_INTRO}\n- 01:15:00: \"last-minute decision\"");
        let u = part_user(Some(&flags), "You: a", 1, 2);
        assert!(u.starts_with(FLAGS_INTRO) && u.contains("Part 2 of 2"));
        let c = combine_user(Some(&flags), &["a".into(), "b".into()]);
        assert!(c.starts_with(FLAGS_INTRO) && c.contains("part 2 of 2"));
        assert!(combine_instruction(2, true).contains("flagged moment"));
        assert!(!combine_instruction(2, false).contains("flagged moment"));
    }

    #[test]
    fn flags_intro_matches_frontend() {
        let ts = include_str!("../../src/components/meetings/model.ts");
        assert!(
            ts.contains(FLAGS_INTRO),
            "model.ts FLAGS_INTRO drifted from notes_parts.rs"
        );
    }

    #[test]
    fn a_split_is_never_one_part() {
        // Whenever notes are read in parts there are at least two, so a long
        // meeting never costs a pointless extra combine call.
        for (turns, flag_len) in [
            (240, 0),
            (240, 9_000),
            (300, 15_000),
            (400, 0),
            (90, 30_000),
        ] {
            let flags = format!("{FLAGS_INTRO}\n- 0:01 (x): {}", "y".repeat(flag_len));
            let t = fixture(turns, 199);
            let text = if flag_len == 0 {
                t.clone()
            } else {
                format!("{flags}\n\n{t}")
            };
            let (f, body) = split_flags(&text);
            if needs_parts(&text) {
                assert!(
                    split_parts(body, part_budget(f)).len() >= 2,
                    "{turns}/{flag_len}"
                );
            }
        }
        // Only a flags block over half a part keeps a long meeting to one pass:
        // ~18,500 characters of transcript fits the 20,000 floor.
        let flags = format!("{FLAGS_INTRO}\n- 0:01 (x): {}", "y".repeat(30_000));
        let t = fixture(90, 199);
        let text = format!("{flags}\n\n{t}");
        assert!(text.chars().count() > SINGLE_PASS_MAX);
        assert!(!needs_parts(&text));
        // Without flags, over the limit means parts; under it, one pass.
        assert!(needs_parts(&fixture(400, 197)));
        assert!(!needs_parts(&fixture(10, 100)));
    }

    #[test]
    fn parts_leave_room_for_the_flags() {
        let flags = format!("{FLAGS_INTRO} {}", "z".repeat(5_000));
        let budget = part_budget(Some(&flags));
        assert_eq!(budget, PART_MAX - flags.chars().count());
        for p in split_parts(&fixture(400, 197), budget) {
            assert!(part_user(Some(&flags), &p, 0, 9).chars().count() <= PART_MAX + 64);
        }
        assert_eq!(
            part_budget(Some(&"q".repeat(90_000))),
            PART_MAX / 2,
            "floor"
        );
    }

    #[test]
    fn too_long_and_ask_notes_read_well() {
        assert!(too_long_message(400_000).contains("400,000"));
        assert!(too_long_message(400_000).contains("320,000"));
        let n = ask_coverage_note(48_000, 64_000);
        assert!(
            n.contains("first 75%") && n.contains("48,000-character"),
            "{n}"
        );
        assert!(
            ask_coverage_note(0, 0).contains("100%"),
            "no divide by zero"
        );
        assert!(
            !n.contains("trim markers"),
            "the live view has no trim markers"
        );
        assert!(partial_note(48_000, 96_000).contains("first 50%"));
    }

    // ---- live test (the test laptop): run with
    // cargo test --lib notes_parts::tests::live -- --ignored --nocapture --test-threads=1
    // A fictional ~80,000-character meeting with a code word planted at the
    // start, in the middle and at the end, plus a flag on the last minute.

    fn live_transcript() -> (String, usize) {
        let filler = "We went through the delivery timeline, the risks on the data migration and \
                      who needs to be in the next workshop. ";
        let mut lines: Vec<String> = Vec::new();
        lines.push("You: First decision, so it is on record: the opening code word is KAURI-7, and Aroha owns the vendor shortlist by Friday.".into());
        for i in 0..380 {
            let who = if i % 2 == 0 { "Others" } else { "You" };
            if i == 190 {
                lines.push("Others: Middle of the meeting, one more decision: the middle code word is TOTARA-3, and Mere will book the venue.".into());
            }
            lines.push(format!("{who}: Item {i}. {}", filler.repeat(2)));
        }
        lines.push("You: Last decision before we close: the closing code word is RIMU-9, and Tama sends the minutes on Monday.".into());
        let body = lines.join("\n");
        let chars = body.chars().count();
        let flags =
            format!("{FLAGS_INTRO}\n- 1:19:30 (closing): \"the closing code word is RIMU-9\"");
        (format!("{flags}\n\n{body}"), chars)
    }

    #[test]
    #[ignore = "live: needs a local Ollama with the model (the test laptop)"]
    fn live_long_meeting_notes_cover_every_part() {
        tauri::async_runtime::block_on(async {
            let provider = crate::settings::get_default_settings()
                .post_process_providers
                .into_iter()
                .find(|p| p.id == "ollama")
                .expect("built-in ollama provider");
            let model = std::env::var("KORERO_LIVE_MODEL").unwrap_or_else(|_| "gemma4:12b".into());
            let (text, chars) = live_transcript();
            assert!(chars > SINGLE_PASS_MAX, "fixture is {chars} chars");
            let system = "Summarise this meeting transcript: key points, decisions, and action items with owners.";
            let (p, m) = (&provider, &model);
            let t0 = std::time::Instant::now();
            let stamps = std::sync::Mutex::new(Vec::<String>::new());
            let (notes, _) = write_in_parts(
                &provider,
                "",
                &model,
                system,
                "",
                &text,
                |_| {},
                |stage, part, of| {
                    stamps.lock().unwrap().push(format!(
                        "{stage} {part}/{of} at {:.0}s",
                        t0.elapsed().as_secs_f32()
                    ));
                },
                |user, sys| async move {
                    crate::llm_client::stream_chat_completion(
                        p,
                        String::new(),
                        m,
                        user,
                        Some(sys),
                        |_| {},
                    )
                    .await
                },
            )
            .await
            .expect("notes in parts");
            let secs = t0.elapsed().as_secs_f32();
            println!(
                "LIVE input chars: {chars}; total {secs:.0}s; {:?}",
                stamps.lock().unwrap()
            );
            println!("LIVE notes ({} chars):\n{notes}", notes.chars().count());
            for w in ["KAURI-7", "TOTARA-3", "RIMU-9"] {
                assert!(notes.contains(w), "notes missed {w}");
            }
        });
    }
}
