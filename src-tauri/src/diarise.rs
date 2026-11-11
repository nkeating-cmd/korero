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
// Kōrero 1.45 (B3): the transcript of an "In the room" meeting, built from turns.
//
// The live transcript cuts speech with an energy gate, and on a far-field room
// microphone most speech sits below it (AMI: 2-9% of labelled speech passed 0.010 in
// three of four meetings). The segmentation model finds that speech well (7-15%
// missed in M0), so the saved transcript of a room meeting is rebuilt from its turns:
// each turn is transcribed on its own, so every line already has its speaker and its
// real start time, and no line straddles a change of speaker.

/// The longest piece transcribed in one call, and the shortest worth a call.
const PIECE_MAX_MS: u64 = 20_000;
const PIECE_MIN_MS: u64 = 400;

/// Turns to transcription pieces: sorted by start; where a turn overlaps the piece
/// before it, it starts where that piece ends (overlapping speech is transcribed once,
/// for whoever was already speaking); pieces under `PIECE_MIN_MS` are dropped; longer
/// than `PIECE_MAX_MS` are split into equal parts.
pub(crate) fn room_pieces(turns: &[SpeakerTurn]) -> Vec<SpeakerTurn> {
    let mut v: Vec<SpeakerTurn> = turns.to_vec();
    v.sort_by_key(|t| (t.start_ms, t.speaker));
    let mut out: Vec<SpeakerTurn> = Vec::new();
    let mut covered = 0u64;
    for t in v {
        let start = t.start_ms.max(covered);
        if t.end_ms <= start || t.end_ms - start < PIECE_MIN_MS {
            continue;
        }
        let len = t.end_ms - start;
        let parts = len.div_ceil(PIECE_MAX_MS).max(1);
        let step = len.div_ceil(parts);
        for k in 0..parts {
            let s = start + k * step;
            let e = (s + step).min(t.end_ms);
            if e > s {
                out.push(SpeakerTurn { start_ms: s, end_ms: e, speaker: t.speaker });
            }
        }
        covered = covered.max(t.end_ms);
    }
    out
}

/// Transcribe each piece of 16 kHz audio `x`. `source` is the side the lines belong to
/// ("you" for the microphone, "others" for a call's audio). `transcribe` is the speech engine;
/// `labelled` says whether speakers were told apart (one voice: lines stay unlabelled).
/// Returns the lines in time order, how many pieces failed, and the time ranges that
/// were transcribed successfully (so the caller can keep live lines outside them).
pub(crate) fn room_lines(
    x: &[f32],
    pieces: &[SpeakerTurn],
    labelled: bool,
    source: &str,
    mut transcribe: impl FnMut(Vec<f32>) -> Result<String, String>,
    mut progress: impl FnMut(usize, usize),
) -> (Vec<crate::meeting::TranscriptSeg>, usize, Vec<(u64, u64)>) {
    let at = |ms: u64| ((ms as usize) * SR / 1000).min(x.len());
    let (mut lines, mut failed, mut covered) = (Vec::new(), 0usize, Vec::new());
    for (i, p) in pieces.iter().enumerate() {
        let audio = x[at(p.start_ms)..at(p.end_ms)].to_vec();
        match transcribe(audio) {
            Ok(text) => {
                // Review M7: no near-floor hallucination drop here. The speaker model has
                // already decided this is speech, and far-field audio is always "near the
                // floor", so real one-word replies ("okay", "thank you") were being lost.
                if let Some(text) = crate::meeting::clean_segment_text(&text, None) {
                    // 1.46 (review 6): a piece only "covers" its time once it has produced
                    // a line. One that transcribed to nothing must not displace a live line.
                    covered.push((p.start_ms, p.end_ms));
                    lines.push(crate::meeting::TranscriptSeg {
                        source: source.into(),
                        text,
                        start_ms: p.start_ms,
                        speaker: labelled.then_some(p.speaker),
                    });
                }
            }
            Err(err) => {
                failed += 1;
                log::warn!("room transcript: piece {} of {} failed: {err}", i + 1, pieces.len());
            }
        }
        progress(i + 1, pieces.len());
    }
    (lines, failed, covered)
}

/// A live line's time stamp is the start of the capture buffer, which in silence
/// restarts every 700 ms: it can sit up to that long BEFORE the first word, while a
/// speaker turn starts at the word. Each rebuilt span is widened backwards by this much
/// before a live line is compared with it.
const LIVE_LEAD_MS: u64 = 1_000;

