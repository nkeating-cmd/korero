//! Kōrero 1.45 (D-007): in-room speaker separation ("who spoke when").
//!
//! Runs after a meeting, on the CPU, on the microphone recording:
//!   1. pyannote segmentation-3.0 (MIT) on 10 s windows. Each frame's powerset class
//!      is decoded into up to 3 window-local speakers, so a change of speaker with no
//!      pause still splits a segment. Timing is re-anchored every window.
//!   2. WeSpeaker ResNet34-LM (CC BY 4.0, VoxCeleb) embeds each local segment, from
//!      80-bin Kaldi filterbanks at 16-bit scale (the model's metadata says
//!      `normalize_samples: 0`), preferring frames where only that speaker talks.
//!   3. Whole-meeting clustering: average-linkage cosine on segments >= 1.5 s, clusters
//!      with under 20 s of speech folded into the nearest kept one, and every other
//!      segment attached to the nearest speaker by voice (or by time if too short).
//!
//! Measured in M0 (16 Oct 2026) on four AMI far-field test meetings: mean DER 23.1%
//! (0.25 s collar), 28.1% strict; about 80 s of CPU per hour of audio on 4 threads.
//! The packaged CAM++ model from `pyannote-rs` separated speakers at chance and must
//! not be used (LESSONS L-012). `diar_ami_der` below guards these numbers.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ndarray::Array3;
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::TensorRef,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use crate::audio_toolkit::audio::FrameResampler;

pub(crate) const SR: usize = 16_000;
const WIN: usize = SR * 10; // segmentation window
const STEP: usize = 270; // samples per segmentation frame (589 frames per window)
const F0: usize = 721; // first frame offset within a window
/// segmentation-3.0 powerset: 3 local speakers, at most 2 at once.
const POWERSET: [&[u8]; 7] = [&[], &[0], &[1], &[2], &[0, 1], &[0, 2], &[1, 2]];
const MIN_SOLE: usize = SR / 2; // use sole-speaker frames for the embedding if >= 0.5 s
const MIN_EMBED: usize = SR / 4; // below 0.25 s, no embedding: attached by time
const CLUSTER_MIN_S: f64 = 1.5; // only these segments shape the clusters
const SPEAKER_MIN_S: f64 = 20.0; // a cluster needs this much speech to stay a speaker
const MAX_CLUSTERED: usize = 700; // longest N segments clustered; the rest attach by voice
/// Cosine distance at which clusters stop merging (M0 leave-one-out: 0.55-0.60).
pub(crate) const THRESHOLD: f32 = 0.6;
const PCM_SCALE: f32 = 32_768.0;

/// A model file Kōrero downloads on first use and checks by size and sha256.
pub(crate) struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

pub(crate) const SEGMENTATION: ModelFile = ModelFile {
    name: "segmentation-3.0.onnx",
    url: "https://github.com/thewh1teagle/pyannote-rs/releases/download/v0.1.0/segmentation-3.0.onnx",
    sha256: "b78fc48113bb46fd247ae6a9aea737079550c647638db961df7e0e1e9f4ba62e",
    bytes: 5_983_836,
};

pub(crate) const EMBEDDING: ModelFile = ModelFile {
    name: "wespeaker_en_voxceleb_resnet34_LM.onnx",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx",
    sha256: "e9848563da86f263117134dfd7ad63c92355b37de492b55e325400c9d9c39012",
    bytes: 26_530_550,
};

// ---------------------------------------------------------------------------
// Models: downloaded on first use, never trusted without size and sha256.

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("{:x}", h.finalize()))
}

/// True when `path` holds exactly this model (size first, then the hash).
pub(crate) fn model_ok(path: &Path, m: &ModelFile) -> bool {
    path.metadata().map(|md| md.len() == m.bytes).unwrap_or(false)
        && sha256_file(path).map(|h| h == m.sha256).unwrap_or(false)
}

