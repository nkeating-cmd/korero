//! Kōrero (v1.16.0): startup update notification.
//!
//! NOTIFY-ONLY by design: checks the fork's GitHub releases once at startup
//! and emits an event the UI turns into a toast linking the release page.
//! Works for both the installed and the portable edition, needs no signing
//! keys, and can never install anything by itself.
//!
//! v1.18.0: the deferral above is RESOLVED — tauri-plugin-updater is back,
//! pointed exclusively at nkeating-cmd/korero (latest.json endpoint in
//! tauri.conf.json), artifacts minisign-signed. The notify flow below stays
//! as the discovery surface; `install_update` does the one-click install.
//! The fork-safety property is preserved: both the notify check AND the
//! updater endpoint only ever see the fork repo, never upstream Handy.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Kōrero (v1.18.0): one-click update install, called from the update toast.
/// Uses the updater plugin (endpoint + pubkey from tauri.conf.json): checks,
/// downloads, verifies the minisign signature, installs, then restarts the
/// app. Any failure returns Err so the UI can fall back to opening the
/// release page for a manual download.
#[tauri::command]
#[specta::specta]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| format!("updater init: {e}"))?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("update check: {e}"))?
        .ok_or_else(|| "No update available.".to_string())?;

    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| format!("download/install: {e}"))?;

    // NSIS install succeeded; relaunch into the new version.
    app.restart();
}

const RELEASES_API: &str = "https://api.github.com/repos/nkeating-cmd/korero/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/nkeating-cmd/korero/releases/latest";

#[derive(Serialize, Clone)]
struct UpdateAvailable {
    version: String,
    url: String,
}

/// Spawned from setup. Silent on every failure path — an update nudge must
/// never affect startup (offline, rate-limited, no releases yet, etc.).
pub fn spawn_update_check(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Let the webview finish mounting its listeners before emitting, and
        // keep the network request off the startup critical path.
        let _ = tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_secs(8))
        })
        .await;

        let Ok(client) = reqwest::Client::builder()
            .user_agent(concat!("korero/", env!("CARGO_PKG_VERSION")))
            .build()
        else {
            return;
        };
        let Ok(resp) = client.get(RELEASES_API).send().await else {
            return;
        };
        if !resp.status().is_success() {
            return;
        }
        let Ok(json) = resp.json::<serde_json::Value>().await else {
            return;
        };
        let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) else {
            return;
        };
        let latest = tag.trim_start_matches('v').trim_start_matches('V');
        if is_newer(latest, env!("CARGO_PKG_VERSION")) {
            let url = json
                .get("html_url")
                .and_then(|v| v.as_str())
                .unwrap_or(RELEASES_PAGE)
                .to_string();
            log::info!(
                "Update available: v{latest} (running v{})",
                env!("CARGO_PKG_VERSION")
            );
            let _ = app.emit(
                "korero://update-available",
                UpdateAvailable {
                    version: latest.to_string(),
                    url,
                },
            );
        }
    });
}

/// Numeric dotted-version comparison; non-digit suffixes are ignored.
/// True only when `latest` is strictly newer than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let a = l.get(i).copied().unwrap_or(0);
        let b = c.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}
