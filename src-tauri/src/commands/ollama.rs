//! Kōrero (v1.3.0): Ollama local model management commands.
//! Provides in-app model pull via Ollama's native /api/pull endpoint,
//! streaming progress back to the frontend as "ollama-pull-progress" events.

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Progress payload for the "ollama-pull-progress" event.
/// Mirrors the NDJSON fields Ollama streams from /api/pull.
#[derive(Debug, Serialize, Clone)]
pub struct OllamaPullProgress {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
}

/// Pull an Ollama model by streaming Ollama's native /api/pull endpoint.
///
/// `base_url`   — the OpenAI-compat v1 URL stored in settings, e.g.
///                "http://localhost:11434/v1".  The native Ollama base is
///                derived by stripping the trailing /v1 path component.
/// `model_name` — the model tag to pull, e.g. "gemma3:4b".
///
/// Emits "ollama-pull-progress" events (payload: OllamaPullProgress) for
/// each NDJSON line received. Returns Ok(()) when Ollama signals completion,
/// Err(String) on connection failure, HTTP error, or Ollama stream error.
#[tauri::command]
#[specta::specta]
pub async fn pull_ollama_model(
    app: AppHandle,
    base_url: String,
    model_name: String,
) -> Result<(), String> {
    // Derive the native Ollama base URL from the OpenAI-compat v1 URL.
    //   "http://localhost:11434/v1"  → "http://localhost:11434"
    //   "http://localhost:11434/v1/" → "http://localhost:11434"
    let native_base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();

    let pull_url = format!("{}/api/pull", native_base);

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model_name,
        "stream": true
    });

    let response = client
        .post(&pull_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to connect to Ollama at {}: {}", pull_url, e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response
            .text()
            .await
            .unwrap_or_else(|_| "no body".to_string());
        return Err(format!(
            "Ollama returned HTTP {} from {}: {}",
            status, pull_url, text
        ));
    }

    // Stream the NDJSON response; emit one event per complete line.
    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("Stream read error: {}", e))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // Drain every newline-terminated JSON object from the buffer.
        // buf.drain(..=nl) removes bytes in-place rather than allocating a new
        // String for the remainder on every line — important for large model pulls
        // that emit hundreds of NDJSON progress lines.
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim().to_string();
            buf.drain(..=nl);

            if line.is_empty() {
                continue;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                // An "error" field in the stream means Ollama failed.
                if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
                    return Err(format!("Ollama pull error: {}", err));
                }

                let progress = OllamaPullProgress {
                    status: json
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    digest: json
                        .get("digest")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    total: json.get("total").and_then(|v| v.as_u64()),
                    completed: json.get("completed").and_then(|v| v.as_u64()),
                };

                let _ = app.emit("ollama-pull-progress", &progress);
            }
        }
    }

    Ok(())
}

/// Check whether Ollama is reachable by probing its /api/tags endpoint.
///
/// `base_url` — the OpenAI-compat v1 URL stored in settings, e.g.
///              "http://localhost:11434/v1".  The native Ollama base is
///              derived by stripping the trailing /v1 path component.
///
/// Returns `true` if Ollama responds with a 2xx status within 4 seconds,
/// `false` on any connection failure or timeout.  Never returns an error —
/// a failed probe is `false`, not an exception.
///
/// Note: this command exists because the frontend cannot use fetch() to probe
/// http://localhost — WebView2 CSP blocks non-listed origins.  All HTTP to
/// local/external services goes through reqwest here in Rust.
#[tauri::command]
#[specta::specta]
pub async fn check_ollama_connection(base_url: String) -> bool {
    let native_base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();

    let tags_url = format!("{}/api/tags", native_base);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    client
        .get(&tags_url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}
