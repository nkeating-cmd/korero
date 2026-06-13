//! Kōrero fork (v1.13.3, Meetings Phase A): streaming-to-disk meeting capture.
//!
//! Replaces the RAM-buffered meeting capture (two `AudioRecorder`s) with a
//! dedicated streaming capture that is fully ISOLATED from the dictation path:
//!
//! * Audio is resampled to 16 kHz mono and written to a WAV on disk
//!   INCREMENTALLY, with a header flush every ~5 s — a hard crash mid-meeting
//!   still leaves a readable recording (the failsafe), and memory stays bounded
//!   to the in-flight frames instead of ~230 MB/hour per stream.
//! * The audio channel is BOUNDED (v1.13.3): if the writer stalls (OneDrive
//!   sync, AV scan, disk pressure) the device callback drops frames instead of
//!   queueing unbounded RAM. Overruns are counted and logged.
//! * Disk-write failures are SURFACED (v1.13.3): past a threshold the worker
//!   aborts, emits a `meeting-capture-error` event for the UI, and returns an
//!   error from `stop()`. Audio written before the failure stays on disk.
//! * The consumer uses a TIMED recv, so a silent stream (e.g. WASAPI loopback
//!   with nothing playing) can never wedge the stop path — the hang we
//!   previously had to band-aid with a stop timeout is fixed by design.
//! * Phase B (live transcription) will tap the same resampled frames for VAD
//!   segmentation — see the marked hook in the worker loop.
//!
//! The cpal `Stream` is created and held on the worker thread (cpal streams are
//! not Send); the capture is controlled via an atomic stop flag.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, Sample, SizedSample};
use tauri::{AppHandle, Emitter, Manager};

use crate::audio_toolkit::audio::FrameResampler;
use crate::audio_toolkit::vad::{SileroVad, SmoothedVad, VoiceActivityDetector};

const TARGET_RATE: usize = 16_000;
/// Resampler frame size; small for low latency. Phase B's VAD consumes the
/// same frames.
const FRAME_DUR: Duration = Duration::from_millis(30);
/// Flush the WAV header every ~5 s of audio so a crash still leaves a valid file.
/// (pub(crate): shared with the native WASAPI loopback sink, v1.13.6.)
pub(crate) const FLUSH_EVERY_SAMPLES: u64 = (TARGET_RATE as u64) * 5;
/// Bounded frame channel: ~256 device callbacks ≈ 2.5 s of audio at WASAPI's
/// typical 10 ms cadence. If the writer stalls longer than that, frames are
/// dropped (counted + logged) rather than growing RAM without limit.
const CHANNEL_CAPACITY: usize = 256;
/// Abort capture once this many samples have failed to write (~0.3 s at
/// 16 kHz). Disk-full fails every sample, so this trips in well under a second
/// instead of silently "recording" nothing.
pub(crate) const MAX_WRITE_ERRORS: u64 = 4_800;
/// A failed header flush is a strong disk-failure signal: weight it so two
/// consecutive flush failures trip the abort threshold on their own.
pub(crate) const FLUSH_ERROR_WEIGHT: u64 = MAX_WRITE_ERRORS / 2 + 1;
/// Cadence for live level/progress events to the Meetings UI meters.
pub(crate) const LEVEL_EVERY: Duration = Duration::from_millis(200);

/// Live meter event (v1.13.5): peak input level for the last window (0..1,
/// pre-resample) plus total 16 kHz samples written so far. The UI uses
/// `written` to show per-stream captured time and to warn when a stream that
/// claims to be recording has produced no audio at all — the failure mode that
/// was previously invisible (e.g. loopback on the wrong output device).
#[derive(serde::Serialize, Clone)]
pub(crate) struct LevelEvent {
    pub(crate) source: &'static str,
    pub(crate) level: f32,
    pub(crate) written: u64,
}

// ---------------------------------------------------------------------------
// Phase B (v1.14.0): live segmentation
// ---------------------------------------------------------------------------

