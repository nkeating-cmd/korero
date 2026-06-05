use crate::managers::history::{HistoryEntry, HistoryManager};
use crate::managers::model::ModelManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings;
use crate::tray_i18n::get_tray_translations;
use log::{error, info, warn};
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Manager, Theme};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Debug, PartialEq)]
pub enum TrayIconState {
    Idle,
    Recording,
    Transcribing,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppTheme {
    Dark,
    Light,
    Colored, // Pink/colored theme for Linux
}

/// Gets the current app theme, with Linux defaulting to Colored theme
pub fn get_current_theme(app: &AppHandle) -> AppTheme {
    if cfg!(target_os = "linux") {
        // On Linux, always use the colored theme
        AppTheme::Colored
    } else {
        // On other platforms, map system theme to our app theme
        if let Some(main_window) = app.get_webview_window("main") {
            match main_window.theme().unwrap_or(Theme::Dark) {
                Theme::Light => AppTheme::Light,
                Theme::Dark => AppTheme::Dark,
                _ => AppTheme::Dark, // Default fallback
            }
        } else {
            AppTheme::Dark
        }
    }
}

/// Gets the appropriate icon path for the given theme and state
pub fn get_icon_path(theme: AppTheme, state: TrayIconState) -> &'static str {
    match (theme, state) {
        // Dark theme uses light icons
        (AppTheme::Dark, TrayIconState::Idle) => "resources/tray_idle.png",
        (AppTheme::Dark, TrayIconState::Recording) => "resources/tray_recording.png",
        (AppTheme::Dark, TrayIconState::Transcribing) => "resources/tray_transcribing.png",
        // Light theme uses dark icons
        (AppTheme::Light, TrayIconState::Idle) => "resources/tray_idle_dark.png",
        (AppTheme::Light, TrayIconState::Recording) => "resources/tray_recording_dark.png",
        (AppTheme::Light, TrayIconState::Transcribing) => "resources/tray_transcribing_dark.png",
        // Colored theme uses pink icons (for Linux)
        (AppTheme::Colored, TrayIconState::Idle) => "resources/handy.png",
        (AppTheme::Colored, TrayIconState::Recording) => "resources/recording.png",
        (AppTheme::Colored, TrayIconState::Transcribing) => "resources/transcribing.png",
    }
}

pub fn change_tray_icon(app: &AppHandle, icon: TrayIconState) {
    let tray = app.state::<TrayIcon>();
    let theme = get_current_theme(app);
    let icon_path = get_icon_path(theme, icon.clone());

    // Kōrero (v1.6.0, S2): converted from .expect() — a missing tray resource
    // should log an error, not crash the process.
    let resolved = match app
        .path()
        .resolve(icon_path, tauri::path::BaseDirectory::Resource)
    {
        Ok(p) => p,
        Err(e) => {
            error!("change_tray_icon: failed to resolve '{}': {}", icon_path, e);
            return;
        }
    };
    let image = match Image::from_path(&resolved) {
        Ok(img) => img,
        Err(e) => {
            error!("change_tray_icon: failed to load icon '{}': {}", icon_path, e);
            return;
        }
    };
    let _ = tray.set_icon(Some(image));

    // Update menu based on state
    update_tray_menu(app, &icon, None);
}

pub fn tray_tooltip() -> String {
    version_label()
}

fn version_label() -> String {
    if cfg!(debug_assertions) {
        format!("Kōrero v{} (Dev)", env!("CARGO_PKG_VERSION"))
    } else {
        format!("Kōrero v{}", env!("CARGO_PKG_VERSION"))
    }
}

/// Public entry point — keeps the `()` return type so all call sites are unchanged.
/// Kōrero (v1.6.0, S2): menu construction errors are now logged instead of panicking.
pub fn update_tray_menu(app: &AppHandle, state: &TrayIconState, locale: Option<&str>) {
    if let Err(e) = update_tray_menu_inner(app, state, locale) {
        error!("update_tray_menu: failed to build tray menu: {}", e);
    }
}

