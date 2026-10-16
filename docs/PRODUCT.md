---
type: product-brief
app: korero
reviewed: 2026-10-16
owner: Nic (decides the order); dev-product skill (maintains this file)
source_of_truth: this file; detail in the Kōrero claude.ai project (KORERO_STATE.md, plans and build notes) and the backlog kept outside the repo
wip_limit: 3
---

# Kōrero: product brief

What this app is for, who it is for, and the order we build in. Checked by
`product-check.py` (kyt-dev-platform). A session starts feature work only on a bet in **Now**
(`product-check.py gate docs/PRODUCT.md <bet id>`); fixes and security work skip the gate.

## Customer

- **Who buys:** nobody yet. Kōrero is free and open source (a fork of Handy), released on GitHub.
- **Who uses:** Nic, on two Windows laptops. Release downloads suggest one or two installs in
  total (E3), so for now the customer is Nic.
- **Job to be done:** when Nic is in a meeting, on a call or in a room, he wants an accurate local record of who
  said what, and notes he can act on, so he can stay in the conversation and follow up without a cloud service
  hearing it. When dictating, he wants New Zealand English and te reo Māori written correctly.
- **What they use today:** Kōrero itself; before that, cloud meeting tools and typing notes by hand.
- **Biggest unknown:** whether anyone besides Nic needs Kōrero. Decided 20 Sept: external users are parked, so
  the brief optimises for Nic's own use until that changes.

## Outcomes

| ID | Outcome | Measure | Target date |
|---|---|---|---|
| O1 | Nothing said in a recorded meeting is lost | Over a month of Nic's meetings: no meeting with audio but an empty transcript, and no notes that silently skip part of the meeting | 2026-11-30 |
| O2 | In-room meetings come out with who said what | Speaker error on the AMI far-field test set at or under the bar Nic sets from M0, and Nic rates 3 real room meetings as usable | 2026-12-18 |
| O3 | New Zealand English and te reo Māori come out right | Te reo term accuracy on Nic's 108-line recording (never measured yet) | 2027-02-26 |
| O4 | Releases are safe and update themselves | Each release signed and gated; the second laptop offered the update on its own | 2026-11-30 |

## Evidence

Basis, in the same words Kriya uses: **research** (a real customer said or did it, or usage data shows it),
**requirement** (a law, rule, contract or accreditation obliges it), **hypothesis** (our belief, untested),
**simulated** (a persona panel or other AI output; never counts as proof). Quote the source so anyone can check it.
Sources named `KORERO_*` are in the Kōrero claude.ai project.

| ID | Basis | Source | Date | What it shows |
|---|---|---|---|---|
| E1 | research | Meetings store on the main laptop, counted for `KORERO_MEETINGS-RELIABILITY_2026-09-25.md` | 2026-09-25 | Of 60 meetings, 5 had audio but an empty transcript and 25 had a transcript but no notes; Nic reported failures and work stopping when he left the Meetings tab |
| E2 | research | Meetings store on the main laptop, `KORERO_1.44_FIX-PLAN_2026-10-08.md` | 2026-10-08 | Notes for 8 of 35 transcribed meetings covered only the first 48,000 characters and lost every flagged moment |
| E3 | research | GitHub release download counts, `KORERO_REVIEW-NEXT-ROUND_2026-10-01.md` | 2026-10-01 | v1.41.0: 1 installer download and 2 `latest.json` downloads in 6 days, so about 1–2 installs |
| E4 | research | Nic's request in the Kōrero project chat | 2026-10-16 | Nic wants meetings designed for in-person use, with speakers separated automatically |
| E5 | research | A 34-minute meeting on the main laptop (T1 in `KORERO_MEETINGS-RELIABILITY_2026-09-25.md`) | 2026-09-14 | The microphone peaked at RMS 0.0073, under the fixed 0.010 speech gate, so nothing was transcribed |
| E6 | research | Nic's report, diagnosed in `KORERO_1.44_FIX-PLAN_2026-10-08.md` | 2026-10-08 | Audio brief failed with "voice engine not found" |
| E7 | hypothesis | Kōrero project plan, 16 Oct | 2026-10-16 | In-room meetings will often be recorded on a phone and imported later |
| E8 | hypothesis | `KORERO_v1.40_CHARTER_2026-09-06.md` | 2026-09-06 | Te reo Māori terms are often transcribed wrongly, and a lexicon or model choice can fix it; never measured |
| E9 | hypothesis | `KORERO_STATE.md` ("still to prove") | 2026-10-02 | Installed copies update themselves; never observed on the second laptop |
| E10 | hypothesis | Kōrero project plan, 16 Oct | 2026-10-16 | Remembering voices across meetings (auto-labelling Nic) would be worth the privacy cost |
| E11 | requirement | Biometric Processing Privacy Code 2025 (Privacy Commissioner), in force 3 Nov 2025 | 2025-11-03 | Any feature that uses a stored voiceprint to recognise people must meet the code's rules; it constrains B7, it does not justify it |
| E12 | hypothesis | Kōrero project plan, 16 Oct | 2026-10-16 | Speakers can be told apart well enough from one laptop or room microphone to be useful |
| E13 | hypothesis | Kōrero project plan, 16 Oct | 2026-10-16 | Remote voices picked up by the room microphone from a loudspeaker can be removed in software well enough |

