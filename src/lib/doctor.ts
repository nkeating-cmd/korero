/**
 * Kōrero 1.49 (B10): the page side of "Check local AI".
 *
 * The checks themselves are worked out in Rust (`src-tauri/src/doctor`).
 * This file decides how each fix is offered and how safe it is to run, the
 * headline for each verdict, and the "Send report to Kyt" email. Pure, so it
 * is unit-tested (unit-tests/doctor.test.ts).
 */

import type { CheckResult, DoctorReport, Fix, Verdict } from "../bindings";

/**
 * How a fix is run:
 * - `auto`: starts something already on this computer; runs on the click.
 * - `confirm`: downloads or changes settings; the button says what it costs.
 * - `guide`: nothing in Kōrero can fix it; the page explains.
 */
export type Safety = "auto" | "confirm" | "guide";

export interface FixOffer {
  safety: Safety;
  /** Button text (`confirm` ones say the cost). Empty for `guide`. */
  button: string;
  /** Plain steps or context shown under the problem. */
  help: string;
}

export const KYT_SUPPORT_EMAIL = "nic@kyt.nz";

const size = (mb: number) => (mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`);

export function fixOffer(f: Fix): FixOffer {
  switch (f.kind) {
    case "start_runtime":
      return { safety: "auto", button: "Start it", help: "" };
    case "start_ollama":
      return { safety: "auto", button: "Start Ollama", help: "" };
    case "turn_on_cleanup":
      return { safety: "confirm", button: "Turn on clean-up", help: "" };
    case "install_runtime":
      return {
        safety: "confirm",
        button: f.download_mb > 0 ? `Add local AI to Kōrero (${size(f.download_mb)})` : "Add local AI to Kōrero",
        help:
          "Kōrero downloads Ollama, a free AI runtime, from its official release and checks every file. It runs only on this computer and stays inside Kōrero's own folder.",
      };
    case "use_runtime":
      return {
        safety: "confirm",
        button: "Use Kōrero's local AI",
        help: "Kōrero's own local AI is already downloaded. This points clean-up at it.",
      };
    case "install_ollama":
      return {
        safety: "guide",
        button: "",
        help: "Install Ollama from ollama.com, then run the check again.",
      };
    case "update_ollama":
      return {
        safety: "guide",
        button: "",
        help: "Update Ollama from ollama.com (or quit it and reopen it to let it update), then run the check again.",
      };
    case "download_model":
      return { safety: "confirm", button: `Download ${f.model}`, help: "The size is shown before it starts." };
    case "choose_model":
      return {
        safety: "guide",
        button: "",
        help: 'Choose a model under "Run AI clean-up on this computer" above. The recommended one fits this computer.',
      };
    case "proxy_help":
      return {
        safety: "guide",
        button: "",
        help:
          "Your network seems to use a proxy. Kōrero 1.49 talks to its local AI directly, so this may already be fixed: run the check again. If it still fails, ask your IT team to let programs on this computer reach 127.0.0.1 without the proxy.",
      };
    case "port_taken":
      return {
        safety: "guide",
        button: "",
        help: `Another program is using port ${f.port}. Close it, or let Kōrero run its own local AI on a free port.`,
      };
    case "free_memory":
      return {
        safety: "guide",
        button: "",
        help: "Close programs you aren't using (browsers with many tabs, video calls), or choose a smaller model.",
      };
  }
}

export interface Headline {
  tone: "ok" | "warn" | "off";
  text: string;
}

export function headline(v: Verdict): Headline {
  switch (v) {
    case "working":
      return { tone: "ok", text: "Local clean-up is working." };
    case "looks_set_up":
      return { tone: "ok", text: "Local clean-up looks set up. Run the full check to test a real answer." };
    case "needs_attention":
      return { tone: "warn", text: "Local clean-up needs attention." };
    case "not_local":
      return { tone: "off", text: "Clean-up uses a cloud provider, so there is nothing local to check." };
    case "off":
      return { tone: "off", text: "AI clean-up is off." };
  }
}

/** The step to fix first: the first failure, else the first warning with a fix. */
export function firstProblem(checks: CheckResult[]): CheckResult | null {
  return (
    checks.find((c) => c.status === "fail") ??
    checks.find((c) => c.status === "warn" && c.fix !== null) ??
    null
  );
}

/**
 * The email the person reviews and sends themselves. Only the report (fixed
 * fields, built in Rust); nothing is sent by Kōrero. Kept under the length
 * mail programs accept in a `mailto:` link.
 */
export function reportMailto(r: DoctorReport, to = KYT_SUPPORT_EMAIL): string {
  const subject = `Kōrero local AI check: ${r.verdict.replace(/_/g, " ")}`;
  const intro =
    "Kia ora Kyt,\n\nKōrero's local AI check found the following. This report holds no dictation, names or keys.\n\n";
  const max = 1800;
  let body = intro + r.report_text;
  if (body.length > max) body = `${body.slice(0, max)}\n(cut short)`;
  return `mailto:${to}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
}
