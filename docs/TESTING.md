# Testing Kōrero

Written for 1.46.0 and updated for 1.47.0 (12 November 2026). It says what is tested, by what, where the
gaps are, and what a release must pass. Update it when a layer changes.

## The layers

| Layer | What it proves | Where | Runs | Size now |
|---|---|---|---|---|
| Rust unit tests | Pure logic: merging a rebuilt call into the live transcript, caller numbering, timing checks, key storage decisions, notes splitting, corrections | `src-tauri/src/**` (`#[cfg(test)]`) | CI on every PR (Windows), `cargo test --lib` on the build laptop | about 320 tests |
| Frontend unit tests | Pure logic in TypeScript: speaker labels, paragraph grouping, auto-notes choice, what happens after Stop, search, model fit | `unit-tests/*.test.ts` | `bun run test:unit`; the Playwright workflow runs it on the same PRs, and the release gates run it on the build laptop | 154 tests in 10 files |
| Browser acceptance check | What the meeting screens show and which backend command a button calls, in a real browser against a stand-in backend | `tests/meeting-reading.spec.ts`, `tests/support/tauriMock.ts` | `bun run test:playwright`; the Playwright workflow runs it on PRs that touch `src/`, `tests/` or `unit-tests/` (switched back on 12 November 2026) | 10 tests |
| Real-recording checks | Speech lost by the live loudness gate; caller separation on real call audio | `diarise.rs` ignored tests (`KORERO_CALLS_DIR`, `KORERO_DIARISE_MODELS`) | By hand on the build laptop. Reports numbers only; recordings never leave it | 5 calls measured |
| Release gates | Type check, lint, licences and advisories (`cargo deny`), secrets (`gitleaks`), signed build, installer hash, update manifest | `gates.ps1`, release kit | Every release | n/a |
| Installed-app check | The signed build installs over the previous one, keeps its data files byte for byte, starts, and logs no errors | `install.ps1` and the app log | Every release | n/a |

## What the browser check covers

It runs the real frontend with the Rust side replaced by `tests/support/tauriMock.ts`.
Every meeting, name and setting in that file is invented.

| Claim | Kind |
|---|---|
| A call shows "You + 2 callers"; six lines read as four paragraphs, one per speaker turn | Positive |
| A named caller shows the name; an unnamed one stays "Caller 2" | Positive |
| Correcting one line in a paragraph changes that line and leaves its neighbour alone | Positive |
| Dragging to select words in a paragraph does not open the editor | Negative |
| "Lines" shows one row per line, and the choice survives a reload | Positive |
| Notes sit beside the transcript at 1500 px wide, and are hidden (one tab away) at 1000 px | Positive and negative |
| Re-transcribe on a call does nothing until confirmed; "Keep" cancels; confirming calls `meeting_call_transcript` with both recordings and the rebuilt lines appear | Guard rail |
| An imported file is transcribed again the plain way and never as a call | Negative |
| The version in use shows beside "Help & about" on every page | Positive |
| "Suggest a title" names an unnamed meeting `<topic> · <its own date>`, with the model's quotes and full stop removed, and leaves the notes alone | Positive |
| Writing notes names an unnamed meeting; a meeting with a name of its own keeps it and is not offered a new one | Positive and negative |
| No uncaught error on any page visited | Health |
| 1.49 "Check local AI": with no local AI, the problem is said plainly, steps after it read "not checked", and the fix button states its size; nothing downloads until it is pressed, then `local_ai_runtime_install` and `local_ai_runtime_use` are called | Guard rail |
| 1.49 the setup card offers Kōrero's own local AI with its size, and "On a work laptop?" shows text for an IT team (loopback only, no administrator rights) | Positive |
| 1.49 a working check shows no problem card; only the quick check runs by itself; the report is emailed only when asked | Negative |
| 1.49 Kōrero's own local AI shows its size and can be repaired, switched off and removed; "Remove" asks first and calls nothing until confirmed | Guard rail |
| 1.49 a runtime download already running when the page opens is shown as progress, not as an error, and no second download starts | Negative |

Seen failing (12 November 2026):

