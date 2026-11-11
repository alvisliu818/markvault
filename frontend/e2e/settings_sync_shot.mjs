import { chromium } from "playwright-core";
const browser = await chromium.launch({ channel: "msedge", headless: true });
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
await page.goto("http://127.0.0.1:7806", { waitUntil: "domcontentloaded" });
await page.waitForSelector(".mv-toolbar", { timeout: 8000 });
await page.click('.mv-toolbar button:has-text("设置")');
await page.waitForSelector(".settings-dialog", { timeout: 6000 });
await page.waitForTimeout(800);
// 点击刷新远端状态 → dummy 凭据 → 错误态着色
const btn = page.locator('#sync-refresh');
if (await btn.count()) {
  await btn.click();
  await page.waitForTimeout(1500);
}
const state = await page.textContent("#sync-state").catch(() => "(no sync section)");
const isErr = await page.locator("#sync-state.sync-error").count();
console.log("SYNC-STATE:", state, "| error-styled:", isErr > 0);
await page.screenshot({ path: "../../_e2e/m7_sync_polish.png" });
await browser.close();