/// A speech segment cut from the live 16 kHz stream, ready to transcribe.
pub(crate) struct LiveSegment {
    pub(crate) source: &'static str,
    pub(crate) samples: Vec<f32>,
    /// v1.17.0: capture-relative start time of this segment, in milliseconds
    /// from the first frame the segmenter saw. Both the "you" and "others"
    /// segmenters count from their own first frame (the two streams start
    /// within a few ms of each other), so `start_ms` is directly comparable
    /// ACROSS sources — that is what lets the final transcript interleave both
    /// speakers in chronological order instead of two per-speaker blocks.
    pub(crate) start_ms: u64,
    /// v1.19.0: the peak frame RMS observed while this segment was buffered.
    /// The live consumer uses it as a cheap "was this near the energy floor?"
    /// signal so a known-hallucination phrase emitted from a barely-audible
    /// segment can be dropped (belt-and-braces behind the VAD).
    pub(crate) peak_rms: f32,
}

/// Bounded sender for live segments (drop-on-full: a lost live segment is
/// only a UI nicety — the WAV on disk always has the complete audio).
pub(crate) type SegmentSender = mpsc::SyncSender<LiveSegment>;

/// Cut a segment after this much sustained quiet…
const SEG_QUIET_SAMPLES: usize = TARGET_RATE * 7 / 10; // 700 ms
/// …or at this hard cap (keeps live-transcription latency bounded).
const SEG_MAX_SAMPLES: usize = TARGET_RATE * 20;
/// Fragments shorter than this aren't worth a model invocation.
const SEG_MIN_SAMPLES: usize = TARGET_RATE / 2;
/// Frame RMS below this counts as quiet; a segment must have peaked above
/// 2× this to be treated as speech at all. (pub(crate): the meeting consumer
/// uses it as the "near the energy floor" threshold for its hallucination
/// guard.)
pub(crate) const SEG_QUIET_RMS: f32 = 0.010;
/// Silero consumes exactly 30 ms frames at 16 kHz (= 480 samples) — the same
/// frame the capture resampler emits. Off-size frames (e.g. the resampler's
/// closing tail) bypass the VAD and fall back to the energy gate.
const VAD_FRAME_SAMPLES: usize = TARGET_RATE * 30 / 1000;

/// Build the optional speech-gating VAD for a meeting segmenter. Resolves the
/// SAME bundled Silero model the dictation recorder uses and wraps it in the
/// SAME smoothing (prefill/hangover/onset) so behaviour is consistent across
/// the app. Returns `None` — falling back to the energy gate alone — if the
/// model can't be resolved or loaded, so a VAD problem can never stop a
/// meeting from recording.
pub(crate) fn build_meeting_vad(
    app: Option<&AppHandle>,
    source: &str,
) -> Option<Box<dyn VoiceActivityDetector>> {
    let app = app?;
    let path = app
        .path()
        .resolve(
            "resources/models/silero_vad_v4.onnx",
            tauri::path::BaseDirectory::Resource,
        )
        .ok()?;
    match SileroVad::new(&path, 0.3) {
        Ok(silero) => {
            // 15-frame prefill + 15-frame hangover + 2-frame onset mirrors the
            // dictation recorder (managers/audio.rs).
            let smoothed = SmoothedVad::new(Box::new(silero), 15, 15, 2);
            Some(Box::new(smoothed) as Box<dyn VoiceActivityDetector>)
        }
        Err(e) => {
            log::warn!(
                "Meeting VAD unavailable for '{source}' ({e}); using the energy gate alone."
            );
            None
        }
    }
}

