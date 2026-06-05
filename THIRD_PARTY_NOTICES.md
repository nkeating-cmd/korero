# Third-Party Notices

Kōrero is a personal fork of **Handy** and is distributed under the same MIT terms.
The upstream copyright notice is retained in `LICENSE` (Copyright (c) 2025 CJ Pais).

This file credits the projects Kōrero builds on. Licences below are stated to the
best of current knowledge; **verify each project's own licence text before any
redistribution** — some entries are marked _(verify)_ where the exact SPDX
identifier has not been independently confirmed in this repo.

## Upstream

| Project | Author | Licence | Link |
|---|---|---|---|
| **Handy** (the project this is forked from) | CJ Pais | MIT (verified — see `LICENSE`) | https://github.com/cjpais/Handy · https://handy.computer |

Kōrero's changes vs Handy: rebrand (Kōrero, aurora/glass theme, Aptos-led fonts),
NZ-English + te-reo default custom-word seed, default post-processing prompt set,
latch recording mode, startup-latency and window-state fixes, OS-keychain secret
storage, and Windows build hardening. See the commit history for specifics.

## Bundled / linked components

| Component | Licence | Link |
|---|---|---|
| Tauri (framework) | MIT / Apache-2.0 | https://tauri.app |
| whisper.cpp (via `transcribe-rs`) | MIT | https://github.com/ggerganov/whisper.cpp |
| nnnoiseless (optional mic denoiser, RNNoise port) | BSD-3-Clause | https://github.com/jneem/nnnoiseless |
| `transcribe-rs` | _(verify)_ | crate on crates.io |
| Silero VAD (via `vad-rs`, cjpais fork) | MIT _(verify the fork)_ | https://github.com/snakers4/silero-vad |
| ONNX Runtime (`ort`) | MIT | https://github.com/microsoft/onnxruntime |
| React | MIT | https://react.dev |
| Vite | MIT | https://vitejs.dev |
| Tailwind CSS | MIT | https://tailwindcss.com |
| lucide icons | ISC | https://lucide.dev |

## Models (downloaded at runtime — NOT redistributed here or in the installer)

Speech models are fetched on first use to the user's app-data directory. Kōrero
does **not** bundle or redistribute model weights; each model is subject to its
own licence at the point of download:

- **Whisper** (ggml/`.bin`) — OpenAI Whisper weights, MIT _(verify per model)_.
- **Parakeet V3** (`nvidia/parakeet-tdt-0.6b-v3`, redistributed as an int8 ONNX export) — **CC-BY-4.0** (verified 2026-05-30; "ready for commercial/non-commercial use"). Redistributable with attribution. The offline portable edition bundles this model and ships a `PARAKEET-MODEL-LICENCE.txt` crediting NVIDIA, linking CC-BY-4.0, and noting the int8 quantization as a modification.

## Fonts

- **Aptos** is **not bundled**. The UI requests Aptos and falls back to Segoe UI
  Variable / system fonts when it is absent. Aptos itself ships with Windows 11 /
  Microsoft 365 under Microsoft's terms and is not redistributed by Kōrero.
