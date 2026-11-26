import { test, expect, type Page } from "@playwright/test";
import {
  argsOf,
  installTauriMock,
  invoked,
  SETTINGS,
} from "./support/tauriMock";

/**
 * Kōrero 1.52 (B14): learning a misheard name from a correction made in
 * Dictation history, and the "Learnt from your corrections" list. Runs the real
 * frontend against the stand-in backend; which edits count is decided and
 * tested in Rust (`learning.rs`). Invented data only.
 */
const entry = {
  id: 1,
  file_name: "handy-1.wav",
  timestamp: 1763600000,
  saved: false,
  title: "",
  transcription_text: "Working with foreign emergency on the plan",
  post_processed_text: null,
  post_process_prompt: null,
  post_process_requested: false,
  post_process_failure: null,
};
const fixed = "Working with Fire and Emergency on the plan";
const pair = {
  heard: "foreign emergency",
  meant: "Fire and Emergency",
  seen: 2,
  off: false,
  last_seen: "2026-11-26",
};
const watched = {
  heard: "fens",
  meant: "FENZ",
  seen: 2,
  off: false,
  last_seen: "2026-11-26",
};

async function correct(page: Page, text: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "Dictation history" }).click();
  await page.getByTitle("Correct this text").first().click();
  const editor = page.locator("textarea").first();
  await editor.fill(text);
  await editor.press("Control+Enter");
}

