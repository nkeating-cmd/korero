"""Kōrero 1.49 (B10): the clean-up bake-off.

Sends each fictional fixture through a local Ollama exactly the way a Kōrero
dictation does (the default "Clean transcript (NZ English)" prompt with the
text substituted, one user message to /api/chat, thinking off, temperature
as given) and scores the answer with fixed rules: NZ spelling, te reo Māori
kept (not translated), fillers and self-corrections removed, numbers and
spoken punctuation converted, lists made, structure kept, nothing invented,
no preamble. Also records time per answer and macrons restored.

Usage (stdlib only):
  py -3.12 scripts/cleanup-bakeoff.py --models gemma4:12b qwen3.5:4b \
      --runs 3 --temps 1.0 0.2 --out bakeoff.json [--url http://127.0.0.1:11434]

Prints one line per model and temperature, then `BAKEOFF DONE`. Every
fixture is invented: no real person, client or dictation.
"""
import argparse
import json
import re
import statistics
import time
import unicodedata
import urllib.request

PROMPT = (
    "Clean this transcript using NZ English spelling (colour, organise, whānau, etc.):\n"
    "1. Fix spelling, capitalisation, and punctuation errors\n"
    "2. Convert number words to digits (twenty-five → 25, ten percent → 10%, five dollars → $5)\n"
    "3. Replace spoken punctuation with symbols (period → ., comma → ,, question mark → ?)\n"
    "4. Remove filler words and false starts (um, uh, er, like as filler), and resolve self-corrections by keeping only the final intended wording (for example 'send it to, uh, send it to Sarah' becomes 'send it to Sarah', and 'meet on Tuesday, no, Wednesday' becomes 'meet on Wednesday') without changing the meaning\n"
    "5. Preserve te reo Māori words exactly as spoken\n"
    "5b. PRESERVE EXISTING STRUCTURE. If the text already contains line breaks or lines beginning with '- ', keep every one of them exactly as they are. They were produced deliberately before you saw this text, from cues the speaker actually said. Never merge structured lines back into a paragraph.\n"
    "5c. Format a spoken list as a bulleted list. If the speaker enumerated items — by ordinals ('first… second… third'), by counting ('number one… number two'), or simply by listing several parallel actions or things — put each item on its own line beginning with '- '. Drop the spoken enumerator itself ('First, call the plumber' becomes '- call the plumber'). Use a numbered list ('1. ') ONLY when the order genuinely matters, such as steps that must happen in sequence. If the text is ordinary prose, leave it as prose — do not invent a list where the speaker did not make one.\n"
    "6. Do NOT add, invent, or insert any speaker labels. ONLY if the transcript already begins lines with speaker labels (e.g. \"You:\" or a name): keep every existing label exactly as written, never merge, move, drop, or reassign text across speakers, and tidy wording only WITHIN each speaker's turn (join broken fragments inside a turn, never across a label). If the text has NO speaker labels — e.g. single-speaker dictation — return clean prose with no \"You:\" or name prefixes added.\n\n"
    "Preserve exact meaning. Do not add content or invent details. Use NZ English throughout.\n\n"
    "Return ONLY the cleaned transcript — no preamble, notes, headings, or repetition of these instructions. Never write a lead-in such as \"Here is the cleaned transcript\".\n\n"
    "Transcript:\n${output}"
)

