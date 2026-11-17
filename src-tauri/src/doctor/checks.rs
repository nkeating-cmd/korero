//! Kōrero 1.49: the checks behind "Check local AI", as a pure function from
//! what was found (`Facts`) to what to tell the person (`CheckResult`s).
//!
//! Rules that keep it honest:
//! - Checks run in order, and a failed step marks the steps that depend on it
//!   "not checked", never "passed": a false all-clear is worse than none.
//! - Each problem gets one main fix, graded in the page by how safe it is to
//!   run (start something: just do it; download or change settings: ask
//!   first; IT, proxy or memory: explain only).
//! - "Working" needs the full check (a test answer). The quick check can only
//!   say "looks set up".

use super::{CheckId, CheckMode, CheckResult, CheckStatus, Facts, Fix, Verdict};
use crate::cleanup_failure::CleanupFailReason;

/// The oldest Ollama Kōrero's native calls (`think`, `truncate`, sized
/// `num_ctx`) were tested against (1.43, 2 Oct 2026). Older may work; it is a
/// warning, not a failure.
pub const TESTED_OLLAMA: (u32, u32) = (0, 34);
/// A dictation waits this long at the cursor before pasting as spoken.
const CURSOR_WAIT_S: f32 = 30.0;

fn gb(mb: u64) -> String {
    let g = mb as f64 / 1024.0;
    if g >= 10.0 {
        format!("{g:.0} GB")
    } else {
        format!("{g:.1} GB")
    }
}

fn version_tuple(v: &str) -> Option<(u32, u32)> {
    let mut it = v.trim().trim_start_matches('v').split(['.', '-']);
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// The plain words for a stored clean-up reason (Today uses the same, in
/// `src/lib/cleanupOutcome.ts`).
pub fn reason_words(r: CleanupFailReason) -> &'static str {
    match r {
        CleanupFailReason::NotSetUp => "clean-up wasn't set up",
        CleanupFailReason::Unreachable => "the AI model didn't answer",
        CleanupFailReason::ProxyBlocked => "a network proxy blocked it",
        CleanupFailReason::ModelMissing => "the model wasn't on this computer",
        CleanupFailReason::OutOfMemory => "there wasn't enough free memory",
        CleanupFailReason::TooLong => "the text was too long",
        CleanupFailReason::TimedOut => "the model took too long (usually still loading)",
        CleanupFailReason::KeyRejected => "the AI provider refused the key",
        CleanupFailReason::HttpError => "the AI model returned an error",
        CleanupFailReason::Empty => "the model answered with nothing",
        CleanupFailReason::StillRunning => "it was still running",
        CleanupFailReason::CameLate => "it came late",
        CleanupFailReason::Other => "something else went wrong",
    }
}

/// Context and runtime on top of the weights (the 1.43 fit allowances).
fn overhead_mb(model_mb: u64) -> u64 {
    if model_mb <= 5 * 1024 {
        1536
    } else if model_mb <= 10 * 1024 {
        2048
    } else {
        3072
    }
}

struct Ladder {
    out: Vec<CheckResult>,
}

impl Ladder {
    fn push(
        &mut self,
        id: CheckId,
        status: CheckStatus,
        title: &str,
        detail: String,
        fix: Option<Fix>,
    ) -> bool {
        self.out.push(CheckResult {
            id,
            status,
            title: title.to_string(),
            detail,
            fix,
        });
        status != CheckStatus::Fail
    }
    fn not_checked(&mut self, ids: &[CheckId], why: &str) {
        for id in ids {
            self.out.push(CheckResult {
                id: *id,
                status: CheckStatus::NotChecked,
                title: title_of(*id).to_string(),
                detail: why.to_string(),
                fix: None,
            });
        }
    }
}

pub fn title_of(id: CheckId) -> &'static str {
    match id {
        CheckId::CleanupOn => "Clean-up is on",
        CheckId::AddressLocal => "Clean-up runs on this computer",
        CheckId::Runtime => "The local AI is running",
        CheckId::AnswersAsOllama => "The local AI answers",
        CheckId::OllamaVersion => "Ollama is recent enough",
        CheckId::ModelChosen => "A model is chosen",
        CheckId::PromptChosen => "A clean-up prompt is chosen",
        CheckId::ModelPresent => "The model is downloaded",
        CheckId::ModelRuns => "Ollama can run the model",
        CheckId::ModelFits => "The model fits this computer",
        CheckId::MemoryNow => "Enough memory is free",
        CheckId::GpuShared => "The graphics card is free",
        CheckId::TestAnswer => "A test clean-up works",
        CheckId::Recent => "Recent clean-ups",
    }
}

