//! Kōrero fork (v1.13.0): meeting recorder with failsafe recovery.
//!
//! Captures your microphone ("You") and the system output via WASAPI loopback
//! ("Others") at once, then transcribes each. Because the two streams are
//! captured separately we get a free "You vs Others" split without diarization.
//!
//! FAILSAFE DESIGN: the recording is the irreplaceable artifact, so on stop we
//! write both streams to WAV on disk FIRST, then transcribe. Transcription is
//! non-fatal — if it fails (or returns nothing) the meeting still comes back
//! with its saved audio paths so it can be re-transcribed from disk later via
//! `meeting_transcribe_file`. `meeting_list_recordings` enumerates everything on
//! disk so a meeting whose app session ended can still be recovered.
//!
//! The model is pre-warmed on start and kept resident for the whole meeting
//! (the idle-unload watcher honours `is_meeting_active()`), so a long call can't
//! have its model unloaded out from under the final transcription.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};

use crate::audio_toolkit::audio::FrameResampler;
use crate::audio_toolkit::list_input_devices;
use crate::meeting_capture::{
    LiveSegment, SegmentSender, Segmenter, StreamCapture, SEG_QUIET_RMS,
};
#[cfg(windows)]
use crate::meeting_capture_wasapi::WasapiLoopback;
use crate::managers::audio::AudioRecordingManager;
use crate::managers::transcription::TranscriptionManager;

/// True while a meeting is being captured. The transcription idle-unload watcher
/// checks this so the model stays loaded for the whole meeting.
static MEETING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether a meeting capture is currently running.
pub fn is_meeting_active() -> bool {
    MEETING_ACTIVE.load(Ordering::Relaxed)
}

/// Result of stopping a meeting. Audio paths are always populated (recording is
/// saved before transcription); transcript fields may be empty if transcription
/// failed — the audio can then be re-transcribed from disk.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct MeetingResult {
    pub you: String,
    pub others: String,
    /// v1.17.0: the transcript as an ORDERED, interleaved list of segments
    /// (each tagged "you"/"others"), so the UI can render both speakers in
    /// chronological order. `you`/`others` are retained (the same text grouped
    /// per speaker) for the per-stream edit / re-transcribe affordances.
    pub segments: Vec<TranscriptSeg>,
    pub mic_path: Option<String>,
    pub system_path: Option<String>,
}

/// v1.17.0: one chronological transcript segment. `source` is "you" or
/// "others"; the UI maps that to the meeting's editable speaker label.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct TranscriptSeg {
    pub source: String,
    pub text: String,
}

/// A meeting WAV on disk, for the recovery list.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct RecordingFile {
    pub path: String,
    pub file_name: String,
    pub modified: u64,
}

/// Phase A: both sides stream straight to WAVs on disk (see meeting_capture.rs).
struct ActiveCapture {
    mic: StreamCapture,
    system: Option<SystemCapture>,
    /// Phase B (v1.14.0): transcript accumulated live during the meeting.
    live: Arc<LiveTranscript>,
    /// Phase B: the segment-transcription consumer thread; joined on stop
    /// AFTER the capture workers (whose exit closes the segment channel).
    consumer: Option<std::thread::JoinHandle<()>>,
    /// v1.14.2: when the capture started — lets the UI restore its recording
    /// state (elapsed clock included) after the page unmounts mid-meeting.
    started_at: std::time::Instant,
    /// v1.19.0: shared pause flag — both capture workers read it and drop
    /// frames while set. The streams stay open (no device re-acquisition).
    paused: Arc<AtomicBool>,
    /// v1.19.0: total time spent paused so far, plus the start of the current
    /// pause (if any). The elapsed clock the UI shows excludes both.
    paused_total: Duration,
    paused_since: Option<std::time::Instant>,
}

impl ActiveCapture {
    /// Wall-clock since start, minus all paused time (including any pause in
    /// progress). This is the "recording time" the UI shows and the WAV length
    /// roughly tracks.
    fn elapsed(&self) -> Duration {
        let ongoing = self.paused_since.map(|t| t.elapsed()).unwrap_or_default();
        self.started_at
            .elapsed()
            .saturating_sub(self.paused_total + ongoing)
    }
    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

/// Phase B (v1.14.0): live transcript, appended to by the consumer thread,
/// read at stop. Poisoning is tolerated — a panicked appender loses one
/// segment, not the meeting (the WAV fallback still exists).
/// v1.17.0: one ORDERED log of segments across BOTH sources, replacing the
/// previous two per-speaker strings. Each entry keeps its capture-relative
/// `start_ms`, so the final transcript can interleave the speakers in the
/// order they actually spoke instead of "all of you, then all of them".
#[derive(Default)]
struct LiveTranscript {
    /// (start_ms, source, text), appended in segment-arrival order.
    segs: Mutex<Vec<(u64, &'static str, String)>>,
}

impl LiveTranscript {
    /// Append a transcribed segment. v1.19.0: collapses a consecutive duplicate
    /// from the SAME source — a near-silence hallucination repeats the same
    /// phrase across back-to-back segments ("You: Thank you." ×9), so a segment
    /// whose normalised text matches this source's most recent entry is
    /// dropped. Returns `true` if the segment was actually appended (the caller
    /// only emits a UI event then), `false` if it was collapsed away.
    fn append(&self, start_ms: u64, source: &'static str, text: &str) -> bool {
        let mut g = self.segs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((_, _, last)) = g.iter().rev().find(|(_, s, _)| *s == source) {
            if normalise_phrase(last) == normalise_phrase(text) {
                return false;
            }
        }
        g.push((start_ms, source, text.to_string()));
        true
    }

