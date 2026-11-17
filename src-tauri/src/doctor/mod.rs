//! Kōrero 1.49 (B10): "Check local AI", the doctor for local clean-up.
//!
//! `gather` finds out what is true now (settings, Kōrero's own runtime, what
//! answers on the clean-up address, the model, memory, recent clean-ups);
//! `checks::evaluate` turns that into plain-English checks, each with at
//! most one fix. Today, the In use panel and the setup guide all read the
//! same result, so they never disagree.
//!
//! Privacy: nothing here reads dictation text. The support report is built
//! from a fixed list of fields (`report_text`), never by trimming free text,
//! so a transcript, a key, a proxy address, a path or a user name cannot get
//! into it.

pub mod checks;

use crate::cleanup_failure::CleanupFailReason;
use crate::local_runtime::RuntimeFacts;
use serde::Serialize;
use specta::Type;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckMode {
    /// Cheap and safe to run often: nothing is loaded or generated.
    Quick,
    /// Adds a test clean-up of a fixed, invented sentence.
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    /// Not run because an earlier step failed (never shown as passed).
    NotChecked,
    /// Does not apply (clean-up uses a cloud provider).
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    CleanupOn,
    AddressLocal,
    Runtime,
    AnswersAsOllama,
    OllamaVersion,
    ModelChosen,
    ModelPresent,
    ModelRuns,
    ModelFits,
    MemoryNow,
    GpuShared,
    TestAnswer,
    Recent,
}

