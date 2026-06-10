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
// Kōrero (v1.17.0): meeting post-processing summarises a whole transcript, which
// can run well past 30s on a local model — long enough that the old 30s cap
// truncated the result mid-summary. Streaming makes the wait visible, and this
// longer ceiling is an idle/total guard against a genuinely wedged provider,
// not a per-token deadline.
const LLM_STREAM_TIMEOUT: Duration = Duration::from_secs(300);

// Kōrero (v1.5.0): cap generated tokens for post-processing prompts.
// Transcription outputs are short; 1500 tokens is generous and prevents
// slow responses from models that run long on ambiguous prompts.
// Providers that ignore max_tokens (e.g. some local Ollama configs) are unaffected.
const DEFAULT_PP_MAX_TOKENS: u32 = 1500;

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

/// Send a chat completion request with structured output support
/// When json_schema is provided, uses structured outputs mode
/// system_prompt is used as the system message when provided
/// reasoning_effort sets the OpenAI-style top-level field (e.g., "none", "low", "medium", "high")
/// reasoning sets the OpenRouter-style nested object (effort + exclude)
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
    // Kōrero (v1.13.x) egress allowlist: transcripts can be confidential, so for
    // providers whose URL is NOT user-editable, refuse to send if the base_url has
    // been altered from its built-in default (e.g. a tampered settings_store.json
    // pointing at an exfiltration host). User-owned providers (custom / local
    // Ollama, allow_base_url_edit = true) are intentionally exempt.
    if !provider.allow_base_url_edit {
        let defaults = crate::settings::get_default_settings();
        if let Some(def) = defaults
            .post_process_providers
            .iter()
            .find(|p| p.id == provider.id)
        {
            if def.base_url.trim_end_matches('/') != provider.base_url.trim_end_matches('/') {
                return Err(format!(
                    "Blocked: the endpoint for provider '{}' was altered to an unexpected URL. \
                     Transcripts are not sent to unverified hosts.",
                    provider.id
                ));
            }
        }
    }

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

/// v1.17.0: streaming chat completion for meeting post-processing. Sends
/// `stream: true`, parses the SSE `data:` events, and invokes `on_delta` with
/// each incremental content chunk as it arrives — so the UI can render the
/// summary as it's generated instead of waiting for the whole thing. Returns
/// the fully-assembled text. `on_delta` is called on the async task; keep it
/// cheap (e.g. emit a Tauri event).
///
/// Only `system` + `user` messages and `max_tokens` are sent — meeting
/// post-processing doesn't use structured output or reasoning fields.
pub async fn stream_chat_completion<F: FnMut(&str)>(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    mut on_delta: F,
) -> Result<String, String> {
    use futures_util::StreamExt;

    // Same egress allowlist as the non-streaming path: never send a transcript
    // to a tampered endpoint for a provider whose URL isn't user-editable.
    if !provider.allow_base_url_edit {
        let defaults = crate::settings::get_default_settings();
        if let Some(def) = defaults
            .post_process_providers
            .iter()
            .find(|p| p.id == provider.id)
        {
            if def.base_url.trim_end_matches('/') != provider.base_url.trim_end_matches('/') {
                return Err(format!(
                    "Blocked: the endpoint for provider '{}' was altered to an unexpected URL. \
                     Transcripts are not sent to unverified hosts.",
                    provider.id
                ));
            }
        }
    }

    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);
    debug!("Sending STREAMING chat completion request to: {}", url);

    // Dedicated client with the longer streaming ceiling (see LLM_STREAM_TIMEOUT).
    let headers = build_headers(provider, &api_key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(LLM_STREAM_TIMEOUT)
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
        max_tokens: Some(DEFAULT_PP_MAX_TOKENS),
        stream: Some(true),
    };

    let mut response = client.post(&url).json(&request_body).send().await;
    // Self-healing for a stopped local Ollama, mirroring the non-stream path.
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

    // Parse the SSE stream incrementally. Frames are `data: {json}\n\n`,
    // terminated by `data: [DONE]`. A frame can split across chunks, so buffer
    // bytes and only consume complete `\n\n`-delimited records.
    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let mut full = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream read failed: {}", e))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buf.find("\n\n") {
            let frame = buf[..idx].to_string();
            buf.drain(..idx + 2);
            for line in frame.lines() {
                let line = line.trim_start();
                let payload = match line.strip_prefix("data:") {
                    Some(p) => p.trim(),
                    None => continue, // comments / event: lines — ignore
                };
                if payload == "[DONE]" {
                    return Ok(full);
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
        }
    }
    // Stream closed. Flush any trailing frame that arrived without a final
    // blank-line terminator (some servers just close the socket after the last
    // delta), so the closing tokens aren't lost.
    for line in buf.lines() {
        let payload = match line.trim_start().strip_prefix("data:") {
            Some(p) => p.trim(),
            None => continue,
        };
        if payload == "[DONE]" {
            break;
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
    Ok(full)
}

/// Fetch available models from an OpenAI-compatible API
/// Returns a list of model IDs
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
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