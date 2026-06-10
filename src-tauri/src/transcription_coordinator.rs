// Kōrero (v1.8.0): overlay replacement of upstream transcription_coordinator.rs.
// Adds latch recording mode: a quick double-tap of the shortcut (press →
// release → press, all within LATCH_WINDOW) activates latched recording so the
// user does not need to hold the key down for long dictations.
// Stopping latch mode: a single press while in RecordingLatched state.
// Visual feedback: the overlay emits show-overlay("recording-latched"), which
// drives an amber/orange CSS theme distinct from the normal aurora-cyan recording
// indicator. See RecordingOverlay.css [data-state="recording-latched"].
//
// All other behaviour (toggle mode, cancel, ProcessingFinished, debounce) is
// unchanged from upstream.
use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use log::{debug, error, warn};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);

/// How long after a PTT key-release to wait for a second press before
/// executing the stop. A second press within this window enters latch mode
/// instead of stopping. 300 ms matches standard double-click timing.
const LATCH_WINDOW: Duration = Duration::from_millis(300);

/// Commands processed sequentially by the coordinator thread.
enum Command {
    Input {
        binding_id: String,
        hotkey_string: String,
        is_pressed: bool,
        push_to_talk: bool,
    },
    Cancel {
        recording_was_active: bool,
    },
    ProcessingFinished,
}

/// Pipeline lifecycle, owned exclusively by the coordinator thread.
enum Stage {
    Idle,
    Recording(String),        // binding_id — PTT hold or toggle
    RecordingLatched(String), // binding_id — double-tap latch; stops on next key press
    Processing,
}

/// Tracks a deferred PTT stop while waiting out the LATCH_WINDOW.
struct PendingStop {
    binding_id: String,
    hotkey_string: String,
    /// When the key was released; used to compute remaining window duration.
    at: Instant,
}

/// Serialises all transcription lifecycle events through a single thread
/// to eliminate race conditions between keyboard shortcuts, signals, and
/// the async transcribe-paste pipeline.
pub struct TranscriptionCoordinator {
    tx: Sender<Command>,
}

pub fn is_transcribe_binding(id: &str) -> bool {
    // Kōrero (v1.14.1): transcribe_alt is the one-handed alternative shortcut;
    // it runs the exact same pipeline as "transcribe".
    id == "transcribe" || id == "transcribe_with_post_process" || id == "transcribe_alt"
}

impl TranscriptionCoordinator {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                let mut last_press: Option<Instant> = None;
                // Kōrero (v1.8.0): pending stop for latch detection.
                // Set on PTT key-release while Recording; cleared on timeout
                // (execute stop), second press (enter latch), or Cancel.
                let mut pending_stop: Option<PendingStop> = None;

                loop {
                    // When a pending stop is set, compute remaining time in
                    // the latch window and use recv_timeout so we execute the
                    // stop automatically if no second press arrives in time.
                    // When idle (no pending stop), block indefinitely.
                    let timeout = pending_stop
                        .as_ref()
                        .map(|p| LATCH_WINDOW.saturating_sub(p.at.elapsed()));

                    let cmd_opt = match timeout {
                        Some(dur) => match rx.recv_timeout(dur) {
                            Ok(cmd) => Some(cmd),
                            Err(RecvTimeoutError::Timeout) => None,
                            Err(RecvTimeoutError::Disconnected) => break,
                        },
                        None => match rx.recv() {
                            Ok(cmd) => Some(cmd),
                            Err(_) => break,
                        },
                    };

                    match cmd_opt {
                        // ── Timeout: latch window expired, execute stop ────────
                        None => {
                            if let Some(ps) = pending_stop.take() {
                                stop(&app, &mut stage, &ps.binding_id, &ps.hotkey_string);
                            }
                        }

                        // ── Command received ──────────────────────────────────
                        Some(Command::Input {
                            binding_id,
                            hotkey_string,
                            is_pressed,
                            push_to_talk,
                        }) => {
                            // Debounce rapid-fire press events (key repeat).
                            // Releases always pass through for push-to-talk.
                            if is_pressed {
                                let now = Instant::now();
                                if last_press
                                    .map_or(false, |t| now.duration_since(t) < DEBOUNCE)
                                {
                                    debug!("Debounced press for '{binding_id}'");
                                    continue;
                                }
                                last_press = Some(now);
                            }

                            if push_to_talk {
                                handle_ptt(
                                    &app,
                                    &mut stage,
                                    &mut pending_stop,
                                    &binding_id,
                                    &hotkey_string,
                                    is_pressed,
                                );
                            } else if is_pressed {
                                // Toggle mode: unchanged from upstream, but also
                                // handles RecordingLatched in case the user
                                // switches modes between sessions.
                                match &stage {
                                    // Kōrero (v1.18.0): Processing no longer
                                    // blocks the next recording (see handle_ptt).
                                    Stage::Idle | Stage::Processing => {
                                        start(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                    Stage::Recording(id) if id == &binding_id => {
                                        stop(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                    Stage::RecordingLatched(id) if id == &binding_id => {
                                        stop(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                    _ => {
                                        debug!(
                                            "Ignoring press for '{binding_id}': pipeline busy"
                                        )
                                    }
                                }
                            }
                        }

                        Some(Command::Cancel { recording_was_active }) => {
                            // Discard any pending latch window; we're cancelling.
                            pending_stop = None;
                            if !matches!(stage, Stage::Processing)
                                && (recording_was_active
                                    || matches!(
                                        stage,
                                        Stage::Recording(_) | Stage::RecordingLatched(_)
                                    ))
                            {
                                stage = Stage::Idle;
                            }
                        }

                        Some(Command::ProcessingFinished) => {
                            // Kōrero (v1.18.0): only reset when actually in
                            // Processing. A new recording may already be live
                            // (press-during-processing) — clobbering it to
                            // Idle would orphan the recorder: the release/stop
                            // press would no longer match Stage::Recording and
                            // the mic would record forever.
                            if matches!(stage, Stage::Processing) {
                                stage = Stage::Idle;
                            }
                        }
                    }
                }
                debug!("Transcription coordinator exited");
            }));
            if let Err(e) = result {
                error!("Transcription coordinator panicked: {e:?}");
            }
        });

        Self { tx }
    }

