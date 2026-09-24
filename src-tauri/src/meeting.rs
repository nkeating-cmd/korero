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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};

use crate::audio_toolkit::audio::FrameResampler;
use crate::audio_toolkit::list_input_devices;
use crate::meeting_capture::{
    live_stats, reset_live_stats, LiveSegment, LiveSourceStats, SegmentSender, Segmenter,
    StreamCapture, SEG_QUIET_RMS,
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
    /// Kōrero (meetings reliability, 2026-09-25): plain-English problems found
    /// while finishing the meeting — a microphone that never heard speech, no
    /// system audio, a speaker rebuilt from the recording because live
    /// transcription missed parts. Previously the only signal was an empty
    /// transcript, with no reason attached.
    pub warnings: Vec<String>,
}

/// v1.17.0: one chronological transcript segment. `source` is "you" or
/// "others"; the UI maps that to the meeting's editable speaker label.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct TranscriptSeg {
    pub source: String,
    pub text: String,
    /// Kōrero (v1.26.0): milliseconds from the START OF THE FILE this segment
    /// was decoded from. Capture-relative, never rebased to a trim window.
    ///
    /// This value has existed internally since v1.17.0 — `LiveSegment.start_ms`
    /// drives the chronological sort in both `meeting_stop_capture` and
    /// `meeting_transcribe_merge` — but every construction site destructured it
    /// away with `|(_, s, t)|` and the frontend never saw it. That made the
    /// ordered transcript untimed, which is why there was no way to say "the
    /// meeting actually starts here".
    ///
    /// Absolute, not window-relative: a segment 12 s into the file reports
    /// 12000 whether or not an in-point is set at 8 s. Anything else and a
    /// trim filter comparing against the marker double-counts the offset.
    ///
    /// 0 for meetings recorded before v1.26.0 (the frontend defaults it).
    pub start_ms: u64,
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
    /// Kōrero (meetings reliability, 2026-09-25): live segments whose
    /// transcription FAILED, per source. These used to be a log line and
    /// nothing else, so an engine error part-way through a meeting left a hole
    /// in the transcript that Stop never noticed. Stop now rebuilds any source
    /// with a non-zero count from its WAV.
    failed_you: AtomicU64,
    failed_others: AtomicU64,
}

impl LiveTranscript {
    fn note_failure(&self, source: &str) {
        let c = if source == "you" { &self.failed_you } else { &self.failed_others };
        c.fetch_add(1, Ordering::Relaxed);
    }

    fn failures(&self, source: &str) -> u64 {
        let c = if source == "you" { &self.failed_you } else { &self.failed_others };
        c.load(Ordering::Relaxed)
    }

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
/// Kōrero (v1.26.0): the ONE place a `(start_ms, source, text)` log becomes the
/// `TranscriptSeg` list the frontend sees.
///
/// Extracted so the offset-preservation contract has a single home and a test.
/// Before this, two call sites built the vector inline with subtly different
/// iterators (`.iter()` + `clone()` vs `.into_iter()`), and BOTH silently
/// dropped the timestamp. A contract enforced in two places is a contract that
/// gets half-changed.
fn to_transcript_segs(log: &[(u64, &'static str, String)]) -> Vec<TranscriptSeg> {
    log.iter()
        .map(|(ms, s, t)| TranscriptSeg {
            source: s.to_string(),
            text: t.clone(),
            start_ms: *ms,
        })
        .collect()
}

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
///
/// v1.30.0: made `pub(crate)` so the DICTATION path can use it too.
///
/// This guard defended meetings and nothing else — `collapse_ngram_runs`
/// occurred 4× in this file and **0×** in `managers/transcription.rs`. The FENZ
/// failure mode (a model emitting a repeated n-gram *instead of* speech) was
/// therefore unmitigated on the path that types into the user's document, which
/// is the app's core function.
///
/// It is safe to share: every threshold below is deliberately conservative, and
/// the function returns its input **verbatim** when nothing collapses, so a
/// clean transcript is never reflowed.
///
/// TODO(v1.31.0): this belongs in `audio_toolkit::text` alongside the other text
/// guards. It lives here because `meeting.rs` is a whole-file overlay copy and
/// `text.rs` is not — moving it now would cost patch re-anchors for no
/// behavioural gain. Move it during the transcribe.cpp re-seat.
pub(crate) fn collapse_repeats(text: &str) -> String {
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
        // v1.24.0: a single "sentence" is exactly what a comma-run decoder
        // loop looks like — it must still pass through the n-gram stage
        // (this early return previously bypassed it; caught by unit test).
        return collapse_ngram_runs(text);
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
    let sentence_collapsed = if !dropped_any {
        text.trim().to_string()
    } else {
        out.join(" ")
    };
    // v1.24.0 (FENZ import bug): the sentence-level pass above misses the OTHER
    // decoder-loop shape — a sub-sentence unit repeated inside one giant
    // comma-run ("ProjectIQ, and the ProjectIQ, and the …", "project, project,
    // project, …" ×100s), which contains no . ! ? boundary at all. Collapse
    // word-level n-gram runs as a second stage.
    collapse_ngram_runs(&sentence_collapsed)
}

/// Collapse pathological word-level n-gram runs — the decoder-loop signature
/// that has no sentence boundaries (observed on a real M4A import: a 3-gram
/// then a 1-gram repeated hundreds of times, comma-separated, in one
/// "sentence"). Deliberately conservative thresholds so natural speech
/// survives: a single word must repeat ≥5 times consecutively ("no, no, no,
/// no" stays), a phrase of 2-8 words ≥4 times ("I know, I know, I know"
/// stays). Loops repeat tens-to-hundreds of times, so the margin is wide.
/// Comparison is case- and punctuation-insensitive per token; the FIRST
/// occurrence's original tokens are kept. Returns the input verbatim when
/// nothing collapses (never reflows clean text).
fn collapse_ngram_runs(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 8 {
        return text.trim().to_string();
    }
    let norm_token = |t: &str| -> String {
        t.trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(c, '.' | '!' | '?' | ',' | ';' | ':' | '-' | '–' | '—' | '"' | '\'' | '…' | '(' | ')')
        })
        .to_lowercase()
    };
    let mut work: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
    let mut collapsed_any = false;
    // Shortest unit FIRST: a run always collapses at its fundamental period
    // (a "project," ×120 run is period-1; "ProjectIQ, and the" ×40 is
    // period-3). Longest-first would first collapse at a multiple of the
    // period (e.g. 8 tokens of a period-1 run) and strand a sub-threshold
    // residual pair; shortest-first leaves exactly one instance.
    for n in 1..=8usize {
        if work.len() < n * 2 {
            continue;
        }
        let norms: Vec<String> = work.iter().map(|t| norm_token(t)).collect();
        let threshold: usize = if n == 1 { 5 } else { 4 };
        let mut out: Vec<String> = Vec::with_capacity(work.len());
        let mut i = 0usize;
        while i < work.len() {
            if i + n <= work.len() && !norms[i..i + n].iter().all(|s| s.is_empty()) {
                let mut reps = 1usize;
                while i + (reps + 1) * n <= work.len()
                    && norms[i + reps * n..i + (reps + 1) * n] == norms[i..i + n]
                {
                    reps += 1;
                }
                if reps >= threshold {
                    out.extend_from_slice(&work[i..i + n]); // keep one instance
                    i += reps * n;
                    collapsed_any = true;
                    continue;
                }
            }
            out.push(work[i].clone());
            i += 1;
        }
        work = out;
    }
    if !collapsed_any {
        return text.trim().to_string();
    }
    log::debug!(
        "collapse_ngram_runs: decoder-loop collapsed ({} -> {} tokens)",
        tokens.len(),
        work.len()
    );
    work.join(" ")
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

/// The app-private default meetings folder (<app-data>\meetings). The
/// meetings.json metadata store ALWAYS lives here regardless of any custom
/// recording folder — transcripts and notes are app data, not user artefacts,
/// and must not follow the recordings onto removable/synced drives.
fn default_meetings_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = crate::portable::app_data_dir(app)
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))?
        .join("meetings");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create meetings dir: {e}"))?;
    Ok(dir)
}

