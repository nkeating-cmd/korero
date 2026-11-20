# Clean-up bake-off: the whole catalogue, 20 November 2026

Harness: `scripts/cleanup-bakeoff.py` at `cf7a39b` (sha256 `b2e8d98d…f44a`): 16 invented fixtures, 3 runs each,
Kōrero's default prompt "Clean transcript (NZ English)" byte for byte, `/api/chat` with thinking off, as a dictation sends
it. Machine: legion-26 (RTX 4080 12 GB). gemma4:12b ran on the user's own Ollama 0.40.16 (17 Nov); the other four on
Kōrero's own copy of Ollama 0.40.16 with its own models folder (20 Nov, `C:\dev\korero-builds\bakeoff-1.49\`). All five
re-scored with the same harness.

| Model | Pass, temp 1.0 (Kōrero's setting) | Pass, temp 0.2 | p50 / p95 | What fails |
|---|---|---|---|---|
| gemma4:12b | 40 / 48 | 42 / 48 | 0.5 / 0.9 s | Only capitals or a full stop on short sentences ("send it to Sarah") |
| gemma4:26b | 40 / 48 | 41 / 48 | 0.5 / 0.9–1.9 s | The same, only |
| qwen3.5:9b | 23 / 48 | 30 / 48 | 0.3 / 0.6 s | Capitals; makes one-line lists of plain sentences (5–10 of 48); kept a filler or self-correction |
| qwen3.5:4b | 17 / 48 | 24 / 48 | 0.2 / 0.4 s | Makes lists of plain sentences (18 of 48); left numbers as words; changed words ("thanks Mere" became "Thanks Māori"; "Saturday" lost) |
| qwen3.5:2b | 6 / 48 | 6 / 48 | 0.2 / 3–4 s | **Pasted Kōrero's own instructions back as the answer (3–5 of 48)**; American spelling kept; numbers not converted; lists everywhere |

What this says:

- **Both gemma4 models are safe for clean-up.** Nothing invented, translated, prefixed or reworded; NZ spelling, numbers,
  spoken punctuation, lists and kept bullets all pass. Their only failures are capitals and full stops on short sentences.
- **qwen3.5:2b is not safe.** Pasting the prompt's instructions into someone's document is the worst failure a clean-up can
  have. It should come off the catalogue.
- **qwen3.5:4b changes meaning** (a name became another word) and **qwen3.5:9b and 4b invent lists**. Neither is a good
  default. Whether to keep them, with a warning, for computers too small for gemma4:12b is a product call: without them the
  smallest offer is gemma4:12b (8 GB download, needs about 10 GB of memory).
- **Temperature 0.2 beat 1.0 for every model** (by 1 to 7 passes out of 48). One model could be noise; five in the same
  direction is a pattern worth testing as a change to Kōrero's dictation setting.

Read with care:

- Three scoring rules were set after reading answers: capitals and full stops (17 Nov), and on 20 Nov the two fixes in
  `cf7a39b` (a faithful "Okay, let's get started" was scored as a preamble; a sentence wrapped as a one-item list was
  scored as missing capitals instead of its own failure). Neither 20 Nov fix changed any gemma4 pass count by more than 3,
  and the qwen ordering is the same under every version of the rules.
- 16 fixtures, 3 runs, one machine, one prompt. It ranks the models; it does not prove a model is good on real dictation.
- Model downloads (29 GB) stay in `C:\dev\korero-builds\bakeoff-1.49\models\` until deleted.