const AFTER_ADDRESS: &[CheckId] = &[
    CheckId::Runtime,
    CheckId::AnswersAsOllama,
    CheckId::OllamaVersion,
    CheckId::ModelChosen,
    CheckId::PromptChosen,
    CheckId::ModelPresent,
    CheckId::ModelRuns,
    CheckId::ModelFits,
    CheckId::MemoryNow,
    CheckId::GpuShared,
    CheckId::TestAnswer,
];

/// The steps after `id`, which a failure at `id` marks "not checked".
fn after(id: CheckId) -> &'static [CheckId] {
    let i = AFTER_ADDRESS
        .iter()
        .position(|x| *x == id)
        .expect("every step after the address is in AFTER_ADDRESS");
    &AFTER_ADDRESS[i + 1..]
}

/// Walk the ladder.
pub fn evaluate(f: &Facts) -> (Vec<CheckResult>, Verdict) {
    use CheckStatus::*;
    let mut l = Ladder { out: Vec::new() };
    let fix_first = "Fix the step above first.";

    // 1. On at all.
    if !f.cleanup_enabled || f.provider_id.is_empty() {
        // Turning clean-up on here is offered only when it would run on this
        // computer: on this page it must never start sending text to a cloud
        // provider (review M6).
        let (detail, fix) = if f.provider_id.is_empty() {
            ("AI clean-up has no provider chosen, so dictations are pasted as spoken. Choose the local AI above to keep clean-up on this computer.".to_string(), None)
        } else if f.provider_local && f.provider_is_ollama {
            (
                "AI clean-up is off, so dictations are pasted as spoken.".to_string(),
                Some(Fix::TurnOnCleanup),
            )
        } else {
            (format!("AI clean-up is off, so dictations are pasted as spoken. Turning it on would send them to {}; choose the local AI above to keep them on this computer.", f.provider_label), None)
        };
        l.push(
            CheckId::CleanupOn,
            Fail,
            title_of(CheckId::CleanupOn),
            detail,
            fix,
        );
        l.not_checked(&[CheckId::AddressLocal], fix_first);
        l.not_checked(AFTER_ADDRESS, fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::Off);
    }
    l.push(
        CheckId::CleanupOn,
        Pass,
        title_of(CheckId::CleanupOn),
        format!("Clean-up uses {}.", f.provider_label),
        None,
    );

    // 2. On this computer at all (a cloud provider is a choice, not a fault).
    if !f.provider_local || !f.provider_is_ollama {
        let detail = if f.provider_local {
            format!(
                "Clean-up uses {}, which this check doesn't cover.",
                f.provider_label
            )
        } else {
            format!(
                "Clean-up sends text to {}, not to this computer.",
                f.provider_label
            )
        };
        l.push(
            CheckId::AddressLocal,
            Skipped,
            title_of(CheckId::AddressLocal),
            detail,
            None,
        );
        l.not_checked(
            AFTER_ADDRESS,
            "Only checked when clean-up runs on this computer.",
        );
        recent(&mut l, f);
        return (l.out, Verdict::NotLocal);
    }
    let port = f.port.unwrap_or(0);
    l.push(
        CheckId::AddressLocal,
        Pass,
        title_of(CheckId::AddressLocal),
        format!("Clean-up talks to this computer (port {port})."),
        None,
    );

    // 3. Something is running there.
    let rt = &f.runtime;
    let runtime_ok = if rt.in_use {
        if !rt.core_installed && !rt.update_needed {
            l.push(CheckId::Runtime, Fail, "Kōrero's local AI isn't downloaded",
                "Clean-up is set to use Kōrero's own local AI, but it isn't on this computer (it may have been removed).".into(),
                Some(Fix::InstallRuntime { download_mb: rt.missing_mb }))
        } else if !rt.alive && f.answered_http.is_some() {
            // Something answers on Kōrero's port, and it isn't Kōrero's
            // own process (review M4).
            l.push(CheckId::Runtime, Fail, "Another program is on Kōrero's port",
                format!("Kōrero's local AI isn't running, and another program is answering on port {port}. Kōrero won't send text to it."),
                Some(Fix::PortTaken { port }))
        } else if !rt.alive || f.answered_http.is_none() {
            l.push(
                CheckId::Runtime,
                Fail,
                "Kōrero's local AI isn't running",
                if rt.alive {
                    "It is running but not answering.".into()
                } else {
                    "It is downloaded but not running.".into()
                },
                Some(Fix::StartRuntime),
            )
        } else if rt.update_needed {
            l.push(CheckId::Runtime, Warn, "Kōrero's local AI needs an update",
                "It is running the version from an earlier Kōrero until this Kōrero's version is downloaded.".into(),
                Some(Fix::InstallRuntime { download_mb: rt.missing_mb }))
        } else {
            l.push(
                CheckId::Runtime,
                Pass,
                title_of(CheckId::Runtime),
                format!("Kōrero's own local AI is running on port {port}."),
                None,
            )
        }
    } else if f.answered_http.is_some() {
        l.push(
            CheckId::Runtime,
            Pass,
            title_of(CheckId::Runtime),
            format!("Something is answering on port {port}."),
            None,
        )
    } else if f.own_ollama_installed {
        l.push(
            CheckId::Runtime,
            Fail,
            "Ollama isn't running",
            "Ollama is installed but not running, so clean-up can't reach it.".into(),
            Some(Fix::StartOllama),
        )
    } else {
        let fix = if rt.supported && rt.core_installed {
            Fix::UseRuntime
        } else if rt.supported {
            Fix::InstallRuntime {
                download_mb: rt.missing_mb,
            }
        } else {
            Fix::InstallOllama
        };
        l.push(CheckId::Runtime, Fail, "There's no local AI on this computer",
            "Clean-up is set to use Ollama, and Ollama isn't installed. Kōrero can add its own local AI instead: it stays on this computer.".into(),
            Some(fix))
    };
    if !runtime_ok {
        l.not_checked(after(CheckId::Runtime), fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::NeedsAttention);
    }

    // 4. It is Ollama, and not a proxy or another program.
    let Some(version) = f.ollama_version.as_deref() else {
        let status = f.answered_http.unwrap_or(0);
        let (detail, fix) = if f.proxy_set || status == 407 {
            ("Something answered for Ollama, and this computer has a network proxy set. A proxy can stand in the way of programs on this computer.".to_string(),
             Fix::ProxyHelp)
        } else {
            (format!("Another program is using port {port} (it answered HTTP {status}), so clean-up can't reach Ollama there."),
             Fix::PortTaken { port })
        };
        l.push(
            CheckId::AnswersAsOllama,
            Fail,
            "Something else is answering",
            detail,
            Some(fix),
        );
        l.not_checked(after(CheckId::AnswersAsOllama), fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::NeedsAttention);
    };
    l.push(
        CheckId::AnswersAsOllama,
        Pass,
        title_of(CheckId::AnswersAsOllama),
        format!("Ollama {version} answered."),
        None,
    );

    // 5. Version (only a user-installed Ollama can be old; Kōrero's is pinned).
    match version_tuple(version) {
        Some(v) if !rt.in_use && v < TESTED_OLLAMA => {
            l.push(
                CheckId::OllamaVersion,
                Warn,
                "Ollama is older than Kōrero was tested with",
                format!(
                    "Ollama {version} is installed; Kōrero was tested with {}.{} and later.",
                    TESTED_OLLAMA.0, TESTED_OLLAMA.1
                ),
                Some(Fix::UpdateOllama),
            );
        }
        _ => {
            l.push(
                CheckId::OllamaVersion,
                Pass,
                title_of(CheckId::OllamaVersion),
                if rt.in_use {
                    format!("Kōrero's own copy, Ollama {version}.")
                } else {
                    format!("Ollama {version}.")
                },
                None,
            );
        }
    }

    // 6–8. The model.
    if f.model.trim().is_empty() {
        l.push(
            CheckId::ModelChosen,
            Fail,
            "No model is chosen",
            "Clean-up has no AI model to use.".into(),
            Some(Fix::ChooseModel),
        );
        l.not_checked(after(CheckId::ModelChosen), fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::NeedsAttention);
    }
    l.push(
        CheckId::ModelChosen,
        Pass,
        title_of(CheckId::ModelChosen),
        f.model.clone(),
        None,
    );

    // 7. A prompt: without one every dictation is pasted as spoken, however
    // well the model runs (review M3).
    if !f.prompt_ok {
        l.push(
            CheckId::PromptChosen,
            Fail,
            "No clean-up prompt is ready",
            "No clean-up prompt is chosen, or the chosen one is empty, so dictations are pasted as spoken.".into(),
            Some(Fix::ChoosePrompt),
        );
        l.not_checked(after(CheckId::PromptChosen), fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::NeedsAttention);
    }
    l.push(
        CheckId::PromptChosen,
        Pass,
        title_of(CheckId::PromptChosen),
        "The clean-up prompt has text.".into(),
        None,
    );

    if !f.tags_read {
        l.push(
            CheckId::ModelPresent,
            NotChecked,
            "Couldn't read the list of models",
            "Ollama didn't list its models in time, so this check can't tell whether the model is downloaded. Run the check again.".into(),
            None,
        );
        l.not_checked(after(CheckId::ModelPresent), fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::NeedsAttention);
    }
    let Some(model_mb) = f.model_mb else {
        let detail = if rt.in_use {
            format!("{} isn't in Kōrero's own models folder. Kōrero's local AI keeps its own models, so it needs its own copy.", f.model)
        } else {
            format!("{} isn't downloaded in Ollama on this computer.", f.model)
        };
        l.push(
            CheckId::ModelPresent,
            Fail,
            "The model isn't downloaded",
            detail,
            Some(Fix::DownloadModel {
                model: f.model.clone(),
            }),
        );
        l.not_checked(after(CheckId::ModelPresent), fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::NeedsAttention);
    };
    l.push(
        CheckId::ModelPresent,
        Pass,
        title_of(CheckId::ModelPresent),
        format!("{} ({}).", f.model, gb(model_mb)),
        None,
    );

    if let Some(why) = &f.show_error {
        let lower = why.to_ascii_lowercase();
        let detail = if lower.contains("unsupported")
            || lower.contains("unknown model architecture")
            || lower.contains("quant")
        {
            "Ollama can't run this model's format. Choose another model.".to_string()
        } else {
            "Ollama couldn't open this model. Choose another model, or download it again."
                .to_string()
        };
        l.push(
            CheckId::ModelRuns,
            Fail,
            "Ollama can't run the model",
            detail,
            Some(Fix::ChooseModel),
        );
        l.not_checked(after(CheckId::ModelRuns), fix_first);
        recent(&mut l, f);
        return (l.out, Verdict::NeedsAttention);
    }
    l.push(
        CheckId::ModelRuns,
        Pass,
        title_of(CheckId::ModelRuns),
        "Ollama can open it.".into(),
        None,
    );

    // 9. Fit (a warning or a failure, but the later checks still run).
    let need = model_mb + overhead_mb(model_mb);
    let fast = u64::from(f.best_vram_mb);
    let ram = u64::from(f.ram_mb);
    if fast > 0 && need * 10 <= fast * 9 {
        l.push(
            CheckId::ModelFits,
            Pass,
            title_of(CheckId::ModelFits),
            format!(
                "It runs on the graphics card ({} needed, {} there).",
                gb(need),
                gb(fast)
            ),
            None,
        );
    } else if ram == 0 || need * 4 <= ram * 3 {
        let detail = if fast == 0 {
            format!("It needs about {}, and this computer has no graphics card that Kōrero can use, so it runs on the processor and clean-up is slower. A smaller model would be quicker.", gb(need))
        } else {
            format!("It needs about {}, more than the graphics card holds, so clean-up is slower. A smaller model would be quicker.", gb(need))
        };
        l.push(CheckId::ModelFits, Warn, "The model runs partly on the processor",
            detail,
            Some(Fix::ChooseModel));
    } else {
        l.push(
            CheckId::ModelFits,
            Fail,
            "The model is too big for this computer",
            format!(
                "It needs about {}, and this computer has {} of memory.",
                gb(need),
                gb(ram)
            ),
            Some(Fix::ChooseModel),
        );
    }

    // 10. Memory free right now (only matters if the model has to load).
    match f.available_ram_mb {
        Some(free) if !f.model_loaded && u64::from(free) < model_mb + 2048 => {
            l.push(CheckId::MemoryNow, Warn, "Memory is short right now",
                format!("Only {} of memory is free. Loading the model may take a minute or more (it took 108 s on a test laptop with 3 GB free), so the first clean-up may come late.", gb(u64::from(free))),
                Some(Fix::FreeMemory));
        }
        _ => {
            l.push(
                CheckId::MemoryNow,
                Pass,
                title_of(CheckId::MemoryNow),
                if f.model_loaded {
                    "The model is already loaded.".into()
                } else {
                    "There is room to load the model.".into()
                },
                None,
            );
        }
    }

    // 11. Other models sharing the card.
    if f.others_loaded.is_empty() {
        l.push(
            CheckId::GpuShared,
            Pass,
            title_of(CheckId::GpuShared),
            "No other model is loaded.".into(),
            None,
        );
    } else {
        l.push(
            CheckId::GpuShared,
            Warn,
            "Other models are loaded",
            format!(
                "Also loaded: {}. They share the graphics card, which can make clean-up slower.",
                f.others_loaded.join(", ")
            ),
            None,
        );
    }

    // 12. A real answer (full check only).
    let mut working = false;
    match (&f.mode, &f.test) {
        (CheckMode::Quick, _) => {
            l.not_checked(
                &[CheckId::TestAnswer],
                "Run the full check to test a real answer.",
            );
        }
        (CheckMode::Full, None) => {
            let why = if f.recording {
                "Not run while recording."
            } else {
                "Not run."
            };
            l.not_checked(&[CheckId::TestAnswer], why);
        }
        (CheckMode::Full, Some(Err(r))) => {
            let r = *r;
            let fix = match r {
                CleanupFailReason::OutOfMemory => Some(Fix::FreeMemory),
                CleanupFailReason::ModelMissing => Some(Fix::DownloadModel {
                    model: f.model.clone(),
                }),
                CleanupFailReason::NotSetUp => Some(Fix::ChoosePrompt),
                CleanupFailReason::Unreachable => Some(if rt.in_use {
                    Fix::StartRuntime
                } else {
                    Fix::StartOllama
                }),
                CleanupFailReason::ProxyBlocked => Some(Fix::ProxyHelp),
                _ => Some(Fix::ChooseModel),
            };
            l.push(
                CheckId::TestAnswer,
                Fail,
                "The test clean-up failed",
                format!("It failed because {}.", reason_words(r)),
                fix,
            );
        }
        (CheckMode::Full, Some(Ok(t))) => {
            let a = t.answer.trim();
            let lower = a.to_lowercase();
            let preamble = ["here is", "here's", "sure", "certainly", "cleaned", "okay,"]
                .iter()
                .any(|p| lower.starts_with(p));
            if a.is_empty() {
                l.push(
                    CheckId::TestAnswer,
                    Fail,
                    "The test clean-up came back empty",
                    "The model answered with nothing. Try another model.".into(),
                    Some(Fix::ChooseModel),
                );
            } else if !lower.contains("hui") || !lower.contains("marae") {
                l.push(CheckId::TestAnswer, Warn, "The model changed te reo Māori words",
                    "The test sentence had \"hui\" and \"marae\", and the answer lost one. This model may not suit te reo.".into(),
                    Some(Fix::ChooseModel));
            } else if preamble {
                l.push(CheckId::TestAnswer, Warn, "The model adds its own words",
                    "It started its answer with a greeting or a label, which would end up in your text.".into(),
                    Some(Fix::ChooseModel));
            } else if t.seconds > CURSOR_WAIT_S {
                l.push(CheckId::TestAnswer, Warn, "The model is slow",
                    format!("A one-sentence clean-up took {:.0} s. Dictations wait {CURSOR_WAIT_S:.0} s, then paste as spoken and clean up later.", t.seconds),
                    Some(Fix::ChooseModel));
            } else {
                let share = t
                    .gpu_share
                    .map(|s| format!(", {:.0}% on the graphics card", s * 100.0))
                    .unwrap_or_default();
                working = l.push(
                    CheckId::TestAnswer,
                    Pass,
                    title_of(CheckId::TestAnswer),
                    format!("Answered in {:.1} s{share}.", t.seconds),
                    None,
                );
            }
        }
    }

    recent(&mut l, f);
    let any_fail = l.out.iter().any(|c| c.status == Fail);
    let verdict = if any_fail {
        Verdict::NeedsAttention
    } else if working {
        Verdict::Working
    } else {
        Verdict::LooksSetUp
    };
    (l.out, verdict)
}

