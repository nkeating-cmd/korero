import { test, expect, type Page } from "@playwright/test";
import { installTauriMock, invoked } from "./support/tauriMock";

/**
 * Kōrero 1.49 (B10): "Check local AI" and the offer to add Kōrero's own local
 * AI. Runs the real frontend against a stand-in backend
 * (tests/support/tauriMock.ts): it proves what the page says and which
 * command a button calls, not that a real Ollama starts (that is the Windows
 * UAT in docs/TESTING.md). All data is invented.
 */

const check = (id: string, status: string, title: string, detail: string, fix: unknown = null) => ({
  id,
  status,
  title,
  detail,
  fix,
});

const report = (verdict: string, checks: unknown[]) => ({
  mode: "quick",
  verdict,
  checks,
  report_text: "Kōrero local AI check: needs_attention (quick check)\nChecks:\n  FAIL runtime\n",
  took_ms: 12,
});

const runtimeStatus = (overrides: Record<string, unknown> = {}) => ({
  supported: true,
  version: "0.40.16",
  installed: [],
  plan: { version: "0.40.16", groups: ["core", "vulkan"], download_mb: 40, unpacked_mb: 121, nvidia: false, nvidia_driver: null },
  missing_mb: 40,
  free_disk_mb: 200000,
  running: false,
  port: 11435,
  in_use: false,
  installing: false,
  update_needed: false,
  disk_mb: 0,
  ...overrides,
});

const profile = { gpus: [], ram_mb: 16384, free_disk_mb: 200000, models_dir_custom: false, os: "windows" };

async function openCleanup(page: Page) {
  await page.goto("/");
  await page.getByText("AI clean-up & notes", { exact: true }).first().click();
}

