# Kōrero — Plan: Configurable Meeting Recording & Export Paths

_Planning doc. Author: Nic (with Claude). Date: 2026-07-05. Status: proposed, not built._
_Confidence: **High** on architecture (read against live source in `C:\dev\korero`); **Medium** on effort (no build run)._

---

## BLUF

Two related but distinct settings:

1. **Recording save path** — where meeting WAVs are written. Today every recording op routes through a single function, `meeting.rs::meetings_dir()`, so this is a one-chokepoint change. **But** the 30-day retention sweep currently deletes *any* `*.wav` in that folder — safe only while the folder is app-private. Making the path user-settable turns that sweep into a **data-loss footgun**. That guard must be fixed *first*, and is the single most important item here.
2. **Custom export path** — where meeting notes/transcripts are written on Export. Today exports land in the *same* `meetings_dir` (buried in `%APPDATA%`), which is a poor default. This is lower-risk and arguably higher user value than (1); recommend shipping it first.

Recommended order: **(P1) fix retention sweep → (P2) export path → (P3) recording path → (P4) recovery/validation hardening → (P5) UI polish**. Est. ~1.5 dev-days plus verify.

---

## Scope

| # | Feature | User story | Risk |
|---|---------|-----------|------|
| A | Recording save path | "Put meeting recordings on my D: drive / a bigger disk." | Medium–High (retention sweep, recovery list, delete guard) |
| B | Export path | "Export notes straight to my Documents/OneDrive, not AppData." | Low |

Out of scope (flag for later): moving *existing* recordings on change; per-meeting one-off destinations; changing the dictation-history location.

---

## Current architecture (verified)

**Path resolution** — `src-tauri/src/portable.rs`
- `portable::app_data_dir(app)` → `<exe>/Data` in portable mode, else `%APPDATA%\com.nkeating.korero`.

**Recordings** — `src-tauri/src/meeting.rs`
- `meetings_dir(app)` = `portable::app_data_dir(app).join("meetings")` + `create_dir_all`. **Single chokepoint.** Callers:
  - `meeting_start_capture` — builds `meeting-{stamp}-you.wav` / `-others.wav`.
  - `meeting_list_recordings` — recovery list (scans this dir only).
  - `meeting_delete_recording` — **security guard**: `canonicalize()` + refuses anything not under `meetings_dir` and not `.wav`.
  - `cleanup_old_recordings` — deletes `*.wav` older than 30 days (const `RECORDING_RETENTION_DAYS = 30`), plus `audio-brief-*.{mp3,txt}` and any `test-*`.
  - `meeting_generate_audio_brief` — writes the MP3 here (played via `convertFileSrc(path,"asset")`).
  - `meeting_export_transcript` — **also** writes here (see below).

**Export** — `meeting.rs::meeting_export_transcript(app, file_name, content)`
- Writes to `meetings_dir` (same folder as recordings), sanitises the filename, forces `.md`/`.txt`, returns the path. Frontend `MeetingsSettings.tsx::exportActive()` builds the markdown and surfaces the result as a copy/"Show in folder" row.

**Settings plumbing** — `settings.rs` + `stores/settingsStore.ts`
- Add a field to `AppSettings` with `#[serde(default = …)]` + a `default_*()` fn, and add it to the `get_default_settings()` constructor.
- Persist via a `#[tauri::command]` that does `get_settings → mutate → write_settings`; register it in `lib.rs` `collect_commands![]`.
- **Trap (my memory + confirmed in `settingsStore.ts`):** `updateSetting(key,…)` only calls the backend if `settingUpdaters[key]` exists — otherwise it mutates React state and logs `No handler for setting`, and the change is **silently lost on restart**. A new key MUST get a `settingUpdaters` entry.
- `bindings.ts` is specta-generated on a **dev** build; a release build skips specta export, so new commands must be regenerated via a dev build (or hand-seeded).

