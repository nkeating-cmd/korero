use crate::settings::PostProcessProvider;
use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

// Kōrero: bound LLM HTTP calls so a dead provider can't hang the post-process
// pipeline forever. 30s total covers slow reasoning models (DeepSeek-R1, o1)
// without becoming a UI freeze; 5s connect catches DNS / TLS failure fast.
const LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const LLM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
// Kōrero (v1.30.2): the v1.17.0 comment here was WRONG, and the error it
// described was the main reason meeting notes "often don't generate".
//
// `reqwest::ClientBuilder::timeout` bounds the WHOLE request — headers *and*
// the streamed body — so a 300 s ceiling is a hard 5-minute deadline on the
// entire generation, not the idle guard it was documented as. A local 12B
// model writing "detailed, well-structured minutes" for an hour-long meeting
// runs past five minutes routinely. When it did, reqwest aborted the body
// stream, the `?` on the chunk threw away every token already produced, and
// the user got a failure toast after watching several thousand words stream
// into the preview. Longer meeting ⇒ more likely to fail, which is exactly
// the "intermittent" pattern reported.
//
// The bound that was actually wanted is the gap BETWEEN chunks: if a provider
// sends nothing for this long it is wedged; if it keeps producing tokens it is
// working and must be left alone. `LLM_STREAM_TOTAL_CEILING` is a backstop
// against a pathological endless stream, deliberately far above any real run.
//
// 180 s of idle covers the worst legitimate silence: Ollama cold-loading a 12B
// model and prompt-processing a 48 000-character transcript before the first
// token appears.
const LLM_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(180);
const LLM_STREAM_TOTAL_CEILING: Duration = Duration::from_secs(3600);

// Kōrero (v1.30.2): the non-streaming SAFETY NET below a failed stream was
// doubly wrong for meetings — it inherited the 30 s total timeout and the
// 1500-token cap, so on the rare provider that ignores `stream: true` it could
// only ever return a truncated summary, slowly, before timing out. Meetings
// get their own ceiling.
const LLM_MEETING_FALLBACK_TIMEOUT: Duration = Duration::from_secs(900);

// Kōrero (v1.5.0): cap generated tokens for post-processing prompts.
// Transcription outputs are short; 1500 tokens is generous and prevents
// slow responses from models that run long on ambiguous prompts.
// Providers that ignore max_tokens (e.g. some local Ollama configs) are unaffected.
const DEFAULT_PP_MAX_TOKENS: u32 = 1500;

// Kōrero (v1.20.0): meeting post-processing often CLEANS a whole transcript,
// where the output length is roughly the input length — far longer than a
// summary. The shared 1500-token cap silently truncated cleaned meeting
// transcripts part-way through. The streaming path (used ONLY by meeting
// post-processing) gets a much higher ceiling; the idle timeout above remains
// the real guard against a wedged provider.
const MEETING_PP_MAX_TOKENS: u32 = 8192;

/// Kōrero (v1.30.2): a streamed generation that stops early must never be
/// thrown away. Several thousand words of usable minutes had already rendered
/// in the preview when the old code hit `?` and returned an error — the user
/// watched the notes being written and then got nothing.
///
/// Partial output is saved, but it is never passed off as complete: the marker
/// is appended so the saved notes say plainly that they were cut short and why.
/// Only a genuinely empty result is an error.
fn partial_or_error(full: String, why: &str) -> Result<String, String> {
    if full.trim().is_empty() {
        Err(format!("The provider produced no output — {why}."))
    } else {
        Ok(format!(
            "{full}\n\n---\n\n*These notes are incomplete — {why}. \
             Everything above was generated before it stopped; \
             re-run post-processing to try for the rest.*"
        ))
    }
}

