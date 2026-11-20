// Kōrero 1.49: how "Check local AI" offers its fixes. Run with `bun test unit-tests`.

import { describe, expect, test } from "bun:test";
import type { CheckResult, DoctorReport, Fix, Verdict } from "../src/bindings";
import { firstProblem, fixOffer, headline, reportMailto } from "../src/lib/doctor";

const ALL_FIXES: Fix[] = [
  { kind: "turn_on_cleanup" },
  { kind: "start_runtime" },
  { kind: "start_ollama" },
  { kind: "install_runtime", download_mb: 40 },
  { kind: "update_runtime", download_mb: 650 },
  { kind: "use_runtime" },
  { kind: "install_ollama" },
  { kind: "update_ollama" },
  { kind: "download_model", model: "gemma4:12b" },
  { kind: "choose_model" },
  { kind: "choose_prompt" },
  { kind: "proxy_help" },
  { kind: "port_taken", port: 11435 },
  { kind: "free_memory" },
];

const check = (status: CheckResult["status"], fix: Fix | null = null, id: CheckResult["id"] = "runtime"): CheckResult => ({
  id,
  status,
  title: "t",
  detail: "d",
  fix,
});

describe("fixOffer", () => {
  test("only starting something runs without asking", () => {
    for (const f of ALL_FIXES) {
      const o = fixOffer(f);
      const auto = f.kind === "start_runtime" || f.kind === "start_ollama";
      expect(o.safety === "auto").toBe(auto);
    }
  });

  test("anything that downloads says what, and the runtime says how much", () => {
    expect(fixOffer({ kind: "install_runtime", download_mb: 40 }).button).toContain("40 MB");
    expect(fixOffer({ kind: "install_runtime", download_mb: 649 }).button).toContain("649 MB");
    expect(fixOffer({ kind: "install_runtime", download_mb: 1543 }).button).toContain("1.5 GB");
    // Review M5: a model download says its size on the button, or says it isn't known.
    expect(fixOffer({ kind: "download_model", model: "gemma4:12b" }).button).toBe("Download gemma4:12b (8.0 GB)");
    expect(fixOffer({ kind: "download_model", model: "qwen3.5:4b:latest" }).button).toContain("3.4 GB");
    expect(fixOffer({ kind: "download_model", model: "someone/odd-model" }).button).toContain("size not known");
    expect(fixOffer({ kind: "install_runtime", download_mb: 40 }).safety).toBe("confirm");
    expect(fixOffer({ kind: "update_runtime", download_mb: 650 }).button).toBe("Update Kōrero's local AI (650 MB)");
  });

  test("guides have help and no button; buttons have words", () => {
    for (const f of ALL_FIXES) {
      const o = fixOffer(f);
      if (o.safety === "guide") {
        expect(o.button).toBe("");
        expect(o.help.length).toBeGreaterThan(20);
      } else {
        expect(o.button.length).toBeGreaterThan(3);
      }
    }
  });

  test("a taken port is named", () => {
    expect(fixOffer({ kind: "port_taken", port: 11436 }).help).toContain("11436");
  });

  test("no American spelling, no exclamations", () => {
    for (const f of ALL_FIXES) {
      const o = fixOffer(f);
      expect(`${o.button} ${o.help}`).not.toMatch(/color|organiz|licens(e|ing) |!/);
    }
  });
});

describe("headline", () => {
  test("only a full check is called working", () => {
    const vs: Verdict[] = ["working", "looks_set_up", "needs_attention", "not_local", "off"];
    for (const v of vs) expect(headline(v).text.length).toBeGreaterThan(10);
    expect(headline("working").text).toContain("working");
    expect(headline("looks_set_up").text).not.toMatch(/is working/);
    expect(headline("needs_attention").tone).toBe("warn");
  });
});

describe("firstProblem", () => {
  test("a failure beats a warning, and a warning needs a fix to count", () => {
    expect(firstProblem([check("pass"), check("warn", null), check("pass")])).toBeNull();
    const w = check("warn", { kind: "free_memory" }, "memory_now");
    const f = check("fail", { kind: "start_runtime" }, "runtime");
    expect(firstProblem([check("pass"), w, f])).toBe(f);
    expect(firstProblem([check("pass"), w])).toBe(w);
    expect(firstProblem([check("not_checked")])).toBeNull();
  });
});

describe("reportMailto", () => {
  const report = (text: string): DoctorReport => ({
    mode: "full",
    verdict: "needs_attention",
    checks: [],
    report_text: text,
    took_ms: 10,
  });

  test("goes to Kyt, carries the report, and greets in te reo", () => {
    const url = reportMailto(report("Kōrero local AI check: needs_attention\nChecks:\n  FAIL runtime\n"));
    expect(url.startsWith("mailto:nic@kyt.nz?subject=")).toBe(true);
    const body = decodeURIComponent(url.split("&body=")[1]);
    expect(body).toContain("Kia ora");
    expect(body).toContain("FAIL runtime");
    expect(decodeURIComponent(url.split("subject=")[1].split("&")[0])).toBe("Kōrero local AI check: needs attention");
  });

  test("a long report is cut to a length mail programs accept", () => {
    const body = decodeURIComponent(reportMailto(report("x".repeat(5000))).split("&body=")[1]);
    expect(body.length).toBeLessThan(1900);
    expect(body.endsWith("(cut short)")).toBe(true);
  });

  test("the limit is on the encoded link, which macrons and new lines make longer", () => {
    const heavy = "Kōrero ā ē ī ō ū\n".repeat(200);
    const url = reportMailto(report(heavy));
    expect(url.length).toBeLessThanOrEqual(1900);
    expect(decodeURIComponent(url.split("&body=")[1]).endsWith("(cut short)")).toBe(true);
    // Short as text, long once encoded.
    expect(reportMailto(report("ā\n".repeat(600))).length).toBeLessThanOrEqual(1900);
    const short = reportMailto(report("Kōrero local AI check: working\n"));
    expect(decodeURIComponent(short.split("&body=")[1])).not.toContain("cut short");
  });
});
