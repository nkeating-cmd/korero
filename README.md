# Kōrero

**Kōrero is a personal fork of [Handy](https://github.com/cjpais/Handy) by CJ Pais** — a free,
open-source, **on-device** speech-to-text app for Windows. All credit for the original project
goes to CJ Pais and the Handy contributors. Kōrero keeps Handy's MIT licence (see [`LICENSE`](LICENSE))
and layers on a full rebrand, several new features, reliability fixes, and security hardening.

> _Kōrero_ is te reo Māori for "to speak / converse."
> For the upstream project's documentation, philosophy, and community, see
> **[handy.computer](https://handy.computer)** and **[cjpais/Handy](https://github.com/cjpais/Handy)**.

Like Handy, Kōrero transcribes **entirely on your machine**: your audio never leaves it. Post-processing
(AI clean-up and meeting notes) is off until you turn it on, and then sends transcript text only to the
LLM provider you choose. Choose Ollama and even that stays on your machine.

Two things do reach the network, and it is worth naming them rather than rounding down to "offline":
speech models are downloaded on first use, and the app asks GitHub once at startup whether a newer
release exists. Neither carries audio, transcripts, or anything about you. No telemetry, ever.

Current version: **v1.41.0** ([release notes](https://github.com/nkeating-cmd/korero/releases/tag/v1.41.0)).

---

## What Kōrero adds over Handy

### Meetings — record both sides of a call (v1.13–1.14)
- **Live transcript while you record** (v1.14): speech is segmented and transcribed on the fly, streaming into the Meetings page — and you can **ask your configured model about the meeting so far**, mid-meeting. Stopping is near-instant because the transcript already exists.
- **Live input meters + device test** (v1.13.5–6): per-stream level meters with device names and captured-time counters, a no-risk *Test audio* button that exercises the real capture path, and **native WASAPI loopback** (with cpal fallback) for reliable system-audio capture.
- **Dual capture**: your microphone ("You") **and system audio via WASAPI loopback ("Others")** — a free speaker split without diarization models.
- **Failsafe by design**: audio streams straight to WAV on disk *while recording* (header flushed every ~5 s), so a crash never loses a meeting; bounded memory even on long calls; on-disk recordings can be recovered and re-transcribed any time.
- Pick the **transcription model** per meeting; **re-transcribe**, **post-process with a custom per-meeting prompt** (rendered as markdown — tables and all), or both; rename, copy, and **export** the transcript + processed notes.
- **Import audio files** — WAV, **M4A**, MP3, FLAC, OGG (v1.16.1), plus **CAF** and **AIFF** (v1.30.1) — and have them transcribed + post-processed through the same bounded-memory pipeline. Both common `.m4a` payloads decode: **AAC** and **ALAC (Apple Lossless)**, so a lossless Voice Memo or QuickTime recording works without converting it first.
- Privacy guard: a warning whenever the configured LLM provider is a cloud endpoint, plus a Rust-side egress allowlist so a tampered config can't redirect transcripts to an unknown host.

### New surfaces & workflows
- **Home dashboard** — a proper landing screen with quick-action cards and your recent dictations, instead of opening straight into a settings list.
- **Notes page** — a built-in dictation canvas: press *Dictate*, ramble, press again, and the text lands at your cursor. *Transcribe + clean up* runs your chosen post-processing prompt over the **whole note** (v1.14.3), and a **Process note** button re-runs it any time with a selectable prompt (saved or custom) and AI model — with one-click Undo. Copy the finished note out in one click. Notes persist across restarts.
- **Help & Guide page** — plain-English guidance on the model, shortcuts, post-processing, and troubleshooting, plus a **Diagnostics** panel.
- **Record-and-clean-up shortcut** (`Ctrl+Shift+Space`) — records, transcribes, then runs your chosen post-processing prompt in one gesture.
- **Alternative one-handed dictation shortcut** (`Ctrl+Shift+Enter`, v1.14.1) — a second, independently rebindable trigger for plain transcription, placed so the right hand can press it alone (Right Ctrl + Right Shift + Enter).
- **Latch / hands-free mode** — double-tap a shortcut to lock recording on for long dictation; tap once to stop.

### Reliability
- **Meetings keep working when you leave the tab** (v1.41). Stop, re-transcribe, import, recover, notes and refine all run in the background, and the spinner and progress are still there when you come back. One queue now handles every read and write of the meetings store, so a page you return to can no longer save its older copy over a finished result.
- **Capture warnings while there's still time to fix them** (v1.41). If the microphone hears nothing for two minutes, or almost nothing comes through from the computer's audio output after 90 seconds, Kōrero warns you mid-meeting with a toast and a taskbar flash. Stop explains why a side came back empty.
- **Missed speech is rebuilt from the recording** (v1.41). If live transcription falls behind or hits an error, Stop re-transcribes that speaker from the saved audio. The speech engine reloads itself after a failure. Re-transcribe reports the real error and keeps your existing transcript.
- **Honest notes** (v1.41). Notes for a transcript cut at the 48,000-character limit say what share of it they cover, and notes cut short by the model's length limit are labelled incomplete.
- **Work finishes even if you leave the tab** (v1.30.3). Meeting post-processing runs in a store that outlives the view, so navigating away no longer discards the result — it lands whether or not you are watching, and returning mid-run shows the spinner and the text generated while you were gone. Transcriptions, imports and note refinements write straight to disk when no view is mounted, and both the Meetings and Notes autosaves now flush on the way out instead of cancelling, so an edit made in the last half-second before you switch tabs survives.
- **Long generations are no longer cut off at five minutes** (v1.30.3). The stream is bounded by silence between tokens rather than a total deadline, and output that stops early is kept and labelled as incomplete instead of being thrown away.
- **Macrons survive the stream** (v1.30.3). Generated notes are decoded at SSE frame boundaries rather than per network chunk, so a *whānau* split across two packets no longer arrives as a replacement character.
- **Fixed: shortcuts going dead after the model auto-unloads.** Model (re)loading is now panic-safe, so an idle unload can no longer wedge transcription until an app restart.
- **Model pre-warm** at startup and **early global-shortcut init** for a faster, more dependable first dictation.
- **Window size & position persistence** across launches.

### Crash reporting & logging
- **Global crash capture** — a panic hook writes a timestamped crash report (with backtrace) to a `crash-reports` folder, and always logs the panic.
- **User-facing diagnostics** — set log verbosity (Trace→Error), open the log folder, and toggle crash-report saving, all without enabling developer mode.

### Audio
- **Optional noise suppression** — RNNoise via the pure-Rust [`nnnoiseless`](https://github.com/jneem/nnnoiseless) crate (off by default; 48 kHz mics).

### Post-processing (LLM clean-up)
- **11 providers** out of the box — DeepSeek (default), OpenAI, Anthropic Claude, Google Gemini, OpenRouter, Groq, Cerebras, z.ai, AWS Bedrock, Ollama (local), and a custom endpoint. Post-processing is off until you turn it on.
- **Local models via Ollama**, including in-app model pull — plus an **Ollama doctor** (v1.17): detects a missing or stopped Ollama, installs it via winget from inside the app, starts it with one click, **auto-restarts it when a clean-up request finds it down**, and checks it's running at startup.
- **Curated default prompts** — clean transcript, client email, Slack/WhatsApp, meeting note, red-team, and **NZ English + te reo Māori** (restores macrons, never translates te reo, fixes common mis-hearings like "far no" → *whānau*).

### Localisation & defaults
- **New Zealand English on by default** (v1.41; Handy defaults to auto-detect). Macrons, NZ place names and NZ spelling apply out of the box. An existing plain "English" setting switches over once; turn it off in Settings → General and it stays off.
- **NZ + te reo Māori custom dictionary** seeded by default, with **macron-aware matching** so words like *whānau* / *hapū* resolve correctly.
- **Teachable corrections** (v1.15) — select a mis-transcribed word anywhere and teach the right one. Fixed deterministically in every future transcription, fed to the AI clean-up as a glossary, and the Notes clean-up even **suggests corrections** it noticed itself.
- Sensible defaults: trailing space on insert, 15-minute model unload timeout.

### Look & feel
- **Quiet Instrument** (v1.33.0) — a true-neutral dark ladder with **one** accent, replacing the aurora-on-navy theme. The animated cyan/purple/pink background is gone: it was three competing hues, a permanently-promoted full-window compositing layer, and the loudest thing on every screen.
- **Glass is chrome only.** Sidebar and toolbars keep the material; anything you *read* sits on an opaque surface. Following Apple's own 2025–26 position that the material exists to bring focus to content, not to sit under it.
- **Typography on the ladder that exists** — the installed Aptos has exactly two faces (Regular and Bold), measured, so hierarchy comes from size, colour and space rather than from weights the renderer silently rounds. Aptos Display for large titles, tabular figures everywhere numbers tick, a 74ch measure on transcripts.
- Keyboard-accessible focus ring expressed as an outline so it can never be clipped; subtle page transitions that honour `prefers-reduced-motion`.
- Colour system unified on design tokens, with a complete light palette defined and ready (not yet switchable).

### Security & supply chain
- **API keys stored in the OS keychain** (Windows Credential Manager), never written to disk in plaintext, with a one-time migration + pre-migration backup and clear failure surfacing.
- **Content-Security-Policy** added to the webview; **filesystem capability narrowed** to the app's own data directory.
- **Signed updates** — the app installs an update only if its signature matches the public key built into it.
- CI runs `cargo test --locked` and a frontend build on every push to `main`, with `clippy` as an advisory check. `src-tauri/deny.toml` holds the `cargo-deny` licence, source and advisory rules.

### Models & acceleration
- **Default model: Parakeet V3** — CPU-efficient, NZ-accent friendly, with DirectML GPU acceleration on Windows.
- **Whisper models run on the CPU on Windows.** This build leaves out Whisper's Vulkan GPU support to avoid a build dependency, so the larger Whisper models can be much slower than real time. Use Parakeet for everyday dictation.

### Build & distribution
- **Windows NSIS installer** with **signed automatic updates** from GitHub Releases.
- **Portable mode** (inherited from Handy): put an empty file named `portable` next to `korero.exe` and Kōrero keeps its settings, models and recordings in a `Data` folder beside it instead of in `%APPDATA%`.

---

## Install

Download **`Korero_<version>_x64-setup.exe`** from the
[latest release](https://github.com/nkeating-cmd/korero/releases/latest) and run it, or build from source below.
After that, Kōrero keeps itself up to date: it checks GitHub once at startup and offers each new version.
Your settings, meetings and recordings are kept.

The installer isn't code-signed yet, so Windows SmartScreen may warn you: choose **More info → Run anyway**.
Updates are protected separately: the app installs one only if its signature matches.

WebView2 runtime is required (ships with Windows 11 / Microsoft Edge).

## Build from source

Kōrero is built and tested on Windows only.

Prerequisites: Rust (stable), Bun, Visual Studio 2022 Build Tools (C++ x64), LLVM, and CMake, plus the
[Tauri prerequisites](https://tauri.app/start/prerequisites/).

```powershell
bun install

# live dev (hot reload)
bun run tauri dev

# tests
bun run test:unit
cd src-tauri; cargo test --locked; cd ..

# installer, built the way CI builds it (unsigned, no update files)
bun run tauri build --bundles nsis --config .github/ci-no-updater.json
```

The installer lands in `src-tauri\target\release\bundle\nsis\`. A plain `bun run tauri build` also writes
update-signature files, so it needs the release signing key; use the `--config` line above instead.

MSI is intentionally not built — WiX 3 can't handle the macron in the product name — so the
NSIS `.exe` is the installer. [`BUILD.md`](BUILD.md) is upstream Handy's guide; it also covers macOS
and Linux, which Kōrero doesn't test.

## Licence & attribution

MIT — Copyright (c) 2025 CJ Pais and Handy contributors (upstream); fork changes
Copyright (c) 2026 Nic Keating. Retaining the upstream notice is required under MIT.
Third-party components and model licences are listed in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

Kōrero is not affiliated with or endorsed by CJ Pais or the Handy project.