- The notes claim failed against the 1.46 branch as it stood (exit 1). The side-by-side
  layout needed a 1,180 px pane, which a 1,500 px window does not give. The threshold is
  now 800 px, which a window about 1,390 px wide or more provides; a 1,366 px laptop
  screen still reads the notes on their own tab. Re-run with the old value: red.
- The select-without-editing claim failed before its fix: an independent review found
  that dragging across a line opened the edit box, and the check reproduced it.
  The check sets the selection directly and sends the closing click, because headless
  Chromium does not drag-select there; a real drag in the app is still a hand check.

- 1.47: with both title guards removed, the "never renames a named one" claim went red
  (exit 1); with only the patch guard removed, two unit tests went red.

- 1.49: with the size taken out of the runtime button, the "fix states its size" claim went
  red, and so did its unit test (`unit-tests/doctor.test.ts`).

Not covered by this check: trimming from a paragraph, search highlighting, and console
errors (it watches uncaught errors only).

Running it where Playwright's pinned Chromium is not installed:
`KORERO_CHROMIUM=/path/to/chromium bun run test:playwright`.

## What nothing automated covers (the gaps, worst first)

1. **A real call, end to end, in the app.** Recording, the rebuild after Stop and the
   automatic notes are joined only by unit tests of each part. Before a release that
   changes them: record one two-person call on the build laptop and read the result.
2. **`meeting_call_transcript` as a whole.** Its parts are tested; the command needs
   the speech and speaker models, which CI does not have. Next step: an ignored Rust
   test that runs it over two short synthetic WAVs when the models are present.
3. **Caller accuracy on call audio.** Unmeasured. Needs a small set of calls with the
   true speaker turns written down (the "accuracy flywheel" in the plan).
4. **Playing both sides together** (`BothSidesPlayer`). The stand-in backend cannot
   serve audio, so the browser check shows its error state only.
5. **Recording, tray, global shortcuts, paste, updater.** Windows-only, by hand.
6. **Settings, Notes, Dictation history, Ask and onboarding screens** have no browser
   check yet. Add one spec per screen when it next changes, reusing `tauriMock.ts`.
7. **Accessibility.** No automated check. Add axe to the browser check, one page per screen.

## Local AI (1.49): by hand on Windows

The Rust checks and the runtime download are unit-tested (a local range server stands in for
GitHub); the real Ollama is not. Before a release that changes `local_runtime` or `doctor`:

1. **Clean machine** (yoga-26, no Ollama installed): AI clean-up & notes, "Add local AI", watch
   the size and the progress, then download the recommended model and see "Check local AI"
   end with "Local clean-up is working." Dictate one sentence and see it cleaned.
2. **Broken on purpose** (legion-26, each put back afterwards): stop Kōrero's runtime
   (Task Manager); choose a model that isn't downloaded; set the Ollama address to a free
   port; hold port 11435 with another program; set `HTTP_PROXY` for Kōrero; dictate with the
   model unloaded and little memory free. Each must show its own problem and fix, and Today
   must show the reason on the dictation.
3. **Your own Ollama is left alone:** with a user-installed Ollama running on 11434, adding
   Kōrero's runtime must not stop it, use its models folder, or change its settings.
4. Close Kōrero from the tray: no `ollama.exe` from `%LOCALAPPDATA%\com.kyt.korero\local-ai`
   is left running.
5. **Stopping frees the graphics card** (review M7): with a model loaded, "Stop using it", then
   Task Manager: no `ollama.exe` (`serve` or runner) from Kōrero's folder, and the graphics
   memory is back. "Stop using it" also puts back the provider and address used before.
6. **A pinned-version update keeps clean-up working** (review M1): copy the runtime folder to
   `runtime\ollama-0.0.1`, delete this version's `runtime\ollama-<version>`, restart Kōrero and
   dictate: it is cleaned (the earlier copy runs), and "Check local AI" offers the update.
   After the update, the earlier folder is gone.
7. **A blocked folder** (review M8): on a laptop with AppLocker or WDAC blocking
   `%LOCALAPPDATA%`, "Add local AI" stops after about 28 MB with the plain "security settings"
   message and no path, before the CUDA files download.
