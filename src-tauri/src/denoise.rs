//! Kōrero (v1.11.0): optional microphone denoiser built on RNNoise via the
//! pure-Rust `nnnoiseless` crate.
//!
//! WHY THIS SHAPE:
//! - RNNoise operates ONLY at 48 kHz on fixed 480-sample (10 ms) mono frames,
//!   and expects f32 samples in i16 range (approx -32768..32767), NOT the
//!   normalised -1..1 that cpal/our pipeline use. We scale on the way in/out.
//! - Kōrero's recorder resamples to 16 kHz *before* VAD/transcription, so the
//!   denoiser MUST sit on the raw 48 kHz stream, before `FrameResampler`. It is
//!   therefore only usable when the capture device runs at 48 kHz.
//! - cpal chunks are arbitrary length, so we buffer until we have >=480 samples,
//!   emit denoised complete frames, and keep the remainder for the next call.
//!
//! Enablement is a process-global atomic so the recorder consumer thread can
//! read it without threading a setting through the whole capture-init chain.
//! Set it from settings on startup and whenever the toggle changes.

use std::sync::atomic::{AtomicBool, Ordering};

/// Process-global on/off. Default OFF — denoising is opt-in because front-end
/// noise suppression can REDUCE ASR accuracy on already noise-robust models.
static DENOISE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    DENOISE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    DENOISE_ENABLED.load(Ordering::Relaxed)
}

/// The sample rate RNNoise requires. The recorder only constructs a `Denoiser`
/// when the device capture rate equals this.
pub const REQUIRED_SAMPLE_RATE: u32 = 48_000;

/// Streaming RNNoise denoiser. One per recording session/consumer.
pub struct Denoiser {
    state: Box<nnnoiseless::DenoiseState<'static>>,
    /// Leftover input samples (< one frame) carried between `process` calls.
    carry: Vec<f32>,
}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            state: nnnoiseless::DenoiseState::new(),
            carry: Vec::with_capacity(nnnoiseless::DenoiseState::FRAME_SIZE * 2),
        }
    }

    /// Denoise a chunk of 48 kHz mono f32 samples (normalised ~ -1..1).
    /// Returns the denoised samples for every COMPLETE 480-sample frame now
    /// available; any tail shorter than a frame is buffered for the next call.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        const FRAME: usize = nnnoiseless::DenoiseState::FRAME_SIZE; // 480
        self.carry.extend_from_slice(input);

        let full_frames = self.carry.len() / FRAME;
        if full_frames == 0 {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(full_frames * FRAME);
        let mut frame_in = [0.0f32; FRAME];
        let mut frame_out = [0.0f32; FRAME];

        for f in 0..full_frames {
            let base = f * FRAME;
            // scale normalised -> i16 range for RNNoise
            for i in 0..FRAME {
                frame_in[i] = self.carry[base + i] * 32768.0;
            }
            let _vad_prob = self.state.process_frame(&mut frame_out, &frame_in);
            // scale back to normalised
            for i in 0..FRAME {
                out.push(frame_out[i] / 32768.0);
            }
        }

        // keep the remainder (< FRAME) for next time
        self.carry.drain(0..full_frames * FRAME);
        out
    }

    /// Flush any buffered tail at end-of-recording, zero-padding the final frame.
    pub fn flush(&mut self) -> Vec<f32> {
        const FRAME: usize = nnnoiseless::DenoiseState::FRAME_SIZE;
        if self.carry.is_empty() {
            return Vec::new();
        }
        while self.carry.len() % FRAME != 0 {
            self.carry.push(0.0);
        }
        let buffered: Vec<f32> = std::mem::take(&mut self.carry);
        let mut out = Vec::with_capacity(buffered.len());
        let mut frame_in = [0.0f32; FRAME];
        let mut frame_out = [0.0f32; FRAME];
        for chunk in buffered.chunks_exact(FRAME) {
            for i in 0..FRAME {
                frame_in[i] = chunk[i] * 32768.0;
            }
            self.state.process_frame(&mut frame_out, &frame_in);
            for i in 0..FRAME {
                out.push(frame_out[i] / 32768.0);
            }
        }
        out
    }
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_partial_frames_and_emits_complete_ones() {
        let mut d = Denoiser::new();
        // 500 samples = one 480 frame + 20 carried over
        let out = d.process(&vec![0.01f32; 500]);
        assert_eq!(out.len(), nnnoiseless::DenoiseState::FRAME_SIZE);
        // next 460 completes a second frame (20 + 460 = 480)
        let out2 = d.process(&vec![0.01f32; 460]);
        assert_eq!(out2.len(), nnnoiseless::DenoiseState::FRAME_SIZE);
    }

    #[test]
    fn enable_flag_roundtrips() {
        set_enabled(true);
        assert!(is_enabled());
        set_enabled(false);
        assert!(!is_enabled());
    }
}