/// Kōrero (v1.24.0, paths): where meeting MEDIA is written (WAV captures,
/// audio-brief MP3s). Honours `meeting_recording_dir` when set; fails OPEN to
/// the default folder (logged) so a vanished custom drive never blocks a
/// recording from starting.
fn meetings_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let settings = crate::settings::get_settings(app);
    if let Some(custom) = settings
        .meeting_recording_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let p = PathBuf::from(custom);
        match std::fs::create_dir_all(&p) {
            Ok(()) if p.is_dir() => return Ok(p),
            Ok(()) => log::warn!(
                "Custom recording path is not a folder; falling back to the default: {custom}"
            ),
            Err(e) => log::warn!(
                "Custom recording folder unavailable ({e}); falling back to the default: {custom}"
            ),
        }
    }
    default_meetings_dir(app)
}

/// Every folder that may hold meeting WAVs: the active recording dir plus the
/// default (when a custom dir is set) — so pre-change recordings stay visible,
/// deletable, and retention-swept after the folder moves (plan decision D4).
fn recording_scan_roots(app: &AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(active) = meetings_dir(app) {
        roots.push(active);
    }
    if let Ok(default) = default_meetings_dir(app) {
        if !roots.contains(&default) {
            roots.push(default);
        }
    }
    roots
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
    // Kōrero (meetings reliability, 2026-09-25): fresh health counters for this
    // meeting, and a flag the consumer reads to know whether a system-audio
    // capture exists at all (it is only known after the consumer starts).
    reset_live_stats();
    let system_on = Arc::new(AtomicBool::new(false));
    let consumer = {
        let live = live.clone();
        let tm = transcription_manager.inner().clone();
        let app_ev = app.clone();
        let system_on = system_on.clone();
        std::thread::spawn(move || {
            #[derive(serde::Serialize, Clone)]
            struct LiveEvent {
                source: &'static str,
                text: String,
                /// Kōrero (v1.26.0): the live path dropped the offset too, so
                /// segments arriving DURING a meeting were untimed while the
                /// same segments after a re-transcribe were timed. That
                /// inconsistency would have made a trim window behave
                /// differently before and after a restart. Field name matches
                /// TranscriptSeg so the frontend can treat them alike.
                start_ms: u64,
            }
            // Kōrero (meetings reliability, 2026-09-25): a TIMED receive, so
            // the capture-health check below still runs while a stream is
            // producing no segments at all — which is exactly the case it is
            // there to catch.
            let mut warned: Vec<CaptureIssue> = Vec::new();
            loop {
                let seg = match seg_rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(seg) => Some(seg),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let you_h = SourceHealth::of(live_stats("you"));
                let others_h = SourceHealth::of(live_stats("others"));
                for issue in capture_issues(
                    you_h,
                    others_h,
                    system_on.load(Ordering::Relaxed),
                    MIC_SILENT_WARN_SECS,
                    SYSTEM_SILENT_WARN_SECS,
                ) {
                    if warned.contains(&issue) {
                        continue;
                    }
                    warned.push(issue);
                    let msg = issue_message(issue, you_h, others_h, true);
                    log::warn!("Meeting capture health: {msg}");
                    use tauri::Emitter;
                    let _ = app_ev.emit("meeting-capture-warning", msg);
                    // The warning is an in-app toast, and during a call Kōrero
                    // is usually behind the meeting app. Flash its taskbar
                    // button so the user knows to look.
                    if let Some(w) = app_ev.get_webview_window("main") {
                        let _ = w.request_user_attention(Some(
                            tauri::UserAttentionType::Informational,
                        ));
                    }
                }
                let Some(seg) = seg else { continue };
                // Kōrero (meetings reliability, 2026-09-25): reload-and-retry.
                // An engine panic unloads the model, and nothing reloaded it,
                // so every later segment of the meeting failed with "Model is
                // not loaded" — silently, for the rest of the call.
                match transcribe_retrying(&tm, seg.samples) {
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
                                start_ms: seg.start_ms,
                            },
                        );
                    }
                    Err(e) => {
                        live.note_failure(seg.source);
                        log::warn!(
                            "Live segment transcription failed for '{}' (Stop will rebuild \
                             that speaker from the WAV): {e}",
                            seg.source
                        );
                    }
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
    system_on.store(system_captured, Ordering::Relaxed);
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
    // Whether a system-audio capture ran at all (its WAV may still be
    // discarded below as empty — which is itself one of the cases to report).
    let sys_was_captured = sys_pathbuf.is_some();
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
    // Kōrero (meetings reliability, 2026-09-25): both capture workers have
    // joined, so these counters are final for this meeting.
    let you_h = SourceHealth::of(live_stats("you"));
    let others_h = SourceHealth::of(live_stats("others"));
    let mut warnings: Vec<String> = Vec::new();
    let mut seg_log = live.snapshot();

    // Kōrero (meetings reliability, 2026-09-25): rebuild a speaker from its WAV
    // when the live transcript for it is EMPTY (as before) or INCOMPLETE (new).
    // Incomplete means at least one live segment was dropped because
    // transcription fell behind, or failed to transcribe. Before this, only an
    // entirely empty side was rebuilt, so a partial loss stayed in the saved
    // transcript with nothing to say it had happened.
    for (source, path) in [("you", mic_path.as_deref()), ("others", system_path.as_deref())] {
        let Some(p) = path else { continue };
        let has_live = seg_log.iter().any(|(_, s, _)| *s == source);
        let lost = live_stats(source).dropped.load(Ordering::Relaxed) + live.failures(source);
        if !needs_offline_rebuild(has_live, lost) {
            continue;
        }
        let label = side_label(source);
        // The idle-unload watcher stops protecting the model the moment
        // MEETING_ACTIVE clears (top of this function), and an engine crash
        // during the meeting may have unloaded it already. Loading is claimed
        // synchronously and the first transcribe() waits for it.
        tm.initiate_model_load();
        match segment_wav_offline(&tm, p, source).await {
            Ok(r) => {
                if r.failed > 0 {
                    warnings.push(format!(
                        "{} of {} parts of {label} could not be transcribed ({}). The audio is \
                         saved — use Re-transcribe to try again.",
                        r.failed,
                        r.attempted,
                        r.last_error.as_deref().unwrap_or("unknown error"),
                    ));
                }
                if adopt_rebuild(has_live, r.failed, r.segs.is_empty()) {
                    if has_live {
                        log::warn!(
                            "Meeting stop: live transcript for '{source}' missed {lost} \
                             segment(s); rebuilt it from {p}."
                        );
                        warnings.push(format!(
                            "Live transcription missed {lost} part(s) of {label}, so that side \
                             was rebuilt from the recording."
                        ));
                    }
                    seg_log.retain(|(_, s, _)| *s != source);
                    seg_log.extend(r.segs);
                }
            }
            Err(e) => {
                log::warn!("Meeting stop: could not rebuild '{source}' from {p}: {e}");
                warnings.push(format!(
                    "Couldn't transcribe {label} from the recording ({e}). The audio is saved — \
                     use Re-transcribe to try again."
                ));
            }
        }
    }
    seg_log.sort_by_key(|(ms, _, _)| *ms);

    // Kōrero (meetings reliability, 2026-09-25): say WHY a side is empty. The
    // thresholds are lower than the in-meeting ones: at Stop the whole meeting
    // is known, so even a short recording can be judged.
    for issue in capture_issues(
        you_h,
        others_h,
        sys_was_captured,
        MIC_SILENT_STOP_SECS,
        SYSTEM_SILENT_STOP_SECS,
    ) {
        warnings.push(issue_message(issue, you_h, others_h, false));
    }

    let segments: Vec<TranscriptSeg> = to_transcript_segs(&seg_log);
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
        warnings,
    })
}

/// v1.13.6: meeting recordings are kept this many days. Transcripts live in
/// the meetings store and survive; only the bulky WAV audio is aged out.
const RECORDING_RETENTION_DAYS: u64 = 30;