// Kōrero (2026-05-17 PM, T2.2): User-Agent / X-Title pinned to the package
// version at compile time so the headers track Cargo.toml automatically. Was
// previously hardcoded "Korero/0.8.3" — a doc-and-code drift waiting to
// happen. Referer still credits upstream Handy as a courtesy.
const KORERO_USER_AGENT: &str = concat!(
    "Korero/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/cjpais/Handy)"
);
const KORERO_X_TITLE: &str = "Korero";

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    // Kōrero (v1.5.0): caps generation length to bound PP latency for short transcriptions.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    // Kōrero (v1.17.0): request SSE token streaming (meeting post-processing).
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

/// Build headers for API requests based on provider type
fn build_headers(provider: &PostProcessProvider, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    // Common headers
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    // Kōrero fork: own user-agent + title for analytics on the LLM providers.
    // Referer kept pointing at upstream as a credit to cjpais.
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/cjpais/Handy"),
    );
    headers.insert(USER_AGENT, HeaderValue::from_static(KORERO_USER_AGENT));
    headers.insert("X-Title", HeaderValue::from_static(KORERO_X_TITLE));

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == "anthropic" {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
        }
    }

    Ok(headers)
}

/// Create an HTTP client with provider-specific headers.
///
/// Kōrero adds explicit timeouts. Without them, reqwest will wait indefinitely
/// for a slow provider, blocking the transcription post-process flow and any
/// downstream UI state. Defaults are conservative — long enough for reasoning
/// models on a slow link, short enough to surface real failures.
fn create_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(LLM_REQUEST_TIMEOUT)
        .connect_timeout(LLM_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Send a chat completion request to an OpenAI-compatible API
/// Returns Ok(Some(content)) on success, Ok(None) if response has no content,
/// or Err on actual errors (HTTP, parsing, etc.)
pub async fn send_chat_completion(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(
        provider,
        api_key,
        model,
        prompt,
        None,
        None,
        reasoning_effort,
        reasoning,
    )
    .await
}

/// Kōrero (v1.13.x) egress allowlist, hoisted into one function in v1.29.0 (R-05).
///
/// Transcripts can be confidential, so for providers whose URL is NOT
/// user-editable we refuse to send if `base_url` has been altered from its
/// built-in default (e.g. a tampered `settings_store.json` pointing at an
/// exfiltration host). User-owned providers (custom / local Ollama,
/// `allow_base_url_edit = true`) are intentionally exempt.
///
/// WHY THIS IS A FUNCTION AND NOT THREE COPIES. Until v1.29.0 this block was
/// inlined in `send_chat_completion_with_schema` and `stream_chat_completion`
/// — and *absent* from `fetch_models`, which sends the same `Authorization` /
/// `x-api-key` headers. A tampered `base_url` therefore leaked the API key the
/// moment the model dropdown refreshed. That is strictly worse than the
/// transcript leak this allowlist was written to prevent: a transcript is one
/// conversation, a key is every future one.
///
/// **Every new outbound call that carries credentials must call this first.**
/// The `llm-egress-allowlist-complete` check in `checks.json` counts the call
/// sites and fails if a `create_client(` appears without one.
pub(crate) fn assert_endpoint_unmodified(provider: &PostProcessProvider) -> Result<(), String> {
    if provider.allow_base_url_edit {
        return Ok(());
    }
    let defaults = crate::settings::get_default_settings();
    let Some(def) = defaults
        .post_process_providers
        .iter()
        .find(|p| p.id == provider.id)
    else {
        // Unknown provider id: not a built-in, so there is no default to compare
        // against. Treat as user-owned rather than blocking a legitimate custom
        // provider that happens to have allow_base_url_edit unset.
        return Ok(());
    };
    if def.base_url.trim_end_matches('/') != provider.base_url.trim_end_matches('/') {
        return Err(format!(
            "Blocked: the endpoint for provider '{}' was altered to an unexpected URL. \
             Transcripts are not sent to unverified hosts.",
            provider.id
        ));
    }
    Ok(())
}

/// Send a chat completion request with structured output support.
/// When json_schema is provided, uses structured outputs mode.
/// system_prompt is used as the system message when provided.
/// reasoning_effort sets the OpenAI-style top-level field (e.g., "none", "low", "medium", "high").
/// reasoning sets the OpenRouter-style nested object (effort + exclude).
pub async fn send_chat_completion_with_schema(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    assert_endpoint_unmodified(provider)?;

    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    // Kōrero log-exposure rule (audit 2026-05-17):
    //   LOG: URL, model id, response status. SAFE — keys live in headers.
    //   NEVER LOG: api_key, build_headers() output, request body, response body,
    //   user_content (transcript text — privacy), or anything containing
    //   `messages`. Adding such a log statement undoes the keychain migration.
    debug!("Sending chat completion request to: {}", url);

    let client = create_client(provider, &api_key)?;

    // Build messages vector
    let mut messages = Vec::new();

    // Add system prompt if provided
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system,
        });
    }

    // Add user message
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content,
    });

    // Build response_format if schema is provided
    let response_format = json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    let request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        response_format,
        reasoning_effort,
        reasoning,
        // Kōrero (v1.5.0): cap tokens to bound post-processing latency.
        max_tokens: Some(DEFAULT_PP_MAX_TOKENS),
        stream: None,
    };

    let mut response = client.post(&url).json(&request_body).send().await;

    // Kōrero (v1.17.0): self-healing for a stopped local Ollama. Only a
    // CONNECTION-level failure to a local provider triggers this — an HTTP
    // error from a running server must surface normally. One restart attempt,
    // one retry.
    if let Err(e) = &response {
        if e.is_connect() && provider.is_local_provider {
            log::info!("Local LLM provider unreachable — attempting to start Ollama and retry.");
            if crate::commands::ollama::ensure_running(&provider.base_url).await {
                response = client.post(&url).json(&request_body).send().await;
            }
        }
    }
    let response = response.map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    Ok(completion
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone()))
}