8. **Remove** frees the space it states, and leaves the user's own Ollama alone.

How to run these without touching the installed Kōrero: `scripts/uat-windows/README.md`.

**Run on legion-26, 18 Nov 2026** (build `48c3777`, portable, driven over the webview; the
installed 1.48.0 and its data unchanged: settings and history hashes equal before and after;
the user's own Ollama kept its process and its 21 model files):

| Step | Result |
|---|---|
| 1 Clean machine (simulated: no Ollama on `PATH` or in the test's `LOCALAPPDATA`) | Pass: "Add local AI (650 MB)" installed core, Vulkan and CUDA 13; qwen3.5:2b downloaded; the check ran by itself and said "Local clean-up is working." (test answer 0.3 s, all on the graphics card). Not yet on a truly clean machine (yoga-26), and no spoken dictation |
| 2 Broken on purpose | Pass: runtime killed ("isn't running", Start it recovers); another program on 11435 (named, Kōrero won't send to it; Start it moves to 11436 and clean-up follows); dead `HTTP_PROXY` (full check still works); model missing after Remove (download offered with its size). Low memory not tried |
| 3 Own Ollama left alone | Pass |
| 4 Nothing left after Kōrero ends | Pass (killed outright: the job object ended the runtime) |
| 5 Stop frees the graphics card | Pass: 4,578 MiB to 1,479 MiB, `llama-server` runner gone. First run found "Stop using it" left clean-up on with DeepSeek when it had been off: fixed in `611bd37`, re-run passed (DeepSeek, off) |
| 6 Update path | Pass: the earlier copy ran, the update was offered and downloaded, the earlier folder was removed, check working |
| 7 Blocked folder (deny-execute ACL, standing in for AppLocker or WDAC) | Pass: stopped after the core (81 MB on disk, no CUDA), plain message, no path; the reason now stays on the card (`48c3777`). AppLocker and WDAC codes themselves not tried |
| 8 Remove | Pass: asked first, "frees 3.5 GB", folders gone. It ran on the build before `611bd37`, so it put clean-up back on with DeepSeek (the step 5 bug, since fixed and re-run through Stop using) |

**Run on yoga-26, 20 Nov 2026** (Intel Arc, no separate graphics card; the user's own Ollama
0.40.16; test builds `52bf20e`, then `af7fdc5` built on yoga; settings backed up first and kept):

| Step | Result |
|---|---|
| Recommendation | Found: qwen3.5:2b, the bake-off's unsafe model, was "Recommended for this computer". 2b dropped from the catalogue in 1.49; the processor pick is now qwen3.5:4b with a caution |
| Download over a dropping connection | Found: the estimate read "1.2e+24 min left". Fixed in `27f0fdc` ("waiting for the connection") |
| "Use" on qwen3.5:4b | Pass: downloaded, tested and switched clean-up on by itself; full check all green (4.6 s warm, 8.8 s cold, on the processor) |
| Spoken dictation with Ctrl+Shift+Space | Found: the shortcut did nothing after "Use" (L-018). Fixed in `af7fdc5`. On the old build, after the switch was turned off and on, a spoken dictation through it came back cleaned in about 6 s (prompt processing on the processor). On a fresh `af7fdc5` build with clean-up off at start-up, "Use" alone registered the shortcut (log) |

## Targets

- Any new pure function that decides what the user keeps or loses (merging, superseding,
  deleting, what is sent off the computer) ships with unit tests for its edges.
- Any change to a meeting screen adds or changes a claim in the browser check, and the
  claim is seen failing once.
- No coverage percentage is set: the risky code is the audio and model pipeline, which
  line coverage would not measure.

## Before a release

1. `cargo test --lib`, `bun run test:unit`, `bunx tsc --noEmit`, `bun run lint`.
2. `bun run test:playwright`.
3. `cargo deny check`, `gitleaks`.
4. Signed build; install over the previous version with the backup script; data files
   unchanged; the app log has no ERROR or WARN from the new build.
5. If recording, rebuild or notes changed: one real call, read by a person.