/// A live line carries no end time. Its spoken length is estimated from its words
/// (about 2.5 a second), between 1 s and the 20 s a live segment can last.
fn live_len_ms(words: usize) -> u64 {
    (words as u64 * 400).clamp(1_000, 20_000)
}

/// The transcribed spans, each widened backwards by `LIVE_LEAD_MS`, sorted and merged.
pub(crate) fn padded_spans(covered: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut v: Vec<(u64, u64)> = covered.iter().map(|&(a, b)| (a.saturating_sub(LIVE_LEAD_MS), b)).collect();
    v.sort();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Whether the rebuild replaces a live line: at least half of the line's estimated
/// spoken length lies inside transcribed spans. Under half, the line is kept, so a
/// stretch the rebuild did not reach is never lost (at worst a few words appear twice).
pub(crate) fn superseded(start_ms: u64, words: usize, spans: &[(u64, u64)]) -> bool {
    let (s, e) = (start_ms, start_ms + live_len_ms(words));
    let inside: u64 = spans.iter().map(|&(a, b)| b.min(e).saturating_sub(a.max(s))).sum();
    inside * 2 >= e - s
}

/// 1.46 (review 7): number the callers 0, 1, 2... in the order they first speak, and
/// return how many have at least one line. The speaker model's own numbers can skip
/// (a voice whose pieces all came back empty), which showed as a caller nobody could
/// find in the transcript.
pub(crate) fn renumber_callers(segs: &mut [crate::meeting::TranscriptSeg]) -> u32 {
    let mut order: Vec<u32> = Vec::new();
    for s in segs.iter_mut().filter(|s| s.source == "others") {
        if let Some(k) = s.speaker {
            let at = order.iter().position(|&o| o == k).unwrap_or_else(|| {
                order.push(k);
                order.len() - 1
            });
            s.speaker = Some(at as u32);
        }
    }
    order.len() as u32
}

/// Why a call's saved lines cannot be matched by time to its recordings, if they
/// cannot: two or more lines that all carry the same start (recorded before 1.26), or
/// lines out of time order (a combined meeting). `None` means the rebuild may run.
pub(crate) fn call_timing_problem(segs: &[crate::meeting::TranscriptSeg]) -> Option<&'static str> {
    let starts: Vec<u64> = segs.iter().filter(|s| !s.text.trim().is_empty()).map(|s| s.start_ms).collect();
    if starts.len() >= 2 && starts.iter().all(|&ms| ms == starts[0]) {
        return Some(
            "This meeting's lines have no timings (it was recorded before Kōrero 1.26), so they \
             can't be matched to the recordings. Re-transcribe it first.",
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

/// Review M6: the rebuilt lines, plus every existing line that no successfully
/// transcribed piece covered (speech the speaker model missed, or a piece that failed).
/// Kept lines lose their speaker label, since nothing measured who said them.
///
/// 1.46 (review 1): "covered" is judged over the line's whole spoken length, not its
/// start alone (`superseded`). Judging by the start kept a live line beside the rebuilt
/// line that replaced it whenever the speaker had paused first, and dropped a long live
/// line whole when only its first second had been rebuilt.
pub(crate) fn merge_room(
    mut lines: Vec<crate::meeting::TranscriptSeg>,
    existing: Vec<crate::meeting::TranscriptSeg>,
    covered: &[(u64, u64)],
) -> Vec<crate::meeting::TranscriptSeg> {
    let spans = padded_spans(covered);
    for mut s in existing {
        let words = s.text.split_whitespace().count();
        if words == 0 || superseded(s.start_ms, words, &spans) {
            continue;
        }
        s.speaker = None;
        lines.push(s);
    }
    lines.sort_by_key(|s| s.start_ms);
    lines
}

/// Review M5: the live meeting has the speech engine first. While a meeting (or the
/// audio test) is recording, a background rebuild waits between steps.
fn yield_to_meeting() {
    while crate::meeting::is_meeting_active() {
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Result of rebuilding a room meeting's transcript.
#[derive(Clone, Serialize, Deserialize, Type)]
pub struct RoomTranscript {
    pub segments: Vec<crate::meeting::TranscriptSeg>,
    pub speakers: u32,
    pub failed: u32,
    pub audio_s: f64,
    pub took_s: f64,
}

/// Kōrero 1.45 (B3): rebuild an "In the room" meeting's transcript from its
/// microphone recording, one line per speaker turn, each labelled with its speaker.
/// The live transcript is kept by the caller if this fails or finds nothing.
#[tauri::command]
#[specta::specta]
pub async fn meeting_room_transcript(
    app: tauri::AppHandle,
    mic_path: String,
    existing: Vec<crate::meeting::TranscriptSeg>,
) -> Result<RoomTranscript, String> {
    use tauri::Manager;
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
    let tm = app.state::<std::sync::Arc<crate::managers::transcription::TranscriptionManager>>().inner().clone();
    tm.initiate_model_load();
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<RoomTranscript, String> {
        emit_progress(&a, "separate", 0, 1);
        let x = decode_wav_16k(&mic_path)?;
        if x.len() < SR * 2 {
            return Err("The recording is too short to transcribe.".into());
        }
        let diar = {
            let mut engine = Engine::load(&seg_path, &emb_path, engine_threads())?;
            engine.run(&x, |d, t| {
                yield_to_meeting();
                emit_progress(&a, "separate", d as u64, t as u64)
            })?
        };
        let pieces = room_pieces(&diar.turns);
        let (lines, failed, covered) = room_lines(
            &x,
            &pieces,
            diar.speakers > 1,
            "you",
            |audio| {
                yield_to_meeting();
                crate::meeting::transcribe_retrying(&tm, audio)
            },
            |d, t| emit_progress(&a, "transcribe", d as u64, t as u64),
        );
        let segments = merge_room(lines, existing, &covered);
        if !pieces.is_empty() && failed == pieces.len() {
            return Err("Every part of the recording failed to transcribe. The live transcript was kept.".into());
        }
        log::info!(
            "room transcript: {} speakers, {} pieces, {} lines, {} failed, {:.0} s audio in {:.1} s",
            diar.speakers, pieces.len(), segments.len(), failed, x.len() as f64 / SR as f64,
            started.elapsed().as_secs_f64()
        );
        Ok(RoomTranscript {
            segments,
            speakers: diar.speakers,
            failed: failed as u32,
            audio_s: x.len() as f64 / SR as f64,
            took_s: started.elapsed().as_secs_f64(),
        })
    })
    .await
    .map_err(|err| format!("The room transcript stopped unexpectedly: {err}"))?
}

// ---------------------------------------------------------------------------
// Kōrero 1.46: the final transcript of a call, built from both recordings.
//
// Until 1.46 a call kept its LIVE transcript: speech cut by a fixed loudness gate
// (RMS 0.010, L-003) into pieces of up to 20 s, with everyone on the far end
// labelled "Others". Room meetings already had something better (above). This gives
// calls the same treatment, one side at a time:
//
//   * the call's audio ("others") is split into speaker turns and each turn is
//     transcribed, so every line has one caller ("Caller 1", "Caller 2", ...) and
//     no line straddles two people;
//   * the microphone ("you") is rebuilt the same way but stays unlabelled: on a
//     call the microphone is you;
//   * speech is found by the speaker model, not by loudness, so a quiet
//     microphone is no longer dropped before anything listens to it.
//
// A side is only replaced when its rebuild produced lines; otherwise the live lines
// for that side stay exactly as they were.

/// One side of a call after its rebuild.
pub(crate) struct CallSide {
    pub source: &'static str,
    pub lines: Vec<crate::meeting::TranscriptSeg>,
    /// Time ranges transcribed successfully (live lines outside them are kept).
    pub covered: Vec<(u64, u64)>,
    pub pieces: usize,
    pub failed: usize,
}

impl CallSide {
    /// Whether this rebuild may replace the side's live lines: it found speech, at
    /// least one piece transcribed, and it produced text.
    pub(crate) fn usable(&self) -> bool {
        self.pieces > 0 && self.failed < self.pieces && !self.lines.is_empty()
    }
}

/// The saved transcript of a call: each usable side replaces that side's existing
/// lines (keeping any existing line its rebuild did not cover, as `merge_room` does);
/// every other existing line is kept untouched, speaker label included. In time order.
pub(crate) fn merge_call(
    existing: Vec<crate::meeting::TranscriptSeg>,
    sides: Vec<CallSide>,
) -> Vec<crate::meeting::TranscriptSeg> {
    let mut out: Vec<crate::meeting::TranscriptSeg> = Vec::new();
    let mut rest = existing;
    for side in sides {
        if !side.usable() {
            continue;
        }
        let (mine, others): (Vec<_>, Vec<_>) = rest.into_iter().partition(|s| s.source == side.source);
        rest = others;
        out.extend(merge_room(side.lines, mine, &side.covered));
    }
    out.extend(rest.into_iter().filter(|s| !s.text.trim().is_empty()));
    // Stable: lines that start together keep the order they were added in.
    out.sort_by_key(|s| s.start_ms);
    out
}

/// Result of rebuilding a call's transcript from its recordings.
#[derive(Clone, Serialize, Deserialize, Type)]
pub struct CallTranscript {
    pub segments: Vec<crate::meeting::TranscriptSeg>,
    /// Voices found in the call's audio; 0 when that side was not rebuilt.
    pub callers: u32,
    /// The sides rebuilt from their recording: "you", "others", both or neither.
    pub rebuilt: Vec<String>,
    /// Pieces that could not be transcribed, over both sides.
    pub failed: u32,
    /// Why a side was left as it was, in plain words (for the activity log).
    pub notes: Vec<String>,
    pub audio_s: f64,
    pub took_s: f64,
}

/// Kōrero 1.46: rebuild a call's transcript from its two recordings (see above).
/// `existing` is the transcript as saved; lines the rebuild cannot improve are kept.
/// Errors only when neither side could be rebuilt, so the caller keeps what it has.
#[tauri::command]
#[specta::specta]
pub async fn meeting_call_transcript(
    app: tauri::AppHandle,
    mic_path: Option<String>,
    system_path: Option<String>,
    existing: Vec<crate::meeting::TranscriptSeg>,
) -> Result<CallTranscript, String> {
    use tauri::Manager;
    let is_wav = |p: &Option<String>| p.as_deref().is_some_and(|p| p.to_ascii_lowercase().ends_with(".wav"));
    if !is_wav(&mic_path) && !is_wav(&system_path) {
        return Err("This meeting has no recording to rebuild the transcript from.".into());
    }
    if let Some(why) = call_timing_problem(&existing) {
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
    let tm = app.state::<std::sync::Arc<crate::managers::transcription::TranscriptionManager>>().inner().clone();
    tm.initiate_model_load();
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<CallTranscript, String> {
        emit_progress(&a, "separate", 0, 1);
        let mut engine = Engine::load(&seg_path, &emb_path, engine_threads())?;
        let mut sides: Vec<CallSide> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let (mut callers, mut audio_s) = (0u32, 0f64);
        // The call's audio first: it carries the speaker labels, and it is the cleaner signal.
        for (source, path, what) in [
            ("others", system_path.as_deref(), "The call's audio"),
            ("you", mic_path.as_deref(), "The microphone"),
        ] {
            let Some(path) = path.filter(|p| p.to_ascii_lowercase().ends_with(".wav")) else { continue };
            let x = match decode_wav_16k(path) {
                Ok(x) if x.len() >= SR * 2 => x,
                Ok(_) => {
                    notes.push(format!("{what} recording is too short to rebuild; its live lines were kept."));
                    continue;
                }
                Err(err) => {
                    log::warn!("call transcript: {source}: {err}");
                    notes.push(format!("{what} recording could not be read; its live lines were kept."));
                    continue;
                }
            };
            audio_s = audio_s.max(x.len() as f64 / SR as f64);
            let diar = match engine.run(&x, |d, t| {
                yield_to_meeting();
                emit_progress(&a, "separate", d as u64, t as u64)
            }) {
                Ok(d) => d,
                Err(err) => {
                    log::warn!("call transcript: {source}: speaker model failed: {err}");
                    notes.push(format!("{what} could not be split into turns; its live lines were kept."));
                    continue;
                }
            };
            let pieces = room_pieces(&diar.turns);
            // Only the far end gets speaker labels: on a call the microphone is you.
            let labelled = source == "others" && diar.speakers > 1;
            let (lines, failed, covered) = room_lines(
                &x,
                &pieces,
                labelled,
                source,
                |audio| {
                    yield_to_meeting();
                    crate::meeting::transcribe_retrying(&tm, audio)
                },
                |d, t| emit_progress(&a, "transcribe", d as u64, t as u64),
            );
            let side = CallSide { source, lines, covered, pieces: pieces.len(), failed };
            if !side.usable() {
                notes.push(if side.pieces == 0 {
                    format!("No speech was found in {}; its live lines were kept.", what.to_lowercase())
                } else {
                    format!("{what} could not be transcribed again; its live lines were kept.")
                });
            }
            log::info!(
                "call transcript: {source}: {} speakers, {} pieces, {} lines, {} failed",
                diar.speakers, side.pieces, side.lines.len(), side.failed
            );
            sides.push(side);
        }
        let rebuilt: Vec<String> = sides.iter().filter(|s| s.usable()).map(|s| s.source.to_string()).collect();
        if rebuilt.is_empty() {
            return Err(if notes.is_empty() {
                "Neither recording could be rebuilt. The live transcript was kept.".into()
            } else {
                format!("{} The live transcript was kept.", notes.join(" "))
            });
        }
        let failed: usize = sides.iter().filter(|s| s.usable()).map(|s| s.failed).sum();
        let mut segments = merge_call(existing, sides);
        // Callers are counted from the lines that survived, in the order they first speak.
        if rebuilt.iter().any(|s| s == "others") {
            callers = renumber_callers(&mut segments);
        }
        log::info!(
            "call transcript: rebuilt {:?}, {} callers, {} lines, {:.0} s audio in {:.1} s",
            rebuilt, callers, segments.len(), audio_s, started.elapsed().as_secs_f64()
        );
        Ok(CallTranscript {
            segments,
            callers,
            rebuilt,
            failed: failed as u32,
            notes,
            audio_s,
            took_s: started.elapsed().as_secs_f64(),
        })
    })
    .await
    .map_err(|err| format!("The call transcript stopped unexpectedly: {err}"))?
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
    fn room_pieces_transcribe_overlap_once_drop_slivers_and_split_long_turns() {
        let t = |s: u64, e: u64, k: u32| SpeakerTurn { start_ms: s, end_ms: e, speaker: k };
        let p = room_pieces(&[t(0, 5_000, 0), t(4_000, 9_000, 1), t(8_900, 9_100, 0), t(10_000, 55_000, 1)]);
        assert_eq!(p[0], t(0, 5_000, 0));
        assert_eq!(p[1], t(5_000, 9_000, 1), "the overlap goes to whoever was already speaking");
        assert!(!p.iter().any(|x| x.start_ms >= 8_900 && x.end_ms <= 9_100), "a 200 ms sliver is dropped");
        let long: Vec<_> = p.iter().filter(|x| x.start_ms >= 10_000).collect();
        assert_eq!(long.len(), 3, "45 s is split into three parts of 15 s");
        assert!(long.iter().all(|x| x.end_ms - x.start_ms <= 20_000 && x.speaker == 1));
        assert_eq!(long.last().unwrap().end_ms, 55_000);
    }

    #[test]
    fn room_lines_label_speakers_only_when_there_is_more_than_one() {
        let x = vec![0.05f32; SR * 4];
        let pieces = vec![
            SpeakerTurn { start_ms: 0, end_ms: 1_500, speaker: 0 },
            SpeakerTurn { start_ms: 2_000, end_ms: 3_500, speaker: 1 },
        ];
        let (two, failed, covered) = room_lines(&x, &pieces, true, "you", |_| Ok("kia ora koutou".into()), |_, _| {});
        assert_eq!(covered, vec![(0, 1_500), (2_000, 3_500)]);
        assert_eq!(failed, 0);
        assert_eq!(two.iter().map(|l| l.speaker).collect::<Vec<_>>(), vec![Some(0), Some(1)]);
        assert_eq!(two[1].start_ms, 2_000);
        let (one, _, _) = room_lines(&x, &pieces, false, "you", |_| Ok("kia ora".into()), |_, _| {});
        assert!(one.iter().all(|l| l.speaker.is_none()));
        let (none, failed, covered) = room_lines(&x, &pieces, true, "you", |_| Err("engine".into()), |_, _| {});
        assert!(none.is_empty());
        assert_eq!(failed, 2);
        assert!(covered.is_empty());
        // Review M7: a genuine short reply in quiet far-field audio is kept.
        let quiet = vec![0.002f32; SR * 2];
        let short = vec![SpeakerTurn { start_ms: 0, end_ms: 1_000, speaker: 0 }];
        let (ok, _, _) = room_lines(&quiet, &short, false, "you", |_| Ok("Okay.".into()), |_, _| {});
        assert_eq!(ok.len(), 1, "\"Okay.\" in quiet audio is a real reply, not a hallucination");
    }

    #[test]
    fn merge_room_keeps_live_lines_nothing_covered_without_their_labels() {
        let seg = |ms: u64, t: &str, k: Option<u32>| crate::meeting::TranscriptSeg {
            source: "you".into(), text: t.into(), start_ms: ms, speaker: k,
        };
        let rebuilt = vec![seg(1_000, "rebuilt one", Some(0)), seg(9_000, "rebuilt two", Some(1))];
        let live = vec![seg(1_200, "live inside", Some(1)), seg(5_000, "live in a gap", Some(1)), seg(20_000, "  ", None)];
        let m = merge_room(rebuilt, live, &[(1_000, 4_000), (9_000, 12_000)]);
        let texts: Vec<&str> = m.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["rebuilt one", "live in a gap", "rebuilt two"]);
        assert_eq!(m[1].speaker, None, "a kept live line has no measured speaker");
    }

    /// Plan B3 `room-gate-transcribes-farfield`: words recovered from AMI far-field
    /// meetings by today's energy path (the Stop rebuild) against the room path.
    /// Local and manual, like `diar_ami_der`; also needs KORERO_PARAKEET_DIR.
    #[test]
    #[ignore]
    fn room_transcript_recovers_far_field_speech() {
        use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
        use transcribe_rs::onnx::Quantization;
        let ami = std::path::PathBuf::from(std::env::var("KORERO_AMI_DIR").expect("KORERO_AMI_DIR"));
        let models = std::path::PathBuf::from(std::env::var("KORERO_DIARISE_MODELS").expect("KORERO_DIARISE_MODELS"));
        let pk = std::path::PathBuf::from(std::env::var("KORERO_PARAKEET_DIR").expect("KORERO_PARAKEET_DIR"));
        let mut engine = Engine::load(&models.join(SEGMENTATION.name), &models.join(EMBEDDING.name), 4).unwrap();
        let mut asr = ParakeetModel::load(&pk, &Quantization::Int8).unwrap();
        let mut words = |audio: Vec<f32>| -> Result<String, String> {
            asr.transcribe_with(&audio, &ParakeetParams::default()).map(|r| r.text).map_err(|e| e.to_string())
        };
        let (mut total_energy, mut total_room) = (0usize, 0usize);
        for m in ["ES2004a", "TS3003a", "IS1009a"] {
            let x = decode_wav_16k(ami.join("wav").join(format!("{m}.Array1-01.wav")).to_str().unwrap()).unwrap();
            // Today's Stop rebuild: the energy segmenter, no VAD.
            let (tx, rx) = std::sync::mpsc::sync_channel::<crate::meeting_capture::LiveSegment>(100_000);
            let mut seg = crate::meeting_capture::Segmenter::new("you", tx, None);
            for f in x.chunks(480) {
                seg.push(f);
            }
            seg.finish();
            drop(seg);
            let mut energy = 0usize;
            for s in rx.try_iter() {
                energy += words(s.samples).map(|t| t.split_whitespace().count()).unwrap_or(0);
            }
            let d = engine.run(&x, |_, _| {}).unwrap();
            let pieces = room_pieces(&d.turns);
            let (lines, failed, _) = room_lines(&x, &pieces, d.speakers > 1, "you", &mut words, |_, _| {});
            let room: usize = lines.iter().map(|l| l.text.split_whitespace().count()).sum();
            println!("{m}: energy path {energy} words; room path {room} words in {} lines ({} pieces, {failed} failed, {} speakers)",
                lines.len(), pieces.len(), d.speakers);
            total_energy += energy;
            total_room += room;
        }
        println!("total: energy {total_energy} words, room {total_room} words");
        assert!(total_room > total_energy, "the room path must recover more far-field speech");
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
    // ---- Kōrero 1.46: the final transcript of a call ----------------------

    fn line(source: &str, text: &str, ms: u64, speaker: Option<u32>) -> crate::meeting::TranscriptSeg {
        crate::meeting::TranscriptSeg { source: source.into(), text: text.into(), start_ms: ms, speaker }
    }

    fn side(source: &'static str, lines: Vec<crate::meeting::TranscriptSeg>, covered: Vec<(u64, u64)>, pieces: usize, failed: usize) -> CallSide {
        CallSide { source, lines, covered, pieces, failed }
    }

    #[test]
    fn call_lines_for_the_far_end_carry_their_caller_and_their_side() {
        let x = vec![0.05f32; SR * 4];
        let pieces = vec![
            SpeakerTurn { start_ms: 0, end_ms: 1_500, speaker: 0 },
            SpeakerTurn { start_ms: 2_000, end_ms: 3_500, speaker: 1 },
        ];
        let (lines, _, _) = room_lines(&x, &pieces, true, "others", |_| Ok("kia ora".into()), |_, _| {});
        assert!(lines.iter().all(|l| l.source == "others"), "a call-audio line must stay on the call's side");
        assert_eq!(lines.iter().map(|l| l.speaker).collect::<Vec<_>>(), vec![Some(0), Some(1)]);
    }

    #[test]
    fn a_rebuilt_side_replaces_its_live_lines_and_leaves_the_other_side_alone() {
        let live = vec![
            line("you", "live you", 500, None),
            line("others", "live others one", 1_000, None),
            line("others", "live others outside", 20_000, None),
        ];
        let rebuilt = side(
            "others",
            vec![line("others", "caller one", 900, Some(0)), line("others", "caller two", 3_000, Some(1))],
            vec![(900, 5_000)],
            2,
            0,
        );
        let out = merge_call(live, vec![rebuilt]);
        let texts: Vec<&str> = out.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["live you", "caller one", "caller two", "live others outside"]);
        assert_eq!(out[1].speaker, Some(0));
        assert_eq!(out[3].speaker, None, "a kept live line has no measured speaker");
    }

    #[test]
    fn a_live_line_stamped_before_its_first_word_is_replaced_not_kept_beside_the_rebuild() {
        // The speaker paused, so the live line is stamped 400 ms before the turn starts.
        let live = vec![line("you", "so the budget is fine", 9_600, None)];
        let out = merge_call(live, vec![side("you", vec![line("you", "So the budget is fine.", 10_000, None)], vec![(10_000, 14_000)], 1, 0)]);
        assert_eq!(out.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), vec!["So the budget is fine."]);
    }

    #[test]
    fn a_long_live_line_is_kept_when_only_its_first_second_was_rebuilt() {
        let long = "word ".repeat(50); // about 20 s of speech
        let live = vec![line("you", long.trim(), 10_000, None)];
        let out = merge_call(live, vec![side("you", vec![line("you", "word", 10_500, None)], vec![(10_500, 11_500)], 1, 0)]);
        assert_eq!(out.len(), 2, "losing 19 s of speech is worse than one word twice");
    }

    #[test]
    fn superseded_needs_half_the_line_inside_the_transcribed_spans() {
        let spans = padded_spans(&[(10_000, 12_000), (11_500, 15_000), (30_000, 31_000)]);
        assert_eq!(spans, vec![(9_000, 15_000), (29_000, 31_000)]);
        assert!(superseded(9_200, 5, &spans)); // 2 s line, all inside
        assert!(superseded(14_000, 5, &spans)); // 14-16 s: exactly half inside
        assert!(!superseded(14_500, 5, &spans)); // a quarter inside
        assert!(!superseded(20_000, 5, &spans));
        assert!(!superseded(0, 1, &[]));
    }

    #[test]
    fn callers_are_numbered_in_the_order_they_first_speak_and_counted_from_their_lines() {
        let mut segs = vec![
            line("you", "a", 0, None),
            line("others", "b", 1_000, Some(3)),
            line("others", "c", 2_000, Some(0)),
            line("others", "kept live", 2_500, None),
            line("others", "d", 3_000, Some(3)),
        ];
        assert_eq!(renumber_callers(&mut segs), 2, "voices with no line are not counted");
        assert_eq!(segs.iter().map(|s| s.speaker).collect::<Vec<_>>(), vec![None, Some(0), Some(1), None, Some(0)]);
    }

    #[test]
    fn a_piece_that_transcribes_to_nothing_covers_nothing() {
        let x = vec![0.05f32; SR * 4];
        let pieces = vec![SpeakerTurn { start_ms: 0, end_ms: 1_500, speaker: 0 }];
        let (lines, failed, covered) = room_lines(&x, &pieces, false, "you", |_| Ok("   ".into()), |_, _| {});
        assert!(lines.is_empty());
        assert_eq!(failed, 0);
        assert!(covered.is_empty(), "an empty piece must not displace a live line");
    }

    #[test]
    fn untimed_or_unordered_lines_are_refused() {
        assert!(call_timing_problem(&[line("you", "a", 0, None), line("others", "b", 1_000, None)]).is_none());
        assert!(call_timing_problem(&[line("you", "a", 0, None)]).is_none());
        assert!(call_timing_problem(&[]).is_none());
        assert!(call_timing_problem(&[line("you", "a", 0, None), line("others", "b", 0, None)]).is_some());
        assert!(call_timing_problem(&[line("you", "a", 5_000, None), line("you", "b", 1_000, None)]).is_some());
    }

    #[test]
    fn a_side_that_found_nothing_or_failed_throughout_keeps_its_live_lines() {
        let live = || vec![line("you", "live you", 0, None), line("others", "live others", 1_000, None)];
        // every piece failed
        let out = merge_call(live(), vec![side("you", vec![], vec![], 3, 3)]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "live you");
        // no speech found at all
        let out = merge_call(live(), vec![side("others", vec![], vec![], 0, 0)]);
        assert_eq!(out[1].text, "live others");
        // pieces transcribed but every one came back empty: nothing to put in its place
        let out = merge_call(live(), vec![side("others", vec![], vec![(0, 9_000)], 2, 0)]);
        assert_eq!(out[1].text, "live others", "an empty rebuild must never wipe a side");
    }

    #[test]
    fn both_sides_rebuilt_come_out_in_time_order_with_only_callers_labelled() {
        let live = vec![line("you", "old you", 0, None), line("others", "old others", 100, None)];
        let out = merge_call(
            live,
            vec![
                side("others", vec![line("others", "b", 2_000, Some(1)), line("others", "d", 6_000, Some(0))], vec![(0, 9_000)], 2, 0),
                side("you", vec![line("you", "a", 500, None), line("you", "c", 4_000, None)], vec![(0, 9_000)], 2, 0),
            ],
        );
        assert_eq!(out.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c", "d"]);
        assert!(out.iter().filter(|s| s.source == "you").all(|s| s.speaker.is_none()));
        assert!(out.iter().filter(|s| s.source == "others").all(|s| s.speaker.is_some()));
    }
    /// Kōrero 1.46 (D-011), manual: what the call rebuild finds on REAL call recordings,
    /// against what the loudness gate finds. Prints numbers only: no file names, no text.
    ///
    ///   KORERO_CALLS_DIR       folder of `meeting-*-you.wav` / `meeting-*-others.wav`
    ///   KORERO_DIARISE_MODELS  folder holding the two pinned models
    ///   KORERO_CALLS_MAX       newest pairs to read (default 4)
    ///
    /// For each side: seconds of speech in speaker turns, the share of that speech that
    /// falls outside every energy-gate segment (what the live cut would not have sent to
    /// the speech model, a lower bound: live also applies the VAD), voices found, pieces,
    /// and the time taken.
    #[test]
    #[ignore]
    fn live_call_recordings_gate_against_turns() {
        let dir = std::path::PathBuf::from(std::env::var("KORERO_CALLS_DIR").expect("set KORERO_CALLS_DIR"));
        let models = std::path::PathBuf::from(std::env::var("KORERO_DIARISE_MODELS").expect("set KORERO_DIARISE_MODELS"));
        let max: usize = std::env::var("KORERO_CALLS_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
        let (seg, emb) = (models.join(SEGMENTATION.name), models.join(EMBEDDING.name));
        assert!(model_ok(&seg, &SEGMENTATION) && model_ok(&emb, &EMBEDDING), "models missing or not the pinned files");
        let mut others: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("-others.wav")))
            .collect();
        others.sort();
        others.reverse();
        let mut engine = Engine::load(&seg, &emb, engine_threads()).unwrap();
        for (i, sys) in others.iter().take(max).enumerate() {
            let mic = std::path::PathBuf::from(sys.to_str().unwrap().replace("-others.wav", "-you.wav"));
            for (source, path) in [("others", sys.clone()), ("you", mic)] {
                if !path.exists() {
                    println!("call {}: {source}: no recording", i + 1);
                    continue;
                }
                let t0 = Instant::now();
                let x = match decode_wav_16k(path.to_str().unwrap()) {
                    Ok(x) if x.len() >= SR * 2 => x,
                    _ => {
                        println!("call {}: {source}: unreadable or under 2 s", i + 1);
                        continue;
                    }
                };
                let d = engine.run(&x, |_, _| {}).unwrap();
                let pieces = room_pieces(&d.turns);
                let gate: Vec<(u64, u64)> = crate::meeting::decode_and_segment(path.to_str().unwrap(), source)
                    .unwrap()
                    .iter()
                    .map(|s| (s.start_ms, s.start_ms + (s.samples.len() * 1000 / SR) as u64))
                    .collect();
                // 10 ms frames: in a piece, and in a piece but in no gate segment.
                let frames = x.len() * 100 / SR + 1;
                let mut in_gate = vec![false; frames];
                for &(a, b) in &gate {
                    for f in (a / 10) as usize..((b / 10) as usize).min(frames) {
                        in_gate[f] = true;
                    }
                }
                let (mut speech, mut outside) = (0u64, 0u64);
                for p in &pieces {
                    for f in (p.start_ms / 10) as usize..((p.end_ms / 10) as usize).min(frames) {
                        speech += 1;
                        if !in_gate[f] {
                            outside += 1;
                        }
                    }
                }
                println!(
                    "call {}: {source}: {:.0} s audio; speech in turns {:.0} s; outside the loudness gate {:.1}%; {} voices; {} pieces; gate segments {}; {:.1} s",
                    i + 1,
                    x.len() as f64 / SR as f64,
                    speech as f64 / 100.0,
                    if speech > 0 { outside as f64 * 100.0 / speech as f64 } else { 0.0 },
                    d.speakers,
                    pieces.len(),
                    gate.len(),
                    t0.elapsed().as_secs_f64()
                );
            }
        }
    }
}
