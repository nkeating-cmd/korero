mod actions;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod apple_intelligence;
mod audio_feedback;
pub mod audio_toolkit;
pub mod cli;
mod clipboard;
mod commands;
mod crash; // Kōrero (v1.12.0): global panic hook + crash report files
mod corrections; // Kōrero (v1.15.0): user-taught transcription corrections
mod update_check; // Kōrero (v1.16.0): notify-only update check (own repo only)
mod window_info; // Kōrero (v1.18.1): contextual-routing spike (active window title)
mod meeting; // Kōrero (v1.13.0): meeting recorder (mic + system loopback)
mod meeting_capture; // Kōrero (v1.13.2, Phase A): streaming-to-disk meeting capture
#[cfg(windows)]
mod meeting_capture_wasapi; // Kōrero (v1.13.6): native WASAPI loopback for "Others"
mod denoise; // Kōrero (v1.11.0): optional RNNoise mic denoiser (nnnoiseless)
mod helpers;
mod input;
mod llm_client;
mod managers;
mod overlay;
pub mod portable;
mod secret_store;
mod settings;
mod shortcut;
mod signal_handle;
mod transcription_coordinator;
mod tray;
mod tray_i18n;
mod utils;

pub use cli::CliArgs;
#[cfg(debug_assertions)]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

use env_filter::Builder as EnvFilterBuilder;
use managers::audio::AudioRecordingManager;
use managers::history::HistoryManager;
use managers::model::ModelManager;
use managers::transcription::TranscriptionManager;
#[cfg(unix)]
use signal_hook::consts::{SIGUSR1, SIGUSR2};
#[cfg(unix)]
use signal_hook::iterator::Signals;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use tauri::image::Image;
pub use transcription_coordinator::TranscriptionCoordinator;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};
// Kōrero (2026-05-17 PM): persist window size + position across launches so
// Nic doesn't have to resize away from the default every session.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_window_state::{StateFlags, WindowExt as WindowStateExt};

use crate::settings::{get_settings, SettingsCache};

// Global atomic to store the file log level filter
// We use u8 to store the log::LevelFilter as a number
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

fn level_filter_from_u8(value: u8) -> log::LevelFilter {
    match value {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        5 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Trace,
    }
}

fn build_console_filter() -> env_filter::Filter {
    let mut builder = EnvFilterBuilder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(log::LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(log::LevelFilter::Info);
        }
    }

    builder.build()
}

fn show_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        if let Err(e) = main_window.unminimize() {
            log::error!("Failed to unminimize webview window: {}", e);
        }
        if let Err(e) = main_window.show() {
            log::error!("Failed to show webview window: {}", e);
        }
        if let Err(e) = main_window.set_focus() {
            log::error!("Failed to focus webview window: {}", e);
        }
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                log::error!("Failed to set activation policy to Regular: {}", e);
            }
        }
        return;
    }

    let webview_labels = app.webview_windows().keys().cloned().collect::<Vec<_>>();
    log::error!(
        "Main window not found. Webview labels: {:?}",
        webview_labels
    );
}

#[allow(unused_variables)]
fn should_force_show_permissions_window(app: &AppHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        let model_manager = app.state::<Arc<ModelManager>>();
        let has_downloaded_models = model_manager
            .get_available_models()
            .iter()
            .any(|model| model.is_downloaded);

        if !has_downloaded_models {
            return false;
        }

        let status = commands::audio::get_windows_microphone_permission_status();
        if status.supported && status.overall_access == commands::audio::PermissionAccess::Denied {
            log::info!(
                "Windows microphone permissions are denied; forcing main window visible for onboarding"
            );
            return true;
        }
    }

    false
}