test.describe("Learning from corrections", () => {
  let errors: string[];
  test.beforeEach(({ page }) => {
    errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
  });
  test.afterEach(() => {
    expect(errors, "uncaught errors on the page").toEqual([]);
  });

  test("a correction in History is handed over with the words as heard", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_history_entries: { entries: [entry], has_more: false },
      learn_from_edit: { learnt: [], watching: 1, switched_off: [] },
    });
    await correct(page, fixed);
    await expect
      .poll(() => argsOf(page, "learn_from_edit"))
      .toEqual({
        original: entry.transcription_text,
        before: entry.transcription_text,
        after: fixed,
      });
    await expect(
      page.getByText("If you keep making this correction", { exact: false }),
    ).toBeVisible();
  });

  test("the second time, it says what it learnt, and Undo switches it off", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_app_settings: { ...SETTINGS, learned_pairs: [pair, watched] },
      get_history_entries: { entries: [entry], has_more: false },
      learn_from_edit: { learnt: [pair], watching: 0, switched_off: [] },
    });
    await correct(page, fixed);
    const toast = page.getByText("is now written as Fire and Emergency", {
      exact: false,
    });
    await expect(toast).toBeVisible();
    await page.getByRole("button", { name: "Undo" }).click();
    await expect
      .poll(() => argsOf(page, "update_learned_pairs"))
      .toEqual({
        pairs: [{ ...pair, off: true }, watched],
      });
  });

  test("a single word corrected three times is offered, and Always makes it a taught correction", async ({
    page,
  }) => {
    const word = {
      heard: "kite",
      meant: "Kyt",
      seen: 3,
      off: false,
      last_seen: "2026-11-26",
      suggest_only: true,
    };
    await installTauriMock(page, [], {
      get_app_settings: { ...SETTINGS, learned_pairs: [pair, word] },
      get_history_entries: { entries: [entry], has_more: false },
      learn_from_edit: {
        learnt: [],
        watching: 0,
        switched_off: [],
        suggested: [word],
      },
    });
    await correct(page, fixed);
    await expect(
      page.getByText('You have corrected "kite" to Kyt three times', {
        exact: false,
      }),
    ).toBeVisible();
    // Nothing is taught until the user says so.
    expect(await invoked(page)).not.toContain("update_transcript_corrections");
    await page.getByRole("button", { name: "Always write it" }).click();
    await expect
      .poll(() => argsOf(page, "update_transcript_corrections"))
      .toEqual({ corrections: [{ wrong: "kite", right: "Kyt" }] });
    await expect
      .poll(() => argsOf(page, "update_learned_pairs"))
      .toEqual({ pairs: [pair] });
  });

  test("changing a learnt correction back is announced", async ({ page }) => {
    await installTauriMock(page, [], {
      get_history_entries: {
        entries: [{ ...entry, post_processed_text: fixed }],
        has_more: false,
      },
      learn_from_edit: {
        learnt: [],
        watching: 0,
        switched_off: [{ ...pair, off: true }],
      },
    });
    await correct(page, entry.transcription_text);
    await expect
      .poll(() => argsOf(page, "learn_from_edit"))
      .toEqual({
        original: entry.transcription_text,
        before: fixed,
        after: entry.transcription_text,
      });
    await expect(
      page.getByText('Kōrero will stop changing "foreign emergency"', {
        exact: false,
      }),
    ).toBeVisible();
  });

  test("with learning switched off, a correction is not handed over", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_app_settings: { ...SETTINGS, learning_enabled: false },
      get_history_entries: { entries: [entry], has_more: false },
    });
    await correct(page, fixed);
    await expect
      .poll(() => invoked(page))
      .toContain("update_post_processed_text");
    // Long enough for a late call to have been made.
    await page.waitForTimeout(700);
    expect(await invoked(page)).not.toContain("learn_from_edit");
  });

  test("a backend that answers nothing shows nothing and breaks nothing", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_history_entries: { entries: [entry], has_more: false },
    });
    await correct(page, fixed);
    await expect.poll(() => invoked(page)).toContain("learn_from_edit");
    await expect(page.getByText("Noted.", { exact: false })).toHaveCount(0);
    await expect(page.getByText("Learnt:", { exact: false })).toHaveCount(0);
  });

  test("the Learnt list shows each pair and what state it is in", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_app_settings: {
        ...SETTINGS,
        learned_pairs: [
          pair,
          watched,
          { ...pair, heard: "kite", meant: "Kyt", off: true },
          { ...pair, heard: "nick", meant: "Nic", seen: 3, suggest_only: true },
          {
            ...pair,
            heard: "mark",
            meant: "Marc",
            seen: 2,
            suggest_only: true,
          },
        ],
      },
    });
    await page.goto("/");
    await page
      .getByText("AI clean-up & notes", { exact: true })
      .first()
      .click();
    const card = page.getByRole("group", {
      name: "Learnt from your corrections",
    });
    await expect(card).toBeVisible();
    const rows = card.getByRole("listitem");
    await expect(rows).toHaveCount(5);
    await expect(rows.nth(0)).toContainText("foreign emergency");
    await expect(rows.nth(0)).toContainText("Fire and Emergency");
    await expect(rows.nth(0)).toContainText("In use · corrected 2 times");
    // A single word needs three: twice is still only watched.
    await expect(rows.nth(1)).toContainText("Watching · corrected 2 times");
    await expect(rows.nth(2)).toContainText("Off");
    // A single word is never in use by itself: suggested at three, watched before.
    await expect(rows.nth(3)).toContainText("Suggested · corrected 3 times");
    await expect(
      rows.nth(3).getByRole("button", { name: "Always write nick as Nic" }),
    ).toBeVisible();
    await expect(rows.nth(4)).toContainText("Watching · corrected 2 times");
    await expect(rows.nth(4).getByRole("button")).toHaveCount(1);
    // Only a pair in use can be promoted.
    await expect(
      rows.nth(0).getByRole("button", { name: /everywhere/ }),
    ).toBeVisible();
    await expect(
      rows.nth(1).getByRole("button", { name: /everywhere/ }),
    ).toHaveCount(0);
    await expect(
      rows.nth(2).getByRole("button", { name: /everywhere/ }),
    ).toHaveCount(0);
  });

  test("Use everywhere makes it a taught correction and takes it off the list", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_app_settings: { ...SETTINGS, learned_pairs: [pair, watched] },
    });
    await page.goto("/");
    await page
      .getByText("AI clean-up & notes", { exact: true })
      .first()
      .click();
    const card = page.getByRole("group", {
      name: "Learnt from your corrections",
    });
    await card
      .getByRole("button", { name: "Use Fire and Emergency everywhere" })
      .click();
    await expect
      .poll(() => argsOf(page, "update_transcript_corrections"))
      .toEqual({
        corrections: [
          { wrong: "foreign emergency", right: "Fire and Emergency" },
        ],
      });
    await expect
      .poll(() => argsOf(page, "update_learned_pairs"))
      .toEqual({ pairs: [watched] });
  });

  test("a pair can be switched off, and the whole feature too", async ({
    page,
  }) => {
    await installTauriMock(page, [], {
      get_app_settings: { ...SETTINGS, learned_pairs: [pair] },
    });
    await page.goto("/");
    await page
      .getByText("AI clean-up & notes", { exact: true })
      .first()
      .click();
    const card = page.getByRole("group", {
      name: "Learnt from your corrections",
    });
    await card
      .getByRole("checkbox", {
        name: "Write foreign emergency as Fire and Emergency",
      })
      .click();
    await expect
      .poll(() => argsOf(page, "update_learned_pairs"))
      .toEqual({ pairs: [{ ...pair, off: true }] });
    const main = card
      .getByText("Learn from my corrections", { exact: true })
      .locator(
        "xpath=ancestor::*[.//input[@type='checkbox']][1]//input[@type='checkbox']",
      );
    await expect(main).toBeChecked();
    await main.dispatchEvent("click");
    await expect
      .poll(() => argsOf(page, "change_learning_enabled_setting"))
      .toEqual({ enabled: false });
  });

  test("nothing learnt yet says how to start", async ({ page }) => {
    await installTauriMock(page, []);
    await page.goto("/");
    await page
      .getByText("AI clean-up & notes", { exact: true })
      .first()
      .click();
    await expect(
      page.getByText("Nothing learnt yet.", { exact: false }),
    ).toBeVisible();
  });
});
