//! Kōrero fork (v1.13.6): native WASAPI loopback capture for the "Others"
//! stream.
//!
//! The cpal loopback path (input stream built on an output device) proved
//! unreliable on real hardware — streams built and started but produced no
//! data. This module talks to WASAPI directly via the `wasapi` crate:
//! shared-mode capture on the DEFAULT render device with AUTOCONVERTPCM, so
//! the audio engine hands us 16 kHz mono f32 regardless of the device's mix
//! format. Event-driven with a 200 ms timeout poll — the known quirk where
//! loopback events don't fire without an active render stream degrades to
//! polling instead of stalling, and a silent system still observes the stop
//! flag promptly.
//!
//! Sink behaviour (streaming WAV with periodic header flush, write-error
//! abort + `meeting-capture-error` event, `meeting-level` meter events)
//! intentionally mirrors `meeting_capture::StreamCapture` so the UI cannot
//! tell the backends apart.

#![cfg(windows)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

use crate::audio_toolkit::audio::FrameResampler;
use crate::meeting_capture::{
    build_meeting_vad, LevelEvent, SegmentSender, Segmenter, FLUSH_ERROR_WEIGHT,
    FLUSH_EVERY_SAMPLES, LEVEL_EVERY, MAX_WRITE_ERRORS,
};

const TARGET_RATE: usize = 16_000;

/// Native WASAPI loopback capture of the default output device to a streaming
/// 16 kHz mono WAV. Public surface mirrors `StreamCapture`.
pub struct WasapiLoopback {
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<u64, String>>>,
    pub path: PathBuf,
}

impl WasapiLoopback {
    /// Start capturing the DEFAULT render device (what Windows is playing) to
    /// `path`. Fails fast (within ~5 s) if WASAPI can't be initialised.
    pub fn start(
        path: PathBuf,
        app: Option<AppHandle>,
        source: &'static str,
        segments: Option<SegmentSender>,
        paused: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_for_worker = stop_flag.clone();
        let worker_path = path.clone();
        let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();

        let worker = std::thread::spawn(move || {
            capture_worker(worker_path, app, source, segments, stop_for_worker, paused, init_tx)
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
                stop_flag.store(true, Ordering::Relaxed);
                Err("WASAPI loopback worker did not initialise in time.".to_string())
            }
        }
    }

    /// Stop and finalise the WAV. Returns the number of 16 kHz samples written.
    /// Cannot hang: the worker waits with a 200 ms timeout, so it observes the
    /// stop flag promptly even on a totally silent system.
    pub fn stop(mut self) -> Result<u64, String> {
        self.stop_flag.store(true, Ordering::Relaxed);
        let handle = self
            .worker
            .take()
            .ok_or_else(|| "Capture already stopped.".to_string())?;
        handle
            .join()
            .map_err(|_| "WASAPI capture worker panicked.".to_string())?
    }
}