fn initialize_core_logic(app_handle: &AppHandle) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    // Initialize the managers
    let recording_manager = Arc::new(
        AudioRecordingManager::new(app_handle).expect("Failed to initialize recording manager"),
    );
    let model_manager =
        Arc::new(ModelManager::new(app_handle).expect("Failed to initialize model manager"));
    let transcription_manager = Arc::new(
        TranscriptionManager::new(app_handle, model_manager.clone())
            .expect("Failed to initialize transcription manager"),
    );
    let history_manager =
        Arc::new(HistoryManager::new(app_handle).expect("Failed to initialize history manager"));

    // Apply accelerator preferences before any model loads
    managers::transcription::apply_accelerator_settings(app_handle);

    // Add managers to Tauri's managed state
    app_handle.manage(recording_manager.clone());
    app_handle.manage(model_manager.clone());
    app_handle.manage(transcription_manager.clone());
    app_handle.manage(history_manager.clone());

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    #[cfg(unix)]
    let signals = Signals::new(&[SIGUSR1, SIGUSR2]).unwrap();
    // Set up signal handlers for toggling transcription
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone(), signals);

    // Apply macOS Accessory policy if starting hidden and tray is available.
    // If the tray icon is disabled, keep the dock icon so the user can reopen.
    #[cfg(target_os = "macos")]
    {
        let settings = settings::get_settings(app_handle);
        if settings.start_hidden && settings.show_tray_icon {
            let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }
    // Get the current theme to set the appropriate initial icon
    let initial_theme = tray::get_current_theme(app_handle);

    // Choose the appropriate initial icon based on theme
    let initial_icon_path = tray::get_icon_path(initial_theme, tray::TrayIconState::Idle);

    let tray = TrayIconBuilder::new()
        .icon(
            Image::from_path(
                app_handle
                    .path()
                    .resolve(initial_icon_path, tauri::path::BaseDirectory::Resource)
                    .unwrap(),
            )
            .unwrap(),
        )
        .tooltip(tray::tray_tooltip())
        .show_menu_on_left_click(true)
        .icon_as_template(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => {
                show_main_window(app);
            }
            "check_updates" => {
                let settings = settings::get_settings(app);
                if settings.update_checks_enabled {
                    show_main_window(app);
                    let _ = app.emit("check-for-updates", ());
                }
            }
            "copy_last_transcript" => {
                tray::copy_last_transcript(app);
            }
            "unload_model" => {
                let transcription_manager = app.state::<Arc<TranscriptionManager>>();
                if !transcription_manager.is_model_loaded() {
                    log::warn!("No model is currently loaded.");
                    return;
                }
                match transcription_manager.unload_model() {
                    Ok(()) => log::info!("Model unloaded via tray."),
                    Err(e) => log::error!("Failed to unload model via tray: {}", e),
                }
            }
            "cancel" => {
                use crate::utils::cancel_current_operation;

                // Use centralized cancellation that handles all operations
                cancel_current_operation(app);
            }
            "quit" => {
                app.exit(0);
            }
            id if id.starts_with("model_select:") => {
                let model_id = id.strip_prefix("model_select:").unwrap().to_string();
                let current_model = settings::get_settings(app).selected_model;
                if model_id == current_model {
                    return;
                }
                let app_clone = app.clone();
                std::thread::spawn(move || {
                    match commands::models::switch_active_model(&app_clone, &model_id) {
                        Ok(()) => {
                            log::info!("Model switched to {} via tray.", model_id);
                        }
                        Err(e) => {
                            log::error!("Failed to switch model via tray: {}", e);
                        }
                    }
                    tray::update_tray_menu(&app_clone, &tray::TrayIconState::Idle, None);
                });
            }
            _ => {}
        })
        .build(app_handle)
        .unwrap();
    app_handle.manage(tray);

    // Initialize tray menu with idle state
    utils::update_tray_menu(app_handle, &utils::TrayIconState::Idle, None);

    // Apply show_tray_icon setting
    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    // Refresh tray menu when model state changes
    let app_handle_for_listener = app_handle.clone();
    app_handle.listen("model-state-changed", move |_| {
        tray::update_tray_menu(&app_handle_for_listener, &tray::TrayIconState::Idle, None);
    });

    // Get the autostart manager and configure based on user setting
    let autostart_manager = app_handle.autolaunch();
    let settings = settings::get_settings(&app_handle);

    if settings.autostart_enabled {
        // Enable autostart if user has opted in
        let _ = autostart_manager.enable();
    } else {
        // Disable autostart if user has opted out
        let _ = autostart_manager.disable();
    }

    // Kōrero (v1.9.0, M1): pre-warm the transcription model in the background so
    // the first Ctrl+Space fires without a cold-start model-load delay.
    // initiate_model_load() is idempotent (no-ops if already loading or loaded),
    // spawns its own thread, reads selected_model from the settings cache, and
    // logs a warning on failure (non-fatal — app falls back to lazy load).
    {
        let tm = app_handle.state::<Arc<TranscriptionManager>>();
        tm.initiate_model_load();
        log::info!("Model pre-warm initiated at startup.");
    }

    // Kōrero (v1.9.0, W1): pre-initialize shortcuts on Windows so Ctrl+Space is
    // live within ~1 s of launch rather than waiting for React to hydrate and
    // call initializeShortcuts(). Skipped on macOS/Linux — macOS needs the
    // frontend to confirm Accessibility permission first; Linux global-shortcut
    // support is unstable. The frontend's initializeShortcuts() command is
    // idempotent: it checks ShortcutsInitialized in app state and no-ops if we
    // already set it here.
    // catch_unwind: rdev global hooks can fail on pathological system
    // configurations. Non-fatal — the frontend's initializeShortcuts() call
    // runs as a fallback after React hydration if this block panics.
    #[cfg(windows)]
    {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::shortcut::init_shortcuts(app_handle);
        }));
        match result {
            Ok(()) => {
                app_handle.manage(crate::commands::ShortcutsInitialized);
                log::info!("Shortcuts pre-initialized at startup (Windows).");
            }
            Err(_) => {
                log::warn!("Shortcut pre-init panicked (non-fatal — frontend will retry on first use).");
            }
        }
    }

    // Create the recording overlay window (hidden by default)
    utils::create_recording_overlay(app_handle);
}