## Bets

Type: **build** (a production feature), **learn** (exists to produce evidence: an interview round, a demo, a
prototype, an experiment), **enable** (platform, security or compliance the product needs either way).
Value and Effort are 1 to 5. Score = Value x Confidence / Effort for build and enable (Confidence comes from the
strongest evidence: research or requirement 1, hypothesis 0.5, simulated 0.25); Value / Effort for learn.
A build or enable bet enters Now only on research or a requirement. Keep Now to `wip_limit` bets.

| ID | Bet | Type | Outcome | Evidence | Value | Effort | Status | Since | Link |
|---|---|---|---|---|---|---|---|---|---|
| B1 | Release 1.44: long-meeting notes read in parts; audio brief finds its engine | build | O1 | E2, E6 | 4 | 1 | now | 2026-10-08 | branch `fix/1.44`; `KORERO_1.44_BUILD_2026-10-08.md` |
| B2 | M0 probe: can speakers be separated on Kōrero's own runtime, from a room mic? | learn | O2 | E4, E12 | 4 | 1 | done | 2026-10-16 | `KORERO_1.45_IN-ROOM-SPEAKERS-PLAN_2026-10-16.md` |
| B3 | "In the room" meeting mode: mic only, no false call-audio warning, a speech gate that hears the far end of the table (M1) | build | O2, O1 | E4, E5 | 4 | 2 | now | 2026-10-16 | plan M1 |
| B4 | Separate speakers after Stop, then name, merge and reassign them; names reach the notes (M2–M3) | build | O2 | E4 | 5 | 4 | now | 2026-10-16 | plan M2–M3; `docs/DECISIONS.md` D-007 |
| B5 | Imports return timed segments and get speakers separated (M4) | build | O2 | E7 | 3 | 2 | next | 2026-10-16 | plan M4 |
| B6 | Confirm the second laptop is offered the next release on its own | learn | O4 | E9 | 3 | 1 | next | 2026-10-16 | `KORERO_STATE.md` |
| B7 | Remember voices across meetings | build | O2 | E10 | 2 | 4 | later | 2026-10-16 | plan D2; must meet E11 if built |
| B8 | Hybrid meetings (room plus remote people): stop remote speech appearing twice (M5) | learn | O2 | E4, E13 | 2 | 2 | later | 2026-10-16 | plan M5 |
| B9 | Record and score the 108-line te reo take; decide on the lexicon work from the number | learn | O3 | E8 | 3 | 2 | later | 2026-09-25 | `KORERO_PLAN_2026-09-25.md` |

## Decisions and why

Dated one-liners for every reorder, drop or gate decision, newest first. Larger product decisions go to
`docs/DECISIONS.md`.

- 2026-10-16: Nic: "Execute D007". B4 moves to Now beside B1 and B3 (3 of 3), and is built on its own branch from `main`, ahead of B3 (M1); D-007 is Accepted.
- 2026-10-16: First brief (dev-product Mode A). Proposed order, for Nic to confirm: B1 then B3 in Now; B6, B4 and B5
  in Next. Nic sets the order; this is the session's proposal.
- 2026-10-16: B2 done. M0 found in-room separation workable on the room microphone (mean error 23.1% on four AMI
  far-field meetings, collar 0.25), but only with a different embedding model from the one `pyannote-rs` ships
  (D-007, L-012). Nic accepted D-007 the same evening.
- 2026-10-16: The "How many people?" prompt was dropped from B3. Told the count, the clusterer did worse (31.5%) than
  finding the count itself (23.1%).
- 2026-10-16: B5 rests on a hypothesis (E7: room meetings get recorded on a phone), so it ranks below B4. Asking Nic
  how he records room meetings would settle it.
- 2026-10-16: **O3 has no bet in Now or Next (checker WARN).** B9's kill date (2 Oct, `KORERO_PLAN_2026-09-25.md`)
  passed with no record of the te reo take. Nic to decide: drop B9 and park O3, or schedule the 25-minute take.
- 2026-10-16: Fixes are not ranked here and skip the gate: cloud provider keys lost on restart (L-007) and the
  release line-ending stop (L-004) are the two open ones.
- 2026-09-20: External users parked (Nic): the brief optimises for Nic's own use until that changes.
