# Testing Kōrero

Written for 1.46.0 (12 November 2026). It says what is tested, by what, where the
gaps are, and what a release must pass. Update it when a layer changes.

## The layers

| Layer | What it proves | Where | Runs | Size now |
|---|---|---|---|---|
| Rust unit tests | Pure logic: merging a rebuilt call into the live transcript, caller numbering, timing checks, key storage decisions, notes splitting, corrections | `src-tauri/src/**` (`#[cfg(test)]`) | CI on every PR (Windows and Linux), `cargo test --lib` on the build laptop | about 320 tests |
| Frontend unit tests | Pure logic in TypeScript: speaker labels, paragraph grouping, auto-notes choice, what happens after Stop, search, model fit | `unit-tests/*.test.ts` | `bun run test:unit` in the release gates on the build laptop. **Not run by CI on a PR** | 133 tests in 9 files |
| Browser acceptance check | What the meeting screens show and which backend command a button calls, in a real browser against a stand-in backend | `tests/meeting-reading.spec.ts`, `tests/support/tauriMock.ts` | `bun run test:playwright`, CI on PRs that touch `src/` or `tests/` | 7 tests |
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
| No uncaught error on any page visited | Health |

Seen failing (12 November 2026):

- The notes claim failed against the 1.46 branch as it stood (exit 1). The side-by-side
  layout needed a 1,180 px pane, which a 1,500 px window does not give. The threshold is
  now 800 px, which a window about 1,390 px wide or more provides; a 1,366 px laptop
  screen still reads the notes on their own tab. Re-run with the old value: red.
- The select-without-editing claim failed before its fix: an independent review found
  that dragging across a line opened the edit box, and the check reproduced it.
  The check sets the selection directly and sends the closing click, because headless
  Chromium does not drag-select there; a real drag in the app is still a hand check.

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
6. **Frontend unit tests are not in PR CI.** They run in the release gates only. Add
   `bun run test:unit` to a PR workflow.
7. **Settings, Notes, Dictation history, Ask and onboarding screens** have no browser
   check yet. Add one spec per screen when it next changes, reusing `tauriMock.ts`.
8. **Accessibility.** No automated check. Add axe to the browser check, one page per screen.

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