/// Speech segmenter (Phase B; Silero-gated since v1.19.0). The cheap energy
/// gate is the first-pass filter — frames below `SEG_QUIET_RMS` never reach the
/// model — and Silero then decides whether above-floor energy is actually
/// SPEECH. That distinction is the root-cause fix for non-speech transients
/// (keyboard typing, mouse clicks) that clear the energy gate but used to be
/// transcribed into "Thank you."-style hallucinations. With no VAD available
/// the original energy-only behaviour is preserved exactly.
pub(crate) struct Segmenter {
    source: &'static str,
    tx: SegmentSender,
    buf: Vec<f32>,
    trailing_quiet: usize,
    speech_peak: f32,
    dropped: u64,
    /// v1.17.0: total frames pushed since this segmenter started — the
    /// capture-relative clock used to stamp each segment's `start_ms`.
    samples_seen: usize,
    /// v1.17.0: sample index (within `samples_seen`) at which the currently
    /// buffered segment began. Captured when the first frame lands in an empty
    /// buffer, so it survives the quiet-skip and 20 s-cap cut paths alike.
    seg_start: usize,
    /// v1.19.0: optional Silero VAD. When present it — not the raw energy peak
    /// — decides whether the buffered audio is worth transcribing.
    vad: Option<Box<dyn VoiceActivityDetector>>,
    /// v1.19.0: did any frame in the current buffer register as speech? With a
    /// VAD this is the cut gate; without one the `speech_peak` test is used.
    had_voice: bool,
}

impl Segmenter {
    pub(crate) fn new(
        source: &'static str,
        tx: SegmentSender,
        vad: Option<Box<dyn VoiceActivityDetector>>,
    ) -> Self {
        Self {
            source,
            tx,
            buf: Vec::with_capacity(SEG_MAX_SAMPLES),
            trailing_quiet: 0,
            speech_peak: 0.0,
            dropped: 0,
            samples_seen: 0,
            seg_start: 0,
            vad,
            had_voice: false,
        }
    }

    /// Feed one resampled 16 kHz frame (the ~30 ms frames the captures emit).
    pub(crate) fn push(&mut self, frame: &[f32]) {
        if frame.is_empty() {
            return;
        }
        // v1.17.0: a fresh segment begins on the first frame after an empty
        // buffer — record its capture-relative start before appending.
        if self.buf.is_empty() {
            self.seg_start = self.samples_seen;
        }
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        self.buf.extend_from_slice(frame);
        self.samples_seen += frame.len();

        // Two-stage gate. Stage 1 (cheap): frames below the energy floor are
        // definitely silence — never pay for a VAD inference on them. Stage 2:
        // above the floor, ask Silero whether the energy is actually SPEECH.
        // Typing/click transients clear stage 1 but Silero rejects them, which
        // is exactly the non-speech audio that used to be hallucinated into
        // "Thank you." With no VAD this reduces to the original energy test.
        let voiced = if rms < SEG_QUIET_RMS {
            false
        } else if let Some(vad) = self.vad.as_mut() {
            if frame.len() == VAD_FRAME_SAMPLES {
                // Fail OPEN: if the VAD errors, trust the energy gate's "loud
                // enough" verdict rather than dropping possible speech.
                vad.is_voice(frame).unwrap_or(true)
            } else {
                true
            }
        } else {
            true
        };

        if voiced {
            self.trailing_quiet = 0;
            self.had_voice = true;
            if rms > self.speech_peak {
                self.speech_peak = rms;
            }
        } else {
            self.trailing_quiet += frame.len();
        }

        if self.buf.len() >= SEG_MAX_SAMPLES {
            self.cut(SEG_MIN_SAMPLES);
        } else if self.trailing_quiet >= SEG_QUIET_SAMPLES {
            if self.speech_enough() && self.buf.len() > self.trailing_quiet {
                self.cut(SEG_MIN_SAMPLES);
            } else {
                // Quiet/non-speech all the way through — nothing to transcribe.
                self.buf.clear();
                self.trailing_quiet = 0;
                self.speech_peak = 0.0;
                self.had_voice = false;
            }
        }
    }

    /// Did the current buffer contain real speech? With a VAD this is the
    /// Silero verdict (`had_voice`); without one it is the original
    /// peaked-above-2×-floor energy test, preserving legacy behaviour exactly.
    fn speech_enough(&self) -> bool {
        if self.vad.is_some() {
            self.had_voice
        } else {
            self.speech_peak >= SEG_QUIET_RMS * 2.0
        }
    }

    /// Flush whatever is buffered (the stream is ending). Uses a LOWER length
    /// floor than mid-stream cuts so a brief closing remark ("thanks, bye")
    /// still makes the live transcript — which the meeting's final transcript
    /// now prefers.
    pub(crate) fn finish(&mut self) {
        if self.speech_enough() {
            self.cut(TARGET_RATE / 4); // ≥ 250 ms
        } else {
            self.buf.clear();
        }
    }

