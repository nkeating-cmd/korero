// Kōrero local AI UAT driver: talks to the running test build's webview over
// CDP (127.0.0.1:9333, opened only by launch.ps1). Usage:
//   node scripts/uat-windows/cdp.mjs <action> [args]
//   pages | shot <file.png> | text | click <name> | clickwatch <name> | goto <text> | eval <js>
// Never closes the app: it only disconnects.
import { chromium } from "playwright-core";

const [action, ...rest] = process.argv.slice(2);
const arg = rest.join(" ");
const browser = await chromium.connectOverCDP("http://127.0.0.1:9333");
const pages = browser.contexts().flatMap((c) => c.pages());
const page = pages.find((p) => !p.url().includes("overlay")) ?? pages[0];
try {
  if (action === "pages") {
    console.log(pages.map((p) => p.url()).join("\n"));
  } else if (action === "shot") {
    await page.screenshot({ path: arg });
    console.log("SHOT", arg);
  } else if (action === "text") {
    console.log((await page.evaluate(() => document.body.innerText)).slice(0, 6000));
  } else if (action === "click") {
    const b = page.getByRole("button", { name: arg, exact: true }).or(page.getByRole("link", { name: arg, exact: true }));
    await b.first().click({ timeout: 8000 });
    console.log("CLICKED", arg);
  } else if (action === "clickwatch") {
    // Click, then report every toast seen in the next 40 s.
    await page.getByRole("button", { name: arg, exact: true }).first().click({ timeout: 8000 });
    const seen = new Set();
    for (let i = 0; i < 80; i++) {
      for (const t of await page.locator("[data-sonner-toast]").allInnerTexts()) seen.add(t.replace(/\s+/g, " "));
      await page.waitForTimeout(500);
    }
    console.log("TOASTS", JSON.stringify([...seen]));
  } else if (action === "goto") {
    await page.getByText(arg, { exact: true }).first().click({ timeout: 8000 });
    console.log("WENT", arg);
  } else if (action === "eval") {
    console.log(JSON.stringify(await page.evaluate(arg), null, 1));
  } else {
    console.log("unknown action");
    process.exitCode = 2;
  }
} catch (e) {
  console.log("ERR", e.message.split("\n")[0]);
  process.exitCode = 1;
}
process.exit();
