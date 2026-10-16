# Lessons learned

What went wrong or nearly went wrong in this repo, and what now stops it happening again. Kyt standard rule S12:
no lesson stays `open` for more than 14 days. Geordi reads this file every Monday.

**Add a row when:** an incident or near miss happens, a code-review finding scores 80 or more, a scanner finds
something real, a release step fails, or a Claude session hits a trap worth not hitting twice. Record it the same
day, as `open`, in plain words.

**Close it by choosing one status** (within 14 days):

| Status | Means | The last column must say |
|---|---|---|
| `check-added` | A test, CI step, hook or script now fails if it happens again | the file or job that fails |
| `template-changed` | The shared template in `kyt-dev-platform` changed, so every repo gets it | the template |
| `skill-changed` | A skill (for example `ship`) now does it differently | the skill and step |
| `advice-only` | No check is practical; the reason is written down | why not |
| `retired` | No longer applies | why |
| `open` | Recorded, not yet decided | (blank) |

Portfolio-wide lessons live in `kyt-dev-platform/lessons/PORTFOLIO-LESSONS.md`; a repo lesson that applies to every
repo is copied there by Geordi's weekly review.

L-001 to L-010 were backfilled on 16 Oct 2026 from the Kōrero project record (state file and build notes). Their
dates are when each was found. Statuses were checked against the code on 16 Oct; where no check could be found,
the row is `open`, so several start past the 14-day limit and need a decision.

| ID | Recorded | What happened | Lesson | Status | Check or change |
|---|---|---|---|---|---|
| L-001 | 2026-09-25 | The New Zealand English pass had never run for anyone on the default settings: it needs `en-NZ` and the default was `en`. A separate `nz_english_pass_enabled` (default on) looked like the switch and did nothing on its own (F13). | Test the shipped defaults, not only the feature with the right settings. | open | Fixed in 1.41 (`ensure_nz_locale_default`, `settings.rs`). No test that the pass runs under default settings was found on 16 Oct. |
| L-002 | 2026-09-25 | Meetings were lost or reverted when the user left the Meetings tab: Stop built the meeting inside the page, and a returning page saved its older copy over background results (B1–B3). | Background work writes through the store, never through the page that started it. | open | Fixed in 1.41 (`fix/meetings-background`). Whether a test covers it was not checked. |
| L-003 | 2026-09-25 | A 34-minute meeting produced no transcript: the microphone peaked at RMS 0.0073, under the fixed 0.010 speech gate, and nothing said so until after Stop (T1). | A fixed loudness gate fails quiet or distant microphones; check the signal while recording. | open | 1.41 reports it at Stop. The in-room plan (M1) replaces the fixed gate. |
| L-004 | 2026-09-25 | Release step B stopped with "uncommitted changes" in 1.41 and again in 1.42: the signed build rewrites `src-tauri/Cargo.toml` line endings (content identical). Restored by hand each time. | A build step must not leave the tree dirty; fix the cause, not each release. | open | A `.gitattributes` rule for `Cargo.toml`, or the release script restoring it, would close this. |
| L-005 | 2026-10-01 | A local `vite build` from `C:\Dev\korero` (capital D) fails on entry pages with inline styles; the folder on disk is `C:\dev`. | Build from the exact-case path. | advice-only | Vite's behaviour, not Kōrero's. The release scripts use `C:\dev\korero`; CI is unaffected. |
| L-006 | 2026-10-01 | Critical, found in review: "Move existing recordings" could write the meetings store after a load error, replacing the user's meetings (R-02). | Never write a store that failed to load. | open | Fixed in 1.42 (`storeLoadError` guards). Whether a test covers it was not checked. |
| L-007 | 2026-10-02 | Cloud AI provider keys are never stored in Windows Credential Manager: `keyring` is built without a platform feature, so it uses a mock store, and the migration then blanks the plaintext key. Keys are lost on restart (F15). Ollama is unaffected. | A secure-storage dependency needs a test that a value survives a restart. | open | Still present on 16 Oct (`keyring = "3.6"`, no platform feature). |
| L-008 | 2026-10-08 | Notes for long meetings covered only the first 48,000 characters on 8 of 35 meetings, and lost every flagged moment, with no warning. | Never cut input silently: either handle all of it or say in the output what was left out. | open | Fixed on `fix/1.44` (`notes_parts.rs`, 13 tests). Becomes `check-added` when 1.44 merges. |
| L-009 | 2026-10-08 | Audio brief failed with "voice engine not found" on the main laptop from the first build with PRIV-1 until 1.44: removing a developer path removed the only lookup that found the engine. | When a lookup path is removed, give its replacement (a setting) and a health check in the same change. | open | Fixed on `fix/1.44` (engine folder setting). |
| L-010 | 2026-09-25 | On the main laptop, local `main` tracks `origin`, which is upstream Handy, not `korero`; a bare `git push` targets the wrong repo. | Always push with `git push korero <branch>`. | open | One-line fix: `git branch -u korero/main main` on each machine. |
| L-011 | 2026-10-16 | The first in-room plan listed `pyannote-rs` as a fallback engine. The M0 probe found it does not compile against Kōrero's `ort` (29 errors), ignores the model's speaker-change output and drifts about 60 ms per 10 s. | Compile a dependency and read its source before naming it in a plan, even as a fallback. | advice-only | `app-dev-pm` Phase 1.5 already requires probing claims; this was a planning slip, caught by M0 before any app code. |
| L-012 | 2026-10-16 | M0's first scores were worse than calling everyone one speaker. The cause was the CAM++ embedding model packaged with `pyannote-rs`: it ran without error and gave stable output, but told AMI speakers apart no better than chance, even on headset audio. The harness was checked first (reference timing, feature scale, window type), then models were compared on clean labelled turns; WeSpeaker ResNet34-LM separated them 82–100% of the time. | A model that runs is not a model that works: score it on labelled data, and check the harness before trusting a bad number. | open | Plan M2 adds a check that scores the shipped models on the AMI test set and fails above the agreed error. |