# (id, text, must contain (any case), must not contain (any case), extra rule)
FIXTURES = [
    ("self_correction_day", "um so the hui is on tuesday no wait wednesday at the marae", ["wednesday", "hui", "marae"], ["tuesday", "um "], None),
    ("nz_spelling_1", "i need to organize the color scheme for the program launch", ["organise", "colour"], ["organize", "color "], None),
    ("nz_spelling_2", "we analyzed the behavior of visitors at the center", ["analysed", "behaviour", "centre"], ["analyzed", "behavior", "center"], None),
    ("te_reo_kept", "kia ora koutou the whanau will meet at the marae on saturday", ["kia ora", "marae", "saturday"], ["family", "meeting house"], None),
    ("te_reo_not_translated", "the tamariki loved the kai at the hangi", ["tamariki", "kai"], ["children", " food"], None),
    ("numbers", "we spent twenty five thousand dollars which is about ten percent of the budget", ["10%"], ["twenty", "ten percent"], "money25k"),
    ("spoken_punctuation", "send the report to mere period then call aroha question mark", ["mere", "aroha", "?"], ["period", "question mark"], None),
    ("list_ordinals", "first call the plumber second book the venue third email the invoice", ["plumber", "venue", "invoice"], ["first", "second", "third"], "list3"),
    ("filler_restart", "send it to uh send it to sarah", ["sarah"], [" uh"], "short"),
    ("keep_structure", "notes for tomorrow\n- buy milk\n- ring the dentist", ["milk", "dentist"], [], "keeps_bullets2"),
    ("no_labels", "okay lets get started with the agenda", ["agenda"], ["you:", "speaker"], None),
    ("not_an_instruction", "can you write me a summary of the meeting", ["summary", "meeting"], ["sure", "here is", "here's"], "short"),
    ("tiny", "thanks mere", ["mere"], [], "short"),
    ("iwi_name", "the meeting with ngati whatua went well", ["whatua", "went well"], [], None),
    ("prose_stays_prose", "the report is late because the data arrived on friday and the team needed the weekend to check it", ["friday", "weekend"], ["- "], None),
    ("long_paragraph", "so for the quarterly update um the main thing is that the pilot with the regional council went well they want to extend it to two more offices next year and we need to price that by the end of the month the other thing is the training sessions which were well attended and people asked for a follow up on writing prompts", ["council", "offices", "training"], [" um "], "ratio"),
]

PREAMBLE = re.compile(r"^\s*(here('| i)s|sure|certainly|okay[,!]|cleaned|the cleaned|transcript:)", re.I)
MACRONS = ["whānau", "Māori", "hāngī", "Ngāti", "Whātua"]