/// Download one model into `dir` unless a verified copy is already there. Writes to
/// `<name>.partial` and renames only after the size and hash match, so a cut-off or
/// substituted download never becomes a model.
pub(crate) async fn ensure_model(
    dir: &Path,
    m: &ModelFile,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    let path = dir.join(m.name);
    if model_ok(&path, m) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let partial = dir.join(format!("{}.partial", m.name));
    let _ = std::fs::remove_file(&partial);
    // Review M3: a stalled connection must not hold the one job slot until restart.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Could not start the download of {}: {e}", m.name))?;
    let fetched = async {
        let resp = client
            .get(m.url)
            .send()
            .await
            .map_err(|e| format!("Could not download {}: {e}", m.name))?;
        if !resp.status().is_success() {
            return Err(format!("Could not download {}: HTTP {}", m.name, resp.status()));
        }
        let mut file = std::fs::File::create(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
        let mut hasher = Sha256::new();
        let (mut done, mut last_pct, mut stream) = (0u64, u64::MAX, resp.bytes_stream());
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download of {} failed: {e}", m.name))?;
            done += chunk.len() as u64;
            if done > m.bytes {
                return Err(format!("{} is larger than expected; download refused.", m.name));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).map_err(|e| format!("{}: {e}", partial.display()))?;
            // Review L12: one event per percent, not one per network chunk.
            let pct = done * 100 / m.bytes.max(1);
            if pct != last_pct {
                last_pct = pct;
                progress(done, m.bytes);
            }
        }
        file.flush().map_err(|e| format!("{}: {e}", partial.display()))?;
        let got = format!("{:x}", hasher.finalize());
        if done != m.bytes || got != m.sha256 {
            return Err(format!(
                "{} did not match its pinned size and checksum; it was discarded.",
                m.name
            ));
        }
        Ok(())
    }
    .await;
    // Review L6: never leave a partial file behind, whatever failed.
    if let Err(err) = fetched {
        let _ = std::fs::remove_file(&partial);
        return Err(err);
    }
    std::fs::rename(&partial, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Audio: any meeting WAV (device rate, any channels, int or float) to 16 kHz mono.

pub(crate) fn decode_wav_16k(path: &str) -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("Could not open {path}: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let mut resampler = FrameResampler::new(spec.sample_rate as usize, SR, Duration::from_millis(30));
    let mut out: Vec<f32> = Vec::new();
    let mut interleave: Vec<f32> = Vec::with_capacity(channels);
    let mut mono: Vec<f32> = Vec::with_capacity(8_192);
    let mut push = |s: f32, out: &mut Vec<f32>, resampler: &mut FrameResampler| {
        interleave.push(s);
        if interleave.len() == channels {
            mono.push(interleave.iter().sum::<f32>() / channels as f32);
            interleave.clear();
        }
        if mono.len() >= 8_192 {
            resampler.push(&mono, |f| out.extend_from_slice(f));
            mono.clear();
        }
    };
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.into_samples::<f32>() {
                push(s.map_err(|e| format!("WAV read error: {e}"))?, &mut out, &mut resampler);
            }
        }
        hound::SampleFormat::Int => {
            let denom = (1i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            for s in reader.into_samples::<i32>() {
                let v = s.map_err(|e| format!("WAV read error: {e}"))? as f32 / denom;
                push(v, &mut out, &mut resampler);
            }
        }
    }
    resampler.push(&mono, |f| out.extend_from_slice(f));
    resampler.finish(|f| out.extend_from_slice(f));
    Ok(out)
}

// ---------------------------------------------------------------------------
// Pure steps (unit-tested without models).

/// One window-local speaker run, in seconds from the start of the file, with the sample
/// ranges used for its embedding.
#[derive(Debug, Clone)]
pub(crate) struct LocalSeg {
    pub start: f64,
    pub end: f64,
    pub audio: Vec<(usize, usize)>,
}

/// Turn per-window powerset classes into local speaker runs. `classes[w][f]` is the
/// argmax class of frame `f` in window `w`; `n` is the number of samples in the file.
pub(crate) fn local_segments(classes: &[Vec<u8>], n: usize) -> Vec<LocalSeg> {
    let mut out = Vec::new();
    let secs = |s: usize| s.min(n) as f64 / SR as f64;
    for (w, cls) in classes.iter().enumerate() {
        let at = |f: usize| w * WIN + F0 + f * STEP; // re-anchored every window
        for k in 0..3u8 {
            let active = |f: usize| POWERSET[cls[f] as usize].contains(&k);
            let mut i = 0;
            while i < cls.len() {
                if !active(i) {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < cls.len() && active(j) {
                    j += 1;
                }
                if at(i) < n {
                    let sole: Vec<(usize, usize)> = (i..j)
                        .filter(|&f| POWERSET[cls[f] as usize] == [k])
                        .map(|f| (at(f).min(n), (at(f) + STEP).min(n)))
                        .collect();
                    let sole_len: usize = sole.iter().map(|(a, b)| b - a).sum();
                    let audio = if sole_len >= MIN_SOLE {
                        sole
                    } else {
                        vec![(at(i).min(n), at(j).min(n))]
                    };
                    out.push(LocalSeg { start: secs(at(i)), end: secs(at(j)), audio });
                }
                i = j;
            }
        }
    }
    out
}

fn unit(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter().map(|x| x / norm).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Average-linkage agglomerative clustering on unit vectors with cosine distance,
/// stopping when the closest pair of clusters is further apart than `threshold`
/// (the same cut as scipy's `fcluster(linkage(X, "average", "cosine"), t, "distance")`).
pub(crate) fn ahc_average(x: &[Vec<f32>], threshold: f32) -> Vec<usize> {
    let n = x.len();
    let mut d = vec![0f32; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            let v = 1.0 - dot(&x[i], &x[j]);
            d[i * n + j] = v;
            d[j * n + i] = v;
        }
    }
    let mut size = vec![1usize; n];
    let mut alive = vec![true; n];
    let mut label: Vec<usize> = (0..n).collect();
    loop {
        let mut best = (f32::INFINITY, 0, 0);
        for i in 0..n {
            if !alive[i] {
                continue;
            }
            for j in (i + 1)..n {
                if alive[j] && d[i * n + j] < best.0 {
                    best = (d[i * n + j], i, j);
                }
            }
        }
        let (dist, a, b) = best;
        if !dist.is_finite() || dist > threshold {
            break;
        }
        // Lance-Williams update for average linkage.
        let (sa, sb) = (size[a] as f32, size[b] as f32);
        for k in 0..n {
            if alive[k] && k != a && k != b {
                let v = (sa * d[a * n + k] + sb * d[b * n + k]) / (sa + sb);
                d[a * n + k] = v;
                d[k * n + a] = v;
            }
        }
        size[a] += size[b];
        alive[b] = false;
        for l in label.iter_mut() {
            if *l == b {
                *l = a;
            }
        }
    }
    label
}

/// Robust whole-meeting clustering (M0 "row 5"). `segs` are (start s, end s, embedding).
/// Returns one speaker index per segment, numbered 0.. in order of first speech, or
/// `None` only when nothing in the meeting could be embedded.
pub(crate) fn cluster(segs: &[(f64, f64, Option<Vec<f32>>)], threshold: f32) -> Vec<Option<usize>> {
    let dur = |i: usize| segs[i].1 - segs[i].0;
    let mut long: Vec<usize> = (0..segs.len())
        .filter(|&i| segs[i].2.is_some() && dur(i) >= CLUSTER_MIN_S)
        .collect();
    if long.len() > MAX_CLUSTERED {
        long.sort_by(|&a, &b| dur(b).total_cmp(&dur(a)));
        long.truncate(MAX_CLUSTERED);
        long.sort_unstable();
    }
    let embedded: Vec<usize> = (0..segs.len()).filter(|&i| segs[i].2.is_some()).collect();
    let mut labels: Vec<Option<usize>> = vec![None; segs.len()];
    if long.len() < 2 {
        // Too little speech to tell voices apart: everyone embedded is one speaker.
        for &i in &embedded {
            labels[i] = Some(0);
        }
    } else {
        let x: Vec<Vec<f32>> = long.iter().map(|&i| unit(segs[i].2.as_ref().unwrap())).collect();
        let lab = ahc_average(&x, threshold);
        let mut total: std::collections::BTreeMap<usize, f64> = Default::default();
        for (p, &i) in long.iter().enumerate() {
            *total.entry(lab[p]).or_default() += dur(i);
        }
        let mut keep: Vec<usize> = total.iter().filter(|(_, &t)| t >= SPEAKER_MIN_S).map(|(&c, _)| c).collect();
        if keep.is_empty() {
            keep.push(*total.iter().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0);
        }
        let cents: Vec<Vec<f32>> = keep
            .iter()
            .map(|&c| {
                let mut m = vec![0f32; x[0].len()];
                for (p, v) in x.iter().enumerate() {
                    if lab[p] == c {
                        m.iter_mut().zip(v).for_each(|(a, b)| *a += b);
                    }
                }
                unit(&m)
            })
            .collect();
        for &i in &embedded {
            let v = unit(segs[i].2.as_ref().unwrap());
            let best = (0..cents.len())
                .max_by(|&a, &b| dot(&v, &cents[a]).total_cmp(&dot(&v, &cents[b])))
                .unwrap();
            labels[i] = Some(best);
        }
    }
    // Too short to embed: take the label of the nearest embedded segment in time.
    let mids: Vec<(f64, usize)> = (0..segs.len())
        .filter_map(|i| labels[i].map(|l| ((segs[i].0 + segs[i].1) / 2.0, l)))
        .collect();
    if !mids.is_empty() {
        for i in 0..segs.len() {
            if labels[i].is_none() {
                let c = (segs[i].0 + segs[i].1) / 2.0;
                let near = mids.iter().min_by(|a, b| (a.0 - c).abs().total_cmp(&(b.0 - c).abs())).unwrap();
                labels[i] = Some(near.1);
            }
        }
    }
    // Renumber 0.. in order of first speech, so "Speaker 1" is whoever spoke first.
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| segs[a].0.total_cmp(&segs[b].0));
    let mut map: std::collections::HashMap<usize, usize> = Default::default();
    for &i in &order {
        if let Some(l) = labels[i] {
            let next = map.len();
            map.entry(l).or_insert(next);
        }
    }
    labels.iter().map(|l| l.map(|l| map[&l])).collect()
}

/// A stretch of one person speaking, in milliseconds from the start of the recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SpeakerTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    /// 0-based; shown to the user as "Speaker {speaker + 1}".
    pub speaker: u32,
}