/// Kōrero (v1.30.2): the non-streaming safety net used when a provider ignores
/// `stream: true`. Deliberately NOT `send_chat_completion_with_schema`: that one
/// is tuned for dictation clean-up (30 s, 1500 tokens), and reusing it here meant
/// the fallback could only ever return a truncated summary, slowly, before its
/// own timeout fired. Same request shape as the streaming call — same token
/// ceiling, a meeting-sized timeout, no schema or reasoning fields.
pub async fn send_chat_completion_meeting(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
) -> Result<String, String> {
    assert_endpoint_unmodified(provider)?;

    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);
    debug!("Sending NON-STREAMING meeting completion request to: {}", url);

    let headers = build_headers(provider, &api_key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(LLM_MEETING_FALLBACK_TIMEOUT)
        .connect_timeout(LLM_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let mut messages = Vec::new();
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system,
        });
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content,
    });

    let request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        response_format: None,
        reasoning_effort: None,
        reasoning: None,
        max_tokens: Some(MEETING_PP_MAX_TOKENS),
        stream: None,
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    Ok(completion
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default())
}

/// v1.17.0: streaming chat completion for meeting post-processing. Sends
/// `stream: true`, parses the SSE `data:` events, and invokes `on_delta` with
/// each incremental content chunk as it arrives — so the UI can render the
/// summary as it's generated instead of waiting for the whole thing. Returns
/// the fully-assembled text. `on_delta` is called on the async task; keep it
/// cheap (e.g. emit a Tauri event).
///
/// Only `system` + `user` messages and `max_tokens` are sent — meeting
/// post-processing doesn't use structured output or reasoning fields.
///
/// Distinguishes "the HTTP call failed" from "the provider went silent", so the
/// Ollama self-heal retry only fires on the former.
enum StreamSendError {
    Http(reqwest::Error),
    Timeout,
}