/// Delete meeting WAVs older than the retention window, plus any leftover
/// device-test files (a crash mid-test can orphan them). Called at startup
/// and after each meeting stop. Best-effort: errors are logged, never fatal.
pub fn cleanup_old_recordings(app: &AppHandle) {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(
            RECORDING_RETENTION_DAYS * 24 * 60 * 60,
        ))
        .unwrap_or(std::time::UNIX_EPOCH);
    let mut removed = 0u32;
    // v1.24.0 (paths, risk R1 — CRITICAL): now that the recording folder can be
    // user-chosen, the sweep must only ever touch files KŌRERO created. It
    // previously deleted ANY old .wav in the folder — safe while app-private,
    // silent data loss if pointed at Documents or a shared drive. Eligible:
    //   - meeting-*.wav   (our capture naming) — aged out after retention
    //   - test-*.wav      (device-test leftovers) — removed at any age
    //   - audio-brief-*.mp3/.txt (v1.22.0 render artifacts) — aged out
    // Both roots are swept so legacy recordings in the default folder still age.
    let default_root = default_meetings_dir(app).ok();
    for dir in recording_scan_roots(app) {
        // Peer-review M3: any-age deletion of test-*.wav is safe only in the
        // app-private default folder. In a USER-CHOSEN folder a file that
        // happens to be named test-*.wav must get the normal 30-day age gate,
        // never a zero-day delete (Kōrero's own device-test files written
        // there still age out).
        let is_default_root = default_root.as_deref() == Some(dir.as_path());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let is_meeting_wav = name.starts_with("meeting-") && ext == "wav";
            let is_test_wav = name.starts_with("test-") && ext == "wav";
            let is_brief = name.starts_with("audio-brief-") && (ext == "mp3" || ext == "txt");
            if !is_meeting_wav && !is_test_wav && !is_brief {
                continue;
            }
            let too_old = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| t < cutoff)
                .unwrap_or(false);
            if (too_old || (is_test_wav && is_default_root))
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
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
    let canon = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("Not found: {e}"))?;
    // v1.24.0 (paths): a recording may live in the active custom folder OR the
    // default folder (dual roots, decision D4). Same guard per root: canonical
    // prefix + .wav only. Roots that fail to canonicalise are skipped, not fatal.
    let in_scope = recording_scan_roots(&app)
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|root| canon.starts_with(&root));
    if !in_scope || canon.extension().and_then(|e| e.to_str()) != Some("wav") {
        return Err("Refusing to delete a file outside the meetings folders.".to_string());
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
        // v1.31.0: + caf/aiff/aif. These are CONTAINER extensions; which codecs
        // can actually be decoded out of them is decided by the symphonia
        // feature list in Cargo.toml, not here.
        "m4a" | "aac" | "mp4" | "mp3" | "flac" | "ogg" | "caf" | "aiff" | "aif" => {
            let (rate, channels, samples) = open_rodio_stream(&path)?;
            // Compressed length isn't cheaply known → indeterminate bar.
            transcribe_stream_chunked(&tm, rate, channels, samples, progress, None).await
        }
        other => Err(format!(
            "Unsupported audio format '.{other}' — use WAV, M4A, MP3, FLAC, OGG, CAF or AIFF."
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
    // Kōrero (meetings reliability, 2026-09-25): errors are now REPORTED. The
    // offline path used to swallow every per-segment failure, so a model that
    // was not loaded produced an empty transcript and the UI said "Still no
    // speech found" — which then replaced a good transcript with nothing.
    // Any failed part now fails the whole re-transcribe, and the frontend keeps
    // the existing transcript untouched.
    for (source, path) in [("you", mic_path.as_deref()), ("others", system_path.as_deref())] {
        let Some(p) = path else { continue };
        let r = segment_wav_offline(&tm, p, source).await?;
        if r.failed > 0 {
            return Err(format!(
                "{} of {} parts of {} could not be transcribed ({}). Your existing transcript \
                 was kept.",
                r.failed,
                r.attempted,
                side_label(source),
                r.last_error.as_deref().unwrap_or("unknown error"),
            ));
        }
        seg_log.extend(r.segs);
    }
    seg_log.sort_by_key(|(ms, _, _)| *ms);
    // Kōrero (v1.26.0): offsets survive here too. On a re-transcribe of BOTH
    // streams they are file-relative per stream and the two files share t=0,
    // so they stay directly comparable -- the property a trim window needs.
    Ok(to_transcript_segs(&seg_log))
}

/// List meeting WAVs saved on disk, newest first (for recovery).
#[tauri::command]
#[specta::specta]
pub async fn meeting_list_recordings(app: AppHandle) -> Result<Vec<RecordingFile>, String> {
    let mut out = Vec::new();
    // v1.24.0 (paths): scan the active custom folder AND the default (D4).
    // In a CUSTOM folder only Kōrero-named meeting-*.wav files are listed —
    // the user's own WAVs there must never appear in a recovery list whose
    // rows carry a Delete button. The default app-private folder keeps the
    // legacy any-wav behaviour (minus transient device tests).
    let default_dir = default_meetings_dir(&app).ok();
    for dir in recording_scan_roots(&app) {
        let is_default = default_dir.as_deref() == Some(dir.as_path());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue; // unplugged/missing custom root degrades gracefully (R3)
        };
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
            if !is_default && !file_name.starts_with("meeting-") {
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
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

/// Kōrero (v1.22.0): resolve the local TTS (audio-brief) engine directory at
/// run time. Order — first that actually contains the engine wins:
/// KORERO_TTS_DIR env, per-user app-data (%APPDATA%\com.kyt.korero\tts),
/// next-to-exe (<exe>\tts or <exe>\resources\tts), then the legacy dev path.
/// Returns the engine dir, or an error listing where it looked. Shared by the
/// audio-brief renderer and the Models-page status probe so they never disagree.
fn resolve_tts_engine() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("KORERO_TTS_DIR") {
        let p = p.trim();
        if !p.is_empty() {
            candidates.push(PathBuf::from(p));
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("com.kyt.korero").join("tts"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("tts"));
            candidates.push(exe_dir.join("resources").join("tts"));
        }
    }
    // Kōrero (2026-07-25, PRIV-1): the developer's own machine path used to be
    // the last-resort candidate here. It is personal data in a public MIT fork,
    // and worse, it is an executable-lookup path: on any machine where that
    // directory happened to exist with a .venv, this would run it. The env var,
    // %APPDATA% and next-to-exe candidates above cover every real case.

    candidates
        .iter()
        .find(|dir| {
            dir.join(r".venv\Scripts\python.exe").exists() && dir.join("audio_brief.py").exists()
        })
        .cloned()
        .ok_or_else(|| {
            let looked = candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "On-device voice engine not found. Set KORERO_TTS_DIR, or install it under %APPDATA%\\com.kyt.korero\\tts. (Looked in: {looked})"
            )
        })
}

/// Kōrero (v1.22.0): report whether the on-device TTS (audio-brief) engine is
/// installed — for the Models page. Ok(path) = found at that directory;
/// Err(message) = not found, with the locations it looked in. Read-only probe.
#[tauri::command]
#[specta::specta]
pub fn tts_engine_status() -> Result<String, String> {
    resolve_tts_engine().map(|p| p.to_string_lossy().to_string())
}

/// Kōrero (v1.22.0): render a spoken "audio brief" MP3 from text (meeting notes
/// or key insights) using a LOCAL TTS batch engine. The engine directory is
/// resolved at run time (KORERO_TTS_DIR env var, then a per-user app-data /
/// bundled-next-to-exe location, then the legacy dev path) so it is no longer
/// pinned to one machine. Fully on-device — nothing leaves the machine. GPU-bound
/// and deliberately SLOW (~1 minute per ~60 spoken words), so this runs on a
/// blocking worker with NO kill-timeout (killing it orphans the GPU worker); the
/// UI owns the long "rendering" state. The MP3 is written into the meetings
/// folder (already asset-scoped, so the frontend can play it via
/// `convertFileSrc`) and its path is returned.
#[tauri::command]
#[specta::specta]
pub async fn meeting_generate_audio_brief(
    app: AppHandle,
    text: String,
    speaker: Option<String>,
    style: Option<String>,
    tempo: Option<f64>,
) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Nothing to narrate — generate notes first.".to_string());
    }
    // Audio is linear and can't be skimmed; keep briefs short. Capping the input
    // also stops a huge transcript from queueing a 30-minute render.
    const MAX_CHARS: usize = 6_000;
    let text = if text.chars().count() > MAX_CHARS {
        text.chars().take(MAX_CHARS).collect::<String>()
    } else {
        text
    };

    let dir = meetings_dir(&app)?;

    tauri::async_runtime::spawn_blocking(move || {
        // Resolve the local TTS engine directory at run time (see
        // resolve_tts_engine) — shared with the Models-page status probe so the
        // two never disagree about whether the engine is installed.
        let engine = resolve_tts_engine()?;
        let py = engine.join(r".venv\Scripts\python.exe");
        let script = engine.join("audio_brief.py");

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let txt = dir.join(format!("audio-brief-{stamp}.txt"));
        let mp3 = dir.join(format!("audio-brief-{stamp}.mp3"));
        std::fs::write(&txt, text.as_bytes())
            .map_err(|e| format!("Failed to write narration script: {e}"))?;

        let mut cmd = std::process::Command::new(&py);
        cmd.arg(&script).arg("--script").arg(&txt).arg("--out").arg(&mp3);
        // Optional preset speaker (--mode speak --speaker <name>); when absent
        // the engine's default designed voice is used.
        if let Some(spk) = speaker
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            cmd.arg("--mode").arg("speak").arg("--speaker").arg(spk);
        }
        // Optional delivery style/emotion (the engine's --instruct). Applies in
        // both speak (preset speaker) and design (default voice) modes; the
        // engine ignores an empty value, so absent style is a no-op.
        if let Some(st) = style
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            cmd.arg("--style").arg(st);
        }
        // Tempo: caller value if positive, else the default 1.12.
        let tempo_val = tempo.filter(|t| *t > 0.0).unwrap_or(1.12);
        cmd.arg("--tempo").arg(format!("{tempo_val}"));
        cmd.current_dir(&engine);
        // Windows: suppress the console window that flashes when launching
        // python.exe (CREATE_NO_WINDOW). Without this a black cmd window pops
        // up for the whole multi-minute render.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd
            .output()
            .map_err(|e| format!("Failed to launch the TTS engine: {e}"))?;

        let _ = std::fs::remove_file(&txt);

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("TTS engine failed ({}). {}", output.status, err.trim()));
        }
        if !mp3.exists() {
            return Err("TTS engine reported success but produced no MP3.".to_string());
        }
        Ok(mp3.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("Audio-brief task failed: {e}"))?
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