fn recent(l: &mut Ladder, f: &Facts) {
    let n = f.recent.len();
    if n == 0 {
        l.push(
            CheckId::Recent,
            CheckStatus::Pass,
            title_of(CheckId::Recent),
            "No dictation has asked for a clean-up yet.".into(),
            None,
        );
        return;
    }
    let mut counts: Vec<(CleanupFailReason, usize)> = Vec::new();
    for (cleaned, why) in &f.recent {
        if *cleaned {
            continue;
        }
        let r = why.unwrap_or(CleanupFailReason::Other);
        if matches!(
            r,
            CleanupFailReason::StillRunning | CleanupFailReason::CameLate
        ) {
            continue;
        }
        match counts.iter_mut().find(|(k, _)| *k == r) {
            Some((_, c)) => *c += 1,
            None => counts.push((r, 1)),
        }
    }
    let failed: usize = counts.iter().map(|(_, c)| c).sum();
    if failed == 0 {
        l.push(
            CheckId::Recent,
            CheckStatus::Pass,
            title_of(CheckId::Recent),
            format!("The last {n} worked."),
            None,
        );
    } else {
        counts.sort_by_key(|c| std::cmp::Reverse(c.1));
        let (top, top_n) = counts[0];
        // Most dictations failing because clean-up isn't set up is a fault
        // now, not history (review M3).
        let status = if top == CleanupFailReason::NotSetUp && failed * 2 > n {
            CheckStatus::Fail
        } else {
            CheckStatus::Warn
        };
        l.push(
            CheckId::Recent,
            status,
            "Some recent clean-ups failed",
            format!(
                "{failed} of the last {n} were pasted as spoken, most often because {} ({top_n}).",
                reason_words(top)
            ),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Facts, TestFacts};
    use super::*;
    use crate::local_runtime::RuntimeFacts;

    fn healthy() -> Facts {
        Facts {
            mode: CheckMode::Full,
            cleanup_enabled: true,
            provider_id: "ollama".into(),
            provider_label: "Ollama (local)".into(),
            provider_local: true,
            provider_is_ollama: true,
            port: Some(11435),
            model: "gemma4:12b".into(),
            runtime: RuntimeFacts {
                supported: true,
                in_use: true,
                core_installed: true,
                update_needed: false,
                alive: true,
                missing_mb: 0,
            },
            own_ollama_installed: false,
            answered_http: Some(200),
            ollama_version: Some("0.40.16".into()),
            proxy_set: false,
            model_mb: Some(8 * 1024),
            show_error: None,
            model_loaded: false,
            others_loaded: vec![],
            best_vram_mb: 12 * 1024,
            ram_mb: 32 * 1024,
            available_ram_mb: Some(16 * 1024),
            prompt_ok: true,
            tags_read: true,
            test: Some(Ok(TestFacts {
                seconds: 3.2,
                gpu_share: Some(1.0),
                answer: "So the hui is on Wednesday at the marae, and Aroha will bring the whānau."
                    .into(),
            })),
            recent: vec![(true, None); 5],
            recording: false,
        }
    }

    fn status(c: &[CheckResult], id: CheckId) -> CheckStatus {
        c.iter()
            .find(|x| x.id == id)
            .unwrap_or_else(|| panic!("{id:?} missing"))
            .status
    }

    fn fix(c: &[CheckResult], id: CheckId) -> Option<Fix> {
        c.iter().find(|x| x.id == id).and_then(|x| x.fix.clone())
    }

    #[test]
    fn a_healthy_full_check_is_working_and_lists_every_check_once() {
        let (c, v) = evaluate(&healthy());
        assert_eq!(v, Verdict::Working);
        assert!(c.iter().all(|x| x.status == CheckStatus::Pass), "{c:#?}");
        let mut ids: Vec<_> = c.iter().map(|x| x.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 14);
    }

    #[test]
    fn the_quick_check_never_says_working() {
        let mut f = healthy();
        f.mode = CheckMode::Quick;
        f.test = None;
        let (c, v) = evaluate(&f);
        assert_eq!(v, Verdict::LooksSetUp);
        assert_eq!(status(&c, CheckId::TestAnswer), CheckStatus::NotChecked);
    }

    #[test]
    fn a_failure_never_lets_later_steps_pass() {
        let mut f = healthy();
        f.runtime.in_use = false;
        f.answered_http = None;
        f.runtime.core_installed = false;
        f.runtime.missing_mb = 40;
        let (c, v) = evaluate(&f);
        assert_eq!(v, Verdict::NeedsAttention);
        assert_eq!(status(&c, CheckId::Runtime), CheckStatus::Fail);
        assert_eq!(
            fix(&c, CheckId::Runtime),
            Some(Fix::InstallRuntime { download_mb: 40 })
        );
        for id in &AFTER_ADDRESS[1..] {
            assert_eq!(status(&c, *id), CheckStatus::NotChecked, "{id:?}");
        }
        // Recent still runs: it is independent.
        assert_eq!(status(&c, CheckId::Recent), CheckStatus::Pass);
    }

    #[test]
    fn the_right_runtime_fix_for_each_situation() {
        let mut f = healthy();
        f.answered_http = None;
        assert_eq!(
            fix(&evaluate(&f).0, CheckId::Runtime),
            Some(Fix::StartRuntime)
        );

        f.runtime.core_installed = false;
        f.runtime.missing_mb = 649;
        assert_eq!(
            fix(&evaluate(&f).0, CheckId::Runtime),
            Some(Fix::InstallRuntime { download_mb: 649 })
        );

        let mut g = healthy();
        g.runtime.in_use = false;
        g.answered_http = None;
        g.own_ollama_installed = true;
        assert_eq!(
            fix(&evaluate(&g).0, CheckId::Runtime),
            Some(Fix::StartOllama)
        );

        g.own_ollama_installed = false; // nothing installed, Kōrero's runtime is
        assert_eq!(
            fix(&evaluate(&g).0, CheckId::Runtime),
            Some(Fix::UseRuntime)
        );

        g.runtime.supported = false;
        assert_eq!(
            fix(&evaluate(&g).0, CheckId::Runtime),
            Some(Fix::InstallOllama)
        );
    }

    #[test]
    fn something_else_on_the_port_is_named_and_a_proxy_is_suspected_when_set() {
        let mut f = healthy();
        f.ollama_version = None;
        f.answered_http = Some(404);
        let (c, _) = evaluate(&f);
        assert_eq!(
            fix(&c, CheckId::AnswersAsOllama),
            Some(Fix::PortTaken { port: 11435 })
        );
        f.proxy_set = true;
        assert_eq!(
            fix(&evaluate(&f).0, CheckId::AnswersAsOllama),
            Some(Fix::ProxyHelp)
        );
        f.proxy_set = false;
        f.answered_http = Some(407);
        assert_eq!(
            fix(&evaluate(&f).0, CheckId::AnswersAsOllama),
            Some(Fix::ProxyHelp)
        );
    }

    #[test]
    fn a_missing_model_offers_to_download_it_and_says_why_for_the_own_runtime() {
        let mut f = healthy();
        f.model_mb = None;
        let (c, _) = evaluate(&f);
        assert_eq!(
            fix(&c, CheckId::ModelPresent),
            Some(Fix::DownloadModel {
                model: "gemma4:12b".into()
            })
        );
        assert!(c
            .iter()
            .find(|x| x.id == CheckId::ModelPresent)
            .unwrap()
            .detail
            .contains("its own copy"));
        assert_eq!(status(&c, CheckId::TestAnswer), CheckStatus::NotChecked);
    }

    #[test]
    fn fit_memory_and_sharing_warn_without_stopping_the_test() {
        let mut f = healthy();
        f.best_vram_mb = 4 * 1024; // 8 GB model + 2 GB on a 4 GB card
        f.available_ram_mb = Some(3 * 1024);
        f.others_loaded = vec!["qwen3.5:4b".into()];
        let (c, v) = evaluate(&f);
        assert_eq!(status(&c, CheckId::ModelFits), CheckStatus::Warn);
        assert_eq!(status(&c, CheckId::MemoryNow), CheckStatus::Warn);
        assert_eq!(status(&c, CheckId::GpuShared), CheckStatus::Warn);
        assert_eq!(status(&c, CheckId::TestAnswer), CheckStatus::Pass);
        assert_eq!(v, Verdict::Working);

        f.ram_mb = 8 * 1024; // and it can't fit in memory either
        assert_eq!(
            status(&evaluate(&f).0, CheckId::ModelFits),
            CheckStatus::Fail
        );
    }

    #[test]
    fn a_bad_answer_is_caught() {
        let mut f = healthy();
        let ans = |f: &mut Facts, a: &str, s: f32| {
            f.test = Some(Ok(TestFacts {
                seconds: s,
                gpu_share: None,
                answer: a.into(),
            }))
        };
        ans(
            &mut f,
            "Here is the cleaned text: the hui is at the marae.",
            2.0,
        );
        assert_eq!(
            status(&evaluate(&f).0, CheckId::TestAnswer),
            CheckStatus::Warn
        );
        ans(&mut f, "The meeting is at the community centre.", 2.0);
        assert_eq!(
            status(&evaluate(&f).0, CheckId::TestAnswer),
            CheckStatus::Warn
        );
        ans(&mut f, "The hui is at the marae.", 45.0);
        assert_eq!(
            status(&evaluate(&f).0, CheckId::TestAnswer),
            CheckStatus::Warn
        );
        ans(&mut f, "  ", 1.0);
        assert_eq!(
            status(&evaluate(&f).0, CheckId::TestAnswer),
            CheckStatus::Fail
        );
        f.test = Some(Err(CleanupFailReason::TimedOut));
        let (c, v) = evaluate(&f);
        assert_eq!(status(&c, CheckId::TestAnswer), CheckStatus::Fail);
        assert_eq!(v, Verdict::NeedsAttention);
        assert!(c
            .iter()
            .find(|x| x.id == CheckId::TestAnswer)
            .unwrap()
            .detail
            .contains("too long"));
    }

    #[test]
    fn off_and_cloud_are_said_plainly_and_not_called_faults() {
        let mut f = healthy();
        f.cleanup_enabled = false;
        let (c, v) = evaluate(&f);
        assert_eq!(v, Verdict::Off);
        assert_eq!(fix(&c, CheckId::CleanupOn), Some(Fix::TurnOnCleanup));

        let mut g = healthy();
        g.provider_local = false;
        g.provider_is_ollama = false;
        g.provider_label = "OpenAI".into();
        let (c, v) = evaluate(&g);
        assert_eq!(v, Verdict::NotLocal);
        assert_eq!(status(&c, CheckId::AddressLocal), CheckStatus::Skipped);
        assert!(c.iter().all(|x| x.status != CheckStatus::Fail));
    }

    #[test]
    fn recent_failures_name_the_commonest_reason_and_ignore_late_ones() {
        let mut f = healthy();
        f.recent = vec![
            (false, Some(CleanupFailReason::TimedOut)),
            (false, Some(CleanupFailReason::TimedOut)),
            (false, Some(CleanupFailReason::Unreachable)),
            (true, Some(CleanupFailReason::CameLate)),
            (false, Some(CleanupFailReason::StillRunning)),
            (true, None),
        ];
        let (c, _) = evaluate(&f);
        let r = c.iter().find(|x| x.id == CheckId::Recent).unwrap();
        assert_eq!(r.status, CheckStatus::Warn);
        assert!(r.detail.starts_with("3 of the last 6"), "{}", r.detail);
        assert!(r.detail.contains("too long"), "{}", r.detail);
    }

    #[test]
    fn an_old_own_ollama_warns_but_korero_s_own_never_does() {
        let mut f = healthy();
        f.ollama_version = Some("0.12.6".into());
        assert_eq!(
            status(&evaluate(&f).0, CheckId::OllamaVersion),
            CheckStatus::Pass
        );
        f.runtime.in_use = false;
        assert_eq!(
            status(&evaluate(&f).0, CheckId::OllamaVersion),
            CheckStatus::Warn
        );
        assert_eq!(version_tuple("v0.40.16"), Some((0, 40)));
        assert_eq!(version_tuple("nonsense"), None);
    }

    // ---- review fixes, 17 Nov 2026

    #[test]
    fn turning_clean_up_on_is_never_offered_for_a_cloud_provider() {
        let mut f = healthy();
        f.cleanup_enabled = false;
        f.provider_local = false;
        f.provider_is_ollama = false;
        f.provider_id = "openai".into();
        f.provider_label = "OpenAI".into();
        let (c, v) = evaluate(&f);
        assert_eq!(v, Verdict::Off);
        assert_eq!(fix(&c, CheckId::CleanupOn), None);
        f.provider_local = true;
        f.provider_is_ollama = true;
        assert_eq!(
            fix(&evaluate(&f).0, CheckId::CleanupOn),
            Some(Fix::TurnOnCleanup)
        );
    }

    #[test]
    fn no_prompt_fails_and_nothing_after_it_passes() {
        let mut f = healthy();
        f.prompt_ok = false;
        let (c, v) = evaluate(&f);
        assert_eq!(v, Verdict::NeedsAttention);
        assert_eq!(status(&c, CheckId::PromptChosen), CheckStatus::Fail);
        assert_eq!(fix(&c, CheckId::PromptChosen), Some(Fix::ChoosePrompt));
        for id in [CheckId::ModelPresent, CheckId::TestAnswer] {
            assert_eq!(status(&c, id), CheckStatus::NotChecked);
        }
    }

    #[test]
    fn another_program_on_korero_port_is_never_taken_for_korero() {
        let mut f = healthy();
        f.runtime.alive = false; // something answers, but not Kōrero's process
        let (c, v) = evaluate(&f);
        assert_eq!(v, Verdict::NeedsAttention);
        assert_eq!(status(&c, CheckId::Runtime), CheckStatus::Fail);
        assert_eq!(
            fix(&c, CheckId::Runtime),
            Some(Fix::PortTaken { port: 11435 })
        );
        f.answered_http = None;
        assert_eq!(
            fix(&evaluate(&f).0, CheckId::Runtime),
            Some(Fix::StartRuntime)
        );
    }

    #[test]
    fn an_earlier_version_still_running_is_a_warning_with_the_update() {
        let mut f = healthy();
        f.runtime.core_installed = false;
        f.runtime.update_needed = true;
        f.runtime.missing_mb = 40;
        let (c, _) = evaluate(&f);
        assert_eq!(status(&c, CheckId::Runtime), CheckStatus::Warn);
        assert_eq!(
            fix(&c, CheckId::Runtime),
            Some(Fix::InstallRuntime { download_mb: 40 })
        );
        assert_eq!(status(&c, CheckId::TestAnswer), CheckStatus::Pass);
    }

    #[test]
    fn an_unreadable_model_list_never_offers_a_download() {
        let mut f = healthy();
        f.tags_read = false;
        f.model_mb = None;
        let (c, v) = evaluate(&f);
        assert_eq!(v, Verdict::NeedsAttention);
        assert_eq!(status(&c, CheckId::ModelPresent), CheckStatus::NotChecked);
        assert!(c
            .iter()
            .all(|x| !matches!(x.fix, Some(Fix::DownloadModel { .. }))));
    }

    #[test]
    fn mostly_not_set_up_recently_is_a_failure() {
        let mut f = healthy();
        f.recent = vec![(false, Some(CleanupFailReason::NotSetUp)); 4];
        f.recent.push((true, None));
        let (c, v) = evaluate(&f);
        assert_eq!(status(&c, CheckId::Recent), CheckStatus::Fail);
        assert_eq!(v, Verdict::NeedsAttention);
        f.recent = vec![(false, Some(CleanupFailReason::TimedOut)); 4];
        assert_eq!(status(&evaluate(&f).0, CheckId::Recent), CheckStatus::Warn);
    }
}