pub async fn stream_chat_completion<F: FnMut(&str)>(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    mut on_delta: F,
) -> Result<String, String> {
    use futures_util::StreamExt;

    assert_endpoint_unmodified(provider)?;

    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);
    debug!("Sending STREAMING chat completion request to: {}", url);

    // Kōrero (v1.30.2): NO total `.timeout()` here — see LLM_STREAM_IDLE_TIMEOUT.
    // A whole-request deadline is the wrong shape for a generation whose length
    // is the user's transcript length. The idle guard is applied per chunk in
    // the read loop below, and the initial `send()` is bounded separately, so a
    // provider that never answers at all still fails fast.
    let headers = build_headers(provider, &api_key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(LLM_CONNECT_TIMEOUT)
        // Review follow-up: removing the total timeout must not leave an
        // UNBOUNDED wait. `read_timeout` is the per-read equivalent of the idle
        // guard below and, unlike `timeout`, it also covers reads this function
        // performs outside the streaming loop — notably `response.text()` on a
        // non-success status, where a provider that sends 503 headers and then
        // stalls the body would otherwise hang forever. A hang there is worse
        // than a failure: the command never resolves, so the job stays
        // "running" and every button stays disabled for the rest of the session.
        .read_timeout(LLM_STREAM_IDLE_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let mut messages = Vec::new();
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system,
        });
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content,
    });

    let request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        response_format: None,
        reasoning_effort: None,
        reasoning: None,
        max_tokens: Some(MEETING_PP_MAX_TOKENS),
        stream: Some(true),
    };

    // Kōrero (v1.30.2): the client no longer carries a total timeout, so bound
    // the handshake explicitly. This covers "provider accepted the TCP
    // connection and then went silent", which connect_timeout does not.
    let send_once = |c: &reqwest::Client| {
        let fut = c.post(&url).json(&request_body).send();
        async move {
            match tokio::time::timeout(LLM_STREAM_IDLE_TIMEOUT, fut).await {
                Ok(r) => r.map_err(StreamSendError::Http),
                Err(_) => Err(StreamSendError::Timeout),
            }
        }
    };
    let mut response = send_once(&client).await;
    // Self-healing for a stopped local Ollama, mirroring the non-stream path.
    if let Err(StreamSendError::Http(e)) = &response {
        if e.is_connect() && provider.is_local_provider {
            log::info!("Local LLM provider unreachable — attempting to start Ollama and retry.");
            if crate::commands::ollama::ensure_running(&provider.base_url).await {
                response = send_once(&client).await;
            }
        }
    }
    let response = response.map_err(|e| match e {
        StreamSendError::Http(e) => format!("HTTP request failed: {}", e),
        StreamSendError::Timeout => format!(
            "The provider accepted the connection but sent no response within {} s.",
            LLM_STREAM_IDLE_TIMEOUT.as_secs()
        ),
    })?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    // Parse the SSE stream incrementally. Frames are `data: {json}\n\n`,
    // terminated by `data: [DONE]`. A frame can split across chunks, so buffer
    // bytes and only consume complete `\n\n`-delimited records.
    let mut stream = response.bytes_stream();
    // Review follow-up: the buffer is BYTES, not a String.
    //
    // `bytes_stream()` splits at arbitrary byte offsets, so a multi-byte
    // character can straddle two chunks. Decoding each chunk with
    // `String::from_utf8_lossy` therefore turned any such character into U+FFFD
    // — silently, and in an app whose whole pitch is te reo Māori, where `ā`
    // (2 bytes) and `—` (3 bytes) are everywhere. Removing the 5-minute
    // deadline makes long generations the normal case, which means more chunks
    // and more boundaries, so this got MORE likely, not less.
    //
    // Accumulating bytes and decoding only complete `\n\n`-delimited frames
    // means a split character is simply still in the buffer when the next chunk
    // arrives. Frame text is valid UTF-8 by then, so the lossy decode has
    // nothing to mangle.
    let mut bytes: Vec<u8> = Vec::new();
    let mut full = String::new();
    let started = std::time::Instant::now();
    loop {
        // Kōrero (v1.30.2): idle guard, not a deadline. The clock restarts on
        // every chunk, so a model that keeps producing tokens is never cut off.
        let chunk = match tokio::time::timeout(LLM_STREAM_IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => {
                // A mid-stream transport failure. Keep what was generated.
                log::warn!("Streaming post-process ended early: {}", e);
                return partial_or_error(full, &format!("the stream failed ({e})"));
            }
            Ok(None) => break, // provider closed the stream normally
            Err(_) => {
                log::warn!(
                    "Streaming post-process idle for {} s — treating the provider as wedged.",
                    LLM_STREAM_IDLE_TIMEOUT.as_secs()
                );
                return partial_or_error(
                    full,
                    &format!(
                        "the provider stopped sending for {} s",
                        LLM_STREAM_IDLE_TIMEOUT.as_secs()
                    ),
                );
            }
        };
        if started.elapsed() > LLM_STREAM_TOTAL_CEILING {
            log::warn!("Streaming post-process hit the absolute ceiling.");
            return partial_or_error(
                full,
                &format!(
                    "it ran past the {} minute absolute ceiling",
                    LLM_STREAM_TOTAL_CEILING.as_secs() / 60
                ),
            );
        }
        bytes.extend_from_slice(&chunk);
        while let Some(idx) = find_frame_end(&bytes) {
            // Safe to decode: a frame boundary is a character boundary, so no
            // multi-byte sequence is split here.
            let frame = String::from_utf8_lossy(&bytes[..idx]).into_owned();
            bytes.drain(..idx + 2);
            if consume_sse_frame(&frame, &mut full, &mut on_delta) {
                return Ok(full);
            }
        }
    }
    // Stream closed. Flush any trailing frame that arrived without a final
    // blank-line terminator (some servers just close the socket after the last
    // delta), so the closing tokens aren't lost.
    let tail = String::from_utf8_lossy(&bytes).into_owned();
    consume_sse_frame(&tail, &mut full, &mut on_delta);
    Ok(full)
}

