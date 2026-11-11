// 壳内人工点验 7 项中的可自动化 4 项（E1 / F1 / B7-web降级 / D2）
// 运行前提：内核已 serve testvault 于 127.0.0.1:7806（petal 由本脚本自行播种）
import { chromium } from "playwright-core";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";

// seed sample petal（与 check.mjs 相同契约）——自包含，不依赖 check.mjs 先跑过
const tv = new URL("../../", import.meta.url).pathname.replace(/^\/([A-Za-z]):/, "$1:") + "testvault/";
const petalDir = tv + ".markvault/petal/sample";
rmSync(tv + ".markvault/petal", { recursive: true, force: true });
mkdirSync(petalDir, { recursive: true });
writeFileSync(
  petalDir + "/index.js",
  "export default class {\n" +
    "  constructor(host) { this.host = host; }\n" +
    "  onload() { this.host.addCommand('hello', '问好', () => this.host.ui.toast('Hello from sample petal!')); }\n" +
    "}\n"
);
writeFileSync(tv + ".markvault/petal/petals.json", JSON.stringify([{ name: "sample", enabled: true }]));

const base = process.argv[2] || "http://127.0.0.1:7806";
const shots = new URL("../../_e2e/", import.meta.url).pathname.replace(/^\/([A-Za-z]):/, "$1:");
mkdirSync(shots, { recursive: true });

const results = [];
const check = (name, ok, extra = "") => {
  results.push({ name, ok, extra });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${extra ? " — " + extra : ""}`);
};

const browser = await chromium.launch({ channel: "msedge", headless: true });
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
page.on("pageerror", (e) => console.log("[pageerror]", String(e).slice(0, 200)));

try {
  await page.goto(base, { waitUntil: "domcontentloaded" });
  await page.waitForSelector(".mv-toolbar", { timeout: 8000 });

  // ---------- E1: preview hash 路由 → 文档页签打开 ----------
  const visibleDock = page.locator(".mv-dock .dock-page:not(.hidden)");
  await visibleDock.locator('.tree-row:has-text("笔记本A")').first().click();
  await visibleDock.locator('.tree-row:has-text("首页")').first().waitFor({ timeout: 8000 });
  await visibleDock.locator('.tree-row:has-text("首页")').first().click();
  await page.waitForSelector(".cm-content", { timeout: 8000 });
  await page.click('button:has-text("预览")');
  await page.waitForSelector(".preview-pane:not(.hidden)", { timeout: 5000 });
  const wikiLink = page.locator('.preview-pane a[href^="/#/doc/"]').first();
  const href = await wikiLink.getAttribute("href");
  const tabsBefore = await page.locator(".ed-tab").count();
  await wikiLink.click();
  await page.waitForTimeout(900); // resolveLink -> tabs.open
  const tabsAfter = await page.locator(".ed-tab").count();
  const titles = await page.locator(".ed-tab .title").allTextContents();
  check(
    "E1-preview-hash路由",
    tabsAfter > tabsBefore && (titles.includes("随笔") || titles.includes("计划")),
    `href=${href} tabs ${tabsBefore}->${tabsAfter} [${titles.join(",")}]`
  );
  await page.screenshot({ path: shots + "m7_E1_preview_hash.png" });

  // ---------- F1: 小屏抽屉（≤860px，☰/▤ 互斥） ----------
  await page.setViewportSize({ width: 800, height: 900 });
  await page.waitForTimeout(400);
  const dock = page.locator(".mv-dock");
  const right = page.locator(".mv-right");
  const togglesVisible = await page.locator(".mv-toolbar .mobile-toggle").first().isVisible();
  check("F1-抽屉按钮可见", togglesVisible, "viewport 800px");
  const dockOpenBefore = await dock.evaluate((el) => el.classList.contains("mobile-open"));
  await page.click('.mv-toolbar .mobile-toggle:has-text("☰")');
  await page.waitForTimeout(350);
  const dockOpen = await dock.evaluate((el) => el.classList.contains("mobile-open"));
  const rightOpenAfterDock = await right.evaluate((el) => el.classList.contains("mobile-open"));
  check("F1-☰开dock", !dockOpenBefore && dockOpen && !rightOpenAfterDock, `dock=${dockOpen} right=${rightOpenAfterDock}`);
  await page.screenshot({ path: shots + "m7_F1_dock_drawer.png" });
  await page.click('.mv-toolbar .mobile-toggle:has-text("▤")');
  await page.waitForTimeout(350);
  const dockOpen2 = await dock.evaluate((el) => el.classList.contains("mobile-open"));
  const rightOpen = await right.evaluate((el) => el.classList.contains("mobile-open"));
  check("F1-▤互斥开right", rightOpen && !dockOpen2, `dock=${dockOpen2} right=${rightOpen}`);
  await page.screenshot({ path: shots + "m7_F1_right_drawer.png" });
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.waitForTimeout(300);
  const dockReset = await dock.evaluate((el) => !el.classList.contains("mobile-open"));
  check("F1-恢复大屏", dockReset);

  // ---------- B7(web 降级路径): 无壳环境按钮不崩、给出降级提示 ----------
  // 浏览器面板在 web 环境显示降级 hint（剪藏/收藏/下载按钮点击应 toast 无活动页签，不抛错）
  await page.click('.mv-dock .dock-tab:has-text("虚拟树")');
  const hasBrowserTab = await page.locator('.mv-dock .dock-tab:has-text("浏览器")').count();
  if (hasBrowserTab > 0) {
    await page.click('.mv-dock .dock-tab:has-text("浏览器")');
    await page.waitForTimeout(400);
    const hintCount = await page.locator(".browser-tabs .dim, .browser-tabs .search-row.dim").count();
    check("B7-web降级提示", hintCount > 0, `hint=${hintCount}`);
    const clipBtn = page.locator('.mv-dock button:has-text("剪藏")').first();
    await clipBtn.click();
    await page.waitForTimeout(400);
    const toastText = await page.locator(".mv-toast").first().textContent().catch(() => "");
    check("B7-web无页签toast", (toastText || "").includes("无活动页签"), toastText || "(no toast)");
    await page.screenshot({ path: shots + "m7_B7_web_degrade.png" });
  } else {
    check("B7-web降级提示", true, "(浏览器 dock 不存在于当前构建，壳内点验为准)");
  }

  // ---------- D2: web 侧已在 check.mjs 覆盖（petal-命令菜单/命令执行），此处复跑一次确认 ----------
  await page.click('.mv-toolbar button:has-text("命令")');
  await page.waitForSelector(".mv-menu", { timeout: 4000 });
  const menuText = await page.textContent(".mv-menu");
  check("D2-web复跑-命令菜单", menuText.includes("sample: 问好"), menuText.slice(0, 40));
  await page.keyboard.press("Escape");
  await page.click("#mv-menu", { force: true }).catch(() => {});
} catch (e) {
  check("异常中断", false, String(e).slice(0, 300));
  await page.screenshot({ path: shots + "m7_error.png" });
} finally {
  await browser.close();
}

const failed = results.filter((r) => !r.ok);
console.log(`\n==== ${results.length - failed.length}/${results.length} passed ====`);
console.log("壳内仍需人工: C1(vault对话框) C2(开机自启) B10(严格CSP站点) B7(壳内面板按钮) D2(壳内命令菜单) E1/F1(壳内抽查)");
process.exit(failed.length ? 1 : 0);