    fn cut(&mut self, min_samples: usize) {
        let samples = std::mem::take(&mut self.buf);
        let start_ms = (self.seg_start as u64) * 1000 / TARGET_RATE as u64;
        let peak_rms = self.speech_peak;
        self.trailing_quiet = 0;
        self.speech_peak = 0.0;
        self.had_voice = false;
        if samples.len() < min_samples {
            return;
        }
        match self.tx.try_send(LiveSegment {
            source: self.source,
            samples,
            start_ms,
            peak_rms,
        }) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped += 1;
                log::warn!(
                    "Live transcription falling behind — dropped segment #{} from '{}' \
                     (the full transcript is still recoverable from the WAV).",
                    self.dropped,
                    self.source
                );
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {}
        }
    }
}

/// One device being captured to one streaming WAV.
pub struct StreamCapture {
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<u64, String>>>,
    pub path: PathBuf,
}

impl StreamCapture {
    /// Start capturing `device` (with its native `config`) to `path` as a
    /// streaming 16 kHz mono WAV. Fails fast (within ~5 s) if the stream can't
    /// be built or started — e.g. loopback unsupported on this device.
    ///
    /// `app` (when provided) receives a `meeting-capture-error` event if the
    /// capture aborts mid-meeting (e.g. disk full), plus `meeting-level`
    /// events every ~200 ms tagged with `source` ("you" / "others") so the UI
    /// can show live input meters and captured-duration counters.
    /// `segments` (Phase B, v1.14.0): when provided, an energy-gated
    /// segmenter taps the same 16 kHz frames and ships speech segments for
    /// live transcription. Purely additive — the WAV sink is unaffected.
    pub fn start(
        device: Device,
        config: cpal::SupportedStreamConfig,
        path: PathBuf,
        app: Option<AppHandle>,
        source: &'static str,
        segments: Option<SegmentSender>,
        paused: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_for_worker = stop_flag.clone();
        let paused_for_worker = paused.clone();
        let worker_path = path.clone();
        let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();

        let worker = std::thread::spawn(move || -> Result<u64, String> {
            let in_rate = config.sample_rate().0 as usize;
            let channels = config.channels() as usize;

            // BOUNDED channel (v1.13.3): back-pressure drops frames in the
            // device callback instead of queueing unbounded RAM if the disk
            // writer stalls.
            let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
            // Buffer pool (v1.13.4): the worker returns drained Vecs to the
            // device callback for reuse, so the steady-state audio path makes
            // no heap allocations (RT-audio hygiene — allocation in a device
            // callback can glitch under memory pressure). Pool population is
            // bounded by the frames in flight (≤ CHANNEL_CAPACITY + 1).
            let (pool_tx, pool_rx) = mpsc::channel::<Vec<f32>>();
            let stream = match build_stream_for(&device, &config, channels, tx, pool_rx) {
                Ok(s) => s,
                Err(e) => {
                    let msg = format!("Failed to build capture stream: {e}");
                    let _ = init_tx.send(Err(msg.clone()));
                    return Err(msg);
                }
            };
            if let Err(e) = stream.play() {
                let msg = format!("Failed to start capture stream: {e}");
                let _ = init_tx.send(Err(msg.clone()));
                return Err(msg);
            }

            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: TARGET_RATE as u32,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = match hound::WavWriter::create(&worker_path, spec) {
                Ok(w) => w,
                Err(e) => {
                    let msg = format!("Failed to create {}: {e}", worker_path.display());
                    let _ = init_tx.send(Err(msg.clone()));
                    return Err(msg);
                }
            };
            let _ = init_tx.send(Ok(()));

            let mut resampler = FrameResampler::new(in_rate, TARGET_RATE, FRAME_DUR);
            let mut written: u64 = 0;
            let mut last_flush: u64 = 0;
            // v1.13.3: failed sample writes / flushes accumulate here; past
            // MAX_WRITE_ERRORS the capture aborts and surfaces the failure.
            let mut write_errors: u64 = 0;
            let mut abort_err: Option<String> = None;
            // v1.13.5: live meter state.
            let mut window_peak: f32 = 0.0;
            let mut last_emit = Instant::now();
            // Phase B (v1.14.0): optional live segmenter. v1.19.0: gated by the
            // same bundled Silero model the dictation path uses (built on this
            // worker thread — the VAD is not Send).
            let mut segmenter =
                segments.map(|tx| Segmenter::new(source, tx, build_meeting_vad(app.as_ref(), source)));

            loop {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(mut raw) => {
                        // v1.19.0: while paused, drop the frame entirely — don't
                        // write it, don't segment it, don't move the meter. The
                        // device stream stays OPEN (no re-acquisition), so the
                        // WAV is simply continuous-with-gaps and paused speech
                        // never reaches the transcript.
                        if !paused_for_worker.load(Ordering::Relaxed) {
                            // Track the window peak for the UI level meter.
                            for &s in raw.iter() {
                                let a = s.abs();
                                if a > window_peak {
                                    window_peak = a;
                                }
                            }
                            resampler.push(&raw, |frame: &[f32]| {
                                // Phase B (v1.14.0): live segmentation taps the
                                // same frames the WAV sink writes.
                                if let Some(seg) = segmenter.as_mut() {
                                    seg.push(frame);
                                }
                                for &s in frame {
                                    let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                    if writer.write_sample(v).is_err() {
                                        write_errors += 1;
                                    } else {
                                        written += 1;
                                    }
                                }
                            });
                        }
                        // v1.13.4: return the drained buffer to the callback's
                        // pool for reuse (allocation-free steady state).
                        raw.clear();
                        let _ = pool_tx.send(raw);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // No audio (silent loopback is normal) — fall through so
                        // the stop flag below is still observed promptly.
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }

                // v1.13.5: live level/progress events for the Meetings meters.
                // Emitted on a timer (not per-buffer) so a totally silent
                // stream still reports level 0 / 0 samples instead of a frozen
                // meter — that distinction is exactly what makes "recording
                // but capturing nothing" visible.
                if let Some(app) = &app {
                    if last_emit.elapsed() >= LEVEL_EVERY {
                        let _ = app.emit(
                            "meeting-level",
                            LevelEvent {
                                source,
                                level: window_peak,
                                written,
                            },
                        );
                        window_peak = 0.0;
                        last_emit = Instant::now();
                    }
                }

                if write_errors > MAX_WRITE_ERRORS {
                    abort_err = Some(format!(
                        "Recording to {} is failing (disk full or unwritable). \
                         Audio captured before the failure has been kept.",
                        worker_path.display()
                    ));
                    break;
                }

                if written.saturating_sub(last_flush) >= FLUSH_EVERY_SAMPLES {
                    if let Err(e) = writer.flush() {
                        log::warn!("Meeting WAV flush failed: {e}");
                        write_errors += FLUSH_ERROR_WEIGHT;
                    }
                    last_flush = written;
                }

                if stop_for_worker.load(Ordering::Relaxed) {
                    // Drain anything already in the channel, then emit the
                    // resampler tail. If we were stopped while paused, the
                    // pending frames are post-pause audio to discard — but the
                    // resampler tail (pre-pause residual) is still flushed.
                    let paused_now = paused_for_worker.load(Ordering::Relaxed);
                    while let Ok(raw) = rx.try_recv() {
                        if paused_now {
                            continue;
                        }
                        resampler.push(&raw, |frame: &[f32]| {
                            if let Some(seg) = segmenter.as_mut() {
                                seg.push(frame);
                            }
                            for &s in frame {
                                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                if writer.write_sample(v).is_err() {
                                    write_errors += 1;
                                } else {
                                    written += 1;
                                }
                            }
                        });
                    }
                    resampler.finish(|frame: &[f32]| {
                        if let Some(seg) = segmenter.as_mut() {
                            seg.push(frame);
                        }
                        for &s in frame {
                            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            if writer.write_sample(v).is_err() {
                                write_errors += 1;
                            } else {
                                written += 1;
                            }
                        }
                    });
                    break;
                }
            }

            drop(stream); // close the device stream before finalising

            // Phase B: ship any final partial segment, then drop the sender so
            // the live-transcription consumer can drain and exit.
            if let Some(seg) = segmenter.as_mut() {
                seg.finish();
            }
            drop(segmenter);

            if let Some(msg) = abort_err {
                // Best-effort finalise so the pre-failure audio stays a valid
                // WAV; the periodic flush already kept the header consistent.
                let _ = writer.finalize();
                log::error!("Meeting capture aborted: {msg}");
                if let Some(app) = &app {
                    let _ = app.emit("meeting-capture-error", msg.clone());
                }
                return Err(msg);
            }

            writer
                .finalize()
                .map_err(|e| format!("Failed to finalise meeting WAV: {e}"))?;
            Ok(written)
        });

        match init_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop_flag,
                worker: Some(worker),
                path,
            }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                // Worker never reported — signal it to stop and bail.
                stop_flag.store(true, Ordering::Relaxed);
                Err("Capture worker did not initialise in time.".to_string())
            }
        }
    }

    /// Stop and finalise the WAV. Returns the number of 16 kHz samples written.
    /// Cannot hang: the worker's recv is timed, so it observes the stop flag
    /// within ~200 ms even on a totally silent stream.
    pub fn stop(mut self) -> Result<u64, String> {
        self.stop_flag.store(true, Ordering::Relaxed);
        let handle = self
            .worker
            .take()
            .ok_or_else(|| "Capture already stopped.".to_string())?;
        handle
            .join()
            .map_err(|_| "Capture worker panicked.".to_string())?
    }
}