/// Offset of the next `\n\n` frame terminator in the raw byte buffer.
///
/// Searching BYTES rather than a decoded string is what keeps a multi-byte
/// character split across two network chunks intact — see the comment in
/// `stream_chat_completion`. `\n` is ASCII and can never appear inside a
/// multi-byte UTF-8 sequence, so a byte search cannot produce a false match.
fn find_frame_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|w| w == b"\n\n")
}

/// Parse one SSE frame, appending any content deltas to `full` and handing each
/// to `on_delta`. Returns true when the frame carried the `[DONE]` sentinel.
///
/// Extracted so the streaming loop and the end-of-stream flush share one
/// implementation — they had drifted into two near-copies, and only one of them
/// treated `[DONE]` as a terminator.
fn consume_sse_frame<F: FnMut(&str)>(frame: &str, full: &mut String, on_delta: &mut F) -> bool {
    for line in frame.lines() {
        let payload = match line.trim_start().strip_prefix("data:") {
            Some(p) => p.trim(),
            None => continue, // comments / `event:` lines — ignore
        };
        if payload == "[DONE]" {
            return true;
        }
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            if let Some(piece) = v["choices"][0]["delta"]["content"].as_str() {
                if !piece.is_empty() {
                    on_delta(piece);
                    full.push_str(piece);
                }
            }
        }
    }
    false
}

/// Fetch available models from an OpenAI-compatible API
/// Returns a list of model IDs
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    // Kōrero v1.29.0 (R-05): this call was missing the allowlist that both
    // completion paths enforced, and it sends the same credential headers.
    // A tampered base_url leaked the API key on every model-dropdown refresh.
    assert_endpoint_unmodified(provider)?;

    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/models", base_url);

    debug!("Fetching models from: {}", url);

    let client = create_client(provider, &api_key)?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!(
            "Model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let mut models = Vec::new();

    // Handle OpenAI format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // Handle array format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }

    Ok(models)
}

