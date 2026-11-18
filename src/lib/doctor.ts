/**
 * Kōrero 1.49 (B10): the page side of "Check local AI".
 *
 * The checks themselves are worked out in Rust (`src-tauri/src/doctor`).
 * This file decides how each fix is offered and how safe it is to run, the
 * headline for each verdict, and the "Send report to Kyt" email. Pure, so it
 * is unit-tested (unit-tests/doctor.test.ts).
 */

import type { CheckResult, DoctorReport, Fix, Verdict } from "../bindings";
import { CATALOGUE } from "./modelCatalogue";

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

/** A model's download size, when the catalogue knows it (tags match with or without `:latest`). */
export function modelDownloadMb(model: string): number | null {
  const norm = (t: string) => t.trim().toLowerCase().replace(/:latest$/, "");
  return CATALOGUE.find((e) => norm(e.tag) === norm(model))?.downloadMb ?? null;
}

export function fixOffer(f: Fix): FixOffer {
  switch (f.kind) {
    case "start_runtime":
      return { safety: "auto", button: "Start it", help: "" };
    case "start_ollama":
      return { safety: "auto", button: "Start Ollama", help: "" };
    case "turn_on_cleanup":
      return { safety: "confirm", button: "Turn on clean-up", help: "Clean-up will run on this computer." };
    case "install_runtime":
      return {
        safety: "confirm",
        button: f.download_mb > 0 ? `Add local AI to Kōrero (${size(f.download_mb)})` : "Add local AI to Kōrero",
        help:
          "Kōrero downloads Ollama, a free AI runtime, from its official release and checks every file. It runs only on this computer and stays inside Kōrero's own folder.",
      };
    case "update_runtime":
      return {
        safety: "confirm",
        button: f.download_mb > 0 ? `Update Kōrero's local AI (${size(f.download_mb)})` : "Update Kōrero's local AI",
        help: "Clean-up keeps working on the earlier version until the update is downloaded and checked; then the earlier one is removed.",
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
    case "download_model": {
      // Review M5: the button says what it costs before anything downloads.
      const mb = modelDownloadMb(f.model);
      return {
        safety: "confirm",
        button: mb ? `Download ${f.model} (${size(mb)})` : `Download ${f.model} (size not known)`,
        help: mb
          ? "It downloads once and stays on this computer. You can cancel it."
          : "Kōrero doesn't know this model's size. Large models can be 10 GB or more. You can cancel it.",
      };
    }
    case "choose_model":
      return {
        safety: "guide",
        button: "",
        help: 'Choose a model under "Run AI clean-up on this computer" above. The recommended one fits this computer.',
      };
    case "choose_prompt":
      return {
        safety: "guide",
        button: "",
        help: 'Choose a clean-up prompt under "Prompts" in these settings, and make sure it has text. "Clean transcript (NZ English)" is the usual one.',
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
      return { tone: "off", text: "Clean-up doesn't use Kōrero's local AI or Ollama on this computer, so there is nothing here to check." };
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
  // The limit is on the encoded link (macrons and new lines grow when
  // encoded), so measure that, not the text.
  const max = 1900;
  const link = (b: string) => `mailto:${to}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(b)}`;
  const full = intro + r.report_text;
  if (link(full).length <= max) return link(full);
  let keep = full.length;
  while (keep > 0 && link(`${full.slice(0, keep)}\n(cut short)`).length > max) keep -= 20;
  return link(`${full.slice(0, Math.max(0, keep))}\n(cut short)`);
}