    /// Send a keyboard/signal input event for a transcribe binding.
    /// For signal-based toggles, use `is_pressed: true` and `push_to_talk: false`.
    pub fn send_input(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        push_to_talk: bool,
    ) {
        if self
            .tx
            .send(Command::Input {
                binding_id: binding_id.to_string(),
                hotkey_string: hotkey_string.to_string(),
                is_pressed,
                push_to_talk,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_cancel(&self, recording_was_active: bool) {
        if self
            .tx
            .send(Command::Cancel {
                recording_was_active,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_processing_finished(&self) {
        if self.tx.send(Command::ProcessingFinished).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }
}

// ── PTT handler (latch logic lives here) ─────────────────────────────────────

/// Handles a PTT key event with latch-mode detection.
///
/// Normal PTT: press → record, release → stop.
/// Latch: press → record, release → start LATCH_WINDOW timer,
///        second press within window → latch ON (amber overlay),
///        next press → stop.
fn handle_ptt(
    app: &AppHandle,
    stage: &mut Stage,
    pending_stop: &mut Option<PendingStop>,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
) {
    if is_pressed {
        // Check for double-tap: a pending stop exists for this binding and
        // the stage is still Recording (hasn't been cancelled underneath us).
        let is_double_tap = pending_stop
            .as_ref()
            .map_or(false, |ps| ps.binding_id == binding_id)
            && matches!(stage, Stage::Recording(id) if id == binding_id);

        if is_double_tap {
            pending_stop.take(); // consume — no longer needed
            enter_latch(app, stage, binding_id);
            return;
        }

        // Not a double-tap: clear any stale pending stop (e.g. a different
        // binding, or stage changed due to cancel) before handling the press.
        pending_stop.take();

        match stage {
            // Kōrero (v1.18.0, UX roadmap item 5 — zero-latency sequencing):
            // a press during Processing starts the NEXT recording immediately
            // instead of being swallowed ("pipeline busy"). The in-flight
            // transcription continues in its own task; the model serialises
            // utterances internally, and ProcessingFinished no longer
            // clobbers a live Recording stage (see the guarded reset).
            Stage::Idle | Stage::Processing => {
                start(app, stage, binding_id, hotkey_string);
            }
            Stage::RecordingLatched(id) if id == binding_id => {
                // Single press while latched → stop.
                stop(app, stage, binding_id, hotkey_string);
            }
            _ => {
                debug!("Ignoring PTT press for '{binding_id}': pipeline busy");
            }
        }
    } else {
        // Key release:
        // If we were recording this binding, defer the stop to allow a
        // second press (latch double-tap) to arrive within LATCH_WINDOW.
        if matches!(stage, Stage::Recording(id) if id == binding_id) {
            *pending_stop = Some(PendingStop {
                binding_id: binding_id.to_string(),
                hotkey_string: hotkey_string.to_string(),
                at: Instant::now(),
            });
        }
        // Release in RecordingLatched state is intentionally ignored — the
        // user must press (not release) the key to end latch recording.
    }
}

// ── Stage helpers ─────────────────────────────────────────────────────────────

fn start(app: &AppHandle, stage: &mut Stage, binding_id: &str, hotkey_string: &str) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.start(app, binding_id, hotkey_string);
    if app
        .try_state::<Arc<AudioRecordingManager>>()
        .map_or(false, |a| a.is_recording())
    {
        *stage = Stage::Recording(binding_id.to_string());
    } else {
        debug!("Start for '{binding_id}' did not begin recording; staying idle");
    }
}

fn stop(app: &AppHandle, stage: &mut Stage, binding_id: &str, hotkey_string: &str) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.stop(app, binding_id, hotkey_string);
    *stage = Stage::Processing;
}

/// Transitions Recording → RecordingLatched and signals the overlay to
/// switch to its amber "latch active" visual.
///
/// The overlay's existing `show-overlay` event handler already reads the
/// payload as the new `OverlayState` string, so emitting
/// `show-overlay("recording-latched")` is sufficient — no new event needed.
fn enter_latch(app: &AppHandle, stage: &mut Stage, binding_id: &str) {
    *stage = Stage::RecordingLatched(binding_id.to_string());
    if let Err(e) = app.emit("show-overlay", "recording-latched") {
        warn!("Failed to emit show-overlay(recording-latched): {e}");
    }
    debug!("Latch mode activated for '{binding_id}'");
}
