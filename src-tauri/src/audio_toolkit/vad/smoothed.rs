use super::{VadFrame, VoiceActivityDetector};
use anyhow::Result;
use std::collections::VecDeque;

pub struct SmoothedVad {
    inner_vad: Box<dyn VoiceActivityDetector>,
    prefill_frames: usize,
    hangover_frames: usize,
    onset_frames: usize,

    frame_buffer: VecDeque<Vec<f32>>,
    hangover_counter: usize,
    onset_counter: usize,
    in_speech: bool,

    temp_out: Vec<f32>,
}

impl SmoothedVad {
    pub fn new(
        inner_vad: Box<dyn VoiceActivityDetector>,
        prefill_frames: usize,
        hangover_frames: usize,
        onset_frames: usize,
    ) -> Self {
        Self {
            inner_vad,
            prefill_frames,
            hangover_frames,
            onset_frames,
            frame_buffer: VecDeque::new(),
            hangover_counter: 0,
            onset_counter: 0,
            in_speech: false,
            temp_out: Vec::new(),
        }
    }
}

impl VoiceActivityDetector for SmoothedVad {
    fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
        // korero-r07-no-double-emit (v1.30.0): buffer for pre-roll ONLY while we
        // are not already emitting. While in_speech every frame is emitted
        // individually below, so buffering it as well meant the next re-onset
        // handed the decoder the same audio a second time.
        if !self.in_speech {
            self.frame_buffer.push_back(frame.to_vec());
            while self.frame_buffer.len() > self.prefill_frames + 1 {
                self.frame_buffer.pop_front();
            }
        }

        // 2. Delegate to the wrapped boolean VAD
        let is_voice = self.inner_vad.is_voice(frame)?;

        match (self.in_speech, is_voice) {
            // Potential start of speech - need to accumulate onset frames
            (false, true) => {
                self.onset_counter += 1;
                if self.onset_counter >= self.onset_frames {
                    // We have enough consecutive voice frames to trigger speech
                    self.in_speech = true;
                    self.hangover_counter = self.hangover_frames;
                    self.onset_counter = 0; // Reset for next time

                    // Collect prefill + current frame
                    self.temp_out.clear();
                    for buf in &self.frame_buffer {
                        self.temp_out.extend(buf);
                    }
                    // korero-r07-drain-preroll (v1.30.0): these frames have now
                    // been emitted. Holding them lets the NEXT trigger emit them
                    // a second time.
                    self.frame_buffer.clear();
                    Ok(VadFrame::Speech(&self.temp_out))
                } else {
                    // Not enough frames yet, still silence
                    Ok(VadFrame::Noise)
                }
            }

            // Ongoing Speech
            (true, true) => {
                self.hangover_counter = self.hangover_frames;
                Ok(VadFrame::Speech(frame))
            }

            // End of Speech or interruption during onset phase
            (true, false) => {
                if self.hangover_counter > 0 {
                    self.hangover_counter -= 1;
                    Ok(VadFrame::Speech(frame))
                } else {
                    self.in_speech = false;
                    Ok(VadFrame::Noise)
                }
            }

            // Silence or broken onset sequence
            (false, false) => {
                self.onset_counter = 0; // Reset onset counter on silence
                Ok(VadFrame::Noise)
            }
        }
    }

    fn reset(&mut self) {
        self.frame_buffer.clear();
        self.hangover_counter = 0;
        self.onset_counter = 0;
        self.in_speech = false;
        self.temp_out.clear();
    }
}

#[cfg(test)]
mod korero_item6_tests {
    use super::*;

    const FRAME: usize = 480; // 30 ms @ 16 kHz, the real frame size

    /// A VAD whose voice/silence decision is scripted, so the test exercises
    /// SmoothedVad's buffering and nothing else.
    struct ScriptedVad {
        script: Vec<bool>,
        i: usize,
    }
    impl VoiceActivityDetector for ScriptedVad {
        fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
            let voiced = self.script.get(self.i).copied().unwrap_or(false);
            self.i += 1;
            if voiced { Ok(VadFrame::Speech(frame)) } else { Ok(VadFrame::Noise) }
        }
    }

    /// Every frame is filled with its own index, so an emitted sample says
    /// exactly which input frame it came from.
    fn run(script: Vec<bool>) -> (Vec<f32>, usize) {
        let n = script.len();
        let inner = Box::new(ScriptedVad { script, i: 0 });
        let mut vad = SmoothedVad::new(inner, 15, 15, 2);
        let mut emitted: Vec<f32> = Vec::new();
        for idx in 0..n {
            let frame = vec![idx as f32; FRAME];
            if let VadFrame::Speech(out) = vad.push_frame(&frame).unwrap() {
                emitted.extend_from_slice(out);
            }
        }
        (emitted, n * FRAME)
    }

    /// speech -> a pause in the 510-930 ms danger window -> speech.
    /// Pre-v1.30.0 this replayed up to 14 frames of already-emitted audio.
    #[test]
    fn korero_item6_no_frame_is_ever_emitted_twice() {
        let script: Vec<bool> = (0..30).map(|_| true)
            .chain((0..8).map(|_| false))
            .chain((0..30).map(|_| true))
            .collect();
        let (emitted, _) = run(script);

        let mut seen = std::collections::HashMap::new();
        for s in &emitted {
            *seen.entry(s.to_bits()).or_insert(0usize) += 1;
        }
        for (bits, count) in seen {
            assert!(
                count <= FRAME,
                "frame {} was emitted {} times (a frame is {} samples) -- \
                 the VAD handed the decoder the same audio twice",
                f32::from_bits(bits),
                count / FRAME,
                FRAME
            );
        }
    }

    /// The general form: a VAD may drop or delay audio, never manufacture it.
    #[test]
    fn korero_item6_never_emits_more_than_it_was_given() {
        for gap in [2usize, 5, 8, 14, 16, 40] {
            let script: Vec<bool> = (0..25).map(|_| true)
                .chain((0..gap).map(|_| false))
                .chain((0..25).map(|_| true))
                .collect();
            let (emitted, pushed) = run(script);
            assert!(
                emitted.len() <= pushed,
                "gap={gap}: emitted {} samples from {} pushed",
                emitted.len(),
                pushed
            );
        }
    }
}