/// Kōrero (v1.30.2): the streaming-resilience contract. These lock in the two
/// behaviours whose absence made meeting notes "often not generate".
#[cfg(test)]
mod korero_v1_30_2_stream_resilience_tests {
    use super::*;

    #[test]
    fn korero_v1302_partial_output_is_kept_not_discarded() {
        // The regression: 4 000 words had streamed into the preview and the old
        // `?` threw all of it away. Partial must come back as Ok.
        let partial = "## Decisions\n\n- Ship the thing on Friday.".to_string();
        let out = partial_or_error(partial.clone(), "the stream failed (broken pipe)")
            .expect("partial output must be returned, not discarded");
        assert!(
            out.starts_with(&partial),
            "the generated text must be preserved verbatim at the start"
        );
    }

    #[test]
    fn korero_v1302_partial_output_says_it_is_partial() {
        // Keeping the text is only safe if it never passes as complete.
        let out = partial_or_error("some notes".to_string(), "the provider stopped sending")
            .expect("partial is Ok");
        assert!(
            out.contains("incomplete"),
            "saved partial notes must state that they are incomplete: {out}"
        );
        assert!(
            out.contains("the provider stopped sending"),
            "the reason must be carried through to the user: {out}"
        );
    }

    #[test]
    fn korero_v1302_empty_output_is_still_an_error() {
        // No text at all is a genuine failure and must not save empty notes.
        for empty in ["", "   ", "\n\n\t"] {
            assert!(
                partial_or_error(empty.to_string(), "the stream failed").is_err(),
                "an empty generation must remain an error, not save blank notes"
            );
        }
    }

    #[test]
    fn korero_v1302_idle_timeout_is_not_a_total_deadline() {
        // The whole bug in one assertion: the guard on a streamed generation
        // must be the gap between chunks, and the absolute backstop must be far
        // above any legitimate run. If someone reintroduces a ~5 minute total
        // ceiling, this fails.
        assert!(
            LLM_STREAM_IDLE_TIMEOUT >= Duration::from_secs(120),
            "the idle guard must tolerate a cold 12B model prompt-processing a \
             long transcript before the first token"
        );
        assert!(
            LLM_STREAM_TOTAL_CEILING >= Duration::from_secs(1800),
            "the absolute ceiling must not double as a generation deadline"
        );
        assert!(
            LLM_STREAM_TOTAL_CEILING > LLM_STREAM_IDLE_TIMEOUT * 4,
            "a ceiling close to the idle guard is a total deadline wearing a hat"
        );
    }

    #[test]
    fn korero_v1302_macron_split_across_chunks_is_not_corrupted() {
        // The review finding this locks in: `bytes_stream()` splits at
        // arbitrary byte offsets, so decoding each chunk on its own turned a
        // macron straddling the boundary into U+FFFD. In a te reo app that is
        // silent corruption of the notes.
        let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"whānau hapū — kōrero\"}}]}\n\n";
        let raw = frame.as_bytes();

        // Split at EVERY byte offset; a correct implementation survives all of them.
        for split in 1..raw.len() {
            let mut bytes: Vec<u8> = Vec::new();
            let mut full = String::new();
            let mut seen = String::new();
            let mut on_delta = |d: &str| seen.push_str(d);

            for chunk in [&raw[..split], &raw[split..]] {
                bytes.extend_from_slice(chunk);
                while let Some(idx) = find_frame_end(&bytes) {
                    let f = String::from_utf8_lossy(&bytes[..idx]).into_owned();
                    bytes.drain(..idx + 2);
                    consume_sse_frame(&f, &mut full, &mut on_delta);
                }
            }
            assert_eq!(
                full, "whānau hapū — kōrero",
                "a chunk boundary at byte {split} corrupted the text"
            );
            assert!(
                !full.contains('\u{FFFD}'),
                "replacement character produced at split {split}"
            );
            assert_eq!(seen, full, "on_delta and the accumulator disagree");
        }
    }