    /// A chronologically-sorted snapshot of the live segments (stable sort by
    /// `start_ms`, preserving arrival order for equal timestamps).
    fn snapshot(&self) -> Vec<(u64, &'static str, String)> {
        let g = self.segs.lock().unwrap_or_else(|p| p.into_inner());
        let mut v: Vec<(u64, &'static str, String)> = g.clone();
        v.sort_by_key(|(ms, _, _)| *ms);
        v
    }
}

/// Join the segments belonging to one source into a single string — the
/// per-speaker view (`you` / `others`) the editing/re-transcribe UI expects.
/// `log` is assumed already chronologically ordered.
fn join_log(log: &[(u64, &'static str, String)], source: &str) -> String {
    let mut out = String::new();
    for (_, _src, text) in log.iter().filter(|(_, s, _)| *s == source) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(text);
    }
    out
}

// ---------------------------------------------------------------------------
// v1.19.0: anti-hallucination text guards (belt-and-braces behind the VAD).
//
// Whisper emits a small set of content-free "outro" phrases when handed
// near-silence or non-speech audio, and loops them ("Thank you." ×9). The VAD
// now stops most non-speech reaching the model at all; these text-level guards
// catch anything that slips through, on BOTH the live and the offline/import
// paths (the latter has no VAD).
// ---------------------------------------------------------------------------

/// Lowercase, collapse internal whitespace, strip surrounding punctuation — the
/// canonical form for duplicate/blocklist comparison.
fn normalise_phrase(s: &str) -> String {
    let lowered = s.trim().to_lowercase();
    let stripped = lowered.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '.' | '!' | '?' | ',' | ';' | ':' | '-' | '–' | '—' | '"' | '\'' | '…')
    });
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whisper's canonical near-silence hallucinations (already in normalised form).
const HALLUCINATION_PHRASES: &[&str] = &[
    "thank you",
    "thank you so much",
    "thank you very much",
    "thanks for watching",
    "thanks for watching everyone",
    "please subscribe",
    "see you next time",
    "see you in the next video",
    "bye",
    "bye bye",
    "you",
    "okay",
    "ok",
];

/// True if `text` is (only) a known content-free hallucination phrase.
fn is_hallucination_phrase(text: &str) -> bool {
    let n = normalise_phrase(text);
    !n.is_empty() && HALLUCINATION_PHRASES.contains(&n.as_str())
}

/// Collapse consecutive repeated sentences inside one transcribed segment — the
/// other half of the hallucination signature, where the loop happens WITHIN a
/// single line ("It's great. It's great. It's great." → "It's great.").
///
/// Two safety properties (v1.19.0 red-team fixes):
///  * A `.` only ends a sentence when neither neighbour is a digit, so decimals
///    and versions ("$3.50", "v1.19") are NEVER split — which previously turned
///    "$3.50" into "$3. 50" on every import window.
///  * If no duplicate is actually removed, the ORIGINAL text is returned
///    verbatim — we never reflow ordinary text, so spacing/punctuation of a
///    clean transcript is left exactly as the model produced it.
fn collapse_repeats(text: &str) -> String {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut sentences: Vec<&str> = Vec::new();
    let mut start = 0usize;
    for idx in 0..chars.len() {
        let (i, c) = chars[idx];
        let is_boundary = match c {
            '!' | '?' => true,
            '.' => {
                let prev_digit = idx > 0 && chars[idx - 1].1.is_ascii_digit();
                let next_digit =
                    idx + 1 < chars.len() && chars[idx + 1].1.is_ascii_digit();
                !prev_digit && !next_digit
            }
            _ => false,
        };
        if is_boundary {
            let end = i + c.len_utf8();
            let s = text[start..end].trim();
            if !s.is_empty() {
                sentences.push(s);
            }
            start = end;
        }
    }
    if start < text.len() {
        let tail = text[start..].trim();
        if !tail.is_empty() {
            sentences.push(tail);
        }
    }
    if sentences.len() < 2 {
        return text.trim().to_string();
    }
    let mut out: Vec<&str> = Vec::with_capacity(sentences.len());
    let mut last_norm = String::new();
    let mut dropped_any = false;
    for s in sentences {
        let n = normalise_phrase(s);
        if n.is_empty() {
            continue;
        }
        if n == last_norm {
            dropped_any = true;
            continue; // consecutive duplicate — drop
        }
        last_norm = n;
        out.push(s);
    }
    // Only reflow when we genuinely removed a duplicate; an ordinary transcript
    // is returned untouched.
    if !dropped_any {
        return text.trim().to_string();
    }
    out.join(" ")
}

/// Apply the segment-level guards to one freshly transcribed segment:
/// intra-segment repeat collapse, plus a near-energy-floor drop of canonical
/// hallucination phrases. `peak_rms` is the segment's peak frame RMS; `None`
/// (offline path) skips the energy test but still collapses repeats. Returns
/// `None` when the whole segment should be discarded.
fn clean_segment_text(text: &str, peak_rms: Option<f32>) -> Option<String> {
    let collapsed = collapse_repeats(text);
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }
    let near_gate = peak_rms.map(|r| r < SEG_QUIET_RMS * 3.0).unwrap_or(false);
    let word_count = trimmed.split_whitespace().count();
    if near_gate && word_count <= 3 && is_hallucination_phrase(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

/// v1.13.6: the "Others" stream can come from two backends — native WASAPI
/// loopback (preferred; the cpal input-stream-on-output-device approach
/// proved unreliable on real hardware) or cpal as the fallback.
enum SystemCapture {
    Cpal(StreamCapture),
    #[cfg(windows)]
    Wasapi(WasapiLoopback),
}

impl SystemCapture {
    fn path(&self) -> &PathBuf {
        match self {
            SystemCapture::Cpal(c) => &c.path,
            #[cfg(windows)]
            SystemCapture::Wasapi(c) => &c.path,
        }
    }
    fn stop(self) -> Result<u64, String> {
        match self {
            SystemCapture::Cpal(c) => c.stop(),
            #[cfg(windows)]
            SystemCapture::Wasapi(c) => c.stop(),
        }
    }
}

/// Start the system-audio ("Others") capture, preferring native WASAPI
/// loopback and falling back to cpal. Returns the capture (None if both
/// backends fail) plus a label naming the backend that ran, which the device
/// test surfaces so a misbehaving backend is identifiable from the UI.
fn start_system_capture(
    app: &AppHandle,
    path: PathBuf,
    segments: Option<SegmentSender>,
    paused: Arc<AtomicBool>,
) -> (Option<SystemCapture>, &'static str) {
    #[cfg(windows)]
    {
        match WasapiLoopback::start(
            path.clone(),
            Some(app.clone()),
            "others",
            segments.clone(),
            paused.clone(),
        ) {
            Ok(c) => return (Some(SystemCapture::Wasapi(c)), "WASAPI"),
            Err(e) => {
                log::warn!("Native WASAPI loopback failed ({e}); trying cpal loopback.");
            }
        }
    }
    let cpal_try = (|| -> Result<StreamCapture, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "No output device found".to_string())?;
        let config = device
            .default_output_config()
            .map_err(|e| format!("Output config: {e}"))?;
        StreamCapture::start(device, config, path, Some(app.clone()), "others", segments, paused)
    })();
    match cpal_try {
        Ok(c) => (Some(SystemCapture::Cpal(c)), "cpal"),
        Err(e) => {
            log::warn!("cpal loopback also failed: {e}");
            (None, "none")
        }
    }
}

pub struct MeetingRecorder {
    active: Mutex<Option<ActiveCapture>>,
}

impl MeetingRecorder {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }
}

impl Default for MeetingRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve the user's selected microphone to a cpal Device, falling back to the
/// system default (None) if none is configured or it can't be found. Mirrors the
/// dictation recorder so a meeting captures the SAME mic the user picked —
/// `open(None)` would silently record the system default, which is the likely
/// cause of an empty "You" transcript when the selected mic isn't the default.
fn selected_input_device(app: &AppHandle) -> Option<cpal::Device> {
    let name = crate::settings::get_settings(app).selected_microphone?;
    list_input_devices()
        .ok()?
        .into_iter()
        .find(|d| d.name == name)
        .map(|d| d.device)
}

fn meetings_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = crate::portable::app_data_dir(app)
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))?
        .join("meetings");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create meetings dir: {e}"))?;
    Ok(dir)
}

