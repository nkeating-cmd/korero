//! Kōrero fork (v1.12.0): crash / panic reporting.
//!
//! Installs a global panic hook that (1) always logs the panic through the
//! normal log pipeline (so it lands in the rotating log file) and (2), when
//! enabled, writes a timestamped crash report with a backtrace to a
//! `crash-reports` folder in the app data dir. This is what turns an otherwise
//! invisible native panic — e.g. a model-reload failure — into something the
//! user can find and send.
//!
//! The hook is installed before the Tauri app is built; the crash directory and
//! the on/off flag are populated during setup once the app handle is known.

use std::backtrace::Backtrace;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// Whether crash reports are written to disk. Logging of panics is unconditional.
pub static SAVE_CRASH_REPORTS: AtomicBool = AtomicBool::new(true);

/// Resolved crash-reports directory. Set once during setup.
static CRASH_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Record the crash-reports directory and ensure it exists. Idempotent.
pub fn set_crash_dir(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = CRASH_DIR.set(dir);
}

/// Install the global panic hook. Chains to the previously-installed hook so
/// default stderr reporting still happens.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };

        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();

        let backtrace = Backtrace::force_capture();

        // (1) Always log — this lands in the rotating log file.
        log::error!("PANIC [{thread}] at {location}: {message}\n{backtrace}");

        // (2) Optionally persist a standalone crash report.
        if SAVE_CRASH_REPORTS.load(Ordering::Relaxed) {
            if let Some(dir) = CRASH_DIR.get() {
                let now = chrono::Local::now();
                let file = dir.join(format!("crash-{}.txt", now.format("%Y%m%d-%H%M%S")));
                let body = format!(
                    "Korero crash report\n\
                     Time:     {}\n\
                     Version:  {}\n\
                     Thread:   {}\n\
                     Location: {}\n\
                     Message:  {}\n\n\
                     Backtrace:\n{}\n",
                    now.to_rfc3339(),
                    env!("CARGO_PKG_VERSION"),
                    thread,
                    location,
                    message,
                    backtrace
                );
                let _ = std::fs::write(&file, body);
            }
        }

        previous(info);
    }));
}

/// Toggle crash-report saving and persist the choice.
#[tauri::command]
#[specta::specta]
pub fn set_save_crash_reports(app: AppHandle, enabled: bool) -> Result<(), String> {
    SAVE_CRASH_REPORTS.store(enabled, Ordering::Relaxed);
    let mut settings = crate::settings::get_settings(&app);
    settings.save_crash_reports = enabled;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// Open the crash-reports folder in the OS file manager (creating it if empty).
#[tauri::command]
#[specta::specta]
pub fn open_crash_reports_dir(app: AppHandle) -> Result<(), String> {
    let dir = crate::portable::app_data_dir(&app)
        .map_err(|e| format!("Failed to get app data directory: {}", e))?
        .join("crash-reports");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open crash reports directory: {}", e))?;
    Ok(())
}