/// What to offer. The page grades them: starting something runs at once;
/// downloads and settings changes ask first; the rest explain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fix {
    TurnOnCleanup,
    StartRuntime,
    StartOllama,
    InstallRuntime { download_mb: u32 },
    UseRuntime,
    InstallOllama,
    UpdateOllama,
    DownloadModel { model: String },
    ChooseModel,
    ProxyHelp,
    PortTaken { port: u16 },
    FreeMemory,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CheckResult {
    pub id: CheckId,
    pub status: CheckStatus,
    pub title: String,
    pub detail: String,
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The full check passed, test answer included.
    Working,
    /// The quick check passed; only a full check can say "working".
    LooksSetUp,
    NeedsAttention,
    /// Clean-up uses a cloud provider: nothing local to check.
    NotLocal,
    Off,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DoctorReport {
    pub mode: CheckMode,
    pub verdict: Verdict,
    pub checks: Vec<CheckResult>,
    /// The support report: fixed fields only, safe to copy or send.
    pub report_text: String,
    pub took_ms: u32,
}

#[derive(Debug, Clone)]
pub struct TestFacts {
    pub seconds: f32,
    pub gpu_share: Option<f32>,
    /// The model's answer to the fixed, invented test sentence. Checked for
    /// quality; never put in the report.
    pub answer: String,
}

/// Everything the checks look at. Built by `gather`; built by hand in tests.
#[derive(Debug, Clone)]
pub struct Facts {
    pub mode: CheckMode,
    pub cleanup_enabled: bool,
    pub provider_id: String,
    pub provider_label: String,
    pub provider_local: bool,
    pub provider_is_ollama: bool,
    pub port: Option<u16>,
    pub model: String,
    pub runtime: RuntimeFacts,
    pub own_ollama_installed: bool,
    /// The HTTP status from `/api/version`, if anything answered at all.
    pub answered_http: Option<u16>,
    /// Ollama's version, if what answered is Ollama.
    pub ollama_version: Option<String>,
    /// An `HTTP(S)_PROXY` or `ALL_PROXY` variable is set (its value is never kept).
    pub proxy_set: bool,
    /// The chosen model's size, if Ollama has it.
    pub model_mb: Option<u64>,
    /// Ollama's own refusal from `/api/show`, if any. Shown in the page,
    /// never in the report.
    pub show_error: Option<String>,
    pub model_loaded: bool,
    pub others_loaded: Vec<String>,
    pub best_vram_mb: u32,
    pub ram_mb: u32,
    pub available_ram_mb: Option<u32>,
    pub test: Option<Result<TestFacts, String>>,
    /// Newest first: (cleaned, stored reason).
    pub recent: Vec<(bool, Option<CleanupFailReason>)>,
    pub recording: bool,
}

const PROBE: Duration = Duration::from_secs(4);
const RECENT: usize = 20;
static RUNNING: AtomicBool = AtomicBool::new(false);

fn port_of(base_url: &str) -> Option<u16> {
    let rest = base_url
        .trim()
        .split_once("://")
        .map_or(base_url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    authority
        .rsplit('@')
        .next()?
        .rsplit_once(':')?
        .1
        .parse()
        .ok()
}

fn proxy_set() -> bool {
    [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

async fn get_json(
    client: &reqwest::Client,
    url: String,
) -> (Option<u16>, Option<serde_json::Value>) {
    match client.get(url).send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = crate::ollama_chat::read_bounded(r, 1 << 20)
                .await
                .unwrap_or_default();
            (Some(status), serde_json::from_str(&body).ok())
        }
        Err(_) => (None, None),
    }
}

/// Find out what is true now. Reads no dictation text.
async fn gather(app: &AppHandle, mode: CheckMode) -> Facts {
    let settings = crate::settings::get_settings(app);
    let provider = settings.active_post_process_provider().cloned();
    let (provider_id, provider_label, base) = provider
        .as_ref()
        .map(|p| (p.id.clone(), p.label.clone(), p.base_url.clone()))
        .unwrap_or_default();
    let model = settings
        .post_process_models
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();
    let provider_local = crate::commands::history::is_loopback_url(&base);
    let recording =
        crate::meeting::is_meeting_active() || crate::commands::audio::is_recording(app.clone());

    let (profile, available) = tauri::async_runtime::spawn_blocking(|| {
        (
            crate::commands::machine::profile_now(),
            crate::commands::machine::available_ram_mb(),
        )
    })
    .await
    .unwrap_or_else(|_| (crate::commands::machine::profile_now(), None));
    let best_vram_mb = profile
        .gpus
        .iter()
        .filter(|g| g.kind != crate::commands::machine::GpuKind::Integrated)
        .map(|g| {
            if g.kind == crate::commands::machine::GpuKind::Apple {
                g.vram_mb * 3 / 4
            } else {
                g.vram_mb
            }
        })
        .max()
        .unwrap_or(0);

    let recent = app
        .try_state::<Arc<crate::managers::history::HistoryManager>>()
        .and_then(|h| h.recent_cleanup_outcomes(RECENT).ok())
        .unwrap_or_default();

    let mut f = Facts {
        mode,
        cleanup_enabled: settings.post_process_enabled,
        provider_id: provider_id.clone(),
        provider_label,
        provider_local,
        provider_is_ollama: provider_id == "ollama",
        port: port_of(&base),
        model: model.clone(),
        runtime: crate::local_runtime::facts().await,
        own_ollama_installed: crate::commands::ollama::find_ollama_exe().is_some(),
        answered_http: None,
        ollama_version: None,
        proxy_set: proxy_set(),
        model_mb: None,
        show_error: None,
        model_loaded: false,
        others_loaded: vec![],
        best_vram_mb,
        ram_mb: profile.ram_mb,
        available_ram_mb: available,
        test: None,
        recent,
        recording,
    };
    if !(f.cleanup_enabled && provider_local && f.provider_is_ollama) {
        return f;
    }

    let native = base
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();
    let Ok(client) = crate::llm_client::direct_to_loopback(reqwest::Client::builder(), &native)
        .connect_timeout(PROBE)
        .timeout(PROBE)
        .build()
    else {
        return f;
    };
    let (status, version) = get_json(&client, format!("{native}/api/version")).await;
    f.answered_http = status;
    f.ollama_version = version
        .as_ref()
        .and_then(|v| v["version"].as_str())
        .map(str::to_string);
    if f.ollama_version.is_none() || model.trim().is_empty() {
        return f;
    }

    let want = crate::ollama_chat::normalise_model(&model);
    if let (_, Some(tags)) = get_json(&client, format!("{native}/api/tags")).await {
        f.model_mb = tags["models"].as_array().and_then(|ms| {
            ms.iter().find_map(|m| {
                let name = m["name"].as_str().or_else(|| m["model"].as_str())?;
                (crate::ollama_chat::normalise_model(name) == want)
                    .then(|| m["size"].as_u64().unwrap_or(0) / (1024 * 1024))
            })
        });
    }
    if let (_, Some(ps)) = get_json(&client, format!("{native}/api/ps")).await {
        for m in ps["models"].as_array().into_iter().flatten() {
            let Some(name) = m["name"].as_str().or_else(|| m["model"].as_str()) else {
                continue;
            };
            if crate::ollama_chat::normalise_model(name) == want {
                f.model_loaded = true;
            } else {
                f.others_loaded.push(name.to_string());
            }
        }
    }
    if f.model_mb.is_none() {
        return f;
    }
    match client
        .post(format!("{native}/api/show"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            let text = crate::ollama_chat::read_bounded(r, crate::ollama_chat::SMALL_BODY)
                .await
                .unwrap_or_default();
            f.show_error = Some(
                serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| v["error"].as_str().map(str::to_string))
                    .unwrap_or(text)
                    .chars()
                    .take(300)
                    .collect(),
            );
        }
        Err(e) => f.show_error = Some(format!("Couldn't ask Ollama about the model: {e}")),
    }
    if mode == CheckMode::Full && f.show_error.is_none() && !recording {
        f.test = Some(
            crate::commands::ollama::test_run(app, &model)
                .await
                .map(|t| TestFacts {
                    seconds: t.seconds,
                    gpu_share: t.gpu_share,
                    answer: t.sample,
                }),
        );
    }
    f
}

fn status_word(s: CheckStatus) -> &'static str {
    match s {
        CheckStatus::Pass => "pass",
        CheckStatus::Warn => "warn",
        CheckStatus::Fail => "FAIL",
        CheckStatus::NotChecked => "not checked",
        CheckStatus::Skipped => "skipped",
    }
}

