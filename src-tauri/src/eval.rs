//! Kōrero (v1.40.0, M1c): the evaluation entry point.
//!
//! `korero.exe --eval-transcribe <wav> --model <id> --out <json> --models-dir <dir> [...]`
//!
//! One file in, one JSON out, then exit. The run uses the IMPORT path
//! (`meeting::transcribe_wav_chunked_eval`), so every configuration receives
//! byte-identical samples, and the same post-engine chain as dictation.
//!
//! Guarantees (each is a check in `checks.json`):
//! - nothing persists: `settings::EVAL_MODE` makes `write_settings` a no-op and
//!   the data dir is sandboxed under `%TEMP%` (`portable::set_data_dir_override`);
//! - no network: the update check and the Ollama probe are skipped in `lib.rs`;
//! - no download: a missing model is exit code 3, never a fetch;
//! - `--out` never overwrites without `--force` and never points inside the
//!   sandbox or the live app-data dir.
//!
//! Exit codes: 0 ok · 2 clap · 3 model missing · 4 WAV unreadable ·
//! 5 engine error (JSON still written with `error`) · 6 refused to overwrite.

use crate::cli::{CliArgs, EvalMatcher, EvalPromptMode, EvalToggle};
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{AppSettings, BiasPromptShape, WordMatching};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Manager};

pub const EXIT_OK: i32 = 0;
pub const EXIT_MODEL_MISSING: i32 = 3;
pub const EXIT_WAV_UNREADABLE: i32 = 4;
pub const EXIT_ENGINE_ERROR: i32 = 5;
pub const EXIT_REFUSED_OVERWRITE: i32 = 6;

/// Where an eval run keeps its (throwaway) settings, history, logs and webview
/// data. Called from `run()` BEFORE `portable::init()`.
pub fn sandbox_dir() -> PathBuf {
    std::env::temp_dir().join(format!("korero-eval-{}", std::process::id()))
}

/// Apply the CLI overrides to the in-memory settings for this process only.
/// Nothing here is persisted (`EVAL_MODE`); the language is left as the user's
/// (i.e. `en-NZ`) unless `--language` says otherwise.
pub fn apply_overrides(settings: &mut AppSettings, args: &CliArgs) {
    if let Some(model) = &args.model {
        settings.selected_model = model.clone();
    }
    if let Some(lang) = &args.language {
        settings.selected_language = lang.clone();
    }
    settings.bias_prompt_shape = match args.prompt_mode {
        Some(EvalPromptMode::Off) => BiasPromptShape::Off,
        Some(EvalPromptMode::List) => BiasPromptShape::List,
        Some(EvalPromptMode::Sentence) => BiasPromptShape::Sentence,
        Some(EvalPromptMode::Probe) => BiasPromptShape::Probe,
        None => settings.bias_prompt_shape,
    };
    settings.whisper_custom_word_matching = match args.matcher {
        Some(EvalMatcher::Fuzzy) => WordMatching::Fuzzy,
        Some(EvalMatcher::Exact) => WordMatching::Exact,
        None => settings.whisper_custom_word_matching,
    };
    if let Some(lex) = args.lexicon {
        settings.reo_lexicon_enabled = matches!(lex, EvalToggle::On);
    }
    // Never touch the network or the LLM in an eval run.
    settings.update_checks_enabled = false;
    settings.post_process_enabled = false;
}

#[derive(Serialize, Debug, Default)]
pub struct EvalResult {
    pub schema: u32,
    pub app_version: String,
    pub git_sha: String,
    pub exe_sha256: String,
    pub model: String,
    pub engine: String,
    pub requested_language: String,
    pub effective_language: String,
    pub prompt_mode: String,
    pub initial_prompt: Option<String>,
    pub echo_stripped: bool,
    pub matcher: String,
    pub lexicon: bool,
    pub wav: String,
    pub audio_seconds: f64,
    pub load_ms: u128,
    pub transcribe_ms: u128,
    pub raw_engine_text: String,
    pub text: String,
    pub error: Option<String>,
}

/// Canonicalise as much of `p` as exists on disk, then re-append the part that
/// does not. BOTH sides of the containment test in `out_path_is_acceptable`
/// must go through this.
///
/// Why (found by the Windows CI gate, invisible on Linux): the old code
/// canonicalised the forbidden root but fell back to the RAW path for
/// `out.parent()` whenever that parent did not exist yet — the normal case when
/// `--out` names a file in a directory the run is about to create. On Windows
/// `canonicalize` returns a verbatim extended-length path (the "?" UNC prefix,
/// long form) while the raw parent stays plain and possibly 8.3-shortened, so
/// `starts_with` compared two spellings of the same directory, found no match,
/// and the guard silently PASSED — letting an eval run write into the live
/// app-data dir it exists to protect (SEC-01). macOS fails the same way via the
/// /tmp -> /private/tmp symlink.
fn canonicalish(p: &Path) -> PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for seg in tail.iter().rev() {
                out.push(seg);
            }
            return out;
        }
        let Some(name) = cur.file_name().map(|n| n.to_os_string()) else {
            return p.to_path_buf();
        };
        tail.push(name);
        if !cur.pop() {
            return p.to_path_buf();
        }
    }
}

