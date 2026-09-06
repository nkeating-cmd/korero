use clap::Parser;

#[derive(Parser, Debug, Clone, Default)]
#[command(name = "handy", about = "Handy - Speech to Text")]
pub struct CliArgs {
    /// Start with the main window hidden
    #[arg(long)]
    pub start_hidden: bool,

    /// Disable the system tray icon
    #[arg(long)]
    pub no_tray: bool,

    /// Toggle transcription on/off (sent to running instance)
    #[arg(long)]
    pub toggle_transcription: bool,

    /// Toggle transcription with post-processing on/off (sent to running instance)
    #[arg(long)]
    pub toggle_post_process: bool,

    /// Cancel the current operation (sent to running instance)
    #[arg(long)]
    pub cancel: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,

    // ---- Kōrero (v1.40.0) evaluation flags --------------------------------
    /// Evaluation run: transcribe this WAV through the import path and exit.
    /// No window, no tray, no network, nothing persisted.
    #[arg(long, value_name = "WAV", requires_all = ["model", "out"])]
    pub eval_transcribe: Option<std::path::PathBuf>,

    /// Model id to evaluate (must already be downloaded; never downloads).
    #[arg(long, requires = "eval_transcribe")]
    pub model: Option<String>,

    /// Where to write the JSON result. Refuses to overwrite unless --force.
    #[arg(long, value_name = "JSON", requires = "eval_transcribe")]
    pub out: Option<std::path::PathBuf>,

    /// Overwrite an existing --out file.
    #[arg(long, requires = "out")]
    pub force: bool,

    /// Language override for the run (e.g. en-NZ, en, mi, auto).
    #[arg(long, requires = "eval_transcribe")]
    pub language: Option<String>,

    /// Whisper initial_prompt shape for the run.
    #[arg(long, value_enum, requires = "eval_transcribe")]
    pub prompt_mode: Option<EvalPromptMode>,

    /// Custom-word matcher policy on Whisper for the run.
    #[arg(long, value_enum, requires = "eval_transcribe")]
    pub matcher: Option<EvalMatcher>,

    /// Macron-restoration lexicon on/off for the run.
    #[arg(long, value_enum, requires = "eval_transcribe")]
    pub lexicon: Option<EvalToggle>,

    /// Directory holding the downloaded models. Required because an eval run
    /// sandboxes its data dir (nothing it does can touch the live install).
    #[arg(long, value_name = "DIR", requires = "eval_transcribe")]
    pub models_dir: Option<std::path::PathBuf>,
}

// ---------------------------------------------------------------------------
// Kōrero (v1.40.0, M1a): evaluation entry point.
//
// `korero.exe --eval-transcribe <wav> --model <id> --out <json> [...]`
// transcribes ONE file through the import path (byte-identical input per
// model), applies the same post-engine chain as dictation, writes a JSON
// result and exits. No window, no tray, no network, no settings persistence
// (see settings::EVAL_MODE). Release builds have no console
// (`windows_subsystem = "windows"`), which is why `--out` is mandatory.
// ---------------------------------------------------------------------------

/// Shape of the Whisper `initial_prompt` built from custom words + corrections.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalPromptMode {
    /// No initial prompt at all.
    Off,
    /// Today's comma-separated term list (the shipped default).
    List,
    /// Terms embedded in a natural-language frame.
    Sentence,
    /// A nonsense token only — the prompt-leakage probe.
    Probe,
}

/// Custom-word matcher policy on Whisper engines.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalMatcher {
    /// The user's fuzzy `word_correction_threshold` (Parakeet behaviour).
    Fuzzy,
    /// Exact-match only (today's Whisper behaviour).
    Exact,
}

/// On/off switch as a CLI value.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalToggle {
    On,
    Off,
}

impl CliArgs {
    /// True when the process was started as an evaluation run.
    pub fn is_eval(&self) -> bool {
        self.eval_transcribe.is_some()
    }
}

#[cfg(test)]
mod eval_cli_tests {
    use super::*;

    #[test]
    fn eval_valid_argument_set_parses() {
        let a = CliArgs::try_parse_from([
            "korero",
            "--eval-transcribe",
            "sample.wav",
            "--model",
            "turbo",
            "--out",
            "r.json",
            "--prompt-mode",
            "sentence",
            "--matcher",
            "exact",
            "--lexicon",
            "off",
            "--models-dir",
            "C:/models",
        ])
        .expect("valid eval args must parse");
        assert!(a.is_eval());
        assert_eq!(a.model.as_deref(), Some("turbo"));
        assert_eq!(a.prompt_mode, Some(EvalPromptMode::Sentence));
        assert_eq!(a.matcher, Some(EvalMatcher::Exact));
        assert_eq!(a.lexicon, Some(EvalToggle::Off));
        assert!(!a.force);
    }

    #[test]
    fn eval_requires_model_and_out() {
        assert!(CliArgs::try_parse_from([
            "korero",
            "--eval-transcribe",
            "s.wav",
            "--model",
            "turbo"
        ])
        .is_err());
        assert!(CliArgs::try_parse_from([
            "korero",
            "--eval-transcribe",
            "s.wav",
            "--out",
            "r.json"
        ])
        .is_err());
    }

    #[test]
    fn eval_rejects_unknown_prompt_mode() {
        assert!(CliArgs::try_parse_from([
            "korero",
            "--eval-transcribe",
            "s.wav",
            "--model",
            "turbo",
            "--out",
            "r.json",
            "--prompt-mode",
            "nonsense"
        ])
        .is_err());
    }

    #[test]
    fn eval_flags_require_eval_transcribe() {
        // `--out` without `--eval-transcribe` is an error, so the flags cannot
        // leak into a normal launch.
        assert!(CliArgs::try_parse_from(["korero", "--out", "r.json"]).is_err());
        assert!(CliArgs::try_parse_from(["korero", "--model", "turbo"]).is_err());
    }

    #[test]
    fn normal_launch_is_not_eval() {
        let a = CliArgs::try_parse_from(["korero", "--start-hidden"]).unwrap();
        assert!(!a.is_eval());
    }
}