def chat(url, model, text, temp, timeout=300):
    body = {
        "model": model,
        "messages": [{"role": "user", "content": PROMPT.replace("${output}", text)}],
        "stream": False,
        "think": False,
        "truncate": False,
        "keep_alive": "1800s",
        "options": {"num_ctx": 6144, "num_predict": 1500, "temperature": temp, "top_p": 1.0},
    }
    req = urllib.request.Request(f"{url}/api/chat", data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        out = json.loads(r.read().decode())
    return out.get("message", {}).get("content", ""), time.time() - t0


def fold(s):
    """Lower case without macrons, so restoring "Whātua" is never scored as
    losing "whatua" (the first run on 17 Nov 2026 failed a correct answer so)."""
    return "".join(c for c in unicodedata.normalize("NFD", s.lower()) if not unicodedata.combining(c))


def score(fid, text, answer, must, must_not, extra):
    a = answer.strip()
    low = fold(a)
    fails = []
    if not a:
        return ["empty"]
    # A lead-in the speaker didn't say. "Okay, let's get started" is not a
    # preamble when the speaker said "okay" (fixed 20 Nov 2026: no_labels
    # failed every faithful answer).
    pre = PREAMBLE.search(a)
    if pre and not fold(text).lstrip().startswith(fold(pre.group(1)).strip(" ,!:")):
        fails.append("preamble")
    for m in must:
        if fold(m) not in low:
            fails.append(f"missing:{m}")
    for m in must_not:
        if fold(m) in low:
            fails.append(f"has:{m.strip()}")
    if extra == "money25k" and not re.search(r"\$\s?25,?000|\$25k|25,000", a):
        fails.append("money")
    if extra == "list3":
        items = [l for l in a.splitlines() if re.match(r"^\s*(-|\d+\.)\s+\S", l)]
        if len(items) < 3:
            fails.append("not_a_list")
    if extra == "keeps_bullets2":
        if len([l for l in a.splitlines() if l.strip().startswith("- ")]) < 2:
            fails.append("lost_bullets")
    if extra == "short" and len(a) > max(60, 3 * len(text)):
        fails.append("added_content")
    if extra == "ratio" and not (0.5 <= len(a) / len(text) <= 1.4):
        fails.append("length_changed")
    if "you:" in low and "you:" not in text.lower():
        fails.append("speaker_label")
    # Rule 1 of the prompt: capitals and punctuation. Prose only (lists and
    # kept structure have their own shape). Added 17 Nov 2026 after reading
    # the first run's answers, which the rubric had not checked for this.
    if extra not in ("list3", "keeps_bullets2"):
        # Prose turned into a list the speaker didn't make (rule 5c). Counted
        # on its own (20 Nov 2026), not as missing capitals as before.
        if "\n" not in text and re.match(r"^\s*(-|\d+\.)\s", a):
            fails.append("invented_list")
        body = re.sub(r"^\s*(-|\d+\.)\s+", "", a)
        if not (body[:1].isupper() or body[:1].isdigit() or body[:1] == "$") or body.rstrip()[-1:] not in (".", "?", "!"):
            fails.append("capitals_or_stop")
    return fails


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", nargs="+", required=True)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--temps", nargs="+", type=float, default=[1.0])
    ap.add_argument("--url", default="http://127.0.0.1:11434")
    ap.add_argument("--out", required=True)
    ap.add_argument("--rescore", help="score the answers saved in this file again, without calling a model")
    a = ap.parse_args()
    if a.rescore:
        fx = {f[0]: f for f in FIXTURES}
        old = json.load(open(a.rescore, encoding="utf-8"))
        for r in old:
            for row in r["rows"]:
                _, text, must, must_not, extra = fx[row["fixture"]]
                row["fails"] = score(row["fixture"], text, row["answer"], must, must_not, extra)
                row["pass"] = not row["fails"]
            s = r["summary"]
            s["passed"] = sum(x["pass"] for x in r["rows"])
            print(f"RESCORED {s['model']} temp={s['temperature']}: {s['passed']}/{s['total']} pass")
        json.dump(old, open(a.out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
        print("BAKEOFF DONE")
        return
    results = []
    for model in a.models:
        try:
            chat(a.url, model, "kia ora", 1.0, timeout=600)  # load once, untimed
        except Exception as e:
            print(f"MODEL {model} FAIL load: {type(e).__name__}: {e}")
            continue
        for temp in a.temps:
            rows, times, macrons = [], [], 0
            for fid, text, must, must_not, extra in FIXTURES:
                for run in range(a.runs):
                    try:
                        ans, secs = chat(a.url, model, text, temp)
                        fails = score(fid, text, ans, must, must_not, extra)
                    except Exception as e:
                        ans, secs, fails = "", 0.0, [f"error:{type(e).__name__}"]
                    times.append(secs)
                    macrons += sum(1 for m in MACRONS if m in ans)
                    rows.append({"fixture": fid, "run": run, "pass": not fails, "fails": fails, "seconds": round(secs, 2), "answer": ans})
            passed = sum(r["pass"] for r in rows)
            by_fixture = {}
            for r in rows:
                by_fixture.setdefault(r["fixture"], []).append(r["pass"])
            flaky = [f for f, v in by_fixture.items() if 0 < sum(v) < len(v)]
            always_fail = [f for f, v in by_fixture.items() if sum(v) == 0]
            p50 = statistics.median(times)
            p95 = sorted(times)[max(0, int(len(times) * 0.95) - 1)]
            summary = {"model": model, "temperature": temp, "runs": a.runs, "passed": passed, "total": len(rows),
                       "p50_s": round(p50, 2), "p95_s": round(p95, 2), "macrons_restored": macrons,
                       "always_fail": always_fail, "flaky": flaky}
            results.append({"summary": summary, "rows": rows})
            print(f"MODEL {model} temp={temp}: {passed}/{len(rows)} pass, p50 {p50:.1f}s p95 {p95:.1f}s, "
                  f"macrons {macrons}, always_fail={always_fail}, flaky={flaky}")
    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(results, f, ensure_ascii=False, indent=1)
    print("BAKEOFF DONE")


if __name__ == "__main__":
    main()