/// Labelled segments to turns: sorted by start; a segment joins the previous turn only
/// when that turn is the same speaker's and they nearly touch (within 100 ms: consecutive
/// windows leave a 61 ms gap, because 589 frames x 270 samples fall short of 10 s).
/// Pauses longer than that are not filled. Turns of different speakers may overlap.
pub(crate) fn to_turns(segs: &[(f64, f64)], labels: &[Option<usize>]) -> Vec<SpeakerTurn> {
    let mut v: Vec<(f64, f64, usize)> = segs
        .iter()
        .zip(labels)
        .filter_map(|(&(s, e), l)| l.map(|l| (s, e, l)))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<SpeakerTurn> = Vec::new();
    for (s, e, l) in v {
        let (s_ms, e_ms) = ((s * 1000.0).round() as u64, (e * 1000.0).round() as u64);
        if let Some(prev) = out.last_mut() {
            if prev.speaker == l as u32 && s_ms <= prev.end_ms + 100 {
                prev.end_ms = prev.end_ms.max(e_ms);
                continue;
            }
        }
        out.push(SpeakerTurn { start_ms: s_ms, end_ms: e_ms, speaker: l as u32 });
    }
    out.sort_by_key(|t| (t.start_ms, t.speaker));
    out
}

/// Give each microphone transcript segment the speaker it overlaps most.
///
/// Live segments carry a start time but no end, so the end is estimated from the word
/// count (2.5 words a second, at least 1 s) and capped at the next segment's start.
/// A segment touching no turn takes the nearest turn within 2 s; otherwise `None`.
/// Lines where the speaker changes mid-sentence get the speaker who said most of it;
/// splitting them with word timestamps is a later step (plan M2, "straddle split").
pub(crate) fn assign(starts_ms: &[u64], words: &[usize], turns: &[SpeakerTurn]) -> Vec<Option<u32>> {
    let mut order: Vec<usize> = (0..starts_ms.len()).collect();
    order.sort_by_key(|&i| starts_ms[i]);
    let mut ends = vec![0u64; starts_ms.len()];
    for (pos, &i) in order.iter().enumerate() {
        let est = ((words[i] as f64 / 2.5).max(1.0) * 1000.0) as u64;
        let next = order.get(pos + 1).map(|&j| starts_ms[j]).unwrap_or(u64::MAX);
        ends[i] = (starts_ms[i] + est).min(next.max(starts_ms[i] + 1));
    }
    (0..starts_ms.len())
        .map(|i| {
            let (s, e) = (starts_ms[i], ends[i]);
            let mut overlap: std::collections::BTreeMap<u32, u64> = Default::default();
            for t in turns {
                let o = e.min(t.end_ms).saturating_sub(s.max(t.start_ms));
                if o > 0 {
                    *overlap.entry(t.speaker).or_default() += o;
                }
            }
            if let Some((&spk, _)) = overlap.iter().max_by_key(|(_, &o)| o) {
                return Some(spk);
            }
            turns
                .iter()
                .map(|t| (t.start_ms.saturating_sub(e).max(s.saturating_sub(t.end_ms)), t.speaker))
                .min()
                .filter(|(gap, _)| *gap <= 2_000)
                .map(|(_, spk)| spk)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The engine (two ONNX sessions on Kōrero's own `ort`, CPU only).

pub(crate) struct Engine {
    seg: Session,
    emb: Session,
}

fn e<X: std::fmt::Display>(x: X) -> String {
    x.to_string()
}

fn session(path: &Path, threads: usize) -> Result<Session, String> {
    Session::builder()
        .map_err(e)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(e)?
        .with_intra_threads(threads)
        .map_err(e)?
        .commit_from_file(path)
        .map_err(|err| format!("Could not load {}: {err}", path.display()))
}

/// The outcome of one run.
pub(crate) struct Diarisation {
    pub turns: Vec<SpeakerTurn>,
    pub speakers: u32,
}

impl Engine {
    pub fn load(seg_path: &Path, emb_path: &Path, threads: usize) -> Result<Self, String> {
        Ok(Self { seg: session(seg_path, threads)?, emb: session(emb_path, threads)? })
    }

    fn frame_classes(&mut self, x: &[f32]) -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::with_capacity(x.len() / WIN + 1);
        for start in (0..x.len()).step_by(WIN) {
            let mut buf = vec![0f32; WIN];
            let end = (start + WIN).min(x.len());
            buf[..end - start].copy_from_slice(&x[start..end]);
            let arr = Array3::from_shape_vec((1, 1, WIN), buf).map_err(e)?;
            let outs = self
                .seg
                .run(ort::inputs![TensorRef::from_array_view(arr.view()).map_err(e)?])
                .map_err(e)?;
            let v = outs.get("output").ok_or("segmentation model has no 'output'")?;
            let (shape, data) = v.try_extract_tensor::<f32>().map_err(e)?;
            let (f, c) = (shape[1] as usize, shape[2] as usize);
            if c != POWERSET.len() {
                return Err(format!("segmentation model gave {c} classes, expected 7"));
            }
            out.push(
                (0..f)
                    .map(|i| {
                        let row = &data[i * c..(i + 1) * c];
                        (0..c).max_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap() as u8
                    })
                    .collect(),
            );
        }
        Ok(out)
    }

    fn embed(&mut self, x: &[f32], audio: &[(usize, usize)]) -> Result<Option<Vec<f32>>, String> {
        let mut pcm: Vec<f32> = Vec::new();
        for &(a, b) in audio {
            pcm.extend(x[a..b].iter().map(|s| s * PCM_SCALE));
        }
        if pcm.len() < MIN_EMBED {
            return Ok(None);
        }
        let fb = knf_rs::compute_fbank(&pcm).map_err(e)?; // mean-normalised, 80 bins
        let (rows, cols) = fb.dim();
        let feats = Array3::from_shape_vec((1, rows, cols), fb.iter().copied().collect()).map_err(e)?;
        let outs = self
            .emb
            .run(ort::inputs!["feats" => TensorRef::from_array_view(feats.view()).map_err(e)?])
            .map_err(e)?;
        let v = outs.get("embs").ok_or("embedding model has no 'embs'")?;
        let (_, data) = v.try_extract_tensor::<f32>().map_err(e)?;
        // Review L4: one NaN would make its cluster's centroid NaN and pull every line to it.
        if data.iter().any(|x| !x.is_finite()) {
            return Ok(None);
        }
        Ok(Some(data.to_vec()))
    }

    /// Separate speakers in 16 kHz mono audio. `progress(done, total)` counts segments.
    pub fn run(&mut self, x: &[f32], mut progress: impl FnMut(usize, usize)) -> Result<Diarisation, String> {
        let classes = self.frame_classes(x)?;
        let locals = local_segments(&classes, x.len());
        let mut segs: Vec<(f64, f64, Option<Vec<f32>>)> = Vec::with_capacity(locals.len());
        for (i, l) in locals.iter().enumerate() {
            segs.push((l.start, l.end, self.embed(x, &l.audio)?));
            progress(i + 1, locals.len());
        }
        let labels = cluster(&segs, THRESHOLD);
        let spans: Vec<(f64, f64)> = segs.iter().map(|s| (s.0, s.1)).collect();
        let turns = to_turns(&spans, &labels);
        let speakers = turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0);
        Ok(Diarisation { turns, speakers })
    }
}

/// Threads for the engine: half the machine, between 1 and 4 (M0 measured 4).
pub(crate) fn engine_threads() -> usize {
    std::thread::available_parallelism().map(|n| (n.get() / 2).clamp(1, 4)).unwrap_or(2)
}


// ---------------------------------------------------------------------------
// The command.

/// Why a transcript's lines cannot be matched by time to its microphone recording, if
/// they cannot (review M1, M2). `None` means it is safe to separate.
pub(crate) fn timing_problem(segs: &[crate::meeting::TranscriptSeg]) -> Option<&'static str> {
    let mic: Vec<u64> = segs
        .iter()
        .filter(|s| s.source == "you" && !s.text.trim().is_empty())
        .map(|s| s.start_ms)
        .collect();
    if mic.is_empty() {
        return Some("This meeting has no microphone lines to label.");
    }
    if mic.len() >= 2 && mic.iter().all(|&ms| ms == mic[0]) {
        return Some(
            "This meeting's lines have no timings (it was recorded before Kōrero 1.26), so they \
             can't be matched to speakers. Re-transcribe it first.",
        );
    }
    if segs.windows(2).any(|w| w[1].start_ms < w[0].start_ms) {
        return Some(
            "This transcript is not in time order (a combined meeting, for example), so its \
             lines can't be matched to one recording.",
        );
    }
    None
}

/// One separation at a time (review L8): two runs would write the same model files.
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct RunGuard;

impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Result of "Separate speakers" on a meeting.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SpeakerSeparation {
    pub speakers: u32,
    pub turns: Vec<SpeakerTurn>,
    /// One entry per input segment, in input order: the speaker of each microphone
    /// ("you") line, `null` for system-audio lines and lines no turn reached.
    pub segment_speakers: Vec<Option<u32>>,
    pub audio_s: f64,
    pub took_s: f64,
}

#[derive(Clone, Serialize)]
struct Progress {
    /// "download" (models, first use) or "separate"
    stage: &'static str,
    done: u64,
    total: u64,
}

fn emit_progress(app: &tauri::AppHandle, stage: &'static str, done: u64, total: u64) {
    use tauri::Emitter;
    let _ = app.emit("meeting-speakers-progress", Progress { stage, done, total });
}

/// Kōrero 1.45 (D-007): separate the speakers in a meeting's microphone recording and
/// say which speaker each microphone line belongs to. Downloads the two models on first
/// use (about 32 MB, checked by sha256). Runs on the CPU; nothing leaves the computer
/// except those two downloads.
#[tauri::command]
#[specta::specta]
pub async fn meeting_separate_speakers(
    app: tauri::AppHandle,
    mic_path: String,
    segments: Vec<crate::meeting::TranscriptSeg>,
) -> Result<SpeakerSeparation, String> {
    if let Some(why) = timing_problem(&segments) {
        return Err(why.to_string());
    }
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("Speakers are already being separated. Try again when that finishes.".into());
    }
    let _running = RunGuard;
    let started = Instant::now();
    let dir = crate::portable::models_dir(&app)
        .map_err(|err| format!("Could not find the models folder: {err}"))?
        .join("diarise");
    let a = app.clone();
    let seg_path = ensure_model(&dir, &SEGMENTATION, |d, t| emit_progress(&a, "download", d, t)).await?;
    let emb_path = ensure_model(&dir, &EMBEDDING, |d, t| emit_progress(&a, "download", d, t)).await?;
    let a = app.clone();
    let (diar, audio_s) = tauri::async_runtime::spawn_blocking(move || -> Result<(Diarisation, f64), String> {
        emit_progress(&a, "separate", 0, 1);
        let x = decode_wav_16k(&mic_path)?;
        if x.len() < SR * 5 {
            return Err("The recording is too short to separate speakers.".into());
        }
        let mut engine = Engine::load(&seg_path, &emb_path, engine_threads())?;
        let d = engine.run(&x, |done, total| emit_progress(&a, "separate", done as u64, total as u64))?;
        Ok((d, x.len() as f64 / SR as f64))
    })
    .await
    .map_err(|err| format!("Speaker separation stopped unexpectedly: {err}"))??;

    let mic: Vec<usize> = (0..segments.len()).filter(|&i| segments[i].source == "you").collect();
    let starts: Vec<u64> = mic.iter().map(|&i| segments[i].start_ms).collect();
    let words: Vec<usize> = mic.iter().map(|&i| segments[i].text.split_whitespace().count()).collect();
    let mut segment_speakers = vec![None; segments.len()];
    for (k, spk) in assign(&starts, &words, &diar.turns).into_iter().enumerate() {
        segment_speakers[mic[k]] = spk;
    }
    log::info!(
        "speakers: {} found in {:.0} s of audio, {} turns, {:.1} s",
        diar.speakers,
        audio_s,
        diar.turns.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(SpeakerSeparation {
        speakers: diar.speakers,
        turns: diar.turns,
        segment_speakers,
        audio_s,
        took_s: started.elapsed().as_secs_f64(),
    })
}

// ---------------------------------------------------------------------------
// Scoring (used by the AMI regression check; kept here so the check and the engine
// cannot drift apart).

/// Diarisation error rate on 10 ms frames, overlap scored, with `collar` seconds
/// removed around reference boundaries (half each side, as pyannote.metrics does).
/// `reference` is (start s, end s, speaker id). Returns (der, missed, false alarm,
/// confusion), each as a share of reference speech.
#[cfg(test)]
pub(crate) fn der(reference: &[(f64, f64, String)], hyp: &[SpeakerTurn], collar: f64) -> (f64, f64, f64, f64) {
    let fr = |t: f64| (t * 100.0).round().max(0.0) as usize;
    let ids: Vec<&String> = {
        let mut v: Vec<&String> = reference.iter().map(|r| &r.2).collect();
        v.sort();
        v.dedup();
        v
    };
    let nh = hyp.iter().map(|t| t.speaker as usize + 1).max().unwrap_or(0);
    let end = reference
        .iter()
        .map(|r| fr(r.1))
        .chain(hyp.iter().map(|t| fr(t.end_ms as f64 / 1000.0)))
        .max()
        .unwrap_or(0)
        + 1;
    let mut r = vec![0u64; end];
    let mut h = vec![0u64; end];
    let mut scored = vec![true; end];
    for (s, e, id) in reference {
        let k = ids.iter().position(|x| *x == id).unwrap();
        for f in fr(*s)..fr(*e) {
            r[f] |= 1 << k;
        }
        let c = fr(collar / 2.0);
        for b in [fr(*s), fr(*e)] {
            for f in b.saturating_sub(c)..(b + c).min(end) {
                scored[f] = false;
            }
        }
    }
    for t in hyp {
        for f in fr(t.start_ms as f64 / 1000.0)..fr(t.end_ms as f64 / 1000.0) {
            h[f] |= 1 << t.speaker;
        }
    }
    // Overlap matrix, then the reference-to-hypothesis mapping that maximises it.
    let nr = ids.len();
    let mut ov = vec![vec![0u64; nh]; nr];
    for f in 0..end {
        if !scored[f] {
            continue;
        }
        for (i, row) in ov.iter_mut().enumerate() {
            if r[f] >> i & 1 == 1 {
                for (j, cell) in row.iter_mut().enumerate() {
                    if h[f] >> j & 1 == 1 {
                        *cell += 1;
                    }
                }
            }
        }
    }
    fn best(i: usize, used: u64, ov: &[Vec<u64>], map: &mut Vec<Option<usize>>, cur: &mut Vec<Option<usize>>, acc: u64, top: &mut u64) {
        if i == ov.len() {
            if acc > *top || map.is_empty() {
                *top = acc;
                *map = cur.clone();
            }
            return;
        }
        cur.push(None);
        best(i + 1, used, ov, map, cur, acc, top);
        cur.pop();
        for j in 0..ov[i].len() {
            if used >> j & 1 == 0 && ov[i][j] > 0 {
                cur.push(Some(j));
                best(i + 1, used | 1 << j, ov, map, cur, acc + ov[i][j], top);
                cur.pop();
            }
        }
    }
    let (mut map, mut top) = (Vec::new(), 0u64);
    best(0, 0, &ov, &mut map, &mut Vec::new(), 0, &mut top);
    let (mut total, mut miss, mut fa, mut conf) = (0u64, 0u64, 0u64, 0u64);
    for f in 0..end {
        if !scored[f] {
            continue;
        }
        let (nr_f, nh_f) = (r[f].count_ones() as u64, h[f].count_ones() as u64);
        let correct = (0..nr)
            .filter(|&i| r[f] >> i & 1 == 1 && map[i].map(|j| h[f] >> j & 1 == 1).unwrap_or(false))
            .count() as u64;
        total += nr_f;
        miss += nr_f.saturating_sub(nh_f);
        fa += nh_f.saturating_sub(nr_f);
        conf += nr_f.min(nh_f) - correct;
    }
    let t = total.max(1) as f64;
    ((miss + fa + conf) as f64 / t, miss as f64 / t, fa as f64 / t, conf as f64 / t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES: usize = 589; // segmentation-3.0 frames per 10 s window (measured in M0)

    fn secs(samples: usize) -> f64 {
        samples as f64 / SR as f64
    }

    #[test]
    fn a_change_of_speaker_without_a_pause_splits_the_segment() {
        let cls = vec![(0..FRAMES).map(|f| if f < 300 { 1 } else { 2 }).collect::<Vec<u8>>()];
        let s = local_segments(&cls, WIN);
        assert_eq!(s.len(), 2, "pyannote-rs read this as one speech region");
        assert!((s[0].start - secs(F0)).abs() < 1e-9);
        assert!((s[0].end - secs(F0 + 300 * STEP)).abs() < 1e-9);
        assert!((s[1].start - s[0].end).abs() < 1e-9);
    }

    #[test]
    fn timing_is_reanchored_every_window_so_it_never_drifts() {
        let cls = vec![vec![1u8; FRAMES]; 200]; // 2,000 s of one speaker
        let s = local_segments(&cls, 200 * WIN);
        let last = s.last().unwrap();
        assert!((last.start - secs(199 * WIN + F0)).abs() < 1e-9);
        // pyannote-rs advanced 589 x 270 samples per window: 199 windows x 970 samples late.
        assert!(secs(199 * (WIN - FRAMES * STEP)) > 12.0);
    }

    #[test]
    fn overlap_frames_count_for_both_speakers_but_embeddings_prefer_sole_frames() {
        // A alone 0..20 (0.34 s), A+B 20..200, B alone 200..400.
        let cls = vec![(0..FRAMES)
            .map(|f| match f { 0..=19 => 1, 20..=199 => 4, 200..=399 => 2, _ => 0 })
            .collect::<Vec<u8>>()];
        let s = local_segments(&cls, WIN);
        assert_eq!(s.len(), 2);
        let (a, b) = (&s[0], &s[1]);
        assert!((a.end - secs(F0 + 200 * STEP)).abs() < 1e-9, "A runs through the overlap");
        assert!((b.start - secs(F0 + 20 * STEP)).abs() < 1e-9, "B starts at the overlap");
        let sole_b: usize = b.audio.iter().map(|(x, y)| y - x).sum();
        assert_eq!(sole_b, 200 * STEP, "B is embedded from its 200 sole frames only");
        let a_len: usize = a.audio.iter().map(|(x, y)| y - x).sum();
        assert_eq!(a_len, 200 * STEP, "A has under 0.5 s alone, so its whole run is used");
    }

    #[test]
    fn speech_past_the_end_of_the_file_is_clipped_or_dropped() {
        let n = WIN / 2; // a 5 s file in a padded 10 s window
        let cls = vec![vec![1u8; FRAMES]];
        let s = local_segments(&cls, n);
        assert_eq!(s.len(), 1);
        assert!((s[0].end - secs(n)).abs() < 1e-9);
        assert!(s[0].audio.iter().all(|&(_, y)| y <= n));
    }

    fn v(angle: f32) -> Vec<f32> {
        vec![angle.cos(), angle.sin(), 0.0]
    }

    #[test]
    fn average_linkage_finds_two_groups_and_stops_at_the_threshold() {
        let x = vec![v(0.0), v(0.1), v(0.05), v(1.6), v(1.5)];
        let l = ahc_average(&x, 0.3);
        assert_eq!(l[0], l[1]);
        assert_eq!(l[0], l[2]);
        assert_eq!(l[3], l[4]);
        assert_ne!(l[0], l[3]);
        assert!(ahc_average(&x, 2.0).iter().all(|&k| k == l[0]), "a loose threshold merges all");
    }

    #[test]
    fn a_tiny_cluster_is_folded_and_speakers_are_numbered_by_first_speech() {
        let mut segs = Vec::new();
        // Speaker X (angle 1.6) speaks first, 30 s; Y (angle 0.0) 30 s; a 3 s stray (angle 3.0).
        for i in 0..10 { segs.push((i as f64 * 3.0, i as f64 * 3.0 + 3.0, Some(v(1.6)))); }
        for i in 10..20 { segs.push((i as f64 * 3.0, i as f64 * 3.0 + 3.0, Some(v(0.0)))); }
        segs.push((60.0, 63.0, Some(v(3.0))));
        segs.push((63.0, 63.2, None)); // too short to embed: takes its neighbour's label
        let l = cluster(&segs, 0.6);
        assert_eq!(l[0], Some(0), "first to speak is speaker 0");
        assert_eq!(l[10], Some(1));
        assert!(l[20] == Some(0) || l[20] == Some(1), "the stray joins a real speaker");
        assert_eq!(l[21], l[20]);
        assert_eq!(l.iter().flatten().max(), Some(&1), "two speakers, not three");
    }

    #[test]
    fn too_little_speech_is_one_speaker() {
        let segs = vec![(0.0, 1.0, Some(v(0.0))), (1.0, 2.0, Some(v(1.6)))];
        assert_eq!(cluster(&segs, 0.6), vec![Some(0), Some(0)]);
        assert_eq!(cluster(&[(0.0, 0.1, None)], 0.6), vec![None]);
    }

    #[test]
    fn turns_join_only_when_touching_and_keep_pauses() {
        let spans = [(0.0, 2.0), (2.02, 4.0), (5.0, 6.0), (6.0, 7.0)];
        let labels = [Some(0), Some(0), Some(0), Some(1)];
        let t = to_turns(&spans, &labels);
        assert_eq!(
            t,
            vec![
                SpeakerTurn { start_ms: 0, end_ms: 4000, speaker: 0 },
                SpeakerTurn { start_ms: 5000, end_ms: 6000, speaker: 0 },
                SpeakerTurn { start_ms: 6000, end_ms: 7000, speaker: 1 },
            ]
        );
    }

    #[test]
    fn one_speaker_across_window_boundaries_is_one_turn() {
        let cls = vec![vec![1u8; FRAMES]; 3]; // 30 s, one speaker throughout
        let s = local_segments(&cls, 3 * WIN);
        let spans: Vec<(f64, f64)> = s.iter().map(|x| (x.start, x.end)).collect();
        let t = to_turns(&spans, &vec![Some(0); spans.len()]);
        assert_eq!(t.len(), 1, "the 61 ms gap between windows must not split a turn");
    }

    fn seg(source: &str, ms: u64) -> crate::meeting::TranscriptSeg {
        crate::meeting::TranscriptSeg { source: source.into(), text: "hello there".into(), start_ms: ms, speaker: None }
    }

    #[test]
    fn transcripts_that_cannot_be_matched_by_time_are_refused() {
        assert!(timing_problem(&[seg("you", 0), seg("others", 1_000), seg("you", 2_000)]).is_none());
        assert!(timing_problem(&[seg("others", 0)]).is_some(), "no microphone lines");
        assert!(timing_problem(&[seg("you", 0), seg("you", 0), seg("you", 0)]).is_some(), "pre-1.26: no timings");
        assert!(timing_problem(&[seg("you", 0), seg("you", 9_000), seg("you", 1_000)]).is_some(), "a combined meeting");
        assert!(timing_problem(&[seg("you", 0)]).is_none(), "a single line at 0 s is a real timing");
    }

    #[test]
    fn lines_take_the_speaker_they_overlap_most_or_the_nearest_within_2_s() {
        let turns = vec![
            SpeakerTurn { start_ms: 0, end_ms: 5000, speaker: 0 },
            SpeakerTurn { start_ms: 5000, end_ms: 9000, speaker: 1 },
        ];
        // 0 s, 10 words (4 s): speaker 0. 4.5 s, 10 words: mostly speaker 1.
        // 10.5 s, 2 words: no overlap, 1.5 s after a turn: speaker 1. 30 s: nobody.
        let a = assign(&[0, 4_500, 10_500, 30_000], &[10, 10, 2, 2], &turns);
        assert_eq!(a, vec![Some(0), Some(1), Some(1), None]);
    }

    #[test]
    fn a_model_file_must_match_size_and_hash() {
        const M: ModelFile = ModelFile {
            name: "abc",
            url: "",
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            bytes: 3,
        };
        let dir = std::env::temp_dir().join(format!("korero-diarise-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("abc");
        std::fs::write(&p, b"abc").unwrap();
        assert!(model_ok(&p, &M));
        std::fs::write(&p, b"abd").unwrap();
        assert!(!model_ok(&p, &M), "same size, wrong hash");
        std::fs::write(&p, b"abcd").unwrap();
        assert!(!model_ok(&p, &M), "wrong size");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plan M2 `diar-ami-der` (LESSONS L-012): the shipped models on four AMI far-field
    /// test meetings. Local and manual (177 MB of audio):
    ///   KORERO_AMI_DIR=<dir with wav/<id>.Array1-01.wav and rttm/<id>.rttm>
    ///   KORERO_DIARISE_MODELS=<dir with the two pinned model files>
    ///   cargo test --release --lib diar_ami_der -- --ignored --nocapture
    /// Fails when the mean DER (0.25 s collar) is above AMI_BAR. With KORERO_AMI_OUT set,
    /// writes each meeting's turns as RTTM so this scorer can be checked against
    /// pyannote.metrics.
    const AMI_BAR: f64 = 0.25;

    #[test]
    #[ignore]
    fn diar_ami_der() {
        let ami = std::path::PathBuf::from(std::env::var("KORERO_AMI_DIR").expect("set KORERO_AMI_DIR"));
        let models = std::path::PathBuf::from(std::env::var("KORERO_DIARISE_MODELS").expect("set KORERO_DIARISE_MODELS"));
        let (seg, emb) = (models.join(SEGMENTATION.name), models.join(EMBEDDING.name));
        assert!(model_ok(&seg, &SEGMENTATION) && model_ok(&emb, &EMBEDDING), "models missing or not the pinned files");
        let out = std::env::var("KORERO_AMI_OUT").ok().map(std::path::PathBuf::from);
        let mut engine = Engine::load(&seg, &emb, 4).unwrap();
        let mut sum = 0.0;
        let meetings = ["EN2002a", "ES2004a", "IS1009a", "TS3003a"];
        for m in meetings {
            let reference: Vec<(f64, f64, String)> = std::fs::read_to_string(ami.join("rttm").join(format!("{m}.rttm")))
                .unwrap()
                .lines()
                .filter_map(|l| {
                    let p: Vec<&str> = l.split_whitespace().collect();
                    (p.first() == Some(&"SPEAKER")).then(|| {
                        let s: f64 = p[3].parse().unwrap();
                        (s, s + p[4].parse::<f64>().unwrap(), p[7].to_string())
                    })
                })
                .collect();
            let t0 = Instant::now();
            let x = decode_wav_16k(ami.join("wav").join(format!("{m}.Array1-01.wav")).to_str().unwrap()).unwrap();
            let d = engine.run(&x, |_, _| {}).unwrap();
            let (c25, miss, fa, conf) = der(&reference, &d.turns, 0.25);
            let c0 = der(&reference, &d.turns, 0.0).0;
            println!(
                "{m}: DER {:.1}% (strict {:.1}%) missed {:.0} / false {:.0} / wrong speaker {:.0}; {} speakers; {:.0} s audio in {:.1} s",
                c25 * 100.0, c0 * 100.0, miss * 100.0, fa * 100.0, conf * 100.0,
                d.speakers, x.len() as f64 / SR as f64, t0.elapsed().as_secs_f64()
            );
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                let rttm: String = d.turns.iter().map(|t| format!(
                    "SPEAKER {m} 1 {:.3} {:.3} <NA> <NA> S{} <NA> <NA>\n",
                    t.start_ms as f64 / 1000.0, (t.end_ms - t.start_ms) as f64 / 1000.0, t.speaker
                )).collect();
                std::fs::write(dir.join(format!("{m}.rttm")), rttm).unwrap();
            }
            sum += c25;
        }
        let mean = sum / meetings.len() as f64;
        println!("mean DER {:.1}% (bar {:.0}%)", mean * 100.0, AMI_BAR * 100.0);
        assert!(mean <= AMI_BAR, "mean DER {:.1}% is above the agreed bar", mean * 100.0);
    }
    #[test]
    fn der_is_zero_for_a_perfect_answer_whatever_the_labels() {
        let r = vec![(0.0, 10.0, "A".to_string()), (10.0, 20.0, "B".to_string())];
        let h = vec![
            SpeakerTurn { start_ms: 0, end_ms: 10_000, speaker: 1 },
            SpeakerTurn { start_ms: 10_000, end_ms: 20_000, speaker: 0 },
        ];
        assert!(der(&r, &h, 0.0).0 < 1e-9);
        let one = vec![SpeakerTurn { start_ms: 0, end_ms: 20_000, speaker: 0 }];
        assert!((der(&r, &one, 0.0).0 - 0.5).abs() < 1e-3, "one speaker for two: half is confusion");
        assert!((der(&r, &[], 0.0).1 - 1.0).abs() < 1e-9, "nothing found: all missed");
    }
}
