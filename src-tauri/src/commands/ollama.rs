//! Kōrero (v1.3.0): Ollama local model management commands.
//! Provides in-app model pull via Ollama's native /api/pull endpoint,
//! streaming progress back to the frontend as "ollama-pull-progress" events.

use futures_util::StreamExt;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Progress payload for the "ollama-pull-progress" event.
/// Mirrors the NDJSON fields Ollama streams from /api/pull.
/// Kōrero 1.43: carries the model, so a listener can tell pulls apart.
#[derive(Debug, Serialize, Clone)]
pub struct OllamaPullProgress {
    pub model: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
}

// Kōrero 1.43 (SEC-143-07, RT-F-05): one pull at a time, and Cancel that works
// even when the download has stalled. Each pull gets a sequence number;
// Cancel marks the current number cancelled, so a later pull is unaffected.
static PULL_ACTIVE: AtomicBool = AtomicBool::new(false);
static PULL_SEQ: AtomicU64 = AtomicU64::new(0);
static CANCELLED_SEQ: AtomicU64 = AtomicU64::new(0);

const PULL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Ollama goes quiet while it verifies a multi-GB blob, so be generous.
const PULL_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const PULL_MAX_LINE: usize = 64 * 1024;
pub const PULL_CANCELLED: &str = "Download cancelled.";

struct ActivePull;
impl Drop for ActivePull {
    fn drop(&mut self) {
        PULL_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// Tags Kōrero will pull: `name` or `name:tag`, lower case. No registry
/// host or namespace (`host/ns/model`), so a pull can't be pointed elsewhere.
pub fn valid_pull_tag(s: &str) -> bool {
    let (name, tag) = match s.split_once(':') {
        Some((n, t)) => (n, Some(t)),
        None => (s, None),
    };
    let ok = |part: &str, first: bool| {
        !part.is_empty()
            && part.len() <= 80
            && part.chars().enumerate().all(|(i, c)| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || (!(first && i == 0) && matches!(c, '.' | '_' | '-'))
            })
    };
    ok(name, true) && tag.map_or(true, |t| ok(t, false))
}

/// The Ollama base URL from settings, never from the webview (SEC-143-08).
pub(crate) fn ollama_native_base(app: &AppHandle) -> Result<String, String> {
    let settings = crate::settings::get_settings(app);
    let p = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == "ollama")
        .ok_or("The Ollama provider is missing from settings.")?;
    Ok(crate::ollama_chat::native_base(&p.base_url))
}

async fn wait_until(is_cancelled: &impl Fn() -> bool) {
    while !is_cancelled() {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Read Ollama's pull stream line by line until it finishes, fails, stalls
/// or is cancelled. Generic over the stream so it can be tested.
pub(crate) async fn read_pull_stream<S, B, E>(
    stream: S,
    is_cancelled: impl Fn() -> bool,
    mut on_line: impl FnMut(serde_json::Value),
) -> Result<(), String>
where
    S: futures_util::Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let mut stream = Box::pin(stream);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let next = tokio::time::timeout(PULL_IDLE_TIMEOUT, stream.next());
        let cancel = wait_until(&is_cancelled);
        futures_util::pin_mut!(next, cancel);
        let item = match futures_util::future::select(next, cancel).await {
            futures_util::future::Either::Right(_) => return Err(PULL_CANCELLED.to_string()),
            futures_util::future::Either::Left((Err(_), _)) => {
                return Err(format!(
                    "The download stalled for {} s. Try again; it resumes where it stopped.",
                    PULL_IDLE_TIMEOUT.as_secs()
                ))
            }
            futures_util::future::Either::Left((Ok(i), _)) => i,
        };
        let Some(chunk) = item else { return Ok(()) };
        let chunk = chunk.map_err(|e| format!("Stream read error: {e}"))?;
        buf.extend_from_slice(chunk.as_ref());
        while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
                    return Err(format!("Ollama couldn't download it: {err}"));
                }
                on_line(json);
            }
        }
        if buf.len() > PULL_MAX_LINE {
            return Err("Ollama sent an unexpected reply; the download was stopped.".to_string());
        }
    }
}