fn id_word(id: CheckId) -> String {
    serde_json::to_value(id)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn fix_word(f: &Fix) -> String {
    serde_json::to_value(f)
        .ok()
        .and_then(|v| v["kind"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The support report. Built only from the fields named here: no paths, no
/// user or computer name, no address beyond the local port, no proxy value,
/// no Ollama error text, no model answer, no dictation.
pub fn report_text(
    f: &Facts,
    checks: &[CheckResult],
    verdict: Verdict,
    korero_version: &str,
) -> String {
    let mut s = String::new();
    let v = serde_json::to_value(verdict)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let mode = if f.mode == CheckMode::Full {
        "full"
    } else {
        "quick"
    };
    s.push_str(&format!("Kōrero local AI check: {v} ({mode} check)\n"));
    s.push_str(&format!(
        "Kōrero {korero_version}, {}, {}\n",
        std::env::consts::OS,
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    let model = if f
        .model
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || ":._-/".contains(c))
    {
        f.model.as_str()
    } else {
        "(unusual name)"
    };
    s.push_str(&format!(
        "Clean-up: {}, provider {}{}, model {}\n",
        if f.cleanup_enabled { "on" } else { "off" },
        if f.provider_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            &f.provider_id
        } else {
            "(custom)"
        },
        if f.provider_local {
            " (this computer)"
        } else {
            " (elsewhere)"
        },
        if model.is_empty() { "(none)" } else { model },
    ));
    s.push_str(&format!(
        "Local AI: {}; Ollama {}; port {}; answered HTTP {}\n",
        if f.runtime.managed {
            if f.runtime.core_installed {
                "Kōrero's own (downloaded)"
            } else {
                "Kōrero's own (not downloaded)"
            }
        } else if f.own_ollama_installed {
            "own Ollama installed"
        } else {
            "no Ollama installed"
        },
        f.ollama_version
            .as_deref()
            .filter(|v| v.len() < 24)
            .unwrap_or("-"),
        f.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
        f.answered_http
            .map(|c| c.to_string())
            .unwrap_or_else(|| "none".into()),
    ));
    s.push_str(&format!(
        "Computer: graphics memory {} MB, memory {} MB ({} MB free); proxy set: {}\n",
        f.best_vram_mb,
        f.ram_mb,
        f.available_ram_mb
            .map(|m| m.to_string())
            .unwrap_or_else(|| "-".into()),
        if f.proxy_set { "yes" } else { "no" },
    ));
    if let Some(Ok(t)) = &f.test {
        s.push_str(&format!(
            "Test answer: {:.1} s, {} on the graphics card\n",
            t.seconds,
            t.gpu_share
                .map(|g| format!("{:.0}%", g * 100.0))
                .unwrap_or_else(|| "-".into())
        ));
    }
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for (cleaned, why) in &f.recent {
        if *cleaned {
            continue;
        }
        let w = why.unwrap_or(CleanupFailReason::Other).as_db();
        match counts.iter_mut().find(|(k, _)| *k == w) {
            Some((_, c)) => *c += 1,
            None => counts.push((w, 1)),
        }
    }
    let list: Vec<String> = counts.iter().map(|(k, c)| format!("{k} {c}")).collect();
    s.push_str(&format!(
        "Recent clean-ups: {} ({})\n",
        f.recent.len(),
        if list.is_empty() {
            "none failed".to_string()
        } else {
            list.join(", ")
        }
    ));
    s.push_str("Checks:\n");
    for c in checks {
        s.push_str(&format!("  {:11} {}", status_word(c.status), id_word(c.id)));
        if let Some(fix) = &c.fix {
            s.push_str(&format!(" (fix: {})", fix_word(fix)));
        }
        s.push('\n');
    }
    s
}

struct Running;
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Kōrero 1.49: run "Check local AI". `full` adds a test clean-up of a fixed,
/// invented sentence (never while recording).
#[tauri::command]
#[specta::specta]
pub async fn local_ai_check(
    app: AppHandle,
    window: tauri::Window,
    full: bool,
) -> Result<DoctorReport, String> {
    crate::commands::require_main_window(&window)?;
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("A check is already running.".into());
    }
    let _running = Running;
    let started = Instant::now();
    let mode = if full {
        CheckMode::Full
    } else {
        CheckMode::Quick
    };
    let facts = gather(&app, mode).await;
    let (checks, verdict) = checks::evaluate(&facts);
    let report_text = report_text(
        &facts,
        &checks,
        verdict,
        &app.package_info().version.to_string(),
    );
    log::info!(
        "local AI check ({}): {:?}, {} fail, {} warn",
        if full { "full" } else { "quick" },
        verdict,
        checks
            .iter()
            .filter(|c| c.status == CheckStatus::Fail)
            .count(),
        checks
            .iter()
            .filter(|c| c.status == CheckStatus::Warn)
            .count()
    );
    Ok(DoctorReport {
        mode,
        verdict,
        checks,
        report_text,
        took_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
    })
}

#[cfg(test)]
mod report_tests {
    use super::*;

    /// Facts carrying things that must never reach a report.
    fn leaky() -> Facts {
        Facts {
            mode: CheckMode::Full,
            cleanup_enabled: true,
            provider_id: "ollama".into(),
            provider_label: "C:\\Users\\aroha.smith\\secret-label".into(),
            provider_local: true,
            provider_is_ollama: true,
            port: Some(11435),
            model: "gemma4:12b".into(),
            runtime: RuntimeFacts {
                supported: true,
                managed: true,
                core_installed: true,
                missing_mb: 0,
            },
            own_ollama_installed: false,
            answered_http: Some(200),
            ollama_version: Some("0.40.16".into()),
            proxy_set: true,
            model_mb: Some(8192),
            show_error: Some("blob at C:\\Users\\aroha.smith\\.ollama is damaged".into()),
            model_loaded: true,
            others_loaded: vec!["private-model-name".into()],
            best_vram_mb: 12288,
            ram_mb: 32768,
            available_ram_mb: Some(10000),
            test: Some(Ok(TestFacts {
                seconds: 2.5,
                gpu_share: Some(1.0),
                answer: "TOP SECRET dictation text about a client".into(),
            })),
            recent: vec![(false, Some(CleanupFailReason::TimedOut)), (true, None)],
            recording: false,
        }
    }

    #[test]
    fn the_report_holds_only_its_fixed_fields() {
        let f = leaky();
        let (c, v) = checks::evaluate(&f);
        let r = report_text(&f, &c, v, "1.49.0");
        for bad in [
            "aroha",
            "Users",
            "secret",
            "SECRET",
            "private-model-name",
            "damaged",
            "client",
            "\\",
        ] {
            assert!(!r.contains(bad), "report contains {bad:?}:\n{r}");
        }
        for good in [
            "Kōrero 1.49.0",
            "gemma4:12b",
            "Ollama 0.40.16",
            "port 11435",
            "proxy set: yes",
            "timed_out 1",
            "pass",
            "model_present",
        ] {
            assert!(r.contains(good), "report lacks {good:?}:\n{r}");
        }
    }

    #[test]
    fn odd_names_are_not_copied_into_the_report() {
        let mut f = leaky();
        f.model = "C:\\Users\\someone\\model.gguf".into();
        f.provider_id = "my provider at https://evil.example".into();
        let (c, v) = checks::evaluate(&f);
        let r = report_text(&f, &c, v, "1.49.0");
        assert!(!r.contains("someone") && !r.contains("evil"), "{r}");
    }

    #[test]
    fn ports_are_read_from_addresses_even_with_user_info() {
        assert_eq!(port_of("http://127.0.0.1:11435/v1"), Some(11435));
        assert_eq!(port_of("http://localhost/v1"), None);
        assert_eq!(port_of("http://u:p@localhost:11434/v1"), Some(11434));
    }
}