/// Kōrero (v1.24.0, paths): export a transcript to a path the user chose in a
/// Save-As dialog (decision D2: ask each time). Forces a .md/.txt extension,
/// then remembers the parent folder in `meeting_export_dir` so the next
/// Save-As opens where the user last saved. The dialog is the consent for the
/// location, so no root confinement applies (unlike recording deletes).
#[tauri::command]
#[specta::specta]
pub async fn meeting_export_transcript_to(
    app: AppHandle,
    path: String,
    content: String,
) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("No destination chosen.".to_string());
    }
    let mut p = PathBuf::from(trimmed);
    match p.extension().and_then(|e| e.to_str()) {
        Some("md") | Some("txt") => {}
        _ => {
            p.set_extension("md");
        }
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Can't create the destination folder: {e}"))?;
    }
    std::fs::write(&p, content).map_err(|e| format!("Failed to write export: {e}"))?;
    // Remember the folder for the next Save-As (best-effort — export succeeded).
    if let Some(parent) = p.parent().map(|d| d.to_string_lossy().to_string()) {
        let mut settings = crate::settings::get_settings(&app);
        settings.meeting_export_dir = Some(parent);
        crate::settings::write_settings(&app, settings);
    }
    Ok(p.to_string_lossy().to_string())
}

/// Kōrero (v1.24.0, paths): the effective storage folders, JSON-encoded (keeps
/// the bindings pre-seed to a plain Result<String,String> — same pragmatic
/// shape as meetings_store_load): { recordingDir, recordingIsCustom,
/// defaultRecordingDir, exportSeedDir }. exportSeedDir = remembered export
/// folder if it still exists, else OS Documents, else the default meetings dir.
#[tauri::command]
#[specta::specta]
pub async fn meeting_dirs_info(app: AppHandle) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    let default_dir = default_meetings_dir(&app)?;
    let recording_dir = meetings_dir(&app)?;
    let recording_is_custom = recording_dir != default_dir;
    let export_seed = settings
        .meeting_export_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && std::path::Path::new(s).is_dir())
        .map(|s| s.to_string())
        .or_else(|| {
            use tauri::Manager;
            app.path()
                .document_dir()
                .ok()
                .map(|d| d.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| default_dir.to_string_lossy().to_string());
    Ok(serde_json::json!({
        "recordingDir": recording_dir.to_string_lossy(),
        "recordingIsCustom": recording_is_custom,
        "defaultRecordingDir": default_dir.to_string_lossy(),
        "exportSeedDir": export_seed,
    })
    .to_string())
}

/// Kōrero (v1.24.0, paths, decision D4b): one-off move of existing meeting
/// media (meeting-*.wav + audio-brief-*.mp3) from the DEFAULT folder into the
/// active custom recording folder. Returns JSON
/// { moved: { oldPath: newPath, ... }, failed, errors[] } — the FRONTEND
/// rewrites micPath/systemPath in its meetings state from that map (risk R8),
/// because the store is frontend-owned with an autosave effect: a Rust-side
/// rewrite of meetings.json would be clobbered by the next in-memory save.
/// Per-file best-effort: rename first, copy+verify+delete across volumes;
/// failures are reported, never fatal. Refuses while a meeting is recording.
#[tauri::command]
#[specta::specta]
pub async fn meeting_move_recordings(
    app: AppHandle,
    meeting: State<'_, Arc<MeetingRecorder>>,
) -> Result<String, String> {
    if meeting
        .active
        .lock()
        .map_err(|_| "lock poisoned")?
        .is_some()
    {
        return Err("A meeting is being recorded — stop it before moving recordings.".to_string());
    }
    let source = default_meetings_dir(&app)?;
    let dest = meetings_dir(&app)?;
    if source == dest {
        return Err("No custom recording folder is set — nothing to move.".to_string());
    }
    let entries =
        std::fs::read_dir(&source).map_err(|e| format!("Can't read the default folder: {e}"))?;
    let mut moved: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut failed = 0u32;
    let mut errors: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_media = (name.starts_with("meeting-") && ext == "wav")
            || (name.starts_with("audio-brief-") && ext == "mp3");
        if !is_media {
            continue; // meetings.json, tmp files, user files: never touched
        }
        let target = dest.join(&name);
        if target.exists() {
            failed += 1;
            errors.push(format!("{name}: already exists in the destination — skipped"));
            continue;
        }
        // rename() is atomic on the same volume; cross-volume it fails, so
        // fall back to copy + verify-size + delete. Peer-review M1: if the
        // SOURCE can't be deleted after a good copy (Windows file lock — e.g.
        // the WAV is open in playback), remove the fresh copy too; otherwise
        // an orphan duplicate lands in the destination, shows twice in the
        // recovery list, and blocks any retry via the target-exists guard.
        let result = std::fs::rename(&path, &target).or_else(|_| {
            std::fs::copy(&path, &target)
                .map_err(|e| e.to_string())
                .and_then(|copied| {
                    let expected = path.metadata().map(|m| m.len()).unwrap_or(copied);
                    if copied == expected {
                        std::fs::remove_file(&path).map_err(|e| {
                            let _ = std::fs::remove_file(&target);
                            format!("file is in use ({e}) — close playback and retry")
                        })
                    } else {
                        let _ = std::fs::remove_file(&target);
                        Err("copy size mismatch".to_string())
                    }
                })
                .map_err(std::io::Error::other)
        });
        match result {
            Ok(()) => {
                moved.insert(
                    path.to_string_lossy().to_string(),
                    target.to_string_lossy().to_string(),
                );
            }
            Err(e) => {
                failed += 1;
                errors.push(format!("{name}: {e}"));
            }
        }
    }
    log::info!(
        "Move recordings: {} moved, {failed} failed.",
        moved.len()
    );
    Ok(serde_json::json!({
        "moved": moved,
        "failed": failed,
        "errors": errors,
    })
    .to_string())
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

/// v1.20.0: strip a leaked instruction preamble from a post-processed result.
///
/// Small local models (e.g. Gemma via Ollama) sometimes echo the system prompt
/// and/or a lead-in like "Here's the cleaned transcript:" before the real
/// output — which previously buried meeting notes under a wall of repeated
/// instructions. This removes, from the START of the text only:
///   * a single wrapping Markdown code fence,
///   * everything up to and including a "here is the <transcript|notes|…>:" lead-in,
///   * a leading run of lines that are verbatim echoes of the prompt.
/// Genuine output (speaker turns, minutes) is never touched, and an all-or-
/// nothing guard returns the original text if stripping would empty the result.
fn strip_llm_preamble(answer: &str, system_prompt: &str) -> String {
    use std::collections::HashSet;

    let mut text = answer.trim().to_string();

    // 1. Unwrap a single surrounding ``` / ```lang fence.
    if text.starts_with("```") {
        if let Some(nl) = text.find('\n') {
            let body = &text[nl + 1..];
            let body = match body.rfind("```") {
                Some(end) => &body[..end],
                None => body,
            };
            text = body.trim().to_string();
        }
    }

    let lines: Vec<&str> = text.lines().collect();

    // 2. Strip an echoed-instruction preamble terminated by a "here's the …:"
    //    lead-in line. To avoid ever deleting genuine output, the cut only fires
    //    when the lines ABOVE the lead-in contain no real speaker turn (so they
    //    are plausibly echoed prompt text) and there is content AFTER it. This
    //    means a legitimate "Here's a summary:" heading sitting below real
    //    minutes is never used to delete that content.
    let is_lead_in = |l: &str| -> bool {
        let low = l.trim().to_lowercase();
        (low.starts_with("here")
            || low.starts_with("sure")
            || low.starts_with("okay, here")
            || low.starts_with("ok, here")
            || low.starts_with("below is"))
            && (low.contains("transcript")
                || low.contains("notes")
                || low.contains("summary")
                || low.contains("minutes")
                || low.contains("clean"))
            && low.ends_with(':')
    };
    // A "Label: text" turn — a short prefix, a colon, then content on the same
    // line. A line that merely ENDS in a colon (e.g. the lead-in itself, or a
    // bare heading) is not a turn.
    let is_speaker_turn = |l: &str| -> bool {
        let t = l.trim();
        match t.find(':') {
            Some(c) if c > 0 && c <= 40 => !t[c + 1..].trim().is_empty(),
            _ => false,
        }
    };
    let head = lines.len().min(60);
    let lead_in_idx = (0..head)
        .rev()
        .find(|&i| is_lead_in(lines[i]) && !lines[..i].iter().any(|&l| is_speaker_turn(l)));
    if let Some(idx) = lead_in_idx {
        let kept = lines[idx + 1..].join("\n").trim().to_string();
        if !kept.is_empty() {
            return kept;
        }
    }

    // 3. No lead-in marker: drop a leading run of blank lines and lines that are
    //    verbatim echoes of the prompt. Short lines are ignored so a genuine
    //    one-word speaker turn is never clipped.
    let prompt_lines: HashSet<String> = system_prompt
        .lines()
        .map(|l| l.trim().to_lowercase())
        .filter(|l| l.len() > 12)
        .collect();
    let mut start = 0;
    while start < lines.len() {
        let l = lines[start].trim();
        if l.is_empty() {
            start += 1;
            continue;
        }
        if prompt_lines.contains(&l.to_lowercase()) {
            start += 1;
            continue;
        }
        break;
    }
    let kept = lines[start..].join("\n").trim().to_string();
    if kept.is_empty() {
        answer.trim().to_string()
    } else {
        kept
    }
}

/// Kōrero (meetings reliability, 2026-09-25): the line appended to notes made
/// from a transcript that was cut to fit the model. `kept`/`total` are char
/// counts. Floors the percentage, so it never over-states the coverage.
fn truncation_note(kept: usize, total: usize) -> String {
    let pct = (kept.min(total) * 100).checked_div(total).unwrap_or(100);
    format!(
        "\n\n---\n\n*These notes cover only the first {pct}% of the transcript: the meeting is \
         longer than the {},{:03}-character limit for notes. To cover a different part, set the \
         trim markers to that part and generate the notes again.*",
        kept / 1000,
        kept % 1000
    )
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
    // Kōrero (meetings reliability, 2026-09-25): remember HOW MUCH was cut.
    // The model was told, but the saved notes never were, so the notes for 6
    // of 60 meetings in one real store silently skipped their final third. The
    // notes now say so, and point at the trim markers as the way to choose
    // the part that matters.
    let total_chars = text.chars().count();
    let text = if total_chars > MAX_CHARS {
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
    // v1.20.0: kept for the post-generation preamble strip (both copies above
    // are moved into the LLM calls).
    let system_for_strip = system.clone();
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
    //
    // v1.30.2: this used the DICTATION helper, which caps at 1500 tokens and
    // 30 s. For a meeting that combination cannot succeed — it would truncate
    // the notes if it returned at all. Now uses the meeting-shaped request.
    let answer = if answer.trim().is_empty() {
        crate::llm_client::send_chat_completion_meeting(
            &provider,
            api_key_fallback,
            &model,
            text_fallback,
            Some(system_fallback),
        )
        .await?
    } else {
        answer
    };

    // v1.20.0: remove any leaked instruction preamble / "Here's the …:" lead-in
    // the model may have prepended (common with small local models) so the saved
    // notes begin at the real content. Applied once to the final text; the live
    // streaming preview is transient and intentionally left untouched.
    let answer = strip_llm_preamble(&answer, &system_for_strip);
    let answer = if total_chars > MAX_CHARS && !answer.trim().is_empty() {
        format!("{answer}{}", truncation_note(MAX_CHARS, total_chars))
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
/// the live path. A decode failure is an `Err`; per-segment transcription
/// failures are counted in the result (see `OfflineTranscript`).
async fn segment_wav_offline(
    tm: &Arc<TranscriptionManager>,
    path: &str,
    source: &'static str,
) -> Result<OfflineTranscript, String> {
    let tm = tm.clone();
    let path = path.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        // Kōrero (meetings reliability, 2026-09-25): a decode failure is an
        // ERROR now, not an empty result. Callers decide what it means: Stop
        // keeps the live text and warns; Re-transcribe keeps the old transcript.
        let segs = decode_and_segment(&path, source)
            .map_err(|e| format!("Could not read the recording: {e}"))?;
        let mut out = OfflineTranscript {
            attempted: segs.len(),
            ..OfflineTranscript::default()
        };
        for s in segs {
            let text = match transcribe_retrying(&tm, s.samples) {
                Ok(t) => t,
                Err(e) => {
                    // Previously `if let Ok(..)` — every failure vanished, so a
                    // model that was not loaded looked exactly like silence.
                    out.failed += 1;
                    out.last_error = Some(e);
                    continue;
                }
            };
            // Same v1.19.0 guards as the live path (offline has energy-only
            // segments, so peak_rms still drives the near-floor drop).
            let text = match clean_segment_text(&text, Some(s.peak_rms)) {
                Some(t) => t,
                None => continue,
            };
            if out
                .segs
                .last()
                .map(|(_, _, last)| normalise_phrase(last) == normalise_phrase(&text))
                .unwrap_or(false)
            {
                continue; // cross-segment duplicate collapse (guard a)
            }
            out.segs.push((s.start_ms, source, text));
        }
        if out.failed > 0 {
            log::warn!(
                "Offline transcription of {path}: {} of {} segment(s) failed; last error: {}",
                out.failed,
                out.attempted,
                out.last_error.as_deref().unwrap_or("?")
            );
        }
        Ok(out)
    })
    .await
    .map_err(|e| format!("Offline transcription task failed: {e}"))?
}

/// Kōrero (meetings reliability, 2026-09-25): the outcome of re-segmenting and
/// transcribing one WAV, with failures COUNTED rather than dropped.
#[derive(Default)]
struct OfflineTranscript {
    segs: Vec<(u64, &'static str, String)>,
    /// Speech segments cut from the file (each one a transcription attempt).
    attempted: usize,
    /// Attempts that failed even after a model reload.
    failed: usize,
    last_error: Option<String>,
}

/// Kōrero (meetings reliability, 2026-09-25): transcribe, and if that failed
/// because the model is no longer loaded, load it and try the same audio once
/// more.
///
/// An engine panic unloads the model ("…will reload on next attempt"), but
/// nothing on the meeting paths ever triggered that reload: `transcribe()`
/// only WAITS for a load already in progress. One bad segment therefore
/// failed every segment after it, for the rest of the meeting or file.
fn transcribe_retrying(tm: &TranscriptionManager, samples: Vec<f32>) -> Result<String, String> {
    match tm.transcribe(samples.clone()) {
        Ok(t) => Ok(t),
        Err(first) => {
            if tm.is_model_loaded() {
                return Err(first.to_string());
            }
            log::warn!(
                "Transcription failed with the model unloaded ({first}); reloading and \
                 retrying once."
            );
            tm.initiate_model_load();
            tm.transcribe(samples).map_err(|e| e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Kōrero (meetings reliability, 2026-09-25): capture health verdicts.
//
// Pure functions over a snapshot of the live counters (meeting_capture.rs), so
// the rules are testable without audio hardware. Used twice: by the live
// consumer, to warn DURING the meeting while the user can still fix a device,
// and by Stop, to say why a side of the transcript is empty.
// ---------------------------------------------------------------------------

/// Seconds of microphone audio, all below the speech gate, before the live
/// meeting warns that the mic is not hearing anyone.
const MIC_SILENT_WARN_SECS: u64 = 60;
/// Seconds of microphone audio before the live meeting judges the system side.
const SYSTEM_SILENT_WARN_SECS: u64 = 90;
/// The same checks at Stop, where the whole meeting is known.
const MIC_SILENT_STOP_SECS: u64 = 10;
const SYSTEM_SILENT_STOP_SECS: u64 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SourceHealth {
    secs: u64,
    loud_frames: u64,
    peak_rms: f32,
}

impl SourceHealth {
    fn of(st: &LiveSourceStats) -> Self {
        Self {
            secs: st.seconds(),
            loud_frames: st.loud_frames.load(Ordering::Relaxed),
            peak_rms: st.peak_rms(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureIssue {
    /// The microphone delivered audio, but never a single frame loud enough to
    /// be speech.
    MicSilent,
    /// A system-audio capture is running but has delivered under a tenth as
    /// much audio as the microphone. WASAPI loopback delivers NOTHING while
    /// its output device is idle, so this is the signature of the meeting
    /// playing on a different device (a headset, say).
    SystemSilent,
}

fn capture_issues(
    you: SourceHealth,
    others: SourceHealth,
    system_on: bool,
    mic_min_secs: u64,
    system_min_secs: u64,
) -> Vec<CaptureIssue> {
    let mut v = Vec::new();
    if you.secs >= mic_min_secs && you.loud_frames == 0 {
        v.push(CaptureIssue::MicSilent);
    }
    if system_on && you.secs >= system_min_secs && others.secs.saturating_mul(10) < you.secs {
        v.push(CaptureIssue::SystemSilent);
    }
    v
}

/// "−52 dBFS" for a peak RMS in 0..1; the speech gate (0.010) is −40 dBFS.
fn fmt_dbfs(rms: f32) -> String {
    if rms <= 0.0 {
        "no sound at all".to_string()
    } else {
        format!("{:.0} dBFS", 20.0 * rms.log10())
    }
}

fn fmt_duration(secs: u64) -> String {
    if secs < 90 {
        format!("{secs} s")
    } else {
        format!("{} min", (secs + 30) / 60)
    }
}

/// The plain-English message for an issue. `live` = said during the meeting
/// (the recording is still running and the user can act); otherwise at Stop.
fn issue_message(
    issue: CaptureIssue,
    you: SourceHealth,
    others: SourceHealth,
    live: bool,
) -> String {
    match (issue, live) {
        (CaptureIssue::MicSilent, true) => format!(
            "Kōrero can't hear your microphone: nothing loud enough to be speech in the last {} \
             (loudest sound {}; speech needs about -40 dBFS). Check which microphone is selected in \
             Settings and that it isn't muted. The recording is still running.",
            fmt_duration(you.secs),
            fmt_dbfs(you.peak_rms),
        ),
        (CaptureIssue::MicSilent, false) => format!(
            "Your microphone recorded nothing loud enough to be speech in this meeting (loudest \
             sound {}; speech needs about -40 dBFS), so your side is empty. Check which microphone \
             is selected in Settings and that it isn't muted.",
            fmt_dbfs(you.peak_rms),
        ),
        (CaptureIssue::SystemSilent, true) => format!(
            "Almost no sound is reaching Kōrero from your computer's audio output ({} in {}). If the \
             call has started and plays through a headset or another device that isn't Windows' \
             default output, the other people won't be transcribed: make that device the default \
             output. If the call hasn't started yet, ignore this.",
            fmt_duration(others.secs),
            fmt_duration(you.secs),
        ),
        (CaptureIssue::SystemSilent, false) => format!(
            "Only {} of computer audio was captured in a {} meeting, so the other people are \
             probably missing from the transcript. The meeting audio was most likely playing on a \
             device other than Windows' default output.",
            fmt_duration(others.secs),
            fmt_duration(you.secs),
        ),
    }
}

/// Stop rebuilds a source from its WAV when its live transcript is empty, or
/// when any live segment for it was dropped or failed.
fn needs_offline_rebuild(has_live: bool, lost: u64) -> bool {
    !has_live || lost > 0
}

/// Whether Stop should REPLACE a source's live segments with the rebuilt ones.
/// With no live text, anything is better than nothing. With live text, only a
/// clean, non-empty rebuild may replace it — a rebuild that itself hit
/// failures could have fewer parts than the live text it would overwrite.
fn adopt_rebuild(has_live: bool, rebuild_failed: usize, rebuild_empty: bool) -> bool {
    if !has_live {
        return true;
    }
    rebuild_failed == 0 && !rebuild_empty
}

fn side_label(source: &str) -> &'static str {
    if source == "you" {
        "your side"
    } else {
        "the other people's side"
    }
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
    // Kōrero (meetings reliability, 2026-09-25): reload-and-retry once, so one
    // engine crash part-way through a long import does not fail every window
    // after it with "Model is not loaded".
    tauri::async_runtime::spawn_blocking(move || transcribe_retrying(&tm, samples))
        .await
        .map_err(|e| format!("Transcription task failed: {e}"))?
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

/// Kōrero (v1.40.0, M1c / RT #1): the import path for the eval harness.
/// A thin wrapper so `TranscribeProgress` can stay module-private.
pub(crate) async fn transcribe_wav_chunked_eval(
    tm: &Arc<TranscriptionManager>,
    path: &str,
) -> Result<String, String> {
    transcribe_wav_chunked(tm, path, None).await
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
    let decoder = rodio::Decoder::try_from(file).map_err(|e| {
        // v1.31.0: "Unrecognized format" is symphonia's message for BOTH "I do
        // not know this container" and "I know the container but have no
        // decoder for the codec inside it". Those are different problems with
        // different fixes, and the bare string sent a real diagnosis down the
        // wrong path for a while. Name the distinction and give the user the
        // one action that always works.
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        format!(
            "Could not decode {name}: {e}.\n\n\
             The file extension only names the container — the audio inside it \
             can be any of several codecs. Kōrero decodes AAC, ALAC, MP3, FLAC, \
             Vorbis and PCM. If this file uses something else, converting it to \
             WAV will always work."
        )
    })?;
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
    // v1.24.0 (paths): the store is PINNED to the default app-private folder —
    // it must not follow a custom recording dir onto removable/synced drives.
    let path = default_meetings_dir(&app)?.join("meetings.json");
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("Failed to read meetings store: {e}")),
    }
}

/// Serialise concurrent writers to the meetings store.
///
/// v1.29.0 (R-03): the temp filename used to be a fixed `meetings.json.tmp`
/// with no lock, so two overlapping saves both create-and-truncate the SAME
/// file and interleave their bytes — after which whichever renames first
/// promotes the mixture. The frontend debounces at 500 ms, but a multi-megabyte
/// store on a slow or synced disk can exceed that, and Tauri commands run
/// concurrently. A unique temp name fixes the interleave; this mutex additionally
/// makes the read-modify-write of the backup coherent.
static MEETINGS_STORE_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Write the meetings store durably: unique temp file, fsync, rename, fsync
/// the directory, keeping one generation of backup.
///
/// v1.29.0 (R-03). The previous implementation was `fs::write` + `fs::rename`
/// and its doc comment claimed a crash "can never truncate it". That was false
/// in three independent ways, and the consequences are severe because meeting
/// WAVs age out after 30 days — past that window this file is the ONLY copy of
/// the transcript.
///
///  1. **No fsync.** `rename` makes the *directory entry* replacement atomic.
///     It does nothing about whether the temp file's *data* reached stable
///     storage. On power loss the entry can point at a zero-length file.
///  2. **Shared temp name.** See the lock above.
///  3. **No backup.** One bad write was terminal.
///
/// The zero-length outcome is the dangerous one, because the frontend used to
/// treat an unreadable store as "no meetings" and then autosave over it — so a
/// torn write became permanent erasure. That half is fixed in
/// MeetingsSettings.tsx; this half stops the tear happening at all.
fn write_meetings_store_durably(dir: &std::path::Path, json: &str) -> std::io::Result<()> {
    use std::io::Write;

    let _guard = MEETINGS_STORE_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let path = dir.join("meetings.json");

    // Unique per call: pid plus nanos. Two concurrent writers can no longer
    // land in the same temp file.
    let unique = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp = dir.join(format!("meetings.json.tmp.{unique}"));

    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        // The load-bearing line. Without it the bytes may still be in the page
        // cache when the rename commits.
        f.sync_all()?;
    }

    // Keep exactly one previous generation. Best-effort: a missing or
    // unreadable current file must not stop the new one being written.
    if path.exists() {
        let backup = dir.join("meetings.json.bak");
        let _ = std::fs::copy(&path, &backup);
    }

    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // Durably record the rename itself. Opening a directory for sync is not
    // supported on Windows, so this is a no-op there and a real barrier on
    // Unix; the rename is still atomic either way.
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }

    Ok(())
}

/// Save the meetings metadata store durably. See `write_meetings_store_durably`
/// for why "atomic rename" alone was not enough.
#[tauri::command]
#[specta::specta]
pub async fn meetings_store_save(app: AppHandle, json: String) -> Result<(), String> {
    // v1.24.0 (paths): pinned to the default folder — see meetings_store_load.
    let dir = default_meetings_dir(&app)?;
    write_meetings_store_durably(&dir, &json)
        .map_err(|e| format!("Failed to commit meetings store: {e}"))
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

// v1.24.0: first unit tests for the meeting.rs text guards — this logic has
// produced two real field bugs ("$3.50" reflow; the FENZ n-gram decoder loop),
// so every collapse behaviour is pinned here.
#[cfg(test)]
mod transcript_offset_tests {
    use super::{to_transcript_segs, TranscriptSeg};

    fn log() -> Vec<(u64, &'static str, String)> {
        vec![
            (0, "you", "kia ora".to_string()),
            (4_000, "others", "morning".to_string()),
            (61_500, "you", "right, agenda".to_string()),
        ]
    }

    /// The regression this whole feature rests on. Both construction sites used
    /// to destructure the offset away with `|(_, s, t)|`; if anyone does that
    /// again, every trim window silently matches nothing.
    #[test]
    fn offsets_survive_conversion() {
        let segs = to_transcript_segs(&log());
        assert_eq!(
            segs.iter().map(|s| s.start_ms).collect::<Vec<_>>(),
            vec![0, 4_000, 61_500],
            "start_ms was dropped or reordered"
        );
        assert_eq!(segs[2].text, "right, agenda");
        assert_eq!(segs[1].source, "others");
    }

    /// A trim window is a half-open comparison against ABSOLUTE offsets. If a
    /// future change ever rebases offsets to the window, this is what breaks:
    /// the caller would subtract the in-point twice.
    #[test]
    fn absolute_offsets_make_a_window_selectable() {
        let segs = to_transcript_segs(&log());
        let (in_ms, out_ms) = (4_000u64, 60_000u64);
        let kept: Vec<&TranscriptSeg> = segs
            .iter()
            .filter(|s| s.start_ms >= in_ms && s.start_ms <= out_ms)
            .collect();
        assert_eq!(kept.len(), 1, "expected only the 4.0s segment in [4s, 60s]");
        assert_eq!(kept[0].text, "morning");
    }

    #[test]
    fn empty_log_is_not_a_panic() {
        assert!(to_transcript_segs(&[]).is_empty());
    }
}

#[cfg(test)]
mod text_guard_tests {
    use super::*;

    #[test]
    fn ngram_collapses_repeated_trigram_run() {
        // The FENZ import signature: "ProjectIQ, and the" repeated dozens of
        // times inside one comma-run with no sentence boundary.
        let unit = "ProjectIQ, and the ";
        let text = format!("So the plan is {}{}done.", unit.repeat(40), "");
        let out = collapse_repeats(&text);
        assert!(
            out.matches("ProjectIQ").count() <= 2,
            "trigram run not collapsed: {out}"
        );
        assert!(out.contains("So the plan is"));
        assert!(out.contains("done."));
    }

    #[test]
    fn ngram_collapses_repeated_single_word_run() {
        let text = format!("and the {}Code Genie builds it.", "project, ".repeat(120));
        let out = collapse_repeats(&text);
        assert!(
            out.matches("project,").count() <= 1,
            "single-word run not collapsed: {out}"
        );
        assert!(out.contains("Code Genie builds it."));
    }

    #[test]
    fn natural_repetition_survives() {
        // ≤4 single-word repeats and ≤3 phrase repeats are natural speech.
        let a = "no, no, no, no, that's not right.";
        assert_eq!(collapse_repeats(a), a);
        let b = "I know, I know, I know. Let's move on. It was very very good.";
        assert_eq!(collapse_repeats(b), b);
    }

    #[test]
    fn clean_text_returned_verbatim() {
        // The no-collapse contract: ordinary transcripts are never reflowed.
        let t = "We agreed the retaining wall quote by Friday.  Two spaces kept.";
        assert_eq!(collapse_repeats(t), t);
    }

    #[test]
    fn decimals_still_never_split() {
        // Regression pin for the v1.19.0 fix.
        let t = "The quote was $3.50 per metre for v1.19 of the plan.";
        assert_eq!(collapse_repeats(t), t);
    }

    #[test]
    fn sentence_level_collapse_still_works() {
        let t = "It's great. It's great. It's great. Moving on.";
        let out = collapse_repeats(t);
        assert_eq!(out.matches("It's great.").count(), 1, "{out}");
        assert!(out.contains("Moving on."));
    }

    #[test]
    fn mixed_trigram_then_word_run_both_collapse() {
        let text = format!(
            "Start. {}{}End.",
            "ProjectIQ, and the ".repeat(25),
            "project, ".repeat(60)
        );
        let out = collapse_repeats(&text);
        assert!(out.matches("ProjectIQ").count() <= 2, "{out}");
        assert!(out.matches("project,").count() <= 2, "{out}");
        assert!(out.starts_with("Start."));
        assert!(out.ends_with("End."));
    }
}

#[cfg(test)]
mod meetings_store_durability_tests {
    use super::write_meetings_store_durably;
    use std::fs;

    /// R-03. Before v1.29.0 the temp filename was a fixed `meetings.json.tmp`,
    /// so this test's second writer would have clobbered the first's temp file
    /// mid-write. Now every call gets its own.
    #[test]
    fn korero_r03_temp_file_is_unique_per_call_and_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..8 {
            write_meetings_store_durably(dir.path(), &format!("[{{\"n\":{i}}}]")).unwrap();
        }
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files must not survive a successful write: {leftovers:?}"
        );
    }

    #[test]
    fn korero_r03_content_round_trips_exactly() {
        let dir = tempfile::tempdir().unwrap();
        // Macrons matter: the store carries te reo meeting titles.
        let payload = r#"[{"id":"1","title":"Hui whakatōhea — kōrero"}]"#;
        write_meetings_store_durably(dir.path(), payload).unwrap();
        let back = fs::read_to_string(dir.path().join("meetings.json")).unwrap();
        assert_eq!(back, payload);
    }

    /// A backup must appear from the SECOND save onward, and must hold the
    /// PREVIOUS contents — not the current ones. If it held the current
    /// contents it would be worthless as a rescue.
    #[test]
    fn korero_r03_backup_holds_the_previous_generation() {
        let dir = tempfile::tempdir().unwrap();
        let bak = dir.path().join("meetings.json.bak");

        write_meetings_store_durably(dir.path(), "[1]").unwrap();
        assert!(
            !bak.exists(),
            "there is nothing to back up on the very first write"
        );

        write_meetings_store_durably(dir.path(), "[1,2]").unwrap();
        assert_eq!(fs::read_to_string(&bak).unwrap(), "[1]");
        assert_eq!(
            fs::read_to_string(dir.path().join("meetings.json")).unwrap(),
            "[1,2]"
        );

        write_meetings_store_durably(dir.path(), "[1,2,3]").unwrap();
        assert_eq!(fs::read_to_string(&bak).unwrap(), "[1,2]");
    }

    /// The defect this replaces: two overlapping saves interleaved their bytes
    /// into one shared temp file, and the rename promoted the mixture. Every
    /// run must leave a file that parses, and whose content is exactly one of
    /// the payloads written — never a splice of two.
    #[test]
    fn korero_r03_concurrent_saves_never_produce_a_torn_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = format!("[\"{}\"]", "a".repeat(200_000));
        let b = format!("[\"{}\"]", "b".repeat(200_000));

        for _ in 0..10 {
            let (pa, pb) = (a.clone(), b.clone());
            let (d1, d2) = (dir.path().to_path_buf(), dir.path().to_path_buf());
            let h1 = std::thread::spawn(move || write_meetings_store_durably(&d1, &pa));
            let h2 = std::thread::spawn(move || write_meetings_store_durably(&d2, &pb));
            h1.join().unwrap().unwrap();
            h2.join().unwrap().unwrap();

            let got = fs::read_to_string(dir.path().join("meetings.json")).unwrap();
            assert!(
                got == a || got == b,
                "torn write: got {} bytes, expected exactly one whole payload",
                got.len()
            );
        }
    }
}

#[cfg(test)]
mod korero_v1_31_decode_tests {
    use super::open_rodio_stream;

    /// v1.31.0. Opt-in decode test: point `KORERO_TEST_AUDIO` at a real audio
    /// file and this asserts the decoder actually produces audio from it.
    ///
    /// It is a no-op without the variable, deliberately. The file that exposed
    /// this bug is a 45-minute, 202 MB client recording — it cannot become a
    /// fixture, for size and for confidentiality. But "it compiles" was never
    /// the question: v1.16.1 compiled fine and still could not read ALAC. The
    /// question is whether SAMPLES come out, so that is what this checks.
    ///
    ///   $env:KORERO_TEST_AUDIO = "C:\path\to\file.m4a"
    ///   cargo test --locked korero_v1_31 -- --nocapture
    #[test]
    fn korero_v1_31_decodes_a_supplied_audio_file() {
        let Ok(path) = std::env::var("KORERO_TEST_AUDIO") else {
            eprintln!("KORERO_TEST_AUDIO not set — skipping decode test");
            return;
        };

        let (rate, channels, mut samples) =
            open_rodio_stream(&path).unwrap_or_else(|e| panic!("decode failed: {e}"));

        assert!(
            (8_000..=192_000).contains(&rate),
            "implausible sample rate {rate}"
        );
        assert!((1..=8).contains(&channels), "implausible channels {channels}");

        // One second of audio is plenty to prove the codec is wired up.
        let want = rate * channels;
        let got: Vec<f32> = samples.by_ref().take(want).filter_map(|r| r.ok()).collect();
        assert!(
            got.len() > want / 2,
            "decoder yielded only {} of {want} samples",
            got.len()
        );

        // A decoder that is present but wrong returns silence rather than
        // failing. Require actual signal, and require it to be in range.
        let peak = got.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.0001, "decoded {} samples but all silent", got.len());
        assert!(peak <= 1.5, "samples out of range (peak {peak}) — not normalised f32");

        eprintln!(
            "decoded {} samples @ {rate} Hz x{channels}, peak {peak:.4}",
            got.len()
        );
    }
}

/// Kōrero (meetings reliability, 2026-09-25). Each test is named for the
/// behaviour it pins, so a failure reads as the regression it would be.
#[cfg(test)]
mod korero_meetings_reliability_tests {
    use super::*;

    fn health(secs: u64, loud_frames: u64, peak_rms: f32) -> SourceHealth {
        SourceHealth {
            secs,
            loud_frames,
            peak_rms,
        }
    }

    /// The in-meeting check (live thresholds).
    fn live(you: SourceHealth, others: SourceHealth, system_on: bool) -> Vec<CaptureIssue> {
        capture_issues(you, others, system_on, MIC_SILENT_WARN_SECS, SYSTEM_SILENT_WARN_SECS)
    }

    /// The check at Stop (lower thresholds: the whole meeting is known).
    fn at_stop(you: SourceHealth, others: SourceHealth, system_on: bool) -> Vec<CaptureIssue> {
        capture_issues(you, others, system_on, MIC_SILENT_STOP_SECS, SYSTEM_SILENT_STOP_SECS)
    }

    #[test]
    fn a_silent_mic_is_reported_once_it_has_run_long_enough() {
        // A real 34-minute meeting: mic peak 0.0073, never one loud frame.
        let silent = health(2067, 0, 0.0073);
        assert_eq!(live(silent, health(0, 0, 0.0), false), vec![CaptureIssue::MicSilent]);
        // ...but not in the first seconds, before there is anything to judge.
        assert!(live(health(20, 0, 0.0), health(0, 0, 0.0), false).is_empty());
    }

    #[test]
    fn one_loud_frame_means_the_mic_is_working() {
        let issues = live(health(600, 1, 0.2), health(600, 50, 0.1), true);
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn loopback_that_delivered_almost_nothing_is_reported() {
        // A real 63-minute meeting: 12 s of system audio in total.
        let issues = live(health(3817, 18_588, 0.34), health(12, 5, 0.13), true);
        assert_eq!(issues, vec![CaptureIssue::SystemSilent]);
    }

    #[test]
    fn no_system_warning_when_no_system_capture_was_running() {
        // Mic-only meetings already get their own "system audio couldn't be
        // captured" message at start; the health check must not add a second.
        let issues = live(health(3817, 18_588, 0.34), health(0, 0, 0.0), false);
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn a_healthy_meeting_raises_nothing() {
        // A real, healthy 47-minute meeting: both sides delivered ~2830 s.
        let issues = at_stop(health(2834, 5289, 0.05), health(2832, 9208, 0.09), true);
        assert!(issues.is_empty(), "{issues:?}");
        // A real meeting where others delivered 886 s against 1368 s
        // of mic — less, but nowhere near the "under a tenth" signature.
        let uneven = at_stop(health(1368, 2544, 0.13), health(886, 4337, 0.08), true);
        assert!(uneven.is_empty(), "{uneven:?}");
    }

    #[test]
    fn messages_name_the_level_and_the_durations() {
        let quiet = health(2067, 0, 0.0073);
        let m = issue_message(CaptureIssue::MicSilent, quiet, health(0, 0, 0.0), false);
        assert!(m.contains("-43 dBFS"), "{m}");
        let long = health(3817, 1, 0.3);
        let s = issue_message(CaptureIssue::SystemSilent, long, health(12, 1, 0.1), false);
        assert!(s.contains("12 s") && s.contains("64 min"), "{s}");
        assert_eq!(fmt_dbfs(0.0), "no sound at all");
        assert_eq!(fmt_duration(89), "89 s");
        assert_eq!(fmt_duration(90), "2 min");
    }

    #[test]
    fn stop_rebuilds_an_empty_or_a_lossy_side_and_nothing_else() {
        assert!(needs_offline_rebuild(false, 0), "empty side: rebuild (the old rule)");
        assert!(needs_offline_rebuild(true, 1), "lossy side: rebuild (new)");
        assert!(!needs_offline_rebuild(true, 0), "complete live side: keep it");
    }

    #[test]
    fn a_rebuild_only_replaces_live_text_when_it_is_clean() {
        assert!(adopt_rebuild(false, 3, true), "nothing live: anything is better");
        assert!(adopt_rebuild(true, 0, false), "clean rebuild replaces a lossy live side");
        assert!(!adopt_rebuild(true, 2, false), "a rebuild with failures must not overwrite");
        assert!(!adopt_rebuild(true, 0, true), "an empty rebuild must not erase live text");
    }

    #[test]
    fn live_failures_are_counted_per_source() {
        let live = LiveTranscript::default();
        live.note_failure("you");
        live.note_failure("you");
        live.note_failure("others");
        assert_eq!(live.failures("you"), 2);
        assert_eq!(live.failures("others"), 1);
    }

    #[test]
    fn truncated_notes_say_how_much_they_cover() {
        // A real 73,594-character transcript, cut to 48,000.
        let n = truncation_note(48_000, 73_594);
        assert!(n.contains("first 65%"), "{n}");
        assert!(n.contains("48,000-character"), "{n}");
        assert!(truncation_note(48_000, 48_000).contains("100%"));
        assert!(truncation_note(0, 0).contains("100%"), "no divide by zero");
    }
}
