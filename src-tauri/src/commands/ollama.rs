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

// ---------------------------------------------------------------------------
// Kōrero (v1.17.0): Ollama doctor — detect / install / start / self-heal.
// PC optimisers and reboots regularly leave Ollama stopped; previously the
// only signal was a red "not reachable" line and the fix was manual.
// ---------------------------------------------------------------------------

/// Install + run state of the local Ollama, for the UI.
#[derive(Debug, Serialize, Clone, specta::Type)]
pub struct OllamaStatus {
    pub installed: bool,
    pub running: bool,
    pub exe_path: Option<String>,
}

/// Probe the native API (same check as `check_ollama_connection`).
pub async fn is_reachable(base_url: &str) -> bool {
    check_ollama_connection(base_url.to_string()).await
}

/// v1.17.0: pre-load `model` into Ollama's memory and pin it there for
/// `keep_alive_secs`, via the NATIVE `/api/generate` endpoint.
///
/// Why native and not a request-body field: Ollama's OpenAI-compatible
/// `/v1/chat/completions` endpoint IGNORES `keep_alive` in the body (ollama
/// issue #11458) and falls back to its 5-minute default — so the only reliable
/// way to keep the post-processing model resident between runs is this native
/// warm call. Fire-and-forget: a cold first post-process otherwise pays the
/// multi-second model-load cost on top of generation. Best-effort; errors are
/// logged, never surfaced.
pub async fn warm_model(base_url: &str, model: &str, keep_alive_secs: u64) {
    if model.trim().is_empty() {
        return;
    }
    let native_base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();
    let url = format!("{}/api/generate", native_base);
    let body = serde_json::json!({
        "model": model,
        "prompt": "",                       // empty prompt = load only, no generation
        "keep_alive": format!("{keep_alive_secs}s"),
        "stream": false
    });
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("warm_model: client build failed: {e}");
            return;
        }
    };
    match client.post(&url).json(&body).send().await {
        Ok(r) if r.status().is_success() => {
            log::info!("Pre-warmed post-processing model '{model}' (keep_alive {keep_alive_secs}s).");
        }
        Ok(r) => log::warn!("warm_model: Ollama returned HTTP {} from {url}", r.status()),
        Err(e) => log::warn!("warm_model: could not reach {url}: {e}"),
    }
}

/// Locate the Ollama executable. Prefers the GUI app ("ollama app.exe" —
/// starting it brings up the tray AND the server), falls back to the CLI,
/// then to a PATH probe (catches custom installs; cross-platform).
fn find_ollama_exe() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let dir = std::path::Path::new(&local).join("Programs").join("Ollama");
            let gui = dir.join("ollama app.exe");
            if gui.exists() {
                return Some(gui);
            }
            let cli = dir.join("ollama.exe");
            if cli.exists() {
                return Some(cli);
            }
        }
    }
    let finder = if cfg!(windows) { "where.exe" } else { "which" };
    let mut cmd = std::process::Command::new(finder);
    cmd.arg("ollama");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| std::path::PathBuf::from(l.trim()))
        .filter(|p| p.exists())
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_status(base_url: String) -> OllamaStatus {
    let exe = find_ollama_exe();
    OllamaStatus {
        installed: exe.is_some(),
        running: is_reachable(&base_url).await,
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
    }
}

/// Start Ollama (GUI app preferred, `ollama serve` fallback) and wait up to
/// ~20 s for the API to come up. Ok(true) = reachable.
pub async fn start_and_wait(base_url: &str) -> Result<bool, String> {
    if is_reachable(base_url).await {
        return Ok(true);
    }
    let exe = find_ollama_exe()
        .ok_or_else(|| "Ollama doesn't appear to be installed on this machine.".to_string())?;
    let mut cmd = std::process::Command::new(&exe);
    if exe.file_name().and_then(|n| n.to_str()) == Some("ollama.exe") {
        cmd.arg("serve");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (harmless for the GUI app)
    }
    cmd.spawn()
        .map_err(|e| format!("Could not start Ollama ({}): {e}", exe.display()))?;
    for _ in 0..40 {
        let _ = tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(500))
        })
        .await;
        if is_reachable(base_url).await {
            return Ok(true);
        }
    }
    Ok(false)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_start(base_url: String) -> Result<bool, String> {
    start_and_wait(&base_url).await
}

/// Quiet best-effort variant for self-healing call sites (llm_client retry,
/// startup check). Never errors; false = couldn't bring it up.
pub async fn ensure_running(base_url: &str) -> bool {
    matches!(start_and_wait(base_url).await, Ok(true))
}

/// Launch a winget install of Ollama in a VISIBLE console so the user can
/// watch the download (it's a few hundred MB). Errors when winget is missing
/// so the UI can fall back to opening ollama.com.
#[tauri::command]
#[specta::specta]
pub async fn ollama_install() -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut probe = std::process::Command::new("where.exe");
        probe.arg("winget");
        {
            use std::os::windows::process::CommandExt;
            probe.creation_flags(0x0800_0000);
        }
        let has_winget = probe.output().map(|o| o.status.success()).unwrap_or(false);
        if !has_winget {
            return Err(
                "winget isn't available on this machine — install Ollama from ollama.com instead."
                    .to_string(),
            );
        }
        std::process::Command::new("cmd")
            .args([
                "/C",
                "start",
                "Ollama install",
                "cmd",
                "/K",
                "winget install -e --id Ollama.Ollama --accept-source-agreements --accept-package-agreements",
            ])
            .spawn()
            .map_err(|e| format!("Could not launch the installer: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err("Automatic install is only wired up on Windows — see ollama.com.".to_string())
}