    #[test]
    fn korero_v1302_done_sentinel_terminates_the_frame() {
        let mut full = String::new();
        let mut on_delta = |_: &str| {};
        assert!(
            consume_sse_frame("data: [DONE]", &mut full, &mut on_delta),
            "[DONE] must be reported so the loop stops"
        );
        assert!(
            !consume_sse_frame(
                "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}",
                &mut full,
                &mut on_delta
            ),
            "an ordinary delta frame is not a terminator"
        );
        assert_eq!(full, "x");
    }

    #[test]
    fn korero_v1302_meeting_fallback_is_not_the_dictation_budget() {
        // The fallback inherited dictation's 30 s / 1500 tokens, which cannot
        // produce meeting minutes. Both must be meeting-sized.
        assert!(
            LLM_MEETING_FALLBACK_TIMEOUT > LLM_REQUEST_TIMEOUT,
            "the meeting fallback must not inherit the dictation timeout"
        );
        assert!(
            MEETING_PP_MAX_TOKENS > DEFAULT_PP_MAX_TOKENS * 4,
            "meeting notes need far more headroom than a dictation clean-up"
        );
    }
}

#[cfg(test)]
mod egress_allowlist_tests {
    use super::*;
    use crate::settings::{get_default_settings, PostProcessProvider};

    /// Take a real built-in provider and optionally rewrite its base_url,
    /// so the test exercises the same comparison the runtime does rather
    /// than a hand-rolled fixture that could drift from the defaults.
    fn builtin(id: &str) -> PostProcessProvider {
        get_default_settings()
            .post_process_providers
            .into_iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("no built-in provider with id '{id}' in default settings"))
    }

    #[test]
    fn korero_r05_unmodified_builtin_is_allowed() {
        for p in get_default_settings().post_process_providers {
            let id = p.id.clone();
            assert!(
                assert_endpoint_unmodified(&p).is_ok(),
                "a pristine built-in provider must never be blocked: {id}"
            );
        }
    }

    #[test]
    fn korero_r05_tampered_builtin_is_blocked() {
        // Pick the first built-in whose URL is NOT user-editable; if the
        // catalogue ever loses all of them this test must fail loudly rather
        // than vacuously pass.
        let locked: Vec<PostProcessProvider> = get_default_settings()
            .post_process_providers
            .into_iter()
            .filter(|p| !p.allow_base_url_edit)
            .collect();
        assert!(
            !locked.is_empty(),
            "no non-editable providers left in defaults — the allowlist would be inert"
        );

        for mut p in locked {
            let id = p.id.clone();
            p.base_url = "https://evil.example.com/v1".to_string();
            let err = assert_endpoint_unmodified(&p)
                .expect_err(&format!("tampered base_url must be blocked for '{id}'"));
            assert!(err.starts_with("Blocked:"), "unexpected error text: {err}");
            assert!(err.contains(&id), "error should name the provider: {err}");
        }
    }

    #[test]
    fn korero_r05_trailing_slash_is_not_tampering() {
        let mut p = builtin("openai");
        p.base_url = format!("{}/", p.base_url.trim_end_matches('/'));
        assert!(
            assert_endpoint_unmodified(&p).is_ok(),
            "a trailing slash must not be mistaken for tampering"
        );
    }

    #[test]
    fn korero_r05_user_owned_provider_is_exempt() {
        let mut p = builtin("openai");
        p.allow_base_url_edit = true;
        p.base_url = "http://127.0.0.1:11434/v1".to_string();
        assert!(
            assert_endpoint_unmodified(&p).is_ok(),
            "providers the user owns are intentionally exempt"
        );
    }

    #[test]
    fn korero_r05_unknown_provider_id_is_not_blocked() {
        let p = PostProcessProvider {
            id: "not-a-builtin".to_string(),
            label: "Custom".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: false,
            suggested_models: Vec::new(),
            is_local_provider: false,
        };
        assert!(
            assert_endpoint_unmodified(&p).is_ok(),
            "an id with no built-in default has nothing to compare against"
        );
    }
}
