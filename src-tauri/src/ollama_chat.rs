//! Kōrero 1.43: chat with a local Ollama through its NATIVE `/api/chat`, with
//! a context window sized to the request.
//!
//! WHY. Ollama's default context is 4 096 tokens on any GPU with less than
//! 24 GB (docs.ollama.com/context-length), and the OpenAI-compatible endpoint
//! Kōrero used cannot change it. Meeting notes and Ask send up to 48 000
//! characters (~12 000 tokens), so on a default install the model saw only part
//! of the meeting, silently. Measured on legion (Ollama 0.34.4, gemma4:12b, a
//! fictional 40 583-character transcript with code words planted at both
//! ends): at num_ctx 4 096 only 2 051 prompt tokens were evaluated and the
//! answer was invented; at 14 336 both code words came back.
//!
//! RULES.
//! * One chooser for every Ollama call (`choose_num_ctx`). Reuse the context
//!   the model is already loaded with whenever it is big enough: a different
//!   `num_ctx` makes Ollama reload the model, which cost ~20 s on legion.
//! * Always send `truncate: false`. Ollama then refuses an over-long prompt
//!   with HTTP 400 and the exact token count instead of dropping text, and we
//!   retry once at the right size. Never silent truncation.
//! * Never send `shift`: changing it forced a reload in the probe, and context
//!   shift cannot trigger anyway because num_ctx >= prompt + num_predict.
//! * Thinking off (`think: false`), as 1.42 does on the OpenAI path.
//!
//! LOGGING: URL, model, num_ctx, status and byte counts only. Never the prompt,
//! the transcript, a response line or a request body (`checks.json`:
//! sec-143-05-no-content-logging).

use crate::settings::PostProcessProvider;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Largest context Kōrero asks for. meeting.rs caps a transcript at 48 000
/// characters (~17 800 tokens at the estimate below) plus 8 192 output tokens,
/// so this holds the largest notes call with room to spare.
pub const MAX_CTX: u32 = 32_768;
const MIN_CTX: u32 = 4_096;
const CTX_STEP: u32 = 2_048;
const MARGIN: u32 = 512;
const MIN_PREDICT: u32 = 1_024;
/// Same residency as the pre-warm, so a call doesn't cut it back to Ollama's
/// 5-minute default (every request resets the expiry).
pub const KEEP_ALIVE: &str = "1800s";

/// Bounds on what a (possibly hostile) local server can make us buffer.
const MAX_NDJSON_LINE: usize = 1 << 20; // 1 MiB
const MAX_OUTPUT: usize = 4 << 20; // 4 MiB of generated text
const MAX_BODY: usize = 4 << 20; // non-streamed chat body
pub(crate) const SMALL_BODY: usize = 64 << 10; // /api/ps, /api/show
/// Ollama reports context_length as a plain number; anything above this is
/// not a real model context and is ignored.
const MAX_SANE_CTX: u64 = 262_144;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DICTATION_TIMEOUT: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(180);
const TOTAL_CEILING: Duration = Duration::from_secs(3_600);

pub const TOO_LONG: &str = "This transcript is too long for the model to read in one pass. \
Use the trim markers to choose the part that matters, then try again.";

/// Which caller is asking. Decides the output allowance and the time budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    /// Dictation clean-up: the user is waiting at a text cursor.
    Dictation,
    /// Meeting notes (streamed).
    Meeting,
    /// Ask a question about a meeting.
    Ask,
    /// Notes page and History re-runs.
    Note,
}