/// Begin capturing the meeting (microphone + system loopback).
#[tauri::command]
#[specta::specta]
pub async fn meeting_start_capture(
    app: AppHandle,
    meeting: State<'_, Arc<MeetingRecorder>>,
    recording_manager: State<'_, Arc<AudioRecordingManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
) -> Result<bool, String> {
    if recording_manager.is_recording() {
        return Err("A dictation recording is in progress. Stop it before starting a meeting.".to_string());
    }

    let mut guard = meeting.active.lock().map_err(|_| "lock poisoned")?;
    if guard.is_some() {
        return Err("A meeting is already being recorded.".to_string());
    }

    // Mark active + pre-warm the model so it is loaded by the time we stop, and
    // so the idle watcher keeps it resident for the whole meeting.
    MEETING_ACTIVE.store(true, Ordering::Relaxed);
    transcription_manager.initiate_model_load();

    // Phase A: streaming WAV paths are created at START — the recording exists
    // on disk from the first seconds (failsafe), and memory stays bounded.
    let dir = match meetings_dir(&app) {
        Ok(d) => d,
        Err(e) => {
            MEETING_ACTIVE.store(false, Ordering::Relaxed);
            return Err(e);
        }
    };
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let you_path = dir.join(format!("meeting-{stamp}-you.wav"));
    let others_path = dir.join(format!("meeting-{stamp}-others.wav"));

    // Phase B (v1.14.0): live transcription pipeline. The captures cut
    // speech segments onto this bounded channel; ONE consumer thread
    // transcribes them in arrival order (the model is resident for the whole
    // meeting) and streams `meeting-live-segment` events to the UI while
    // accumulating the transcript for stop. Drop-on-full + non-fatal errors
    // throughout: the WAV on disk remains the source of truth.
    let (seg_tx, seg_rx) = std::sync::mpsc::sync_channel::<LiveSegment>(8);
    let live = Arc::new(LiveTranscript::default());
    // v1.19.0: one pause flag shared by both capture workers.
    let paused = Arc::new(AtomicBool::new(false));
    let consumer = {
        let live = live.clone();
        let tm = transcription_manager.inner().clone();
        let app_ev = app.clone();
        std::thread::spawn(move || {
            #[derive(serde::Serialize, Clone)]
            struct LiveEvent {
                source: &'static str,
                text: String,
            }
            while let Ok(seg) = seg_rx.recv() {
                match tm.transcribe(seg.samples) {
                    Ok(text) => {
                        // v1.19.0 guards (d)+(b): collapse intra-segment repeats
                        // and drop near-floor hallucination phrases.
                        let text = match clean_segment_text(&text, Some(seg.peak_rms)) {
                            Some(t) => t,
                            None => continue,
                        };
                        // guard (a): append collapses a cross-segment duplicate
                        // and tells us whether to surface the event.
                        if !live.append(seg.start_ms, seg.source, &text) {
                            continue;
                        }
                        use tauri::Emitter;
                        let _ = app_ev.emit(
                            "meeting-live-segment",
                            LiveEvent {
                                source: seg.source,
                                text,
                            },
                        );
                    }
                    Err(e) => log::warn!(
                        "Live segment transcription failed (the WAV still has the audio): {e}"
                    ),
                }
            }
        })
    };

    // Microphone ("You") — the user's selected mic, else the default input.
    let mic = (|| -> Result<StreamCapture, String> {
        let device = match selected_input_device(&app) {
            Some(d) => d,
            None => cpal::default_host()
                .default_input_device()
                .ok_or_else(|| "No input device found".to_string())?,
        };
        let config = device
            .default_input_config()
            .map_err(|e| format!("Mic config: {e}"))?;
        StreamCapture::start(
            device,
            config,
            you_path,
            Some(app.clone()),
            "you",
            Some(seg_tx.clone()),
            paused.clone(),
        )
    })();
    let mic = match mic {
        Ok(m) => m,
        Err(e) => {
            MEETING_ACTIVE.store(false, Ordering::Relaxed);
            // seg_tx (and the clone, on the failed worker) drop here, so the
            // consumer thread exits on its own.
            return Err(format!("Microphone start failed: {e}"));
        }
    };

    // System ("Others") — v1.13.6: native WASAPI loopback first, cpal
    // fallback. Best-effort: mic-only if both fail. `seg_tx` is MOVED here so
    // no sender survives outside the capture workers — that's what lets the
    // consumer exit when the workers finish.
    let (system, backend) = start_system_capture(&app, others_path, Some(seg_tx), paused.clone());
    let system_captured = system.is_some();
    if system_captured {
        log::info!("Meeting system capture started (backend: {backend}).");
    } else {
        log::warn!("Meeting: system-loopback capture unavailable; recording microphone only.");
    }

    *guard = Some(ActiveCapture {
        mic,
        system,
        live,
        consumer: Some(consumer),
        started_at: std::time::Instant::now(),
        paused,
        paused_total: Duration::ZERO,
        paused_since: None,
    });
    Ok(system_captured)
}

/// v1.14.2: snapshot of an in-progress recording, so the Meetings page can
/// RESTORE its UI after being unmounted (user navigated away) mid-meeting.
/// Previously the page came back showing idle while the backend kept
/// recording — with no way to stop the meeting short of restarting the app.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct MeetingStatus {
    pub elapsed_secs: u32,
    pub system_captured: bool,
    /// v1.19.0: whether the meeting is currently paused.
    pub paused: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_recording_status(
    meeting: State<'_, Arc<MeetingRecorder>>,
) -> Result<Option<MeetingStatus>, String> {
    let guard = meeting.active.lock().map_err(|_| "lock poisoned")?;
    Ok(guard.as_ref().map(|c| MeetingStatus {
        // v1.19.0: elapsed excludes paused time.
        elapsed_secs: c.elapsed().as_secs().min(u32::MAX as u64) as u32,
        system_captured: c.system.is_some(),
        paused: c.is_paused(),
    }))
}

/// v1.19.0: pause the meeting — both capture workers start dropping frames, the
/// elapsed clock stops, and the live transcript receives no new segments. The
/// device streams stay OPEN (no re-acquisition risk), so the mic indicator
/// stays on. Idempotent: pausing an already-paused meeting is a no-op.
#[tauri::command]
#[specta::specta]
pub async fn meeting_pause(
    meeting: State<'_, Arc<MeetingRecorder>>,
) -> Result<bool, String> {
    let mut guard = meeting.active.lock().map_err(|_| "lock poisoned")?;
    let cap = guard.as_mut().ok_or("No meeting is being recorded.")?;
    if !cap.is_paused() {
        cap.paused.store(true, Ordering::Relaxed);
        cap.paused_since = Some(std::time::Instant::now());
    }
    Ok(true)
}

/// v1.19.0: resume a paused meeting — capture workers start writing again and
/// the elapsed clock advances. The time spent paused is folded into
/// `paused_total` so it is permanently excluded from elapsed. Idempotent.
#[tauri::command]
#[specta::specta]
pub async fn meeting_resume(
    meeting: State<'_, Arc<MeetingRecorder>>,
) -> Result<bool, String> {
    let mut guard = meeting.active.lock().map_err(|_| "lock poisoned")?;
    let cap = guard.as_mut().ok_or("No meeting is being recorded.")?;
    if cap.is_paused() {
        cap.paused.store(false, Ordering::Relaxed);
        if let Some(since) = cap.paused_since.take() {
            cap.paused_total += since.elapsed();
        }
    }
    Ok(false)
}