impl Drop for WasapiLoopback {
    fn drop(&mut self) {
        // Error paths drop without stop(): signal the worker so it exits and
        // finalises the file within ~200 ms. Not joined to keep drops cheap.
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

fn capture_worker(
    path: PathBuf,
    app: Option<AppHandle>,
    source: &'static str,
    segments: Option<SegmentSender>,
    stop_flag: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    init_tx: mpsc::Sender<Result<(), String>>,
) -> Result<u64, String> {
    macro_rules! fail_init {
        ($msg:expr) => {{
            let msg: String = $msg;
            let _ = init_tx.send(Err(msg.clone()));
            return Err(msg);
        }};
    }

    // Fresh worker thread — MTA init is safe here (never do this on a UI thread).
    if let Err(e) = initialize_mta().ok() {
        fail_init!(format!("COM init failed: {e}"));
    }

    let enumerator = match DeviceEnumerator::new() {
        Ok(e) => e,
        Err(e) => fail_init!(format!("Device enumerator: {e}")),
    };
    let device = match enumerator.get_default_device(&Direction::Render) {
        Ok(d) => d,
        Err(e) => fail_init!(format!("No default output device: {e}")),
    };
    let mut audio_client = match device.get_iaudioclient() {
        Ok(c) => c,
        Err(e) => fail_init!(format!("Audio client: {e}")),
    };
    let buffer_duration_hns = audio_client
        .get_device_period()
        .map(|(default, _min)| default)
        .unwrap_or(200_000); // 20 ms

    // Ask the engine for 16 kHz mono f32 directly — AUTOCONVERTPCM does the
    // downmix + resample for us. Fall back to float at the device's own
    // rate/channels (then we resample) if the request is refused.
    let mut rate = TARGET_RATE;
    let mut channels: usize = 1;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns,
    };
    let desired = WaveFormat::new(32, 32, &SampleType::Float, rate, channels, None);
    if let Err(first_err) = audio_client.initialize_client(&desired, &Direction::Capture, &mode) {
        // A failed Initialize leaves the client unusable — take a fresh one.
        audio_client = match device.get_iaudioclient() {
            Ok(c) => c,
            Err(e) => fail_init!(format!("Audio client (retry): {e}")),
        };
        let mix = match audio_client.get_mixformat() {
            Ok(m) => m,
            Err(e) => fail_init!(format!(
                "Loopback init failed ({first_err}); mix format also failed: {e}"
            )),
        };
        rate = mix.get_samplespersec() as usize;
        channels = (mix.get_nchannels() as usize).max(1);
        let fallback = WaveFormat::new(32, 32, &SampleType::Float, rate, channels, None);
        if let Err(e) = audio_client.initialize_client(&fallback, &Direction::Capture, &mode) {
            fail_init!(format!(
                "WASAPI loopback init failed: {first_err}; fallback also failed: {e}"
            ));
        }
    }

    let h_event = match audio_client.set_get_eventhandle() {
        Ok(h) => h,
        Err(e) => fail_init!(format!("Event handle: {e}")),
    };
    let capture_client = match audio_client.get_audiocaptureclient() {
        Ok(c) => c,
        Err(e) => fail_init!(format!("Capture client: {e}")),
    };
    if let Err(e) = audio_client.start_stream() {
        fail_init!(format!("Failed to start loopback stream: {e}"));
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = match hound::WavWriter::create(&path, spec) {
        Ok(w) => w,
        Err(e) => fail_init!(format!("Failed to create {}: {e}", path.display())),
    };
    let _ = init_tx.send(Ok(()));

    // Pass-through when the engine already converts to 16 kHz mono.
    let mut resampler = FrameResampler::new(rate, TARGET_RATE, Duration::from_millis(30));
    let bytes_per_frame = channels * 4; // interleaved f32
    let mut deque: VecDeque<u8> = VecDeque::new();
    let mut mono: Vec<f32> = Vec::with_capacity(8_192);
    let mut written: u64 = 0;
    let mut last_flush: u64 = 0;
    let mut write_errors: u64 = 0;
    let mut abort_err: Option<String> = None;
    let mut window_peak: f32 = 0.0;
    let mut last_emit = Instant::now();
    // Phase B (v1.14.0): optional live segmenter. v1.19.0: Silero-gated, same
    // bundled model as the dictation path (built on this worker thread).
    let mut segmenter =
        segments.map(|tx| Segmenter::new(source, tx, build_meeting_vad(app.as_ref(), source)));

    // Drains `deque` into the WAV via the resampler; shared by the main loop
    // and the stop path.
    macro_rules! drain_deque {
        () => {{
            let frames = deque.len() / bytes_per_frame;
            if frames > 0 {
                mono.clear();
                for _ in 0..frames {
                    let mut acc = 0.0f32;
                    for _ in 0..channels {
                        let b = [
                            deque.pop_front().unwrap_or(0),
                            deque.pop_front().unwrap_or(0),
                            deque.pop_front().unwrap_or(0),
                            deque.pop_front().unwrap_or(0),
                        ];
                        acc += f32::from_le_bytes(b);
                    }
                    let s = acc / channels as f32;
                    let a = s.abs();
                    if a > window_peak {
                        window_peak = a;
                    }
                    mono.push(s);
                }
                resampler.push(&mono, |frame: &[f32]| {
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
        }};
    }

    loop {
        // Event-driven with a timeout poll: timeouts are normal during
        // silence AND cover the loopback-events quirk — the read below is
        // harmless when no packets are pending.
        let _ = h_event.wait_for_event(200);
        if let Err(e) = capture_client.read_from_device_to_deque(&mut deque) {
            abort_err = Some(format!("System loopback read failed: {e}"));
            break;
        }
        // v1.19.0: while paused, still READ from the device (so the OS buffer
        // never overflows) but discard the bytes — no WAV write, no segment, no
        // meter movement. The stream stays open; the WAV is gap-continuous.
        if paused.load(Ordering::Relaxed) {
            deque.clear();
        } else {
            drain_deque!();
        }

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
                path.display()
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

        if stop_flag.load(Ordering::Relaxed) {
            // Final read + resampler tail, then finalise. Discard the final read
            // if we were stopped mid-pause; the resampler tail still flushes.
            let _ = capture_client.read_from_device_to_deque(&mut deque);
            if paused.load(Ordering::Relaxed) {
                deque.clear();
            } else {
                drain_deque!();
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

    let _ = audio_client.stop_stream();

    // Phase B: ship any final partial segment, then drop the sender so the
    // live-transcription consumer can drain and exit.
    if let Some(seg) = segmenter.as_mut() {
        seg.finish();
    }
    drop(segmenter);

    if let Some(msg) = abort_err {
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
}
