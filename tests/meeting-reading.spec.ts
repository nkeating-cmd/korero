import { test, expect, type Page } from "@playwright/test";
import {
  argsOf,
  callMeeting,
  importedMeeting,
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
  await page
    .getByRole("button", { name: new RegExp(title.replace(/[()]/g, "\\$&")) })
    .click();
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
    await installTauriMock(page, [callMeeting(), importedMeeting()]);
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
});