test.describe("Check local AI", () => {
  let pageErrors: string[];
  test.beforeEach(({ page }) => {
    pageErrors = [];
    page.on("pageerror", (e) => pageErrors.push(e.message));
  });
  test.afterEach(() => {
    expect(pageErrors, "uncaught errors on the page").toEqual([]);
  });

  test("no local AI: the problem is said plainly and the fix states its size", async ({ page }) => {
    await installTauriMock(page, [], {
      local_ai_check: report("needs_attention", [
        check("cleanup_on", "pass", "Clean-up is on", "Clean-up uses Ollama (local)."),
        check("address_local", "pass", "Clean-up runs on this computer", "Clean-up talks to this computer (port 11434)."),
        check("runtime", "fail", "There's no local AI on this computer", "Ollama isn't installed.", {
          kind: "install_runtime",
          download_mb: 40,
        }),
        check("answers_as_ollama", "not_checked", "The local AI answers", "Fix the step above first."),
        check("recent", "pass", "Recent clean-ups", "No dictation has asked for a clean-up yet."),
      ]),
      local_ai_runtime_status: runtimeStatus(),
      ollama_status: { installed: false, running: false, exe_path: null },
      get_machine_profile: profile,
    });
    await openCleanup(page);
    await expect(page.getByText("Local clean-up needs attention.")).toBeVisible();
    await expect(page.getByText("There's no local AI on this computer", { exact: true }).first()).toBeVisible();
    const add = page.getByRole("button", { name: "Add local AI to Kōrero (40 MB)" });
    await expect(add).toBeVisible();
    // A step the engine marked "not checked" shows as that, not as passed.
    // (Which steps get marked is the Rust ladder's job, tested there:
    // `a_failure_never_lets_later_steps_pass`.)
    await expect(page.getByRole("img", { name: "Not checked" }).first()).toBeVisible();
    // Nothing downloads until the button is pressed.
    expect(await invoked(page)).not.toContain("local_ai_runtime_install");
    await add.click();
    await expect.poll(() => invoked(page)).toContain("local_ai_runtime_install");
    await expect.poll(() => invoked(page)).toContain("local_ai_runtime_use");
  });

  test("the setup card offers Kōrero's own local AI with its size, and IT text", async ({ page }) => {
    await installTauriMock(page, [], {
      local_ai_check: report("needs_attention", []),
      local_ai_runtime_status: runtimeStatus(),
      ollama_status: { installed: false, running: false, exe_path: null },
      get_machine_profile: profile,
    });
    await openCleanup(page);
    await expect(page.getByRole("button", { name: "Add local AI (40 MB)" })).toBeVisible();
    await page.getByRole("button", { name: "On a work laptop?" }).click();
    const it = page.getByText(/would like to run Ollama/);
    await expect(it).toBeVisible();
    await expect(it).toContainText("127.0.0.1");
    await expect(it).toContainText("No administrator rights");
  });

  test("working: no problem card, and the report goes by email only when asked", async ({ page }) => {
    await installTauriMock(page, [], {
      local_ai_check: report("working", [
        check("cleanup_on", "pass", "Clean-up is on", "Clean-up uses Ollama (local)."),
        check("test_answer", "pass", "A test clean-up works", "Answered in 2.1 s, 100% on the graphics card."),
      ]),
      local_ai_runtime_status: runtimeStatus({ installed: ["core", "vulkan"], missing_mb: 0, running: true, in_use: true }),
      ollama_status: { installed: true, running: true, exe_path: null },
      get_machine_profile: profile,
    });
    await openCleanup(page);
    await expect(page.getByText("Local clean-up is working.")).toBeVisible();
    await expect(page.getByRole("button", { name: /Add local AI/ })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Send report to Kyt" })).toBeVisible();
    expect(await invoked(page)).not.toContain("plugin:opener|open_url");
    // Only the quick check ran by itself; the full one waits for a click.
    const calls = await invoked(page);
    expect(calls.filter((c) => c === "local_ai_check").length).toBe(1);
  });

  test("Kōrero's own local AI can be repaired, switched off and removed, and removing asks first", async ({ page }) => {
    await installTauriMock(page, [], {
      local_ai_check: report("working", [
        check("cleanup_on", "pass", "Clean-up is on", "Clean-up uses Ollama (local)."),
        check("test_answer", "pass", "A test clean-up works", "Answered in 2.1 s."),
      ]),
      local_ai_runtime_status: runtimeStatus({
        installed: ["core", "vulkan"],
        missing_mb: 0,
        running: true,
        in_use: true,
        disk_mb: 8300,
      }),
      ollama_status: { installed: true, running: true, exe_path: null },
      get_machine_profile: profile,
    });
    await openCleanup(page);
    await expect(page.getByText(/Kōrero's local AI \(Ollama 0\.40\.16\) · 8\.1 GB/)).toBeVisible();
    await expect(page.getByRole("button", { name: "Repair" })).toBeVisible();
    await page.getByRole("button", { name: "Stop using it" }).click();
    await expect.poll(() => invoked(page)).toContain("local_ai_runtime_stop_using");
    await page.getByRole("button", { name: "Remove…" }).click();
    await expect(page.getByText("Remove Kōrero's local AI and its models? This frees 8.1 GB.")).toBeVisible();
    expect(await invoked(page)).not.toContain("local_ai_runtime_remove");
    await page.getByRole("button", { name: "Remove", exact: true }).click();
    await expect.poll(() => invoked(page)).toContain("local_ai_runtime_remove");
  });

  test("a download already running when the page opens is followed, not shown as an error", async ({ page }) => {
    await installTauriMock(page, [], {
      local_ai_check: report("needs_attention", [
        check("runtime", "fail", "There's no local AI on this computer", "Ollama isn't installed.", {
          kind: "install_runtime",
          download_mb: 40,
        }),
      ]),
      local_ai_runtime_status: runtimeStatus({ installing: true }),
      ollama_status: { installed: false, running: false, exe_path: null },
      get_machine_profile: profile,
    });
    await openCleanup(page);
    await expect(page.locator(`[role="progressbar"][aria-label="Downloading Kōrero's local AI"]:visible`).first()).toBeVisible();
    expect(await invoked(page)).not.toContain("local_ai_runtime_install");
  });
});
