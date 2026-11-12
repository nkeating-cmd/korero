import { test, expect, type Page } from "@playwright/test";
import {
  argsOf,
  callMeeting,
  importedMeeting,
  unnamedMeeting,
  installTauriMock,
  invoked,
} from "./support/tauriMock";

/**
 * Acceptance check for the 1.46 meeting screens: reading a call by speaker,
 * named callers, the Paragraphs/Lines switch, notes beside the transcript, and
 * which meetings a call rebuild is offered for.
 *
 * It runs the real frontend against a stand-in backend (tests/support/tauriMock.ts),
 * so it proves what the screens show and which backend command a button calls.
 * It does not prove recording, speech-to-text, caller separation or notes
 * writing: those need the Windows app (see docs/TESTING.md).
 */

const WIDE = { width: 1500, height: 900 };
const NARROW = { width: 1000, height: 900 };

const TURN_START = /^Start the trim at this turn/;

async function openMeeting(page: Page, title: string) {
  await page.goto("/");
  await page.getByText("Meetings", { exact: true }).first().click();
  // A plain string matches any button whose name contains the title.
  await page.getByRole("button", { name: title }).click();
  await expect(
    page.getByRole("tablist", { name: "Meeting views" }),
  ).toBeVisible();
}

test.describe("meeting reading screens", () => {
  let pageErrors: string[];

  test.beforeEach(async ({ page }) => {
    pageErrors = [];
    page.on("pageerror", (e) => pageErrors.push(e.message));
    await page.setViewportSize(WIDE);
    await installTauriMock(page, [
      callMeeting(),
      importedMeeting(),
      unnamedMeeting(),
    ]);
  });

  test.afterEach(() => {
    expect(pageErrors, "uncaught errors on the page").toEqual([]);
  });

  test("a call reads as paragraphs by speaker, with callers named", async ({
    page,
  }) => {
    await openMeeting(page, "Budget call (fixture)");
    const panel = page.getByRole("tabpanel");

    await expect(page.getByText("You + 2 callers")).toBeVisible();
    // Six lines, four turns: each caller's consecutive lines sit in one paragraph.
    await expect(panel.getByRole("button", { name: TURN_START })).toHaveCount(
      4,
    );
    await expect(
      panel.getByText("Shall we start with the budget?"),
    ).toBeVisible();
    await expect(panel.getByText("Yes. Let's do that.")).toBeVisible();
    // A named caller shows the name; an unnamed one keeps its number.
    await expect(
      panel.getByRole("button", { name: "Rename Caller 1, currently Mere" }),
    ).toBeVisible();
    await expect(
      panel.getByRole("button", { name: "Name Caller 2" }),
    ).toBeVisible();
    // Every line can still be corrected one at a time.
    await expect(
      panel.getByRole("button", { name: /^Correct this line/ }),
    ).toHaveCount(6);
  });

  test("correcting one line in a paragraph changes that line only", async ({
    page,
  }) => {
    await openMeeting(page, "Budget call (fixture)");
    const panel = page.getByRole("tabpanel");
    await panel
      .getByRole("button", { name: /^Correct this line, said at 0:06/ })
      .click();
    const box = panel.getByRole("textbox", { name: "Edit this line" });
    await expect(box).toHaveValue("with the budget?");
    await box.fill("with the forecast?");
    await box.press("Enter");

    await expect(
      panel.getByText("Shall we start with the forecast?"),
    ).toBeVisible();
    await expect(panel.getByText("Yes. Let's do that.")).toBeVisible();
    // The neighbouring line of the same paragraph is untouched.
    await expect(
      panel.getByRole("button", {
        name: "Correct this line, said at 0:04: Shall we start",
      }),
    ).toBeVisible();
  });

  test("selecting words in a paragraph does not open the editor", async ({
    page,
  }) => {
    await openMeeting(page, "Budget call (fixture)");
    const panel = page.getByRole("tabpanel");
    const line = panel.getByRole("button", {
      name: /^Correct this line, said at 0:06/,
    });
    // A drag across the words ends in a click on the line while the words are
    // still selected. Headless Chromium will not drag-select here, so the
    // selection is made directly and the closing click is sent on its own.
    await line.evaluate((el) => {
      const range = document.createRange();
      range.selectNodeContents(el);
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
    });
    await line.dispatchEvent("click");

    expect(
      await page.evaluate(() => window.getSelection()?.toString().trim()),
    ).toContain("budget");
    await expect(
      panel.getByRole("textbox", { name: "Edit this line" }),
    ).toHaveCount(0);
  });

  test("Lines shows one row per line, and the choice is remembered", async ({
    page,
  }) => {
    await openMeeting(page, "Budget call (fixture)");
    const layout = page.getByRole("group", {
      name: "How the transcript is laid out",
    });
    await layout.getByRole("button", { name: "Lines" }).click();

    const panel = page.getByRole("tabpanel");
    await expect(
      panel.getByText("Shall we start", { exact: true }),
    ).toBeVisible();
    await expect(
      panel.getByText("with the budget?", { exact: true }),
    ).toBeVisible();
    await expect(
      panel.getByText("Shall we start with the budget?"),
    ).toHaveCount(0);

    await openMeeting(page, "Budget call (fixture)");
    await expect(
      page.getByRole("tabpanel").getByText("Shall we start", { exact: true }),
    ).toBeVisible();
  });

  test("notes sit beside the transcript on a wide window only", async ({
    page,
  }) => {
    await openMeeting(page, "Budget call (fixture)");
    const panel = page.getByRole("tabpanel");
    await expect(panel.getByText("Kia ora koutou.")).toBeVisible();
    await expect(
      panel.getByText("Mere: send the figures by Friday"),
    ).toBeVisible();

    await page.setViewportSize(NARROW);
    await expect(panel.getByText("Kia ora koutou.")).toBeVisible();
    await expect(
      panel.getByText("Mere: send the figures by Friday"),
    ).toBeHidden();
    // The notes are still one click away.
    await page.getByRole("tab", { name: "Notes" }).click();
    await expect(
      page.getByRole("tabpanel").getByText("Mere: send the figures by Friday"),
    ).toBeVisible();
  });

  test("Re-transcribe on a call asks first, then rebuilds both sides by caller", async ({
    page,
  }) => {
    await openMeeting(page, "Budget call (fixture)");
    await page.getByRole("tab", { name: "Audio" }).click();
    const panel = page.getByRole("tabpanel");
    await expect(
      panel.getByText(/splits the call's side by caller/),
    ).toBeVisible();
    await expect(
      panel.getByText("Last time it found 2 voices on the call."),
    ).toBeVisible();

    // It replaces corrections and names, so the button alone must not start it.
    await panel
      .getByRole("button", { name: "Re-transcribe", exact: true })
      .click();
    const question = page.getByText(
      "Transcribe this call again from its recordings?",
    );
    await expect(question).toBeVisible();
    expect(await invoked(page)).not.toContain("meeting_call_transcript");
    await page.getByRole("button", { name: "Keep", exact: true }).click();
    await expect(question).toBeHidden();
    expect(await invoked(page)).not.toContain("meeting_call_transcript");

    await panel
      .getByRole("button", { name: "Re-transcribe", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Transcribe again", exact: true })
      .click();
    await expect.poll(() => invoked(page)).toContain("meeting_call_transcript");
    expect(await argsOf(page, "meeting_call_transcript")).toMatchObject({
      micPath: "C:/fixtures/call-you.wav",
      systemPath: "C:/fixtures/call-others.wav",
    });
    // The rebuilt lines replace the old ones on the Transcript tab.
    await page.getByRole("tab", { name: "Transcript" }).click();
    await expect(
      page.getByRole("tabpanel").getByText("Mōrena, rebuilt."),
    ).toBeVisible();
    expect(await invoked(page)).not.toContain("meeting_transcribe_merge");
  });

  test("an imported file is transcribed again the plain way, never as a call", async ({
    page,
  }) => {
    await openMeeting(page, "Imported file (fixture)");
    await page.getByRole("tab", { name: "Audio" }).click();
    const panel = page.getByRole("tabpanel");
    await expect(
      panel.getByText(/Re-run speech-to-text from the saved recording/),
    ).toBeVisible();
    await expect(
      panel.getByText(/splits the call's side by caller/),
    ).toHaveCount(0);

    await panel
      .getByRole("button", { name: "Re-transcribe", exact: true })
      .click();
    await expect
      .poll(() => invoked(page))
      .toContain("meeting_transcribe_merge");
    expect(await invoked(page)).not.toContain("meeting_call_transcript");
  });

  test("the version in use is always in view", async ({ page }) => {
    await page.goto("/");
    const help = page.getByRole("button", { name: /Help & about/ });
    await expect(help).toContainText("v9.8.7");
    await page.getByText("Meetings", { exact: true }).first().click();
    await expect(help).toContainText("v9.8.7");
  });

  test("a meeting nobody named can be named by its topic, with its date", async ({
    page,
  }) => {
    await openMeeting(page, "Meeting 5 Nov 2026, 6:03 pm");
    const title = page.getByRole("textbox", { name: "Meeting title" });
    await expect(title).toHaveValue("Meeting 5 Nov 2026, 6:03 pm");

    await page.getByRole("button", { name: "Suggest a title" }).click();
    // The model's quotes and full stop are gone; the meeting's own date is added.
    await expect(title).toHaveValue("Budget forecast and hiring · 5 Nov 2026");
    // Named now, so the offer goes away.
    await expect(
      page.getByRole("button", { name: "Suggest a title" }),
    ).toHaveCount(0);
    // The notes were not touched by the naming.
    await page.getByRole("tab", { name: "Notes" }).click();
    await expect(
      page.getByRole("tabpanel").getByText("Mere: send the figures by Friday"),
    ).toBeVisible();
  });

  test("writing notes names an unnamed meeting, and never renames a named one", async ({
    page,
  }) => {
    await openMeeting(page, "Meeting 5 Nov 2026, 6:03 pm");
    await page.getByRole("tab", { name: "Notes" }).click();
    await page.getByRole("button", { name: "Regenerate" }).click();
    await expect(
      page.getByRole("textbox", { name: "Meeting title" }),
    ).toHaveValue("Budget forecast and hiring · 5 Nov 2026");
    await expect(
      page.getByRole("tabpanel").getByText("Mere: send the revised figures"),
    ).toBeVisible();

    // A meeting with a name of its own: new notes, same name, and no offer to rename.
    await openMeeting(page, "Budget call (fixture)");
    await expect(
      page.getByRole("button", { name: "Suggest a title" }),
    ).toHaveCount(0);
    await page.getByRole("tab", { name: "Notes" }).click();
    await page.getByRole("button", { name: "Regenerate" }).click();
    await expect(
      page.getByRole("tabpanel").getByText("Mere: send the revised figures"),
    ).toBeVisible();
    await expect(
      page.getByRole("textbox", { name: "Meeting title" }),
    ).toHaveValue("Budget call (fixture)");
  });
});