/// Inner function that does the real work.  Uses `?` so any menu-construction
/// failure propagates cleanly up to the caller rather than panicking.
///
/// Kōrero (v1.6.0, S2): separator items are bound to named variables rather than
/// using `&closure()?` inline in array literals.  Taking `&(expr?)` in an array
/// may not trigger Rust's temporary lifetime extension (the value produced by `?`
/// is inside the desugared match, not directly a `&expr` temporary).  Named
/// variables have unambiguous lifetimes that clearly outlive the slice reference.
fn update_tray_menu_inner(
    app: &AppHandle,
    state: &TrayIconState,
    locale: Option<&str>,
) -> anyhow::Result<()> {
    let settings = settings::get_settings(app);

    let locale = locale.unwrap_or(&settings.app_language);
    let strings = get_tray_translations(Some(locale.to_string()));

    // Platform-specific accelerators
    #[cfg(target_os = "macos")]
    let (settings_accelerator, quit_accelerator) = (Some("Cmd+,"), Some("Cmd+Q"));
    #[cfg(not(target_os = "macos"))]
    let (settings_accelerator, quit_accelerator) = (Some("Ctrl+,"), Some("Ctrl+Q"));

    // Common menu items
    let version_label = version_label();
    let version_i =
        MenuItem::with_id(app, "version", &version_label, false, None::<&str>)?;
    let settings_i = MenuItem::with_id(
        app,
        "settings",
        &strings.settings,
        true,
        settings_accelerator,
    )?;
    let check_updates_i = MenuItem::with_id(
        app,
        "check_updates",
        &strings.check_updates,
        settings.update_checks_enabled,
        None::<&str>,
    )?;
    let copy_last_transcript_i = MenuItem::with_id(
        app,
        "copy_last_transcript",
        &strings.copy_last_transcript,
        true,
        None::<&str>,
    )?;
    let model_loaded = app.state::<Arc<TranscriptionManager>>().is_model_loaded();
    let quit_i = MenuItem::with_id(app, "quit", &strings.quit, true, quit_accelerator)?;

    // Build model submenu — label is the active model name
    let model_manager = app.state::<Arc<ModelManager>>();
    let models = model_manager.get_available_models();
    let current_model_id = &settings.selected_model;

    let mut downloaded: Vec<_> = models.into_iter().filter(|m| m.is_downloaded).collect();
    downloaded.sort_by(|a, b| a.name.cmp(&b.name));

    let submenu_label = downloaded
        .iter()
        .find(|m| m.id == *current_model_id)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| strings.model.clone());

    let model_submenu = {
        let submenu = Submenu::with_id(app, "model_submenu", &submenu_label, true)?;
        for model in &downloaded {
            let is_active = model.id == *current_model_id;
            let item_id = format!("model_select:{}", model.id);
            let item = CheckMenuItem::with_id(
                app,
                &item_id,
                &model.name,
                true,
                is_active,
                None::<&str>,
            )?;
            let _ = submenu.append(&item);
        }
        submenu
    };

    let unload_model_i = MenuItem::with_id(
        app,
        "unload_model",
        &strings.unload_model,
        model_loaded,
        None::<&str>,
    )?;

    let menu = match state {
        TrayIconState::Recording | TrayIconState::Transcribing => {
            let cancel_i =
                MenuItem::with_id(app, "cancel", &strings.cancel, true, None::<&str>)?;
            // Named separator variables — each position needs its own instance.
            let (s1, s2, s3, s4) = (
                PredefinedMenuItem::separator(app)?,
                PredefinedMenuItem::separator(app)?,
                PredefinedMenuItem::separator(app)?,
                PredefinedMenuItem::separator(app)?,
            );
            Menu::with_items(
                app,
                &[
                    &version_i, &s1,
                    &cancel_i, &s2,
                    &copy_last_transcript_i, &s3,
                    &settings_i, &check_updates_i, &s4,
                    &quit_i,
                ],
            )?
        }
        TrayIconState::Idle => {
            let (s1, s2, s3, s4) = (
                PredefinedMenuItem::separator(app)?,
                PredefinedMenuItem::separator(app)?,
                PredefinedMenuItem::separator(app)?,
                PredefinedMenuItem::separator(app)?,
            );
            Menu::with_items(
                app,
                &[
                    &version_i, &s1,
                    &copy_last_transcript_i, &s2,
                    &model_submenu, &unload_model_i, &s3,
                    &settings_i, &check_updates_i, &s4,
                    &quit_i,
                ],
            )?
        }
    };

    let tray = app.state::<TrayIcon>();
    let _ = tray.set_menu(Some(menu));
    let _ = tray.set_icon_as_template(true);
    let _ = tray.set_tooltip(Some(version_label));
    Ok(())
}

fn last_transcript_text(entry: &HistoryEntry) -> &str {
    entry
        .post_processed_text
        .as_deref()
        .unwrap_or(&entry.transcription_text)
}

pub fn set_tray_visibility(app: &AppHandle, visible: bool) {
    let tray = app.state::<TrayIcon>();
    if let Err(e) = tray.set_visible(visible) {
        error!("Failed to set tray visibility: {}", e);
    } else {
        info!("Tray visibility set to: {}", visible);
    }
}

pub fn copy_last_transcript(app: &AppHandle) {
    let history_manager = app.state::<Arc<HistoryManager>>();
    let entry = match history_manager.get_latest_completed_entry() {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            warn!("No completed transcription history entries available for tray copy.");
            return;
        }
        Err(err) => {
            error!(
                "Failed to fetch last completed transcription entry: {}",
                err
            );
            return;
        }
    };

    let text = last_transcript_text(&entry);
    if text.trim().is_empty() {
        warn!("Last completed transcription is empty; skipping tray copy.");
        return;
    }

    if let Err(err) = app.clipboard().write_text(text) {
        error!("Failed to copy last transcript to clipboard: {}", err);
        return;
    }

    info!("Copied last transcript to clipboard via tray.");
}

#[cfg(test)]
mod tests {
    use super::last_transcript_text;
    use crate::managers::history::HistoryEntry;

    fn build_entry(transcription: &str, post_processed: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id: 1,
            file_name: "handy-1.wav".to_string(),
            timestamp: 0,
            saved: false,
            title: "Recording".to_string(),
            transcription_text: transcription.to_string(),
            post_processed_text: post_processed.map(|text| text.to_string()),
            post_process_prompt: None,
            post_process_requested: false,
        }
    }

    #[test]
    fn uses_post_processed_text_when_available() {
        let entry = build_entry("raw", Some("processed"));
        assert_eq!(last_transcript_text(&entry), "processed");
    }

    #[test]
    fn falls_back_to_raw_transcription() {
        let entry = build_entry("raw", None);
        assert_eq!(last_transcript_text(&entry), "raw");
    }
}