/// Stop the meeting: save both streams to disk, then transcribe (non-fatal).
#[tauri::command]
#[specta::specta]
pub async fn meeting_stop_capture(
    app: AppHandle,
    meeting: State<'_, Arc<MeetingRecorder>>,
) -> Result<MeetingResult, String> {
    MEETING_ACTIVE.store(false, Ordering::Relaxed);

    let capture = {
        let mut guard = meeting.active.lock().map_err(|_| "lock poisoned")?;
        guard.take()
    };
    let capture = capture.ok_or_else(|| "No meeting is being recorded.".to_string())?;

    // Phase A: stop + finalise both streaming captures on a blocking thread.
    // Joins are BOUNDED by design (the capture worker uses a timed recv), so a
    // silent loopback can no longer wedge the stop path.
    let ActiveCapture {
        mic,
        system,
        live,
        consumer,
        started_at: _,
        paused: _,
        paused_total: _,
        paused_since: _,
    } = capture;
    let mic_pathbuf = mic.path.clone();
    let sys_pathbuf = system.as_ref().map(|s| s.path().clone());
    let (mic_written, sys_written) = tauri::async_runtime::spawn_blocking(move || {
        let m = mic.stop().unwrap_or_else(|e| {
            log::error!("Mic capture stop failed: {e}");
            0
        });
        let s = match system {
            Some(sc) => sc.stop().unwrap_or_else(|e| {
                log::error!("System capture stop failed: {e}");
                0
            }),
            None => 0,
        };
        // Phase B: with both workers joined, every segment sender is dropped —
        // the consumer drains what's queued (a few seconds at most) and exits.
        if let Some(h) = consumer {
            let _ = h.join();
        }
        (m, s)
    })
    .await
    .map_err(|e| format!("Capture stop task failed: {e}"))?;

    log::info!(
        "Meeting stop: mic={} samples (~{}s) streamed to disk, system={} samples (~{}s)",
        mic_written,
        mic_written / 16_000,
        sys_written,
        sys_written / 16_000,
    );

    // The recording is ALREADY on disk (streamed during capture). Keep non-empty
    // files; discard empties so they don't clutter the recovery list.
    let mic_path = keep_or_discard(mic_pathbuf, mic_written);
    let system_path = match sys_pathbuf {
        Some(p) => keep_or_discard(p, sys_written),
        None => None,
    };

    // --- Build the transcript (Phase B, v1.14.0; chronological in v1.17.0):
    // prefer the LIVE segment log accumulated during the meeting (stop becomes
    // near-instant). For any source whose live text came up empty but whose WAV
    // demonstrably has audio, segment that WAV OFFLINE (same energy gate, so the
    // recovered segments carry comparable start times) and merge it in. The
    // whole log is then sorted by start time, so both speakers interleave in the
    // order they actually spoke instead of "all of you, then all of them".
    // Non-fatal throughout: the audio is safe on disk regardless.
    let tm = app.state::<Arc<TranscriptionManager>>().inner().clone();
    let mut seg_log = live.snapshot();
    let has_you = seg_log.iter().any(|(_, s, _)| *s == "you");
    let has_others = seg_log.iter().any(|(_, s, _)| *s == "others");
    if !has_you {
        if let Some(p) = &mic_path {
            seg_log.extend(segment_wav_offline(&tm, p, "you").await);
        }
    }
    if !has_others {
        if let Some(p) = &system_path {
            seg_log.extend(segment_wav_offline(&tm, p, "others").await);
        }
    }
    seg_log.sort_by_key(|(ms, _, _)| *ms);

    let segments: Vec<TranscriptSeg> = seg_log
        .iter()
        .map(|(_, s, t)| TranscriptSeg {
            source: s.to_string(),
            text: t.clone(),
        })
        .collect();
    let you = join_log(&seg_log, "you");
    let others = join_log(&seg_log, "others");

    // v1.13.6: opportunistic retention sweep — keeps the meetings folder from
    // growing without bound (~230 MB per recorded meeting-hour).
    cleanup_old_recordings(&app);

    Ok(MeetingResult {
        you,
        others,
        segments,
        mic_path,
        system_path,
    })
}

/// v1.13.6: meeting recordings are kept this many days. Transcripts live in
/// the meetings store and survive; only the bulky WAV audio is aged out.
const RECORDING_RETENTION_DAYS: u64 = 30;

/// Delete meeting WAVs older than the retention window, plus any leftover
/// device-test files (a crash mid-test can orphan them). Called at startup
/// and after each meeting stop. Best-effort: errors are logged, never fatal.
pub fn cleanup_old_recordings(app: &AppHandle) {
    let Ok(dir) = meetings_dir(app) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(
            RECORDING_RETENTION_DAYS * 24 * 60 * 60,
        ))
        .unwrap_or(std::time::UNIX_EPOCH);
    let mut removed = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wav") {
            continue;
        }
        let is_test = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("test-"))
            .unwrap_or(false);
        let too_old = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| t < cutoff)
            .unwrap_or(false);
        if (too_old || is_test) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        log::info!(
            "Meetings cleanup: removed {removed} recording(s) (retention {RECORDING_RETENTION_DAYS} days)."
        );
    }
}

/// Delete a saved meeting WAV (recovery list / freeing space). Refuses
/// anything that isn't a .wav inside the meetings folder.
#[tauri::command]
#[specta::specta]
pub async fn meeting_delete_recording(app: AppHandle, path: String) -> Result<(), String> {
    let dir = meetings_dir(&app)?;
    let canon_dir = dir
        .canonicalize()
        .map_err(|e| format!("Meetings dir: {e}"))?;
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("Not found: {e}"))?;
    if !canon.starts_with(&canon_dir) || canon.extension().and_then(|e| e.to_str()) != Some("wav")
    {
        return Err("Refusing to delete a file outside the meetings folder.".to_string());
    }
    std::fs::remove_file(&canon).map_err(|e| format!("Delete failed: {e}"))
}

/// Transcribe a saved or imported audio file from disk.
/// v1.13.4: chunked — memory stays bounded regardless of recording length.
/// v1.16.1: non-WAV formats (m4a/aac/mp3/flac/ogg) decode via rodio into the
/// same bounded-memory windowing pipeline.
#[tauri::command]
#[specta::specta]
pub async fn meeting_transcribe_file(app: AppHandle, path: String) -> Result<String, String> {
    let tm = app.state::<Arc<TranscriptionManager>>().inner().clone();
    // v1.16.3: imports and re-transcribes can run with the model idle-unloaded.
    // Recording and Notes pre-warm at start; this path never did, so
    // transcribe() failed with "Model is not loaded for transcription".
    // initiate_model_load() claims the loading flag synchronously, and the
    // first transcribe() call blocks on the loading condvar until ready.
    tm.initiate_model_load();
    let ext = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    // v1.19.0: surface per-window progress to the import / re-transcribe UI.
    let progress = Some(TranscribeProgress {
        app: app.clone(),
        id: path.clone(),
    });
    match ext.as_str() {
        "wav" => transcribe_wav_chunked(&tm, &path, progress).await,
        "m4a" | "aac" | "mp4" | "mp3" | "flac" | "ogg" => {
            let (rate, channels, samples) = open_rodio_stream(&path)?;
            // Compressed length isn't cheaply known → indeterminate bar.
            transcribe_stream_chunked(&tm, rate, channels, samples, progress, None).await
        }
        other => Err(format!(
            "Unsupported audio format '.{other}' — use WAV, M4A, MP3, FLAC, or OGG."
        )),
    }
}