/// Pull an Ollama model by streaming Ollama's native /api/pull endpoint.
///
/// Kōrero 1.43: the base URL comes from settings, the tag is validated, only
/// one pull runs at a time, and `cancel_ollama_pull` stops it even mid-stall.
/// Emits "ollama-pull-progress" (payload: OllamaPullProgress) per NDJSON line.
#[tauri::command]
#[specta::specta]
pub async fn pull_ollama_model(
    app: AppHandle,
    window: tauri::Window,
    model_name: String,
) -> Result<(), String> {
    super::require_main_window(&window)?;
    if !valid_pull_tag(&model_name) {
        return Err(format!(
            "“{model_name}” isn't a model name Kōrero can download."
        ));
    }
    let native_base = ollama_native_base(&app)?;
    if PULL_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A model download is already running.".to_string());
    }
    let _active = ActivePull;
    let id = PULL_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let is_cancelled = move || CANCELLED_SEQ.load(Ordering::SeqCst) >= id;

    let pull_url = format!("{}/api/pull", native_base);
    let client = reqwest::Client::builder()
        .connect_timeout(PULL_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
    let body = serde_json::json!({ "model": model_name, "stream": true });

    let send = client.post(&pull_url).json(&body).send();
    let cancel = wait_until(&is_cancelled);
    futures_util::pin_mut!(send, cancel);
    let response = match futures_util::future::select(send, cancel).await {
        futures_util::future::Either::Right(_) => return Err(PULL_CANCELLED.to_string()),
        futures_util::future::Either::Left((r, _)) => {
            r.map_err(|e| format!("Failed to connect to Ollama at {}: {}", pull_url, e))?
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let text = crate::ollama_chat::read_bounded(response, crate::ollama_chat::SMALL_BODY)
            .await
            .unwrap_or_else(|_| "no body".to_string());
        return Err(format!(
            "Ollama returned HTTP {} from {}: {}",
            status, pull_url, text
        ));
    }

    let model = model_name.clone();
    read_pull_stream(response.bytes_stream(), is_cancelled, |json| {
        let progress = OllamaPullProgress {
            model: model.clone(),
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
    })
    .await
}

/// Kōrero 1.43: stop the running pull (a no-op when nothing is running).
#[tauri::command]
#[specta::specta]
pub fn cancel_ollama_pull(window: tauri::Window) -> Result<(), String> {
    super::require_main_window(&window)?;
    CANCELLED_SEQ.store(PULL_SEQ.load(Ordering::SeqCst), Ordering::SeqCst);
    Ok(())
}

/// What a test run measured.
#[derive(Debug, Serialize, Clone, specta::Type)]
pub struct OllamaTestRun {
    pub seconds: f32,
    /// The model's answer to a fixed fictional sentence (first 200 chars).
    pub sample: String,
    /// Share of the loaded model on the graphics card (Ollama /api/ps
    /// `size_vram / size`). Below ~0.9, part runs on the processor.
    pub gpu_share: Option<f32>,
}

const TEST_SENTENCE: &str =
    "um so the hui is on tuesday no wait wednesday at the marae and aroha will bring the whanau";
const TEST_SYSTEM: &str = "Clean up this dictation: fix punctuation and capitals, remove filler \
words, keep te reo Māori words. Reply with the cleaned sentence only.";

fn gpu_share_from(ps_json: &str, model: &str) -> Option<f32> {
    let v: serde_json::Value = serde_json::from_str(ps_json).ok()?;
    let want = crate::ollama_chat::normalise_model(model);
    v["models"].as_array()?.iter().find_map(|m| {
        let name = m["name"].as_str().or_else(|| m["model"].as_str())?;
        if crate::ollama_chat::normalise_model(name) != want {
            return None;
        }
        let size = m["size"].as_f64()?;
        let vram = m["size_vram"].as_f64()?;
        (size > 0.0).then(|| (vram / size).clamp(0.0, 1.0) as f32)
    })
}

/// Kōrero 1.43: prove a model works before Kōrero relies on it. `/api/show`
/// first (it names an unsupported format clearly), then one clean-up of a
/// fixed fictional sentence, timed.
#[tauri::command]
#[specta::specta]
pub async fn ollama_test_model(
    app: AppHandle,
    window: tauri::Window,
    model: String,
) -> Result<OllamaTestRun, String> {
    super::require_main_window(&window)?;
    if model.trim().is_empty() || model.len() > 200 || model.chars().any(char::is_whitespace) {
        return Err("That isn't a model name.".to_string());
    }
    let base = ollama_native_base(&app)?;
    let client = reqwest::Client::builder()
        .connect_timeout(PULL_CONNECT_TIMEOUT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
    let show = client
        .post(format!("{base}/api/show"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach Ollama: {e}"))?;
    if !show.status().is_success() {
        let status = show.status();
        let text = crate::ollama_chat::read_bounded(show, crate::ollama_chat::SMALL_BODY)
            .await
            .unwrap_or_default();
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or(text);
        return Err(format!("Ollama can't use {model} (HTTP {status}): {msg}"));
    }
    let settings = crate::settings::get_settings(&app);
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == "ollama")
        .cloned()
        .ok_or("The Ollama provider is missing from settings.")?;
    let started = std::time::Instant::now();
    let answer = match crate::ollama_chat::chat(
        &provider,
        "",
        &model,
        Some(TEST_SYSTEM),
        TEST_SENTENCE,
        crate::ollama_chat::CallKind::Note,
    )
    .await?
    {
        crate::ollama_chat::Outcome::Done(Some(a)) => a,
        crate::ollama_chat::Outcome::Done(None) => {
            return Err(format!("{model} answered with nothing. Try another model."))
        }
        crate::ollama_chat::Outcome::NotOllama => {
            return Err("The server at the Ollama address isn't Ollama.".to_string())
        }
    };
    let seconds = started.elapsed().as_secs_f32();
    let gpu_share = match client.get(format!("{base}/api/ps")).send().await {
        Ok(r) if r.status().is_success() => {
            crate::ollama_chat::read_bounded(r, crate::ollama_chat::SMALL_BODY)
                .await
                .ok()
                .and_then(|t| gpu_share_from(&t, &model))
        }
        _ => None,
    };
    Ok(OllamaTestRun {
        seconds,
        sample: answer.trim().chars().take(200).collect(),
        gpu_share,
    })
}

/// Kōrero 1.43 (SEC-143-02): switch clean-up to a local Ollama model in ONE
/// settings write, and only if the Ollama address is this computer. Three
/// separate frontend writes could leave clean-up on with the previous
/// (possibly cloud) provider if one failed.
#[tauri::command]
#[specta::specta]
pub fn use_local_ollama_model(
    app: AppHandle,
    window: tauri::Window,
    model: String,
) -> Result<(), String> {
    super::require_main_window(&window)?;
    if model.trim().is_empty() || model.chars().any(char::is_whitespace) {
        return Err("That isn't a model name.".to_string());
    }
    let mut settings = crate::settings::get_settings(&app);
    let base = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == "ollama")
        .map(|p| p.base_url.clone())
        .ok_or("The Ollama provider is missing from settings.")?;
    if !crate::commands::history::is_loopback_url(&base) {
        return Err(format!(
            "Ollama is set to {base}, which isn't this computer. Change the address in AI clean-up \
             settings first."
        ));
    }
    settings.post_process_provider_id = "ollama".to_string();
    settings
        .post_process_models
        .insert("ollama".to_string(), model);
    settings.post_process_enabled = true;
    crate::settings::write_settings(&app, settings);
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
            log::info!(
                "Pre-warmed post-processing model '{model}' (keep_alive {keep_alive_secs}s)."
            );
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
pub async fn ollama_install(window: tauri::Window) -> Result<(), String> {
    super::require_main_window(&window)?;
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
                "winget install -e --id Ollama.Ollama --source winget --accept-source-agreements --accept-package-agreements",
            ])
            .spawn()
            .map_err(|e| format!("Could not launch the installer: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("Automatic install is only wired up on Windows — see ollama.com.".to_string())
    }
}

#[cfg(test)]
mod korero_143_pull_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn korero_143_pull_tag_validation() {
        for ok in [
            "gemma4:12b",
            "qwen3.5:4b",
            "llama3.2",
            "glm-ocr:latest",
            "phi4:14b",
        ] {
            assert!(valid_pull_tag(ok), "{ok}");
        }
        for bad in [
            "",
            "hf.co/prism-ml/model:q2",
            "evil.example/ns/model",
            "Gemma4:12b",
            "gemma4 12b",
            "-gemma",
            "gemma4:",
            "a:b:c",
            "../x",
        ] {
            assert!(!valid_pull_tag(bad), "{bad}");
        }
    }

    #[test]
    fn korero_143_pull_cancel_stalled() {
        tauri::async_runtime::block_on(async {
            let flag = Arc::new(AtomicBool::new(false));
            let f = flag.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                f.store(true, Ordering::SeqCst);
            });
            let started = std::time::Instant::now();
            // A stream that never yields: the stalled-download case.
            let stalled = futures_util::stream::pending::<Result<Vec<u8>, String>>();
            let r = read_pull_stream(stalled, move || flag.load(Ordering::SeqCst), |_| {}).await;
            assert_eq!(r.unwrap_err(), PULL_CANCELLED);
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "{:?}",
                started.elapsed()
            );
        });
    }

    #[test]
    fn pull_stream_reports_progress_and_errors() {
        tauri::async_runtime::block_on(async {
            let lines: Vec<Result<Vec<u8>, String>> = vec![
                Ok(
                    b"{\"status\":\"pulling\",\"digest\":\"sha256:a\",\"total\":10,\"compl"
                        .to_vec(),
                ),
                Ok(b"eted\":5}\n{\"status\":\"success\"}\n".to_vec()),
            ];
            let mut seen = Vec::new();
            let r = read_pull_stream(
                futures_util::stream::iter(lines),
                || false,
                |j| seen.push(j),
            )
            .await;
            assert!(r.is_ok());
            assert_eq!(seen.len(), 2);
            assert_eq!(seen[0]["completed"], 5);

            let bad: Vec<Result<Vec<u8>, String>> = vec![Ok(
                b"{\"error\":\"pull model manifest: file does not exist\"}\n".to_vec(),
            )];
            let r = read_pull_stream(futures_util::stream::iter(bad), || false, |_| {}).await;
            assert!(r.unwrap_err().contains("file does not exist"));
        });
    }

    #[test]
    fn korero_143_second_pull_refused() {
        // The guard is the same compare_exchange the command uses.
        let _ = PULL_ACTIVE.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
        assert!(PULL_ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err());
        drop(ActivePull);
        assert!(!PULL_ACTIVE.load(Ordering::SeqCst));
    }

    #[test]
    fn gpu_share_parses_ps() {
        let ps = r#"{"models":[{"name":"gemma4:12b","size":8000,"size_vram":8000},{"name":"x:latest","size":100,"size_vram":40}]}"#;
        assert_eq!(gpu_share_from(ps, "gemma4:12b"), Some(1.0));
        assert_eq!(gpu_share_from(ps, "x"), Some(0.4));
        assert_eq!(gpu_share_from(ps, "missing"), None);
    }
}