#[tauri::command]
#[specta::specta]
fn trigger_update_check(app: AppHandle) -> Result<(), String> {
    let settings = settings::get_settings(&app);
    if !settings.update_checks_enabled {
        return Ok(());
    }
    app.emit("check-for-updates", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn show_main_window_command(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(cli_args: CliArgs) {
    // Kōrero (v1.12.0): install the global panic hook before anything else so a
    // panic anywhere in startup or runtime is logged and (if enabled) written to
    // a crash report. The crash directory + on/off flag are populated in setup.
    crash::install_panic_hook();

    // Detect portable mode before anything else
    portable::init();

    // Parse console logging directives from RUST_LOG, falling back to info-level logging
    // when the variable is unset
    let console_filter = build_console_filter();

    let specta_builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            shortcut::change_binding,
            shortcut::reset_binding,
            shortcut::change_ptt_setting,
            shortcut::change_audio_feedback_setting,
            shortcut::change_audio_feedback_volume_setting,
            shortcut::change_sound_theme_setting,
            shortcut::change_start_hidden_setting,
            shortcut::change_autostart_setting,
            shortcut::change_translate_to_english_setting,
            shortcut::change_selected_language_setting,
            shortcut::change_overlay_position_setting,
            shortcut::change_debug_mode_setting,
            shortcut::change_word_correction_threshold_setting,
            shortcut::change_extra_recording_buffer_setting,
            shortcut::change_paste_delay_ms_setting,
            shortcut::change_paste_method_setting,
            shortcut::get_available_typing_tools,
            shortcut::change_typing_tool_setting,
            shortcut::change_external_script_path_setting,
            shortcut::change_clipboard_handling_setting,
            shortcut::change_auto_submit_setting,
            shortcut::change_auto_submit_key_setting,
            shortcut::change_post_process_enabled_setting,
            shortcut::change_experimental_enabled_setting,
            shortcut::change_post_process_base_url_setting,
            shortcut::change_post_process_api_key_setting,
            shortcut::change_post_process_model_setting,
            shortcut::set_post_process_provider,
            shortcut::fetch_post_process_models,
            shortcut::add_post_process_prompt,
            shortcut::update_post_process_prompt,
            shortcut::delete_post_process_prompt,
            shortcut::set_post_process_selected_prompt,
            shortcut::update_custom_words,
            shortcut::suspend_binding,
            shortcut::resume_binding,
            shortcut::change_mute_while_recording_setting,
            shortcut::change_denoise_enabled_setting,
            shortcut::change_append_trailing_space_setting,
            shortcut::change_lazy_stream_close_setting,
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_keyboard_implementation_setting,
            shortcut::get_keyboard_implementation,
            shortcut::change_show_tray_icon_setting,
            shortcut::change_whisper_accelerator_setting,
            shortcut::change_ort_accelerator_setting,
            shortcut::change_whisper_gpu_device,
            shortcut::get_available_accelerators,
            shortcut::handy_keys::start_handy_keys_recording,
            shortcut::handy_keys::stop_handy_keys_recording,
            trigger_update_check,
            update_check::install_update,
            window_info::get_active_window_title,
            settings::update_post_process_prompt_full,
            // Kōrero fork (v1.19.2): full-array persistence for the settings the
            // store had no updater for — taught corrections (the headline bug:
            // they never reached the backend) and the prompts array (add/delete).
            corrections::update_transcript_corrections,
            settings::update_post_process_prompts,
            show_main_window_command,
            commands::cancel_operation,
            commands::is_portable,
            commands::get_app_dir_path,
            commands::get_app_settings,
            commands::get_default_settings,
            commands::get_log_dir_path,
            commands::set_log_level,
            commands::open_recordings_folder,
            commands::open_log_dir,
            commands::open_app_data_dir,
            commands::check_apple_intelligence_available,
            commands::initialize_enigo,
            commands::initialize_shortcuts,
            commands::models::get_available_models,
            commands::models::get_model_info,
            commands::models::download_model,
            commands::models::delete_model,
            commands::models::cancel_download,
            commands::models::set_active_model,
            commands::models::get_current_model,
            commands::models::get_transcription_model_status,
            commands::models::is_model_loading,
            commands::models::has_any_models_available,
            commands::models::has_any_models_or_downloads,
            commands::audio::update_microphone_mode,
            commands::audio::get_microphone_mode,
            commands::audio::get_windows_microphone_permission_status,
            commands::audio::open_microphone_privacy_settings,
            commands::audio::get_available_microphones,
            commands::audio::set_selected_microphone,
            commands::audio::get_selected_microphone,
            commands::audio::get_available_output_devices,
            commands::audio::set_selected_output_device,
            commands::audio::get_selected_output_device,
            commands::audio::play_test_sound,
            commands::audio::check_custom_sounds,
            commands::audio::set_clamshell_microphone,
            commands::audio::get_clamshell_microphone,
            commands::audio::is_recording,
            commands::transcription::set_model_unload_timeout,
            commands::transcription::get_model_load_status,
            commands::transcription::unload_model_manually,
            // Kōrero fork (v1.12.0): Notes page dictation commands.
            commands::notes::note_start_dictation,
            commands::notes::note_stop_dictation,
            commands::notes::note_cancel_dictation,
            // Kōrero fork (v1.12.0): crash-report controls.
            crash::set_save_crash_reports,
            crash::open_crash_reports_dir,
            // Kōrero fork (v1.13.0): meeting recorder commands.
            meeting::meeting_start_capture,
            meeting::meeting_stop_capture,
            meeting::meeting_transcribe_file,
            meeting::meeting_transcribe_merge,
            meeting::meeting_list_recordings,
            meeting::meeting_export_transcript,
            meeting::meeting_query,
            meeting::meeting_post_process,
            meeting::meeting_prewarm_post_process,
            meeting::meeting_provider_is_local,
            // Kōrero fork (v1.13.4): meetings metadata store on disk.
            meeting::meetings_store_load,
            meeting::meetings_store_save,
            // Kōrero fork (v1.13.5): capture diagnostics (meters + device test).
            meeting::meeting_capture_devices,
            meeting::meeting_test_capture,
            // Kōrero fork (v1.13.6): recording retention / deletion.
            meeting::meeting_delete_recording,
            // Kōrero fork (v1.14.2): restore recording UI after page remount.
            meeting::meeting_recording_status,
            // Kōrero fork (v1.19.0): pause / resume a live meeting.
            meeting::meeting_pause,
            meeting::meeting_resume,
            // Kōrero fork (v1.14.3): whole-note post-processing (prompt + model
            // selectable per run).
            commands::notes::note_post_process,
            // Kōrero fork (v1.17.0): Ollama doctor — detect / start / install.
            commands::ollama::ollama_status,
            commands::ollama::ollama_start,
            commands::ollama::ollama_install,
            commands::history::get_history_entries,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::retry_history_entry_transcription,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            commands::ollama::pull_ollama_model,
            commands::ollama::check_ollama_connection,
            commands::history_extra::update_post_processed_text,
            helpers::clamshell::is_laptop,
        ])
        .events(collect_events![managers::history::HistoryUpdatePayload,]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            Typescript::default().bigint(BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let invoke_handler = specta_builder.invoke_handler();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            LogBuilder::new()
                .level(log::LevelFilter::Trace) // Set to most verbose level globally
                .max_file_size(500_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .clear_targets()
                .targets([
                    // Console output respects RUST_LOG environment variable
                    Target::new(TargetKind::Stdout).filter({
                        let console_filter = console_filter.clone();
                        move |metadata| console_filter.enabled(metadata)
                    }),
                    // File logs respect the user's settings (stored in FILE_LOG_LEVEL atomic)
                    Target::new(if let Some(data_dir) = portable::data_dir() {
                        TargetKind::Folder {
                            path: data_dir.join("logs"),
                            file_name: Some("korero".into()), // Kōrero (v1.1.0): was "handy"
                        }
                    } else {
                        TargetKind::LogDir {
                            file_name: Some("korero".into()), // Kōrero (v1.1.0): was "handy"
                        }
                    })
                    .filter(|metadata| {
                        let file_level = FILE_LOG_LEVEL.load(Ordering::Relaxed);
                        metadata.level() <= level_filter_from_u8(file_level)
                    }),
                ])
                .build(),
        );

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle-transcription") {
                signal_handle::send_transcription_input(app, "transcribe", "CLI");
            } else if args.iter().any(|a| a == "--toggle-post-process") {
                signal_handle::send_transcription_input(app, "transcribe_with_post_process", "CLI");
            } else if args.iter().any(|a| a == "--cancel") {
                crate::utils::cancel_current_operation(app);
            } else {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        // Kōrero (v1.18.0): updater plugin RESTORED — endpoint locked to the
        // fork repo in tauri.conf.json, artifacts minisign-verified against
        // the pubkey there. The fork-time removal protected against pulling
        // upstream Handy builds; the locked endpoint preserves that property.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        // Kōrero (2026-05-17 PM): window-state plugin auto-saves window size
        // and position on close, auto-restores on next launch. The explicit
        // restore_state call after window build below is required because
        // Kōrero builds its main window programmatically (not via tauri.conf
        // declarative windows), so the plugin's auto-restore hook doesn't fire.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(cli_args.clone())
        .setup(move |app| {
            specta_builder.mount_events(app);

            // Create main window programmatically so we can set data_directory
            // for portable mode (redirects WebView2 cache to portable Data dir)
            // Kōrero (2026-05-17 PM): default bumped from 820x640 to 900x820
            // Kōrero (v1.7.0): default reduced from 900×820 to 820×640 so the
            // window launches at a compact size that fits most settings pages
            // without blank space. Users can resize freely; the window-state
            // plugin persists their preferred size on close and restores it on
            // the next launch, so the default only applies to first run.
            let mut win_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
                    .title("Kōrero")
                    .inner_size(820.0, 640.0)
                    .min_inner_size(720.0, 560.0)
                    .resizable(true)
                    .maximizable(true)
                    .visible(false);

            if let Some(data_dir) = portable::data_dir() {
                win_builder = win_builder.data_directory(data_dir.join("webview"));
            }

            let main_window = win_builder.build()?;

            // Kōrero (v1.9.0, W2): one-time window-state migration.
            // Pre-v1.7.0 builds stored 900×820 in .window-state.json. If that stale
            // file is still present it overrides the new 820×640 default on every
            // launch. We delete it once (on first v1.9.0 launch) and write a marker
            // so subsequent launches restore normally — the user's own resize choices
            // accumulate in a fresh file and persist from that point forward.
            // Failure is non-fatal: if we can't delete, restore_state handles it.
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                if let Ok(data_dir) = app.path().app_data_dir() {
                    let marker = data_dir.join(".korero-window-reset-v190");
                    if !marker.exists() {
                        let state_file = data_dir.join(".window-state.json");
                        if state_file.exists() {
                            match std::fs::remove_file(&state_file) {
                                Ok(()) => log::info!(
                                    "window-state: cleared pre-v1.7.0 state file (one-time migration)."
                                ),
                                Err(e) => log::warn!(
                                    "window-state: could not clear stale state file: {e}"
                                ),
                            }
                        }
                        let _ = std::fs::write(&marker, "v1.9.0");
                    }
                }
            }

            // Kōrero (2026-05-17 PM): restore saved window state (size,
            // position, maximised). The plugin auto-saves on close; we call
            // restore manually because programmatic windows don't trip the
            // plugin's auto-restore hook. StateFlags::all() covers size +
            // position + maximised + fullscreen + visible. Failure is non-
            // fatal — if the state file is missing or corrupt, the window
            // launches at the inner_size defaults above and a fresh state
            // file is written on next close.
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if let Err(err) = main_window.restore_state(StateFlags::all()) {
                log::warn!("window-state: failed to restore main window state: {err}");
            }

            let mut settings = get_settings(&app.handle());

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            // Store the file log level in the atomic for the filter to use
            FILE_LOG_LEVEL.store(file_log_level.to_level_filter() as u8, Ordering::Relaxed);

            // Kōrero (v1.12.0): seed crash-report saving from settings and point
            // the panic hook at <app data>/crash-reports.
            crash::SAVE_CRASH_REPORTS.store(settings.save_crash_reports, Ordering::Relaxed);
            if let Ok(data_dir) = portable::app_data_dir(&app.handle()) {
                crash::set_crash_dir(data_dir.join("crash-reports"));
            }

            let app_handle = app.handle().clone();
            app.manage(TranscriptionCoordinator::new(app_handle.clone()));
            // Kōrero (v1.13.0): meeting recorder singleton (mic + system loopback).
            app.manage(std::sync::Arc::new(meeting::MeetingRecorder::new()));
            // Kōrero (v1.13.6): age out meeting WAVs past the 30-day retention
            // window + any orphaned device-test files.
            meeting::cleanup_old_recordings(app.handle());
            // Kōrero (v1.16.0): one-shot update notification (fork repo only;
            // delayed 8 s; silent on any failure).
            update_check::spawn_update_check(app.handle().clone());
            // Kōrero (v1.17.0): if post-processing runs on local Ollama,
            // quietly make sure it's actually up (PC optimisers and reboots
            // routinely leave it stopped). Best-effort; never blocks startup.
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = tauri::async_runtime::spawn_blocking(|| {
                        std::thread::sleep(std::time::Duration::from_secs(12))
                    })
                    .await;
                    let settings = crate::settings::get_settings(&handle);
                    if settings.post_process_enabled {
                        if let Some(p) = settings.active_post_process_provider() {
                            if p.is_local_provider {
                                let up =
                                    crate::commands::ollama::ensure_running(&p.base_url).await;
                                log::info!("Startup Ollama check: running={up}");
                            }
                        }
                    }
                });
            }

            // Kōrero (v1.7.0, S1): seed the settings cache from the already-loaded
            // settings (which include hydrated keychain keys and any CLI overrides).
            // Registered BEFORE initialize_core_logic so every get_settings() call
            // during init hits RAM, not disk + keychain.
            app.manage(SettingsCache(Arc::new(RwLock::new(settings.clone()))));

            initialize_core_logic(&app_handle);

            // Pre-warm GPU/accelerator enumeration on a background thread.
            // The first call into transcribe_rs::whisper_cpp::gpu::list_gpu_devices
            // loads the Metal/Vulkan backend and probes devices, which can take
            // several seconds. Without this, that cost is paid synchronously the
            // first time the user opens the Advanced settings page (which calls
            // the get_available_accelerators command), causing a UI freeze.
            // Result is cached in a OnceLock inside the transcription manager.
            std::thread::spawn(|| {
                let _ = crate::managers::transcription::get_available_accelerators();
            });

            // Hide tray icon if --no-tray was passed
            if cli_args.no_tray {
                tray::set_tray_visibility(&app_handle, false);
            }

            // Show main window only if not starting hidden.
            // CLI --start-hidden flag overrides the setting.
            // But if permission onboarding is required, always show the window.
            let should_hide = settings.start_hidden || cli_args.start_hidden;
            let should_force_show = should_force_show_permissions_window(&app_handle);

            // If start_hidden but tray is disabled, we must show the window
            // anyway. Without a tray icon, the dock is the only way back in.
            let tray_available = settings.show_tray_icon && !cli_args.no_tray;
            if should_force_show || !should_hide || !tray_available {
                show_main_window(&app_handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _res = window.hide();

                #[cfg(target_os = "macos")]
                {
                    let settings = get_settings(&window.app_handle());
                    let tray_visible =
                        settings.show_tray_icon && !window.app_handle().state::<CliArgs>().no_tray;
                    if tray_visible {
                        // Tray is available: hide the dock icon, app lives in the tray
                        let res = window
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
                        if let Err(e) = res {
                            log::error!("Failed to set activation policy: {}", e);
                        }
                    }
                    // No tray: keep the dock icon visible so the user can reopen
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                // Update tray icon to match new theme, maintaining idle state
                utils::change_tray_icon(&window.app_handle(), utils::TrayIconState::Idle);
            }
            _ => {}
        })
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &event {
                show_main_window(app);
            }
            let _ = (app, event); // suppress unused warnings on non-macOS
        });
}