/// Refuse `--out` paths that could clobber anything that matters.
pub fn out_path_is_acceptable(
    out: &Path,
    force: bool,
    forbidden_roots: &[PathBuf],
) -> Result<(), String> {
    if out.exists() && !force {
        return Err(format!(
            "{} already exists (pass --force to overwrite)",
            out.display()
        ));
    }
    let canon_parent = out.parent().map(canonicalish).unwrap_or_default();
    for root in forbidden_roots {
        let root_c = canonicalish(root);
        if canon_parent.starts_with(&root_c) {
            return Err(format!(
                "{} is inside {} — an eval result must not be written into an app data dir",
                out.display(),
                root.display()
            ));
        }
    }
    Ok(())
}

fn sha256_of_current_exe() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return "unknown".into();
    };
    let Ok(bytes) = std::fs::read(exe) else {
        return "unknown".into();
    };
    format!("{:x}", Sha256::digest(bytes))
}

fn wav_seconds(path: &Path) -> Result<f64, String> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    let spec = reader.spec();
    let frames = reader.duration() as f64;
    Ok(frames / spec.sample_rate.max(1) as f64)
}

fn write_json(out: &Path, result: &EvalResult) -> Result<(), String> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(result).map_err(|e| e.to_string())?;
    std::fs::write(out, json).map_err(|e| e.to_string())
}

/// The whole run. Returns the process exit code. Called via
/// `tauri::async_runtime::block_on` from a std thread in `lib.rs` (RT #7).
pub async fn run(app: AppHandle, args: CliArgs) -> i32 {
    let wav = args.eval_transcribe.clone().expect("eval flag present");
    let out = args.out.clone().expect("clap requires --out");
    let model = args.model.clone().expect("clap requires --model");

    let mut forbidden: Vec<PathBuf> = vec![sandbox_dir()];
    if let Ok(real) = app.path().app_data_dir() {
        forbidden.push(real);
    }
    if let Err(e) = out_path_is_acceptable(&out, args.force, &forbidden) {
        log::error!("eval: {e}");
        return EXIT_REFUSED_OVERWRITE;
    }

    let settings = crate::settings::get_settings(&app);
    let mut result = EvalResult {
        schema: 1,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: option_env!("KORERO_GIT_SHA")
            .unwrap_or("unknown")
            .to_string(),
        exe_sha256: sha256_of_current_exe(),
        model: model.clone(),
        requested_language: settings.selected_language.clone(),
        prompt_mode: format!("{:?}", settings.bias_prompt_shape).to_lowercase(),
        matcher: format!("{:?}", settings.whisper_custom_word_matching).to_lowercase(),
        lexicon: settings.reo_lexicon_enabled,
        wav: wav.display().to_string(),
        ..Default::default()
    };

    result.audio_seconds = match wav_seconds(&wav) {
        Ok(s) => s,
        Err(e) => {
            result.error = Some(e);
            let _ = write_json(&out, &result);
            return EXIT_WAV_UNREADABLE;
        }
    };

    let tm = app.state::<Arc<TranscriptionManager>>().inner().clone();
    let mm = app
        .state::<Arc<crate::managers::model::ModelManager>>()
        .inner()
        .clone();
    match mm.get_model_info(&model) {
        Some(info) if info.is_downloaded => {
            result.engine = format!("{:?}", info.engine_type).to_lowercase();
        }
        Some(_) => {
            result.error = Some(format!(
                "Model '{model}' is not downloaded (eval never downloads)"
            ));
            let _ = write_json(&out, &result);
            return EXIT_MODEL_MISSING;
        }
        None => {
            result.error = Some(format!("Unknown model id '{model}'"));
            let _ = write_json(&out, &result);
            return EXIT_MODEL_MISSING;
        }
    }

    let t0 = Instant::now();
    if let Err(e) = tm.load_model(&model) {
        result.error = Some(format!("load_model failed: {e}"));
        let _ = write_json(&out, &result);
        return EXIT_MODEL_MISSING;
    }
    result.load_ms = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let outcome = crate::meeting::transcribe_wav_chunked_eval(&tm, &wav.to_string_lossy()).await;
    result.transcribe_ms = t1.elapsed().as_millis();

    // Traces are accumulated per chunk and merged here, so `raw_engine_text`
    // covers the WHOLE file even when the 300 s chunker split it (VERIFY-M1
    // DEFECT-2). `result.text` still comes from the import path's own join,
    // which is authoritative; the merged trace text is not used for it.
    if let Some(trace) = tm.take_traces() {
        result.effective_language = trace.effective_language;
        result.initial_prompt = trace.initial_prompt;
        result.echo_stripped = trace.echo_stripped;
        result.raw_engine_text = trace.raw;
    }

    let code = match outcome {
        Ok(text) => {
            result.text = text;
            EXIT_OK
        }
        Err(e) => {
            result.error = Some(e);
            EXIT_ENGINE_ERROR
        }
    };
    if let Err(e) = write_json(&out, &result) {
        log::error!("eval: could not write {}: {e}", out.display());
        return EXIT_ENGINE_ERROR;
    }
    code
}

