import { chromium } from "playwright-core";
const browser = await chromium.launch({ channel: "msedge", headless: true });
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
page.on("pageerror", (e) => console.log("[pageerror]", String(e).slice(0, 400)));
page.on("console", (m) => {
  if (m.type() === "error" || m.type() === "warning") console.log("[console." + m.type() + "]", m.text().slice(0, 400));
});
await page.goto("http://127.0.0.1:7806", { waitUntil: "domcontentloaded" });
await page.waitForSelector(".tree-node", { timeout: 8000 });
const dock = page.locator(".mv-dock .dock-page:not(.hidden)");
await dock.locator('.tree-row:has-text("笔记本A")').first().click();
await dock.locator('.tree-row:has-text("首页")').first().click();
await page.waitForSelector(".cm-content", { timeout: 8000 });
await page.waitForTimeout(800);
const r = await page.evaluate(() => {
  const content = document.querySelector(".cm-content");
  const line6 = Array.from(document.querySelectorAll(".cm-line")).find((l) => l.textContent?.startsWith("# "));
  return {
    h1: document.querySelectorAll(".cm-line.mv-h1").length,
    hl: document.querySelectorAll(".mv-highlight").length,
    wiki: document.querySelectorAll(".mv-wikilink").length,
    contentClasses: content?.className,
    line6HTML: line6?.innerHTML.slice(0, 200),
    pluginCount: content?.querySelectorAll(":scope > *").length,
  };
});
console.log("PROBE:" + JSON.stringify(r, null, 1));
await browser.close();