**Config** — `src-tauri/tauri.conf.json`
- `assetProtocol.scope.allow = ["**"]` → audio-brief/WAV playback works from **any** path. No scope change needed; noted as a latent coupling (don't tighten it without revisiting custom dirs).

---

## Design decisions (recommendation first)

| # | Decision | Options | Recommendation |
|---|----------|---------|----------------|
| D1 | One setting or two? | (a) one path for both; (b) separate recording + export paths | **(b) two** — recordings are large, app-managed and auto-pruned; exports are user artefacts for Documents/OneDrive. Different lifecycles → different settings. |
| D2 | Export write UX | (a) write silently to the configured default folder; (b) native Save-As dialog each time, seeded with the folder | **(a)** — preserves today's one-click flow; keep the existing "Show in folder" affordance. Optionally add a later "Ask each time" toggle. |
| D3 | Default export dir | (a) keep current (`…/meetings` in AppData); (b) default to OS Documents | **(b) Documents** for new installs (much better default), `None` sentinel = "resolve to Documents at runtime". Preserves back-compat (no stored path migrated). |
| D4 | Recording dir change semantics | (a) forward-only + make recovery tolerate the old dir; (b) offer to move existing WAVs | **(a)** — no risky bulk file move; recovery list + delete guard accept both the active dir and the legacy default as roots. |
| D5 | Retention sweep after custom dir | (a) keep "delete any old `*.wav`"; (b) restrict to Kōrero-created files | **(b) — mandatory.** Match only `meeting-*.wav`, `test-*.wav`, `audio-brief-*.{mp3,txt}`. Without this, pointing at a shared folder deletes the user's own WAVs. |

D2/D3 are the only genuinely user-facing choices — see "Open questions" at the end.

---

## Implementation

### Shared plumbing (both features)

1. **`settings.rs`** — add two fields to `AppSettings`:
   - `meeting_recording_dir: Option<String>` (`#[serde(default)]`, `None` = default `…/meetings`).
   - `meeting_export_dir: Option<String>` (`#[serde(default)]`, `None` = resolve to Documents at runtime — D3).
   Add both to `get_default_settings()` (both `None`).
2. **New commands** (`meeting.rs` or `commands/`): `change_meeting_recording_dir(Option<String>)`, `change_meeting_export_dir(Option<String>)`, and read helpers `get_effective_recording_dir()` / `get_effective_export_dir()` returning the resolved absolute path for display. Each validates the path (see Risk R3) before `write_settings`. Register all in `lib.rs::collect_commands![]`.
3. **`settingsStore.ts`** — add `meeting_recording_dir` and `meeting_export_dir` entries to `settingUpdaters` (else silent no-op — the D5-of-plumbing trap). Regenerate `bindings.ts` via a dev build.

### Feature A — recording save path

- **`meetings_dir(app)`** becomes the resolver: if `settings.meeting_recording_dir` is `Some(valid)` use it (still `create_dir_all`), else the current default. Because every caller already funnels through here, capture/list/delete/cleanup/audio-brief all inherit it. *(This also moves exports unless B decouples them — do B's `export_dir()` in the same change.)*
- **`cleanup_old_recordings`** — restrict deletion to Kōrero artefacts only (R1). Do this even if A ships later; it's a latent bug the moment the dir is user-set.
- **`meeting_list_recordings`** + **`meeting_delete_recording`** — accept **two** roots: the active recording dir and the legacy default (`app_data_dir/meetings`), so pre-change recordings stay visible and deletable (D4). Keep the `.wav`-only + canonical-prefix guard for each root.

### Feature B — export path

- Add **`export_dir(app)`** used *only* by `meeting_export_transcript`: `settings.meeting_export_dir` if set, else OS Documents (D3), else fall back to `meetings_dir`. `create_dir_all` + validate.
- `meeting_export_transcript` swaps its `meetings_dir(&app)?` for `export_dir(&app)?`. No frontend logic change; the existing `exportedPath` row already shows where it landed.

### Frontend UI (both)

- Two new components in `components/settings/` (e.g. `MeetingRecordingLocation.tsx`, `MeetingExportLocation.tsx`), placed in the Meetings settings block. Reuse the established pattern: `SettingContainer` + a path row (extend `PathDisplay` to add **Change…** and **Reset**) + **Open**.
- **Change…** → `open({ directory: true })` from `@tauri-apps/plugin-dialog` (already imported in `MeetingsSettings.tsx` for audio import) → `updateSetting("meeting_recording_dir", picked)`.
- **Reset** → `updateSetting(key, null)` (falls back to default). Show the *effective* resolved path from the `get_effective_*` command, not the raw `Option`.

---

## Red-team — risk register

| ID | Sev | Risk | Mitigation |
|----|-----|------|-----------|
| R1 | **Critical** | `cleanup_old_recordings` deletes *any* old `*.wav`. A custom dir pointed at a shared/Documents folder → **silent deletion of the user's own WAVs**. | Restrict the sweep to `meeting-*.wav` / `test-*.wav` / `audio-brief-*.{mp3,txt}`. Land this before A. |
| R2 | Major | Changing the recording dir orphans old recordings — recovery list scans only the new dir; delete guard's canonical root is the new dir. | Dual-root list + delete (D4). |
| R3 | Major | Bad path: non-existent, not writable, a file, unplugged USB, UNC, OneDrive placeholder. `canonicalize()` also errors on missing paths → list/delete throw. | Backend validates on set (exists-or-creatable, is-dir, write-probe); reject → toast, don't persist. List/delete degrade gracefully (skip a missing root, no throw). |
| R4 | Minor | Portable mode: an absolute custom path defeats "all data travels on the USB". | If `portable::is_portable()`, warn (or constrain to under the portable `Data` dir). Document the trade-off. |
| R5 | Minor | Large WAVs written into a OneDrive-synced folder → upload churn, placeholder/reparse issues (cf. prior OneDrive sync-race notes). | Tooltip: recommend a **local** disk for recordings; OneDrive/Documents is ideal for **exports**, not raw recordings. |
| R6 | Minor | Asset scope is `["**"]`, so playback works from anywhere today — but it's an implicit dependency. | Note in code; don't tighten `assetProtocol.scope` without revisiting custom dirs. |
| R7 | Minor | New setting saved to React state only (no `settingUpdaters` entry) → lost on restart. | Add both updater entries; verify persistence across restart in the gate. |

**Sound-approach check:** the one-chokepoint design (`meetings_dir`) makes A genuinely small — the risk is entirely in the *side effects* (R1/R2), not the core change. B is clean. No manufactured critique: this is a well-scoped change *provided R1 is fixed first*.

---

## Back-compat & migration

- Both fields default to `None` → existing installs behave exactly as today until the user changes something. No data migrated, no store rewrite required (serde `#[serde(default)]` handles old JSON).
- D3 (Documents default for export) only bites when `meeting_export_dir` is `None` *and* the user exports — new path, no existing file moved.
- Version bump: touches Rust + config surface but adds no new capability gate. Bump **both** `Cargo.toml` and `tauri.conf.json` (version-drift gotcha — `update_check.rs` reads `CARGO_PKG_VERSION`).

## Build & verification gate

1. `bun run tsc` (or the project's typecheck) green; `cargo check --locked` green.
2. Dev build to regenerate `bindings.ts`; confirm the four new commands appear.
3. Runtime (Nic): set a custom recording dir on **D:** → record 20 s → WAVs land there → appears in recovery list → **older recordings in the old dir still listed** (R2) → export goes to the export dir → **Reset** returns both to default.
4. **R1 proof:** drop an unrelated `keepme.wav` (>30 days old, mtime-forced) into the custom dir → trigger cleanup (restart / stop a meeting) → **it survives**; a fake `meeting-*.wav` of the same age is removed.
5. Invalid path (delete the dir / eject USB) → list & export degrade with a toast, no panic.
6. Portable smoke: `portable` marker present → custom-dir warning shows.

## Effort & sequencing (Medium confidence — no build run)

- Backend: ~0.5 day (resolvers, 4 commands, sweep constraint, dual-root, validation).
- Frontend: ~0.5 day (2 components, `PathDisplay` Change/Reset, store wiring, bindings regen).
- Verify: ~2–3 h.  **Total ≈ 1.5 days.**

Ship order: **P1 R1 sweep fix** (standalone, do now) → **P2 Feature B** (low risk, high value) → **P3 Feature A** → **P4 R2/R3 hardening** → **P5 UI polish, R4/R5 guards**.

## Open questions for Nic (don't block the plan)

1. **D2** — export: silent write to the default folder (recommended, keeps one-click), or a Save-As dialog each time?
2. **D3** — change the *default* export location to your Documents folder, or keep the current AppData default for continuity?
3. **A scope** — forward-only with dual-root recovery (recommended), or do you also want a one-off "move my existing recordings" action?

_Files to touch: `src-tauri/src/settings.rs`, `src-tauri/src/meeting.rs`, `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`, `src/stores/settingsStore.ts`, `src/bindings.ts` (regen), `src/components/settings/*` (2 new), `src/components/ui/PathDisplay.tsx`._

---

## Decisions locked (2026-07-05) — supersedes the recommendations above

| Q | Decision | Delta vs my recommendation |
|---|----------|----------------------------|
| Export UX (D2) | **Save-As dialog each time**, seeded to the default export folder | Was (a) silent write |
| Export default (D3) | **Documents** | As recommended |
| Recording default | **Documents** (`…\OneDrive\Documents`), placed in a dedicated `Kōrero\Recordings` subfolder | New default (was AppData). Nic accepts the OneDrive trade-off (R5) as an informed choice |
| Existing files on change (D4) | **Move existing recordings** into the new folder | Was (a) forward-only |

### Implementation deltas

- **Export** → use `save()` from `@tauri-apps/plugin-dialog` (`defaultPath` = export dir, filter `.md`), then write to the chosen path. Add `meeting_export_transcript_to(path, content)` (or write via `plugin-fs`); `meeting_export_dir` now only seeds the dialog's starting directory, not a silent destination. The old `exportedPath` row still shows where it landed.
- **Recording default** = `<document_dir>\Kōrero\Recordings` (dedicated subfolder — never the Documents root, so the R1 sweep can never see unrelated files and OneDrive churn is contained). Resolve via Tauri `app.path().document_dir()`; portable mode still overrides to `<exe>\Data\meetings`.
- **Move-on-change** (replaces D4a): on folder change, move `meeting-*` / `test-*` / `audio-brief-*` files old→new, then **rewrite the stored absolute paths** (R8). Implement as a backend command that moves and returns an old→new path map; the frontend patches `micPath`/`systemPath` in `meetings.json` in the same transaction. This removes the need for dual-root recovery (R2) — there's only ever one active dir.

### New risk from "move existing"

| ID | Sev | Risk | Mitigation |
|----|-----|------|-----------|
| R8 | **Major** | The meetings store (`meetings.json`) persists **absolute** `micPath`/`systemPath`. Moving the WAVs makes those stale → re-transcribe, delete, and audio-brief silently break for existing meetings. | Move command returns an old→new map; frontend rewrites stored paths in the same step. Partial-move failure → keep originals + roll back (never leave a half-moved, half-rewritten state). Longer-term option: store paths **relative** to the recording dir. |

### Revised effort & sequencing

- +~0.5 day for the move/migration + path-rewrite and its failure handling → **≈ 2 days** total.
- R5 (OneDrive) is now **accepted, not mitigated** — surface it in the recording-folder setting's description text so it stays an informed choice at the UI.
- Ship order unchanged except **R2 dual-root is dropped** (move-on-change makes it moot): **P1 R1 sweep fix → P2 export (Save-As) → P3 recording default + move-on-change (with R8 path-rewrite) → P4 validation/R3 + R8 rollback → P5 UI polish + OneDrive caveat text**.
