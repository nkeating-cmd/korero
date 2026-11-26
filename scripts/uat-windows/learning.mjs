// Kōrero 1.52 UAT: learning from corrections, on the real build (not the stand-in backend).
// Needs a portable test copy started by launch.ps1 whose history holds the invented rows
// below (see docs/TESTING.md, "Learning from corrections"). Never run against real data:
// it edits history entries. Exit 0 = every claim held, 1 = a claim failed, 2 = harness.
//   node scripts/uat-windows/learning.mjs
import { chromium } from "playwright-core";

const claims = [];
const claim = (name, ok, detail = "") => {
  claims.push(ok);
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  " + detail : ""}`);
};

let browser;
try {
  browser = await chromium.connectOverCDP("http://127.0.0.1:9333");
  const pages = browser.contexts().flatMap((c) => c.pages());
  const page = pages.find((p) => !p.url().includes("overlay")) ?? pages[0];

  const toasts = async (ms = 3500) => {
    const seen = new Set();
    for (let i = 0; i < ms / 250; i++) {
      for (const t of await page.locator("[data-sonner-toast]").allInnerTexts())
        seen.add(t.replace(/\s+/g, " ").trim());
      await page.waitForTimeout(250);
    }
    return [...seen];
  };
  /** Wait until the messages from the step before have gone, so each step reads its own. */
  const quiet = async () => {
    for (let i = 0; i < 120; i++) {
      if ((await page.locator("[data-sonner-toast]").count()) === 0) return;
      await page.waitForTimeout(250);
    }
    throw new Error("earlier messages never cleared");
  };
  /** Correct the history entry that shows `shown` so that `from` reads `to`. */
  const correct = async (shown, from, to) => {
    await quiet();
    const card = page
      .locator("div")
      .filter({ hasText: shown })
      .filter({ has: page.getByTitle("Correct this text") })
      .last();
    await card.getByTitle("Correct this text").first().click({ timeout: 8000 });
    const editor = page.locator("textarea").first();
    const text = await editor.inputValue();
    if (!text.includes(from)) throw new Error(`entry does not contain "${from}"`);
    await editor.fill(text.replace(from, to));
    await editor.press("Control+Enter");
    return toasts();
  };
  const has = (list, part) => list.some((t) => t.includes(part));

  await page.getByRole("button", { name: "Dictation history" }).click();
  await page.getByText("Today I am working with foreign emergency").first().waitFor({ timeout: 8000 });

  // A phrase: noted once, a rule the second time.
  let t = await correct("Today I am working with", "foreign emergency", "Fire and Emergency");
  claim("first correction of a phrase is noted, not learnt", has(t, "Noted.") && !has(t, "Learnt:"), JSON.stringify(t));
  t = await correct("Then a call with", "foreign emergency", "Fire and Emergency");
  claim("second correction makes it a rule", has(t, 'Learnt: "foreign emergency" is now written as Fire and Emergency'), JSON.stringify(t));

  // A single word in three different sentences: offered once, a rule only on the click.
  t = await correct("Please send it to", "kite", "Kyt");
  claim("a single word is noted", has(t, "Noted.") && !has(t, "Learnt:"), JSON.stringify(t));
  t = await correct("I will ask", "kite", "Kyt");
  claim("a single word is still only noted the second time", !has(t, "Learnt:") && !has(t, "three times"), JSON.stringify(t));
  t = await correct("A note from", "kite", "Kyt");
  claim("the third time it is offered, not applied", has(t, 'You have corrected "kite" to Kyt three times') && !has(t, "Learnt:"), JSON.stringify(t));
  await page.getByRole("button", { name: "Always write it" }).first().click({ timeout: 8000 });
  await page.waitForTimeout(1200);

  // The list in settings shows what the backend really stored.
  await page.getByText("AI clean-up & notes", { exact: true }).first().click();
  const card = page.getByRole("group", { name: "Learnt from your corrections" });
  await card.waitFor({ timeout: 8000 });
  const rows = (await card.getByRole("listitem").allInnerTexts()).map((r) => r.replace(/\s+/g, " ").trim());
  console.log("LIST", JSON.stringify(rows));
  const row = (start) => rows.find((r) => r.startsWith(start)) ?? "";
  claim("the phrase is in use", row("foreign emergency → Fire and Emergency").includes("In use · corrected 2 times"));
  claim("the accepted word is in use", row("kite → Kyt").includes("In use · corrected 3 times"));
  claim("its phrases are only watched", rows.filter((r) => r.startsWith("kite ") && r.includes("Watching")).length >= 2);

  // Changing an applied rule back switches it off. The first entry holds the words as heard
  // as its original and the corrected text as shown, exactly as an applied rule leaves it.
  await page.getByRole("button", { name: "Dictation history" }).click();
  t = await correct("Today I am working with", "Fire and Emergency", "foreign emergency");
  claim("changing it back switches the rule off", has(t, 'Kōrero will stop changing "foreign emergency" to Fire and Emergency'), JSON.stringify(t));
  await page.getByText("AI clean-up & notes", { exact: true }).first().click();
  await card.waitFor({ timeout: 8000 });
  const after = (await card.getByRole("listitem").allInnerTexts()).map((r) => r.replace(/\s+/g, " ").trim());
  claim("the list shows it off", (after.find((r) => r.startsWith("foreign emergency →")) ?? "").includes("Off"));
} catch (e) {
  console.log("HARNESS", e.message.split("\n")[0]);
  process.exitCode = 2;
}
if (process.exitCode !== 2) {
  const failed = claims.filter((c) => !c).length;
  console.log(failed ? `RESULT ${failed} failed EXIT=1` : `RESULT all ${claims.length} passed EXIT=0`);
  process.exitCode = failed ? 1 : 0;
}
process.exit();