impl CallKind {
    pub fn num_predict(self) -> u32 {
        match self {
            CallKind::Meeting => 8_192,
            _ => 1_500,
        }
    }
    /// Time allowed before the first byte (stream) or the whole response
    /// (non-stream). Ollama sends nothing until prompt evaluation finishes, so
    /// a long transcript needs more than a short dictation.
    fn budget(self, prompt_tokens: u32) -> Duration {
        match self {
            CallKind::Dictation => DICTATION_TIMEOUT,
            _ => {
                let extra = 20 * u64::from(prompt_tokens / 4_000 + 1);
                Duration::from_secs((180 + extra).min(900))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// `http://localhost:11434/v1/` → `http://localhost:11434`.
pub fn native_base(base_url: &str) -> String {
    base_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .trim_end_matches('/')
        .to_string()
}

fn is_dense_script(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F   // CJK symbols and punctuation (。、「」)
        | 0x3040..=0x30FF // hiragana, katakana
        | 0x3400..=0x4DBF // CJK extension A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xAC00..=0xD7AF // hangul
        | 0xF900..=0xFAFF // CJK compatibility
        | 0xFF00..=0xFFEF) // full-width forms
}

/// Conservative prompt-token estimate. Latin text at 0.37 tokens per
/// character (legion measured 0.262 on prose and 0.358 on digit-heavy text
/// with gemma4); CJK, kana and hangul at a full token each. `truncate:false`
/// catches the rest.
pub fn estimate_tokens(text: &str) -> u32 {
    let mut dense: u64 = 0;
    let mut other: u64 = 0;
    for c in text.chars() {
        if is_dense_script(c) {
            dense += 1;
        } else {
            other += 1;
        }
    }
    let est = dense + (other * 37).div_ceil(100) + 256;
    est.min(u64::from(u32::MAX)) as u32
}

/// `llama3.2` and `llama3.2:latest` are the same model to Ollama.
pub fn normalise_model(name: &str) -> String {
    let n = name.trim().to_ascii_lowercase();
    if n.contains(':') {
        n
    } else {
        format!("{n}:latest")
    }
}

fn round_up(n: u32) -> u32 {
    n.div_ceil(CTX_STEP).saturating_mul(CTX_STEP)
}

/// Pick `(num_ctx, num_predict)` for a request.
///
/// Reuse first: if the model is already loaded with enough room, use exactly
/// that context so Ollama doesn't reload it. Otherwise ask for the smallest
/// step that holds prompt + output, within [MIN_CTX, MAX_CTX]. If even
/// MAX_CTX can't hold it, shrink the output allowance; if the prompt alone
/// doesn't fit, refuse rather than truncate.
pub fn choose_num_ctx(
    prompt_tokens: u32,
    num_predict: u32,
    loaded: Option<u32>,
) -> Result<(u32, u32), String> {
    let need = prompt_tokens
        .saturating_add(num_predict)
        .saturating_add(MARGIN);
    if let Some(l) = loaded {
        if l >= need {
            return Ok((l, num_predict));
        }
    }
    if need <= MAX_CTX {
        return Ok((round_up(need).clamp(MIN_CTX, MAX_CTX), num_predict));
    }
    // Too big for MAX_CTX at the full allowance: write less rather than read less.
    let best = loaded.unwrap_or(0).max(MAX_CTX);
    let room = best.saturating_sub(prompt_tokens.saturating_add(MARGIN));
    if room >= MIN_PREDICT {
        return Ok((best, room.min(num_predict)));
    }
    Err(TOO_LONG.to_string())
}

/// Ollama's refusal under `truncate:false`:
/// `request (29702 tokens) exceeds the available context size (8192 tokens)`.
/// The message can arrive JSON-escaped inside another error, so search the raw
/// text for the number after `request (`.
pub fn parse_exceeds(body: &str) -> Option<u32> {
    let i = body.find("request (")? + "request (".len();
    let digits: String = body[i..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if body[i + digits.len()..].starts_with(" tokens)") {
        digits.parse().ok()
    } else {
        None
    }
}

fn is_oom(status: u16, body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    (status >= 500
        && (b.contains("out of memory") || (b.contains("memory") && b.contains("alloc"))))
        || b.contains("requires more system memory")
}

/// The JSON body for `/api/chat`.
pub fn build_body(
    model: &str,
    system: Option<&str>,
    user: &str,
    num_ctx: u32,
    num_predict: u32,
    stream: bool,
) -> Value {
    let mut messages = Vec::new();
    if let Some(s) = system {
        messages.push(json!({"role": "system", "content": s}));
    }
    messages.push(json!({"role": "user", "content": user}));
    json!({
        "model": model,
        "messages": messages,
        "stream": stream,
        "think": false,
        "truncate": false,
        "keep_alive": KEEP_ALIVE,
        "options": {
            "num_ctx": num_ctx,
            "num_predict": num_predict,
            // Parity with the OpenAI-compatible path Kōrero used before 1.43,
            // which (we believe) defaults both to 1.0 when the request omits
            // them. Quality changes belong to the clean-up bake-off, not here.
            "temperature": 1.0,
            "top_p": 1.0
        }
    })
}

/// What one NDJSON line meant.
#[derive(Debug, PartialEq)]
pub enum Line {
    Content,
    Done(Option<String>),
    Error(String),
    Ignored,
}

/// Incremental NDJSON parser. Bytes are buffered and only complete lines are
/// decoded, so a multi-byte character split across chunks is never mangled
/// (`ā` is 2 bytes; this app is about te reo).
#[derive(Default)]
pub struct Ndjson {
    buf: Vec<u8>,
    pub full: String,
    pub done: Option<Option<String>>,
    pub error: Option<String>,
}

impl Ndjson {
    /// Feed a chunk. Returns `Err(why)` when a bound is exceeded.
    pub fn feed<F: FnMut(&str)>(&mut self, chunk: &[u8], on_delta: &mut F) -> Result<(), String> {
        self.buf.extend_from_slice(chunk);
        while let Some(nl) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            self.line(&line[..line.len() - 1], on_delta)?;
            if self.done.is_some() || self.error.is_some() {
                return Ok(());
            }
        }
        if self.buf.len() > MAX_NDJSON_LINE {
            return Err("the local model sent an over-long line".to_string());
        }
        Ok(())
    }

    /// End of stream: consume a last line that arrived without a newline.
    pub fn finish<F: FnMut(&str)>(&mut self, on_delta: &mut F) -> Result<(), String> {
        if !self.buf.is_empty() && self.done.is_none() && self.error.is_none() {
            let rest = std::mem::take(&mut self.buf);
            self.line(&rest, on_delta)?;
        }
        Ok(())
    }

    fn line<F: FnMut(&str)>(&mut self, raw: &[u8], on_delta: &mut F) -> Result<(), String> {
        let text = String::from_utf8_lossy(raw);
        match classify(text.trim(), &mut self.full, on_delta) {
            Line::Done(reason) => self.done = Some(reason),
            Line::Error(e) => self.error = Some(e),
            Line::Content | Line::Ignored => {}
        }
        if self.full.len() > MAX_OUTPUT {
            return Err("the output passed the 4 MB safety limit".to_string());
        }
        Ok(())
    }

    /// Turn the parse state into the caller's result, keeping partial output.
    pub fn into_result(self) -> Result<String, String> {
        if let Some(e) = self.error {
            if self.full.trim().is_empty() {
                return Err(format!("Ollama: {e}"));
            }
            return crate::llm_client::partial_or_error(
                self.full,
                &format!("Ollama reported an error ({e})"),
            );
        }
        match self.done {
            Some(Some(r)) if r == "length" => crate::llm_client::partial_or_error(
                self.full,
                "the model reached its output-length limit",
            ),
            Some(_) => Ok(self.full),
            None => crate::llm_client::partial_or_error(
                self.full,
                "the local model stopped before it said it was finished",
            ),
        }
    }
}

fn classify<F: FnMut(&str)>(line: &str, full: &mut String, on_delta: &mut F) -> Line {
    if line.is_empty() {
        return Line::Ignored;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        log::warn!(
            "ollama_chat: skipped an unparseable line ({} bytes)",
            line.len()
        );
        return Line::Ignored;
    };
    if let Some(e) = v.get("error") {
        return Line::Error(
            e.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| e.to_string()),
        );
    }
    let mut got = Line::Ignored;
    if let Some(piece) = v["message"]["content"].as_str() {
        if !piece.is_empty() {
            on_delta(piece);
            full.push_str(piece);
            got = Line::Content;
        }
    }
    if v["done"].as_bool() == Some(true) {
        return Line::Done(v["done_reason"].as_str().map(str::to_string));
    }
    got
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

/// Read a response body, refusing more than `cap` bytes.
pub(crate) async fn read_bounded(resp: reqwest::Response, cap: usize) -> Result<String, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut s = resp.bytes_stream();
    while let Some(chunk) = s.next().await {
        let chunk = chunk.map_err(|e| format!("Reading Ollama's reply failed: {e}"))?;
        if out.len() + chunk.len() > cap {
            return Err("Ollama's reply was larger than expected and was not read.".to_string());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The context the model is loaded with right now, if any (`/api/ps`).
/// Never fails: anything unexpected is `None`.
pub async fn loaded_context(base: &str, model: &str) -> Option<u32> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(format!("{base}/api/ps")).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = read_bounded(resp, SMALL_BODY).await.ok()?;
    loaded_context_from(&text, model)
}

pub fn loaded_context_from(ps_json: &str, model: &str) -> Option<u32> {
    let v: Value = serde_json::from_str(ps_json).ok()?;
    let want = normalise_model(model);
    v["models"].as_array()?.iter().find_map(|m| {
        let name = m["name"].as_str().or_else(|| m["model"].as_str())?;
        if normalise_model(name) != want {
            return None;
        }
        let ctx = m["context_length"].as_u64()?;
        (ctx > 0 && ctx <= MAX_SANE_CTX).then_some(ctx as u32)
    })
}

/// The native call failed in a way that means "this isn't Ollama".
#[derive(Debug)]
pub enum Outcome<T> {
    Done(T),
    /// A 404 without Ollama's JSON error shape: the `ollama` provider points at
    /// another OpenAI-compatible server. The caller falls back to `/v1`.
    NotOllama,
}

fn client(
    provider: &PostProcessProvider,
    api_key: &str,
    total: Option<Duration>,
) -> Result<reqwest::Client, String> {
    let headers = crate::llm_client::build_headers(provider, api_key)?;
    let mut b = reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(CONNECT_TIMEOUT);
    b = match total {
        Some(t) => b.timeout(t),
        None => b.read_timeout(IDLE_TIMEOUT),
    };
    b.build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

fn ollama_json_error(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body.trim()).ok()?;
    v.get("error").map(|e| {
        e.as_str()
            .map(str::to_string)
            .unwrap_or_else(|| e.to_string())
    })
}

/// Size the request and send it, retrying once for a refused-as-too-long or
/// out-of-memory answer and once after starting a stopped Ollama.
async fn send_sized(
    provider: &PostProcessProvider,
    http: &reqwest::Client,
    model: &str,
    system: Option<&str>,
    user: &str,
    kind: CallKind,
    stream: bool,
) -> Result<Outcome<reqwest::Response>, String> {
    let base = native_base(&provider.base_url);
    let url = format!("{base}/api/chat");
    let mut prompt = estimate_tokens(user) + system.map(estimate_tokens).unwrap_or(0);
    let loaded = loaded_context(&base, model).await;
    let (mut num_ctx, mut num_predict) = choose_num_ctx(prompt, kind.num_predict(), loaded)?;
    let budget = kind.budget(prompt);
    let mut restarted = false;
    let mut resized = false;

    loop {
        log::info!(
            "ollama_chat: POST {url} model={model} num_ctx={num_ctx} num_predict={num_predict} kind={kind:?} (loaded={loaded:?})"
        );
        let body = build_body(model, system, user, num_ctx, num_predict, stream);
        let send = http.post(&url).json(&body).send();
        let resp = match tokio::time::timeout(budget, send).await {
            Err(_) => {
                return Err(format!(
                    "The local model didn't start answering within {} s.",
                    budget.as_secs()
                ))
            }
            Ok(Err(e)) if e.is_connect() && !restarted && provider.is_local_provider => {
                restarted = true;
                log::info!("ollama_chat: Ollama unreachable, starting it and retrying once");
                if crate::commands::ollama::ensure_running(&provider.base_url).await {
                    continue;
                }
                return Err(format!("Couldn't reach Ollama: {e}"));
            }
            Ok(Err(e)) => return Err(format!("HTTP request failed: {e}")),
            Ok(Ok(r)) => r,
        };
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(Outcome::Done(resp));
        }
        let text = read_bounded(resp, SMALL_BODY).await.unwrap_or_default();
        log::warn!("ollama_chat: HTTP {status} ({} bytes)", text.len());
        if status == 404 && ollama_json_error(&text).is_none() {
            return Ok(Outcome::NotOllama);
        }
        if !resized {
            if let Some(actual) = parse_exceeds(&text) {
                resized = true;
                prompt = actual;
                (num_ctx, num_predict) = choose_num_ctx(prompt, kind.num_predict(), None)?;
                continue;
            }
            if is_oom(status, &text) {
                resized = true;
                // The smallest context that still holds the whole prompt.
                let min_ok = round_up(prompt + MIN_PREDICT + MARGIN).max(MIN_CTX);
                if min_ok < num_ctx {
                    num_predict = min_ok - prompt - MARGIN;
                    num_ctx = min_ok;
                    continue;
                }
            }
        }
        if is_oom(status, &text) {
            return Err(
                "This model needs more memory than is free right now. Close other apps that use \
                 the graphics card, or choose a smaller model."
                    .to_string(),
            );
        }
        let msg = ollama_json_error(&text).unwrap_or(text);
        return Err(format!("Ollama returned HTTP {status}: {msg}"));
    }
}

/// One non-streamed answer. `Ok(None)` for empty content (so a dictation never
/// pastes an empty string).
pub async fn chat(
    provider: &PostProcessProvider,
    api_key: &str,
    model: &str,
    system: Option<&str>,
    user: &str,
    kind: CallKind,
) -> Result<Outcome<Option<String>>, String> {
    // Ollama sends a non-streamed reply only when it is complete, so the budget
    // in `send_sized` already bounds the whole call; the client's own limit sits
    // just above it as a backstop.
    let prompt = estimate_tokens(user) + system.map(estimate_tokens).unwrap_or(0);
    let http = client(
        provider,
        api_key,
        Some(kind.budget(prompt) + Duration::from_secs(5)),
    )?;
    let resp = match send_sized(provider, &http, model, system, user, kind, false).await? {
        Outcome::Done(r) => r,
        Outcome::NotOllama => return Ok(Outcome::NotOllama),
    };
    let text = read_bounded(resp, MAX_BODY).await?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse Ollama's reply: {e}"))?;
    if let Some(e) = v.get("error") {
        return Err(format!("Ollama: {e}"));
    }
    let content = v["message"]["content"].as_str().unwrap_or("").to_string();
    Ok(Outcome::Done(if content.trim().is_empty() {
        None
    } else {
        Some(content)
    }))
}

/// A streamed answer for meeting notes. Keeps everything generated if the
/// stream stops early, and says so.
pub async fn chat_stream<F: FnMut(&str)>(
    provider: &PostProcessProvider,
    api_key: &str,
    model: &str,
    system: Option<&str>,
    user: &str,
    kind: CallKind,
    mut on_delta: F,
) -> Result<Outcome<String>, String> {
    let http = client(provider, api_key, None)?;
    let resp = match send_sized(provider, &http, model, system, user, kind, true).await? {
        Outcome::Done(r) => r,
        Outcome::NotOllama => return Ok(Outcome::NotOllama),
    };
    let mut stream = resp.bytes_stream();
    let mut p = Ndjson::default();
    let started = Instant::now();
    loop {
        let next = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
            Err(_) => {
                return crate::llm_client::partial_or_error(
                    p.full,
                    &format!(
                        "the local model stopped sending for {} s",
                        IDLE_TIMEOUT.as_secs()
                    ),
                )
                .map(Outcome::Done)
            }
            Ok(n) => n,
        };
        match next {
            None => break,
            Some(Err(e)) => {
                log::warn!("ollama_chat: stream ended early: {e}");
                return crate::llm_client::partial_or_error(
                    p.full,
                    &format!("the stream failed ({e})"),
                )
                .map(Outcome::Done);
            }
            Some(Ok(chunk)) => {
                if let Err(why) = p.feed(&chunk, &mut on_delta) {
                    return crate::llm_client::partial_or_error(p.full, &why).map(Outcome::Done);
                }
                if p.done.is_some() || p.error.is_some() {
                    break;
                }
            }
        }
        if started.elapsed() > TOTAL_CEILING {
            return crate::llm_client::partial_or_error(
                p.full,
                &format!(
                    "it ran past the {} minute absolute ceiling",
                    TOTAL_CEILING.as_secs() / 60
                ),
            )
            .map(Outcome::Done);
        }
    }
    if let Err(why) = p.finish(&mut on_delta) {
        return crate::llm_client::partial_or_error(p.full, &why).map(Outcome::Done);
    }
    p.into_result().map(Outcome::Done)
}

/// Load a model ahead of use, at the context the coming call will want, so
/// the call itself doesn't pay for a reload. Best-effort; never errors.
pub async fn warm(base_url: &str, model: &str, prompt_tokens: u32, num_predict: u32) {
    if model.trim().is_empty() {
        return;
    }
    let base = native_base(base_url);
    let loaded = loaded_context(&base, model).await;
    let Ok((num_ctx, _)) = choose_num_ctx(prompt_tokens, num_predict, loaded) else {
        return;
    };
    if loaded == Some(num_ctx) {
        log::info!("ollama_chat: '{model}' already loaded at num_ctx={num_ctx}");
        return;
    }
    let body = json!({
        "model": model,
        "prompt": "",
        "keep_alive": KEEP_ALIVE,
        "stream": false,
        "options": {"num_ctx": num_ctx}
    });
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
    else {
        return;
    };
    match client
        .post(format!("{base}/api/generate"))
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            log::info!("ollama_chat: pre-warmed '{model}' at num_ctx={num_ctx}")
        }
        Ok(r) => log::warn!("ollama_chat: pre-warm got HTTP {}", r.status()),
        Err(e) => log::warn!("ollama_chat: pre-warm failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(chunks: &[&[u8]]) -> (Ndjson, Vec<String>) {
        let mut p = Ndjson::default();
        let mut seen = Vec::new();
        let mut cb = |s: &str| seen.push(s.to_string());
        for c in chunks {
            p.feed(c, &mut cb).unwrap();
        }
        p.finish(&mut cb).unwrap();
        (p, seen)
    }

    #[test]
    fn native_base_strips_v1() {
        assert_eq!(
            native_base("http://localhost:11434/v1"),
            "http://localhost:11434"
        );
        assert_eq!(
            native_base("http://localhost:11434/v1/"),
            "http://localhost:11434"
        );
        assert_eq!(
            native_base("http://localhost:11434"),
            "http://localhost:11434"
        );
        assert_eq!(
            native_base(" http://127.0.0.1:11434/ "),
            "http://127.0.0.1:11434"
        );
    }

    #[test]
    fn estimate_covers_measured_densities() {
        // legion, gemma4: 40 583 chars of prose -> 10 644 tokens;
        // 82 889 digit-heavy chars -> 29 702 tokens.
        let prose = "a".repeat(40_583);
        assert!(estimate_tokens(&prose) >= 10_644);
        let digits = "b".repeat(82_889);
        assert!(estimate_tokens(&digits) >= 29_702);
    }

    #[test]
    fn estimate_counts_cjk_as_a_token_each() {
        let ja = "会議の議事録です。".repeat(1_000); // 9 000 dense chars
        assert!(estimate_tokens(&ja) >= 9_000);
        let mixed = "Kia ora whānau, 会議";
        assert!(estimate_tokens(mixed) > 256);
    }

    #[test]
    fn reuse_first_no_floor() {
        // A short dictation must reuse a 4k-loaded model, not reload it.
        assert_eq!(
            choose_num_ctx(900, 1_500, Some(4_096)).unwrap(),
            (4_096, 1_500)
        );
        // A big server default is reused, not clamped down to MAX_CTX.
        assert_eq!(
            choose_num_ctx(18_000, 8_192, Some(65_536)).unwrap(),
            (65_536, 8_192)
        );
    }

    #[test]
    fn sizes_up_when_loaded_is_too_small() {
        let (ctx, np) = choose_num_ctx(10_644, 8_192, Some(4_096)).unwrap();
        assert!(ctx >= 10_644 + 8_192 && ctx % 2_048 == 0 && ctx <= MAX_CTX);
        assert_eq!(np, 8_192);
    }

    #[test]
    fn min_ctx_when_nothing_loaded() {
        assert_eq!(choose_num_ctx(300, 1_500, None).unwrap(), (4_096, 1_500));
    }

    #[test]
    fn shrinks_output_before_refusing() {
        let (ctx, np) = choose_num_ctx(28_000, 8_192, None).unwrap();
        assert_eq!(ctx, MAX_CTX);
        assert!(np >= MIN_PREDICT && 28_000 + np + MARGIN <= MAX_CTX);
        assert!(choose_num_ctx(32_000, 8_192, None).is_err());
    }

    #[test]
    fn every_choice_holds_prompt_plus_output() {
        for prompt in (0..40_000).step_by(997) {
            for &np in &[1_500u32, 8_192] {
                for loaded in [None, Some(4_096), Some(8_192), Some(24_576), Some(65_536)] {
                    if let Ok((ctx, p)) = choose_num_ctx(prompt, np, loaded) {
                        assert!(
                            prompt + p + MARGIN <= ctx,
                            "prompt={prompt} np={np} loaded={loaded:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn model_names_normalise() {
        assert_eq!(normalise_model("llama3.2"), "llama3.2:latest");
        assert_eq!(normalise_model("Gemma4:12B"), "gemma4:12b");
        let ps = r#"{"models":[{"name":"llama3.2:latest","context_length":8192},{"name":"x","context_length":999999999}]}"#;
        assert_eq!(loaded_context_from(ps, "llama3.2"), Some(8_192));
        assert_eq!(loaded_context_from(ps, "x"), None); // insane value ignored
        assert_eq!(loaded_context_from("not json", "x"), None);
    }

    #[test]
    fn parses_the_too_long_refusal() {
        let raw = r#"{"error":"{\"error\":{\"code\":400,\"message\":\"request (29702 tokens) exceeds the available context size (8192 tokens), try increasing it\"}}"}"#;
        assert_eq!(parse_exceeds(raw), Some(29_702));
        assert_eq!(parse_exceeds("model not found"), None);
        assert_eq!(parse_exceeds("request (12 apples)"), None);
    }

    #[test]
    fn body_has_no_shift_and_turns_thinking_and_truncation_off() {
        let b = build_body("m", Some("sys"), "user", 8_192, 1_500, true);
        assert_eq!(b["think"], false);
        assert_eq!(b["truncate"], false);
        assert_eq!(b["keep_alive"], KEEP_ALIVE);
        assert_eq!(b["options"]["num_ctx"], 8_192);
        assert_eq!(b["options"]["num_predict"], 1_500);
        assert!(b.get("shift").is_none() && b["options"].get("shift").is_none());
        assert_eq!(b["messages"][0]["role"], "system");
    }

    #[test]
    fn ndjson_keeps_a_split_macron() {
        let line = "{\"message\":{\"content\":\"whānau\"},\"done\":false}\n";
        let bytes = line.as_bytes();
        let cut = line.find('ā').unwrap() + 1; // inside the 2-byte ā
        let done = b"{\"done\":true,\"done_reason\":\"stop\"}\n";
        let (p, seen) = collect(&[&bytes[..cut], &bytes[cut..], done]);
        assert_eq!(seen, vec!["whānau"]);
        assert_eq!(p.into_result().unwrap(), "whānau");
    }

    #[test]
    fn error_after_content_keeps_partial() {
        let (p, _) = collect(&[
            b"{\"message\":{\"content\":\"Decisions: widen the doors.\"}}\n",
            b"{\"error\":\"runner crashed\"}\n",
        ]);
        let out = p.into_result().unwrap();
        assert!(out.starts_with("Decisions: widen the doors."));
        assert!(out.contains("incomplete"));
    }

    #[test]
    fn error_before_content_is_an_error() {
        let (p, _) = collect(&[b"{\"error\":\"model not found\"}\n"]);
        assert!(p.into_result().is_err());
    }

    #[test]
    fn eof_without_done_is_marked() {
        let (p, _) = collect(&[b"{\"message\":{\"content\":\"Half the notes\"}}\n"]);
        let out = p.into_result().unwrap();
        assert!(out.contains("Half the notes") && out.contains("incomplete"));
    }

    #[test]
    fn tail_without_newline_is_consumed() {
        let (p, seen) = collect(&[
            b"{\"message\":{\"content\":\"Kia \"}}\n",
            b"{\"message\":{\"content\":\"ora\"},\"done\":true,\"done_reason\":\"stop\"}",
        ]);
        assert_eq!(seen.concat(), "Kia ora");
        assert_eq!(p.into_result().unwrap(), "Kia ora");
    }

    #[test]
    fn length_stop_is_marked() {
        let (p, _) = collect(&[
            b"{\"message\":{\"content\":\"Actions:\"},\"done\":true,\"done_reason\":\"length\"}\n",
        ]);
        let out = p.into_result().unwrap();
        assert!(out.contains("output-length limit"));
    }

    #[test]
    fn bounded_line_and_output() {
        let mut p = Ndjson::default();
        let mut cb = |_: &str| {};
        let big = vec![b'x'; MAX_NDJSON_LINE + 10];
        assert!(p.feed(&big, &mut cb).is_err());

        let mut p = Ndjson::default();
        let piece = "y".repeat(1 << 16);
        let line = format!("{{\"message\":{{\"content\":\"{piece}\"}}}}\n");
        let mut failed = false;
        for _ in 0..80 {
            if p.feed(line.as_bytes(), &mut cb).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "4 MiB output cap must trip");
    }

    #[test]
    fn oom_detection() {
        assert!(is_oom(500, "{\"error\":\"CUDA error: out of memory\"}"));
        assert!(is_oom(
            500,
            "model requires more system memory (12 GiB) than is available"
        ));
        assert!(!is_oom(404, "{\"error\":\"model not found\"}"));
    }

    // ---- live tests (legion): run with
    // cargo test --lib ollama_chat::tests::live -- --ignored --nocapture --test-threads=1

    /// A fictional meeting with a code word planted at each end (same shape as
    /// the Phase 1.5 probe). ~40 000 characters.
    fn fictional_transcript() -> String {
        let people = [
            "Aroha", "Mere", "Tama", "Hemi", "Sione", "Priya", "Lena", "Wiremu",
        ];
        let topics = [
            "the community hall roof",
            "the bus timetable",
            "the library hours",
            "the school gala",
            "the walking track",
            "the stormwater report",
            "the volunteer roster",
            "the marae kitchen",
        ];
        let verbs = [
            "thinks we should revisit",
            "wants a quote for",
            "will follow up on",
            "is worried about",
            "has a draft for",
            "suggests we delay",
        ];
        let mut seed: u64 = 143;
        let mut next = |n: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % n
        };
        let mut out = String::from(
            "You: Kia ora koutou. Before we start, the first code word is KAHURANGI-7.\n",
        );
        while out.len() < 40_000 {
            let p = people[next(people.len())];
            let q = people[next(people.len())];
            let v = verbs[next(verbs.len())];
            let t = topics[next(topics.len())];
            let w = 2 + next(8);
            out.push_str(&format!(
                "{p}: {q} {v} {t}, and I agree it matters for the next {w} weeks.\n"
            ));
        }
        out.push_str(
            "You: Last thing before we close: the last code word is TOTARA-3. Ngā mihi.\n",
        );
        out
    }

    fn live_model() -> String {
        std::env::var("KORERO_LIVE_MODEL").unwrap_or_else(|_| "gemma4:12b".to_string())
    }

    fn live_provider() -> PostProcessProvider {
        crate::settings::get_default_settings()
            .post_process_providers
            .into_iter()
            .find(|p| p.id == "ollama")
            .expect("built-in ollama provider")
    }

    /// Put the model into the bug state: loaded at Ollama's 4k default.
    async fn load_at(base: &str, model: &str, ctx: u32) {
        let c = reqwest::Client::new();
        let _ = c
            .post(format!("{base}/api/generate"))
            .json(&json!({"model": model, "keep_alive": 0}))
            .send()
            .await;
        let _ = c
            .post(format!("{base}/api/generate"))
            .json(&json!({"model": model, "prompt": "", "keep_alive": "10m", "options": {"num_ctx": ctx}}))
            .send()
            .await;
    }

    const ASK: &str = "Answer from the transcript only. Reply with exactly two lines: \
        'first: <code word>' and 'last: <code word>'.";

    fn one_line(s: &str) -> String {
        s.replace('\n', " | ")
    }

    #[test]
    #[ignore = "live: needs a local Ollama with the model (legion)"]
    fn live_long_transcript_production_paths() {
        tauri::async_runtime::block_on(async {
            let provider = live_provider();
            let model = live_model();
            let base = native_base(&provider.base_url);
            let transcript = fictional_transcript();
            load_at(&base, &model, 4_096).await;
            assert_eq!(
                loaded_context(&base, &model).await,
                Some(4_096),
                "bug state not reached"
            );

            // Meeting notes: the streamed production path.
            let notes = crate::llm_client::stream_chat_completion(
                &provider,
                String::new(),
                &model,
                transcript.clone(),
                Some(ASK.to_string()),
                |_| {},
            )
            .await
            .expect("notes call");
            println!(
                "LIVE notes ({} chars in): {}",
                transcript.len(),
                one_line(&notes)
            );
            assert!(
                notes.contains("KAHURANGI") && notes.contains("TOTARA"),
                "notes missed an end"
            );

            // Ask: the non-streamed production path with its own budget.
            let ask = crate::llm_client::send_chat_completion_with_schema(
                &provider,
                String::new(),
                &model,
                transcript.clone(),
                Some(ASK.to_string()),
                None,
                None,
                None,
                CallKind::Ask,
            )
            .await
            .expect("ask call")
            .unwrap_or_default();
            println!("LIVE ask: {}", one_line(&ask));
            assert!(
                ask.contains("KAHURANGI") && ask.contains("TOTARA"),
                "Ask missed an end"
            );
        });
    }

    #[test]
    #[ignore = "live: needs a local Ollama with the model (legion)"]
    fn live_no_reload_across_a_session() {
        tauri::async_runtime::block_on(async {
            let provider = live_provider();
            let model = live_model();
            let base = native_base(&provider.base_url);
            let transcript = fictional_transcript();
            load_at(&base, &model, 4_096).await;
            // Warm for this transcript's notes, as meeting_prewarm_post_process does.
            let prompt = estimate_tokens(&"a".repeat(transcript.len())) + 512;
            warm(
                &provider.base_url,
                &model,
                prompt,
                CallKind::Meeting.num_predict(),
            )
            .await;
            let warmed = loaded_context(&base, &model).await;
            println!("LIVE warmed at {warmed:?}");
            let short = "um so the hui is on tuesday no wednesday at the marae".to_string();
            let calls = [
                (short.clone(), CallKind::Dictation),
                (transcript.clone(), CallKind::Meeting),
                (short.clone(), CallKind::Dictation),
                (transcript.clone(), CallKind::Ask),
            ];
            let mut seen = Vec::new();
            for (i, (user, kind)) in calls.into_iter().enumerate() {
                let t = std::time::Instant::now();
                let r = chat(
                    &provider,
                    "",
                    &model,
                    Some("Reply with one short sentence."),
                    &user,
                    kind,
                )
                .await;
                assert!(r.is_ok(), "call {i} failed: {r:?}");
                let ctx = loaded_context(&base, &model).await;
                println!(
                    "LIVE call {i} {kind:?}: {:.1}s, context {ctx:?}",
                    t.elapsed().as_secs_f32()
                );
                seen.push(ctx);
            }
            assert!(
                seen.iter().all(|c| *c == warmed),
                "the model was reloaded: {seen:?} vs {warmed:?}"
            );
        });
    }

    #[test]
    fn budgets() {
        assert_eq!(CallKind::Dictation.budget(10_000), DICTATION_TIMEOUT);
        assert!(CallKind::Ask.budget(18_000) > CallKind::Ask.budget(1_000));
        assert!(CallKind::Meeting.budget(u32::MAX) <= Duration::from_secs(900));
        assert_eq!(CallKind::Meeting.num_predict(), 8_192);
        assert_eq!(CallKind::Ask.num_predict(), 1_500);
    }
}