/// v1.17.0: re-transcribe a recorded meeting's two WAVs and return a single
/// CHRONOLOGICAL, interleaved segment log (the same shape `meeting_stop_capture`
/// returns). Either path may be absent (mic-only or system-only meeting). Used
/// by the Re-transcribe button so a rebuilt transcript stays in speaking order
/// instead of collapsing back to two per-speaker blocks. WAV only — recorded
/// meetings are always WAV; single-file imports use `meeting_transcribe_file`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_transcribe_merge(
    app: AppHandle,
    mic_path: Option<String>,
    system_path: Option<String>,
) -> Result<Vec<TranscriptSeg>, String> {
    let tm = app.state::<Arc<TranscriptionManager>>().inner().clone();
    // Match meeting_transcribe_file: the model may be idle-unloaded on this path.
    tm.initiate_model_load();
    let mut seg_log: Vec<(u64, &'static str, String)> = Vec::new();
    if let Some(p) = &mic_path {
        seg_log.extend(segment_wav_offline(&tm, p, "you").await);
    }
    if let Some(p) = &system_path {
        seg_log.extend(segment_wav_offline(&tm, p, "others").await);
    }
    seg_log.sort_by_key(|(ms, _, _)| *ms);
    Ok(seg_log
        .into_iter()
        .map(|(_, s, t)| TranscriptSeg {
            source: s.to_string(),
            text: t,
        })
        .collect())
}

/// List meeting WAVs saved on disk, newest first (for recovery).
#[tauri::command]
#[specta::specta]
pub async fn meeting_list_recordings(app: AppHandle) -> Result<Vec<RecordingFile>, String> {
    let dir = meetings_dir(&app)?;
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("Failed to read meetings dir: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wav") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        // v1.13.5: device-test WAVs are transient — keep them out of recovery.
        if file_name.starts_with("test-") {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(RecordingFile {
            path: path.to_string_lossy().to_string(),
            file_name,
            modified,
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

/// Export a transcript to a Markdown/text file in the meetings folder.
#[tauri::command]
#[specta::specta]
pub async fn meeting_export_transcript(
    app: AppHandle,
    file_name: String,
    content: String,
) -> Result<String, String> {
    let dir = meetings_dir(&app)?;
    let safe: String = file_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = if safe.trim().is_empty() {
        "meeting".to_string()
    } else {
        safe.trim().to_string()
    };
    let name = if base.ends_with(".md") || base.ends_with(".txt") {
        base
    } else {
        format!("{base}.md")
    };
    let path = dir.join(name);
    std::fs::write(&path, content).map_err(|e| format!("Failed to write export: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// Ask a question about a meeting transcript using the configured post-processing
/// LLM provider (e.g. Gemma via Ollama). Returns the model's answer.
#[tauri::command]
#[specta::specta]
pub async fn meeting_query(
    app: AppHandle,
    transcript: String,
    question: String,
) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| {
            "No post-processing provider is configured. Set one under Post Process.".to_string()
        })?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(format!(
            "No model is configured for provider '{}'. Set one under Post Process.",
            provider.id
        ));
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    // Cap the transcript so a very long meeting can't blow the model's context.
    const MAX_CHARS: usize = 48_000;
    let transcript = if transcript.chars().count() > MAX_CHARS {
        let kept: String = transcript.chars().take(MAX_CHARS).collect();
        format!("{kept}\n\n[Transcript truncated to fit the model's context window.]")
    } else {
        transcript
    };

    let system =
        "You are answering questions about a meeting transcript. Use only the information in the \
         transcript. If the answer is not present, say you cannot find it in the meeting."
            .to_string();
    let user = format!("Meeting transcript:\n\n{transcript}\n\n---\nQuestion: {question}");

    let answer = crate::llm_client::send_chat_completion_with_schema(
        &provider, api_key, &model, user, Some(system), None, None, None,
    )
    .await?;
    answer.ok_or_else(|| "The model returned no answer.".to_string())
}

/// Post-process a meeting transcript with a custom, per-meeting prompt, using the
/// configured post-processing provider/model. `prompt` becomes the system
/// instruction; `text` (the transcript) is the content. Returns the result.
#[tauri::command]
#[specta::specta]
pub async fn meeting_post_process(
    app: AppHandle,
    text: String,
    prompt: String,
) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| {
            "No post-processing provider is configured. Set one under Post Process.".to_string()
        })?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(format!(
            "No model is configured for provider '{}'. Set one under Post Process.",
            provider.id
        ));
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    // Cap the transcript so it can't blow the model's context window.
    const MAX_CHARS: usize = 48_000;
    let text = if text.chars().count() > MAX_CHARS {
        let kept: String = text.chars().take(MAX_CHARS).collect();
        format!("{kept}\n\n[Transcript truncated to fit the model's context window.]")
    } else {
        text
    };

    let mut system = if prompt.trim().is_empty() {
        "Summarise this meeting transcript: key points, decisions, and action items with owners."
            .to_string()
    } else {
        prompt
    };
    // v1.19.0: saved prompts (shared with dictation/Notes) are authored with a
    // trailing "Transcript:\n${output}" / "Dictation:\n${output}" block. Here the
    // transcript is the SEPARATE user message, so strip the placeholder tail (and
    // a now-dangling label line) to avoid a literal ${output} in the system text.
    if system.contains("${output}") {
        system = system.replace("${output}", "");
        let trimmed = system.trim_end();
        let cut = match trimmed.rfind('\n') {
            Some(nl) if trimmed[nl + 1..].trim_end().ends_with(':') => nl,
            _ => trimmed.len(),
        };
        system = trimmed[..cut].trim_end().to_string();
    }
    // v1.15.0: known mis-transcription glossary so the model corrects
    // near-miss variants while it works.
    if let Some(g) = crate::corrections::glossary_block(&settings.transcript_corrections) {
        system.push_str(&g);
    }

    // v1.17.0: stream the summary so tokens render as they generate instead of
    // the user staring at a spinner until the whole thing is done. Each delta is
    // emitted as a `meeting-postprocess-delta` event; the full text is also
    // returned so callers that don't listen still get the result.
    use tauri::Emitter;
    let app_ev = app.clone();
    // Keep copies for the non-streaming fallback below.
    let text_fallback = text.clone();
    let system_fallback = system.clone();
    let api_key_fallback = api_key.clone();
    let answer = crate::llm_client::stream_chat_completion(
        &provider,
        api_key,
        &model,
        text,
        Some(system),
        |delta| {
            let _ = app_ev.emit("meeting-postprocess-delta", delta.to_string());
        },
    )
    .await?;

    // Safety net: a provider that ignores `stream: true` (i.e. doesn't emit
    // OpenAI-style SSE deltas) yields no streamed text. Rather than regress to
    // an empty result, fall back to one ordinary non-streaming request.
    let answer = if answer.trim().is_empty() {
        crate::llm_client::send_chat_completion_with_schema(
            &provider,
            api_key_fallback,
            &model,
            text_fallback,
            Some(system_fallback),
            None,
            None,
            None,
        )
        .await?
        .unwrap_or_default()
    } else {
        answer
    };

    // Signal completion so the UI can stop its streaming indicator.
    let _ = app.emit("meeting-postprocess-done", answer.clone());
    if answer.trim().is_empty() {
        return Err("The model returned no output.".to_string());
    }
    Ok(answer)
}

/// v1.17.0: pre-warm the post-processing model so the FIRST "Generate notes"
/// click doesn't pay the cold model-load cost on top of generation. Best-effort
/// and local-only: for a cloud provider there's nothing to warm. The frontend
/// calls this fire-and-forget when a transcript becomes available.
#[tauri::command]
#[specta::specta]
pub async fn meeting_prewarm_post_process(app: AppHandle) -> Result<(), String> {
    let settings = crate::settings::get_settings(&app);
    let provider = match settings.active_post_process_provider().cloned() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !provider.is_local_provider {
        return Ok(());
    }
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    // Keep it resident for 30 min — comfortably covers reviewing a transcript
    // then clicking Generate, and a re-run or two after that.
    crate::commands::ollama::warm_model(&provider.base_url, &model, 1_800).await;
    Ok(())
}

/// Whether the active post-processing provider is a LOCAL endpoint (localhost),
/// so the UI can warn that a cloud provider would send the transcript off-machine.
#[tauri::command]
#[specta::specta]
pub async fn meeting_provider_is_local(app: AppHandle) -> Result<bool, String> {
    let settings = crate::settings::get_settings(&app);
    let base = match settings.active_post_process_provider() {
        Some(p) => p.base_url.to_lowercase(),
        None => return Ok(false),
    };
    Ok(base.contains("localhost")
        || base.contains("127.0.0.1")
        || base.contains("0.0.0.0")
        || base.contains("[::1]"))
}

/// Keep a streamed WAV that has audio; delete an empty one so it doesn't
/// clutter the recovery list. Returns the path string when kept.
///
/// v1.13.3: also checks the file's actual on-disk size — if `stop()` errored
/// (e.g. an aborted capture after a disk-write failure) `samples_written` is
/// reported as 0, but the WAV may still hold real pre-failure audio. Never
/// delete a file that has data beyond the 44-byte WAV header: that file IS
/// the failsafe.
fn keep_or_discard(path: PathBuf, samples_written: u64) -> Option<String> {
    let has_data_on_disk = std::fs::metadata(&path)
        .map(|m| m.len() > 44)
        .unwrap_or(false);
    if samples_written > 0 || has_data_on_disk {
        Some(path.to_string_lossy().to_string())
    } else {
        let _ = std::fs::remove_file(&path);
        None
    }
}

/// Transcribe a streamed WAV from disk, swallowing errors to an empty string —
/// the recording itself is already safe on disk and can be re-transcribed.
/// v1.13.4: chunked — memory stays bounded regardless of recording length.
/// v1.17.1: currently unreferenced (stop prefers the live transcript and the
/// remaining callers route through transcribe_wav_chunked directly), but kept
/// as the documented full-WAV fallback seam — allow(dead_code) keeps CI's
/// clippy -D warnings green without deleting the seam.
#[allow(dead_code)]
async fn transcribe_path_lossy(tm: &Arc<TranscriptionManager>, path: &str) -> String {
    match transcribe_wav_chunked(tm, path, None).await {
        Ok(text) => text,
        Err(e) => {
            log::error!("Meeting transcription failed for {path}: {e} (file is preserved on disk)");
            String::new()
        }
    }
}

/// v1.17.0: segment a WAV OFFLINE with the same energy gate the live path
/// uses, then transcribe each segment, returning `(start_ms, source, text)`.
/// Used by the recovery / re-transcribe paths so a meeting reconstructed from
/// the on-disk WAVs interleaves the two speakers chronologically, exactly like
/// the live path. Best-effort: a decode/transcribe failure yields no segments
/// for that source (the caller still has the per-stream plain transcript).
async fn segment_wav_offline(
    tm: &Arc<TranscriptionManager>,
    path: &str,
    source: &'static str,
) -> Vec<(u64, &'static str, String)> {
    let tm = tm.clone();
    let path = path.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let segs = match decode_and_segment(&path, source) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Offline segmenting failed for {path}: {e} (WAV preserved)");
                return Vec::new();
            }
        };
        let mut out: Vec<(u64, &'static str, String)> = Vec::with_capacity(segs.len());
        for s in segs {
            if let Ok(text) = tm.transcribe(s.samples) {
                // Same v1.19.0 guards as the live path (offline has energy-only
                // segments, so peak_rms still drives the near-floor drop).
                let text = match clean_segment_text(&text, Some(s.peak_rms)) {
                    Some(t) => t,
                    None => continue,
                };
                if out
                    .last()
                    .map(|(_, _, last)| normalise_phrase(last) == normalise_phrase(&text))
                    .unwrap_or(false)
                {
                    continue; // cross-segment duplicate collapse (guard a)
                }
                out.push((s.start_ms, source, text));
            }
        }
        out
    })
    .await
    .unwrap_or_default()
}

/// Decode a WAV to 16 kHz mono and run the live `Segmenter` over it, returning
/// the cut speech segments (each carrying its capture-relative `start_ms`).
/// Streaming: the resampler and segmenter hold at most one in-flight segment,
/// so memory stays bounded regardless of recording length.
fn decode_and_segment(path: &str, source: &'static str) -> Result<Vec<LiveSegment>, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("Could not open {path}: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let in_rate = spec.sample_rate as usize;

    // Capacity comfortably exceeds the segment count of a multi-hour meeting;
    // we drain only after the producer is dropped, so nothing is lost in
    // practice. A genuine overflow degrades to a dropped segment (logged by the
    // segmenter), the same non-fatal failure class as the live path.
    let (tx, rx) = std::sync::mpsc::sync_channel::<LiveSegment>(8192);
    // Offline path: no AppHandle here, so the segmenter runs energy-only (the
    // VAD is a live-capture refinement; the offline collapse/blocklist guards
    // below still clean up any hallucinated repeats).
    let mut seg = Segmenter::new(source, tx, None);
    let mut resampler = FrameResampler::new(in_rate, 16_000, Duration::from_millis(30));
    let mut interleave: Vec<f32> = Vec::with_capacity(channels);
    let mut mono: Vec<f32> = Vec::with_capacity(8_192);

    // One iterator shape for both PCM encodings, normalised to f32 in [-1, 1].
    let path_owned = path.to_string();
    let samples: Box<dyn Iterator<Item = Result<f32, String>>> = match spec.sample_format {
        hound::SampleFormat::Float => Box::new(
            reader
                .into_samples::<f32>()
                .map(move |s| s.map_err(|e| format!("WAV read error in {path_owned}: {e}"))),
        ),
        hound::SampleFormat::Int => {
            let denom = (1i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            Box::new(reader.into_samples::<i32>().map(move |s| {
                s.map(|v| v as f32 / denom)
                    .map_err(|e| format!("WAV read error in {path_owned}: {e}"))
            }))
        }
    };

    for s in samples {
        interleave.push(s?);
        if interleave.len() == channels {
            mono.push(interleave.iter().sum::<f32>() / channels as f32);
            interleave.clear();
        }
        if mono.len() >= 8_192 {
            resampler.push(&mono, |frame| seg.push(frame));
            mono.clear();
        }
    }
    if !mono.is_empty() {
        resampler.push(&mono, |frame| seg.push(frame));
    }
    resampler.finish(|frame| seg.push(frame));
    seg.finish();
    drop(seg); // drops the sender so the receiver can be fully drained
    Ok(rx.try_iter().collect())
}

/// Transcribe a buffer; propagate errors (used by the recovery command).
async fn transcribe_buffer(
    tm: &Arc<TranscriptionManager>,
    samples: Vec<f32>,
) -> Result<String, String> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    let tm = tm.clone();
    tauri::async_runtime::spawn_blocking(move || tm.transcribe(samples))
        .await
        .map_err(|e| format!("Transcription task failed: {e}"))?
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// v1.13.4: chunked (bounded-memory) WAV transcription
// ---------------------------------------------------------------------------

/// Chunk size for windowed transcription: 5 minutes of 16 kHz mono ≈ 19 MB as
/// f32. Bounds memory regardless of meeting length — a 2-hour recording
/// previously loaded ~460 MB per stream in one go.
const CHUNK_SAMPLES: usize = 16_000 * 300;
/// Search the final 20 s of a chunk for the quietest point to split at, so a
/// chunk boundary doesn't cut through the middle of a word.
const SPLIT_WINDOW: usize = 16_000 * 20;
/// Energy is measured over 100 ms frames.
const SPLIT_FRAME: usize = 1_600;

/// Index (within `buf`) of the centre of the lowest-energy 100 ms frame in the
/// final `SPLIT_WINDOW` — the least-bad place to cut a chunk.
fn quietest_split(buf: &[f32]) -> usize {
    let start = buf.len().saturating_sub(SPLIT_WINDOW);
    let mut best_idx = buf.len();
    let mut best_energy = f32::INFINITY;
    let mut i = start;
    while i + SPLIT_FRAME <= buf.len() {
        let energy: f32 = buf[i..i + SPLIT_FRAME].iter().map(|s| s * s).sum();
        if energy < best_energy {
            best_energy = energy;
            best_idx = i + SPLIT_FRAME / 2;
        }
        i += SPLIT_FRAME;
    }
    // Never return 0: a zero-length head would make the caller loop forever.
    best_idx.max(1)
}

/// Transcribe a WAV from disk in bounded-memory chunks. Decodes any PCM WAV
/// (int or float, any rate / channel count), downmixes to mono, resamples to
/// 16 kHz, and transcribes ~5-minute windows split at the quietest point.
///
/// This replaces the previous full-file `read_wav_samples` load and also fixes
/// a latent import bug: WAVs that weren't already 16 kHz mono were fed to the
/// model at the wrong rate (no resample) and with interleaved channels.
/// v1.19.0: optional progress reporter for chunked transcription. `id` is the
/// path being transcribed so the UI can match progress events to the row.
#[derive(Clone)]
struct TranscribeProgress {
    app: AppHandle,
    id: String,
}

#[derive(serde::Serialize, Clone)]
struct TranscribeProgressEvent {
    id: String,
    window: u32,
    /// Approximate total window count — `None` for compressed imports whose
    /// length isn't cheaply known (the UI shows an indeterminate bar then).
    total: Option<u32>,
}

impl TranscribeProgress {
    fn emit(&self, window: u32, total: Option<u32>) {
        use tauri::Emitter;
        let _ = self.app.emit(
            "meeting-transcribe-progress",
            TranscribeProgressEvent {
                id: self.id.clone(),
                window,
                total,
            },
        );
    }
}

async fn transcribe_wav_chunked(
    tm: &Arc<TranscriptionManager>,
    path: &str,
    progress: Option<TranscribeProgress>,
) -> Result<String, String> {
    let reader =
        hound::WavReader::open(path).map_err(|e| format!("Could not open {path}: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let in_rate = spec.sample_rate as usize;

    // v1.19.0: the WAV header gives us the per-channel sample count cheaply, so
    // we can estimate the window total for a determinate progress bar. (The
    // chunker splits at the quietest point, so this is approximate.)
    let per_channel = reader.duration() as u64;
    let samples_16k = if in_rate > 0 {
        per_channel * 16_000 / in_rate as u64
    } else {
        0
    };
    let total_windows = Some(
        ((samples_16k + CHUNK_SAMPLES as u64 - 1) / CHUNK_SAMPLES as u64).max(1) as usize,
    );

    // One iterator shape for both PCM encodings, normalised to f32 in [-1, 1].
    let path_owned = path.to_string();
    let samples: Box<dyn Iterator<Item = Result<f32, String>> + Send> = match spec.sample_format {
        hound::SampleFormat::Float => Box::new(
            reader
                .into_samples::<f32>()
                .map(move |s| s.map_err(|e| format!("WAV read error in {path_owned}: {e}"))),
        ),
        hound::SampleFormat::Int => {
            let denom = (1i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            Box::new(reader.into_samples::<i32>().map(move |s| {
                s.map(|v| v as f32 / denom)
                    .map_err(|e| format!("WAV read error in {path_owned}: {e}"))
            }))
        }
    };
    transcribe_stream_chunked(tm, in_rate, channels, samples, progress, total_windows).await
}

/// v1.16.1: decode a compressed audio file (m4a/aac/mp3/flac/ogg) via rodio
/// into the same shape the WAV path produces — rate, channel count, and a
/// streaming f32 sample iterator. Decoding is incremental, so memory stays
/// bounded by the chunker's window like the WAV path.
fn open_rodio_stream(
    path: &str,
) -> Result<
    (
        usize,
        usize,
        Box<dyn Iterator<Item = Result<f32, String>> + Send>,
    ),
    String,
> {
    use rodio::Source;
    let file = std::fs::File::open(path).map_err(|e| format!("Could not open {path}: {e}"))?;
    // v1.16.2: Decoder::try_from(File) — NOT Decoder::new(BufReader) — so the
    // decoder gets a seekable stream with a known byte length. M4A/MP4 keeps
    // its index (moov atom) at the END of the file, so symphonia's container
    // probe needs seek+len; without them it reports "Unrecognized format".
    // try_from(File) is the fork's own documented canonical constructor.
    let decoder = rodio::Decoder::try_from(file)
        .map_err(|e| format!("Could not decode {path}: {e}"))?;
    let rate = decoder.sample_rate() as usize;
    let channels = (decoder.channels() as usize).max(1);
    // The rodio fork is the 0.21-era architecture: `Sample` is universally
    // `f32` (verified in the fork's common.rs — amplitude -1.0..1.0), so the
    // decoder iterates f32 natively and no conversion adapter exists or is
    // needed. (`convert_samples` died with the multi-sample-type design.)
    let iter = decoder.map(|s| Ok::<f32, String>(s));
    Ok((rate, channels, Box::new(iter)))
}

/// The decoder-agnostic core: downmix → resample to 16 kHz → transcribe
/// ~5-minute windows split at the quietest point. Shared by the WAV (hound)
/// and compressed-audio (rodio) paths.
async fn transcribe_stream_chunked(
    tm: &Arc<TranscriptionManager>,
    in_rate: usize,
    channels: usize,
    mut samples: Box<dyn Iterator<Item = Result<f32, String>> + Send>,
    progress: Option<TranscribeProgress>,
    total_windows: Option<usize>,
) -> Result<String, String> {
    let mut resampler = FrameResampler::new(in_rate, 16_000, Duration::from_millis(30));
    let mut buf: Vec<f32> = Vec::with_capacity(CHUNK_SAMPLES + SPLIT_WINDOW);
    let mut mono_block: Vec<f32> = Vec::with_capacity(8_192);
    let mut interleave: Vec<f32> = Vec::with_capacity(channels);
    let mut out = String::new();
    let mut eof = false;
    // v1.19.0: per-window progress for the import / re-transcribe UI.
    let total_u32 = total_windows.map(|t| t as u32);
    let mut window_idx: u32 = 0;

    loop {
        // Fill the window (decode → downmix → resample), bounded by CHUNK_SAMPLES.
        while buf.len() < CHUNK_SAMPLES && !eof {
            mono_block.clear();
            for _ in 0..32_768 {
                match samples.next() {
                    Some(Ok(v)) => {
                        interleave.push(v);
                        if interleave.len() == channels {
                            mono_block
                                .push(interleave.iter().sum::<f32>() / channels as f32);
                            interleave.clear();
                        }
                    }
                    // Errors arrive pre-formatted from the decoder adapters.
                    Some(Err(e)) => return Err(e),
                    None => {
                        eof = true;
                        break;
                    }
                }
            }
            resampler.push(&mono_block, |frame| buf.extend_from_slice(frame));
        }
        if eof {
            // Safe to call more than once: finish() is a no-op when drained.
            resampler.finish(|frame| buf.extend_from_slice(frame));
        }
        if buf.is_empty() {
            break;
        }
        let take = if eof { buf.len() } else { quietest_split(&buf) };
        let head: Vec<f32> = buf[..take].to_vec();
        buf.drain(..take);
        let text = transcribe_buffer(tm, head).await?;
        // v1.19.0: report progress as each window finishes. `total` may exceed
        // the eventual count (quietest-split shifts boundaries); clamp window to
        // total so the bar never overshoots.
        if let Some(p) = &progress {
            window_idx += 1;
            let shown = total_u32.map(|t| window_idx.min(t)).unwrap_or(window_idx);
            p.emit(shown, total_u32);
        }
        // v1.19.0 guard (d): collapse hallucinated repeats within the window
        // before appending (imports have no VAD, so this is their main guard).
        let text = collapse_repeats(&text);
        let text = text.trim();
        if !text.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(text);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// v1.13.4: meetings metadata store (off localStorage)
// ---------------------------------------------------------------------------

/// Load the meetings metadata store (opaque JSON owned by the frontend).
/// Returns an empty string when no store exists yet.
#[tauri::command]
#[specta::specta]
pub async fn meetings_store_load(app: AppHandle) -> Result<String, String> {
    let path = meetings_dir(&app)?.join("meetings.json");
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("Failed to read meetings store: {e}")),
    }
}

/// Save the meetings metadata store ATOMICALLY (temp file + rename, which
/// replaces the destination on Windows) so a crash or sync interruption can
/// never truncate it. Replaces the localStorage store, whose ~5 MB quota
/// silently dropped writes once a few long transcripts accumulated.
#[tauri::command]
#[specta::specta]
pub async fn meetings_store_save(app: AppHandle, json: String) -> Result<(), String> {
    let dir = meetings_dir(&app)?;
    let tmp = dir.join("meetings.json.tmp");
    let path = dir.join("meetings.json");
    std::fs::write(&tmp, json.as_bytes())
        .map_err(|e| format!("Failed to write meetings store: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("Failed to commit meetings store: {e}"))
}

// ---------------------------------------------------------------------------
// v1.13.5: capture diagnostics — device names + a no-risk test capture
// ---------------------------------------------------------------------------

/// The devices a meeting capture would use right now, for the UI meters.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct CaptureDevices {
    pub mic: String,
    pub system: String,
}

/// Result of a device test capture: how many 16 kHz samples each stream
/// actually produced. Zero from the system stream with audio playing means
/// loopback is capturing the wrong device (it records the DEFAULT output).
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct MeetingTestResult {
    pub mic_device: String,
    pub system_device: String,
    pub mic_samples: u64,
    pub system_samples: u64,
}

/// Names of the devices meeting capture would use (mic + default output).
#[tauri::command]
#[specta::specta]
pub async fn meeting_capture_devices(app: AppHandle) -> Result<CaptureDevices, String> {
    let mic = match selected_input_device(&app) {
        Some(d) => d.name().unwrap_or_else(|_| "Selected microphone".to_string()),
        None => cpal::default_host()
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_else(|| "Default microphone".to_string()),
    };
    let system = cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "Default output".to_string());
    Ok(CaptureDevices { mic, system })
}

/// Run the EXACT meeting capture path against both devices for `secs` seconds
/// (writing to throwaway WAVs that are deleted afterwards), emitting the same
/// `meeting-level` events the real recording emits. Lets the user verify mic
/// AND system-audio capture work before trusting a real meeting to them.
#[tauri::command]
#[specta::specta]
pub async fn meeting_test_capture(
    app: AppHandle,
    meeting: State<'_, Arc<MeetingRecorder>>,
    recording_manager: State<'_, Arc<AudioRecordingManager>>,
    secs: u32,
) -> Result<MeetingTestResult, String> {
    if recording_manager.is_recording() {
        return Err("A dictation recording is in progress. Stop it first.".to_string());
    }
    {
        let guard = meeting.active.lock().map_err(|_| "lock poisoned")?;
        if guard.is_some() {
            return Err("A meeting is already being recorded.".to_string());
        }
    }
    // Claim the meeting flag for the test so dictation shortcuts stay guarded
    // and the two paths can't fight over the microphone.
    if MEETING_ACTIVE.swap(true, Ordering::Relaxed) {
        return Err("A meeting is already being recorded.".to_string());
    }
    let result = run_test_capture(&app, secs.clamp(2, 30)).await;
    MEETING_ACTIVE.store(false, Ordering::Relaxed);
    result
}

async fn run_test_capture(app: &AppHandle, secs: u32) -> Result<MeetingTestResult, String> {
    let dir = meetings_dir(app)?;
    let mic_path = dir.join("test-you.wav");
    let sys_path = dir.join("test-others.wav");

    // Microphone — same device resolution as a real meeting.
    let (mic_device, mic_cap) = {
        let device = match selected_input_device(app) {
            Some(d) => d,
            None => cpal::default_host()
                .default_input_device()
                .ok_or_else(|| "No input device found".to_string())?,
        };
        let name = device.name().unwrap_or_else(|_| "Microphone".to_string());
        let config = device
            .default_input_config()
            .map_err(|e| format!("Mic config: {e}"))?;
        let cap = StreamCapture::start(
            device,
            config,
            mic_path.clone(),
            Some(app.clone()),
            "you",
            None, // no live transcription during a device test
            Arc::new(AtomicBool::new(false)), // device test is never paused
        )?;
        (name, cap)
    };

    // System loopback — same backend selection as a real meeting (v1.13.6:
    // WASAPI first, cpal fallback); the verdict names the backend that ran.
    let sys_name = cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "System audio".to_string());
    let (sys_cap, backend) =
        start_system_capture(app, sys_path.clone(), None, Arc::new(AtomicBool::new(false)));
    let system_device = if sys_cap.is_some() {
        format!("{sys_name} [{backend}]")
    } else {
        format!("{sys_name} — unavailable (both WASAPI and cpal failed; see log)")
    };

    // Let both streams run for the test window.
    let wait = u64::from(secs);
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_secs(wait))
    })
    .await
    .map_err(|e| format!("Test wait failed: {e}"))?;

    let (mic_samples, system_samples) = tauri::async_runtime::spawn_blocking(move || {
        let m = mic_cap.stop().unwrap_or(0);
        let s = match sys_cap {
            Some(c) => c.stop().unwrap_or(0),
            None => 0,
        };
        (m, s)
    })
    .await
    .map_err(|e| format!("Test stop failed: {e}"))?;

    // Throwaway files — the test is about signal, not audio worth keeping.
    let _ = std::fs::remove_file(&mic_path);
    let _ = std::fs::remove_file(&sys_path);

    Ok(MeetingTestResult {
        mic_device,
        system_device,
        mic_samples,
        system_samples,
    })
}