impl Drop for StreamCapture {
    fn drop(&mut self) {
        // If dropped without stop() (e.g. an error path), signal the worker so
        // it exits within ~200 ms and finalises the file. Not joined here to
        // keep drops non-blocking.
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

fn build_stream_for(
    device: &Device,
    config: &cpal::SupportedStreamConfig,
    channels: usize,
    tx: mpsc::SyncSender<Vec<f32>>,
    pool: mpsc::Receiver<Vec<f32>>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    match config.sample_format() {
        cpal::SampleFormat::F32 => build_typed::<f32>(device, config, channels, tx, pool),
        cpal::SampleFormat::I16 => build_typed::<i16>(device, config, channels, tx, pool),
        cpal::SampleFormat::I32 => build_typed::<i32>(device, config, channels, tx, pool),
        cpal::SampleFormat::U8 => build_typed::<u8>(device, config, channels, tx, pool),
        cpal::SampleFormat::I8 => build_typed::<i8>(device, config, channels, tx, pool),
        _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
    }
}

fn build_typed<T>(
    device: &Device,
    config: &cpal::SupportedStreamConfig,
    channels: usize,
    tx: mpsc::SyncSender<Vec<f32>>,
    pool: mpsc::Receiver<Vec<f32>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: Sample + SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    // Overrun accounting (v1.13.3): if the bounded channel is full we drop the
    // frame here in the device callback — never block the audio thread.
    let mut overruns: u64 = 0;
    let cb = move |data: &[T], _: &cpal::InputCallbackInfo| {
        // v1.13.4: reuse a recycled buffer from the worker when one is
        // available; only the first few callbacks ever allocate.
        let mut mono = pool.try_recv().unwrap_or_default();
        mono.clear();
        if channels == 1 {
            mono.extend(data.iter().map(|&s| s.to_sample::<f32>()));
        } else {
            mono.extend(data.chunks_exact(channels).map(|frame| {
                frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
            }));
        }
        match tx.try_send(mono) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                overruns += 1;
                if overruns == 1 || overruns % 512 == 0 {
                    log::warn!(
                        "Meeting capture: writer back-pressure — {overruns} frame(s) dropped so far"
                    );
                }
            }
            // Worker exited (stop or abort); the stream is about to be dropped.
            Err(mpsc::TrySendError::Disconnected(_)) => {}
        }
    };
    device.build_input_stream(
        &config.clone().into(),
        cb,
        |e| log::error!("Meeting capture stream error: {e}"),
        None,
    )
}
