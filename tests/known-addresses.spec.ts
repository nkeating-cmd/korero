import { test, expect } from "@playwright/test";
import {
  argsOf,
  installTauriMock,
  invoked,
  SETTINGS,
} from "./support/tauriMock";

/**
 * Kōrero 1.51: "Your email addresses" and learning an address from a History
 * correction. Runs the real frontend against the stand-in backend; the
 * matching itself is tested in Rust (`known_addresses.rs`). Invented data only.
 */
const entry = {
  id: 1,
  file_name: "handy-1.wav",
  timestamp: 1763600000,
  saved: false,
  title: "",
  transcription_text: "Send it to nick@kite.co.nz please",
  post_processed_text: null,
  post_process_prompt: null,
  post_process_requested: false,
  post_process_failure: null,
};

test.describe("Known addresses", () => {
  let errors: string[];
  test.beforeEach(({ page }) => {
    errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
  });
  test.afterEach(() => {
    expect(errors, "uncaught errors on the page").toEqual([]);
  });

  test("an address added in Advanced is saved in lower case", async ({
    page,
  }) => {
    await installTauriMock(page, []);
    await page.goto("/");
    await page.getByText("Advanced", { exact: true }).first().click();
    await expect(
      page.getByText("Your email addresses", { exact: true }),
    ).toBeVisible();
    const box = page.getByRole("textbox", { name: "Your email addresses" });
    await box.fill("not an address");
    await box.press("Enter");
    await expect
      .poll(() => invoked(page))
      .not.toContain("update_known_addresses");
    await box.fill("Nic@Kyt.NZ");
    await box.press("Enter");
    await expect
      .poll(() => argsOf(page, "update_known_addresses"))
      .toEqual({ addresses: ["nic@kyt.nz"] });
  });

  test("correcting an address in History offers to remember it", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_history_entries: { entries: [entry], has_more: false },
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Dictation history" }).click();
    await expect(
      page.getByText("nick@kite.co.nz", { exact: false }).first(),
    ).toBeVisible();
    // Every dictation can be corrected, not only cleaned-up ones.
    await page.getByTitle("Correct this text").first().click();
    const editor = page.locator("textarea").first();
    await editor.fill("Send it to nic@kyt.nz please");
    await editor.press("Control+Enter");
    await expect
      .poll(() => argsOf(page, "update_post_processed_text"))
      .toEqual({ id: 1, text: "Send it to nic@kyt.nz please" });
    const card = page.getByRole("group", { name: "Remember this address?" });
    await expect(card).toBeVisible();
    await expect(card).toContainText("nick@kite.co.nz");
    await expect(card).toContainText("nic@kyt.nz");
    await card.getByRole("button", { name: "Remember" }).click();
    await expect
      .poll(() => argsOf(page, "update_known_addresses"))
      .toEqual({ addresses: ["nic@kyt.nz"] });
    await expect
      .poll(() => argsOf(page, "update_transcript_corrections"))
      .toEqual({
        corrections: [{ wrong: "nick@kite.co.nz", right: "nic@kyt.nz" }],
      });
    await expect(card).toBeHidden();
  });

  test("an edit that swaps no address offers nothing", async ({ page }) => {
    await installTauriMock(page, [], {
      get_history_entries: {
        entries: [{ ...entry, transcription_text: "The costs are high" }],
        has_more: false,
      },
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Dictation history" }).click();
    await page.getByTitle("Correct this text").first().click();
    const editor = page.locator("textarea").first();
    await editor.fill("The cost is high");
    await editor.press("Control+Enter");
    await expect
      .poll(() => invoked(page))
      .toContain("update_post_processed_text");
    await expect(
      page.getByRole("group", { name: "Remember this address?" }),
    ).toHaveCount(0);
  });

  test("two addresses corrected at once are both remembered", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_history_entries: {
        entries: [
          {
            ...entry,
            transcription_text: "Copy nick@kite.co.nz and mike@zero.com",
          },
        ],
        has_more: false,
      },
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Dictation history" }).click();
    await page.getByTitle("Correct this text").first().click();
    const editor = page.locator("textarea").first();
    await editor.fill("Copy nic@kyt.nz and mike@xero.com");
    await editor.press("Control+Enter");
    const card = page.getByRole("group", { name: "Remember this address?" });
    await card.getByRole("button", { name: "Remember" }).click();
    await expect
      .poll(() => argsOf(page, "update_transcript_corrections"))
      .toEqual({
        corrections: [
          { wrong: "nick@kite.co.nz", right: "nic@kyt.nz" },
          { wrong: "mike@zero.com", right: "mike@xero.com" },
        ],
      });
    await expect
      .poll(() => argsOf(page, "update_known_addresses"))
      .toEqual({ addresses: ["nic@kyt.nz", "mike@xero.com"] });
  });

  test("correcting your own address to another is never learnt", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_app_settings: { ...SETTINGS, known_addresses: ["nic@kyt.nz"] },
      get_history_entries: {
        entries: [
          { ...entry, transcription_text: "Send it to nic@kyt.nz please" },
        ],
        has_more: false,
      },
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Dictation history" }).click();
    await page.getByTitle("Correct this text").first().click();
    const editor = page.locator("textarea").first();
    await editor.fill("Send it to nick@kyt.nz please");
    await editor.press("Control+Enter");
    await expect
      .poll(() => invoked(page))
      .toContain("update_post_processed_text");
    await expect(
      page.getByRole("group", { name: "Remember this address?" }),
    ).toHaveCount(0);
  });

  test("saving an unchanged text saves nothing", async ({ page }) => {
    await installTauriMock(page, [], {
      get_history_entries: { entries: [entry], has_more: false },
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Dictation history" }).click();
    await page.getByTitle("Correct this text").first().click();
    await page.locator("textarea").first().press("Control+Enter");
    await expect(page.locator("textarea")).toHaveCount(0);
    expect(await invoked(page)).not.toContain("update_post_processed_text");
  });
});