#[cfg(test)]
mod eval_tests {
    use super::*;

    fn args(extra: &[&str]) -> CliArgs {
        use clap::Parser;
        let mut v = vec![
            "korero",
            "--eval-transcribe",
            "s.wav",
            "--model",
            "turbo",
            "--out",
            "r.json",
        ];
        v.extend_from_slice(extra);
        CliArgs::try_parse_from(v).unwrap()
    }

    #[test]
    fn eval_overrides_never_enable_network_or_llm() {
        let mut s = crate::settings::get_default_settings();
        s.update_checks_enabled = true;
        s.post_process_enabled = true;
        apply_overrides(&mut s, &args(&[]));
        assert!(!s.update_checks_enabled);
        assert!(!s.post_process_enabled);
        assert_eq!(s.selected_model, "turbo");
    }

    #[test]
    fn eval_overrides_map_every_knob() {
        let mut s = crate::settings::get_default_settings();
        apply_overrides(
            &mut s,
            &args(&[
                "--prompt-mode",
                "probe",
                "--matcher",
                "fuzzy",
                "--lexicon",
                "off",
                "--language",
                "mi",
            ]),
        );
        assert_eq!(s.bias_prompt_shape, BiasPromptShape::Probe);
        assert_eq!(s.whisper_custom_word_matching, WordMatching::Fuzzy);
        assert!(!s.reo_lexicon_enabled);
        assert_eq!(s.selected_language, "mi");
    }

    #[test]
    fn eval_overrides_leave_unspecified_knobs_alone() {
        let mut s = crate::settings::get_default_settings();
        s.bias_prompt_shape = BiasPromptShape::Sentence;
        apply_overrides(&mut s, &args(&[]));
        assert_eq!(s.bias_prompt_shape, BiasPromptShape::Sentence);
        assert_eq!(
            s.selected_language,
            crate::settings::get_default_settings().selected_language
        );
    }

    #[test]
    fn eval_out_refuses_existing_without_force_and_app_data() {
        let dir = std::env::temp_dir().join("korero_eval_out_test");
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("r.json");
        std::fs::write(&existing, "{}").unwrap();
        assert!(out_path_is_acceptable(&existing, false, &[]).is_err());
        assert!(out_path_is_acceptable(&existing, true, &[]).is_ok());
        let inside = dir.join("sub").join("r2.json");
        assert!(out_path_is_acceptable(&inside, false, &[dir.clone()]).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn eval_out_guard_survives_path_spelling_differences() {
        // Regression for the Windows CI failure. The forbidden root EXISTS, so it
        // canonicalises to the verbatim long form; the out parent does NOT exist,
        // so it used to keep its raw, possibly 8.3-shortened spelling, and the two
        // never matched. Both sides must normalise identically now.
        let root = std::env::temp_dir().join("korero_eval_spelling_test");
        std::fs::create_dir_all(&root).unwrap();

        let deep = root.join("a").join("b").join("c").join("r.json");
        assert!(
            out_path_is_acceptable(&deep, false, &[root.clone()]).is_err(),
            "a deep, not-yet-created path under a forbidden root must be refused"
        );

        // A sibling whose name has the root's name as a STRING prefix must not be
        // caught: `starts_with` is component-wise, and it has to stay that way.
        let sibling = std::env::temp_dir()
            .join("korero_eval_spelling_test_other")
            .join("r.json");
        assert!(out_path_is_acceptable(&sibling, false, &[root.clone()]).is_ok());

        let _ = std::fs::remove_dir_all(root);
    }
}
