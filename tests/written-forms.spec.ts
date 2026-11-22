import { test, expect } from "@playwright/test";
import { installTauriMock, invoked } from "./support/tauriMock";

/**
 * Kōrero 1.50 (B11): the "Write numbers as figures" switch. Runs the real
 * frontend against the stand-in backend (tests/support/tauriMock.ts). The
 * rules themselves are tested in Rust (`audio_toolkit/written_forms.rs`).
 */
test.describe("Write numbers as figures", () => {
  test("is on for an older settings store, and switching it off saves", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    // The fixture settings have no written_forms_enabled, like a store from 1.49.
    await installTauriMock(page, []);
    await page.goto("/");
    await page.getByText("Advanced", { exact: true }).first().click();
    const title = page.getByText("Write numbers as figures", { exact: true });
    await expect(title).toBeVisible();
    const box = title.locator(
      "xpath=ancestor::*[.//input[@type='checkbox']][1]//input[@type='checkbox']",
    );
    await expect(box).toBeChecked();
    await box.dispatchEvent("click");
    await expect.poll(() => invoked(page)).toContain("change_written_forms_setting");
    expect(errors, "uncaught errors on the page").toEqual([]);
  });
});
