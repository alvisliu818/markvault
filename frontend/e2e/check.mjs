// markvault E2E smoke check — headless Edge via playwright-core
// Usage: node e2e/check.mjs [baseUrl]
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";

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
page.on("console", (m) => {
  if (m.type() === "error") console.log("[console.error]", m.text().slice(0, 200));
});
page.on("pageerror", (e) => console.log("[pageerror]", String(e).slice(0, 200)));

try {
  // 1. load
  await page.goto(base, { waitUntil: "domcontentloaded" });
  await page.waitForSelector(".mv-toolbar", { timeout: 8000 });
  check("页面加载与布局渲染", true);
  await page.screenshot({ path: shots + "t1_layout.png" });

  // 2. filetree shows notebooks
  await page.waitForSelector(".tree-node", { timeout: 8000 });
  const nbText = await page.textContent(".mv-dock .tree");
  check("文件树显示笔记本", nbText.includes("笔记本A"), nbText.slice(0, 60));

  // 3. expand 笔记本A then open doc 首页 (scope: visible dock page)
  const visibleDock = page.locator(".mv-dock .dock-page:not(.hidden)");
  await visibleDock.locator('.tree-row:has-text("笔记本A")').first().click();
  await visibleDock.locator('.tree-row:has-text("首页")').first().waitFor({ timeout: 8000 });
  const docRow = visibleDock.locator('.tree-row:has-text("首页")').first();
  await docRow.click();
  await page.waitForSelector(".cm-content", { timeout: 8000 });
  await page.waitForTimeout(400);
  const cmText = await page.textContent(".cm-content");
  check("打开文档(首页)到编辑器", cmText.includes("欢迎来到 markvault"), cmText.slice(0, 50));
  await page.screenshot({ path: shots + "t2_open_doc.png" });

  // 4. edit → autosave (append text + a ==highlight== line)
  await page.click(".cm-content");
  await page.keyboard.press("Control+End");
  await page.keyboard.type("\n\nE2E编辑测试行\n\n==E2E高亮测试==");
  await page.waitForTimeout(1400); // debounce 900ms + save
  const saved = await page.evaluate(async () => {
    const r = await fetch("/api/file/getFile", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/首页.md" }),
    });
    return r.text();
  });
  check("编辑自动保存到内核", saved.includes("E2E编辑测试行") && saved.includes("==E2E高亮测试=="));
  await page.screenshot({ path: shots + "t3_edit_save.png" });

  // 5. preview
  await page.click('button:has-text("预览")');
  await page.waitForSelector(".preview-pane:not(.hidden)", { timeout: 5000 });
  await page.waitForFunction(
    () => document.querySelector(".preview-pane")?.textContent?.includes("E2E编辑测试行"),
    { timeout: 8000 }
  );
  const prev = await page.textContent(".preview-pane");
  check("渲染预览", prev.includes("欢迎来到 markvault") && prev.includes("E2E编辑测试行"));
  const markCount = await page.locator(".preview-pane mark").count();
  check("预览==高亮==渲染", markCount > 0, `mark=${markCount}`);
  await page.screenshot({ path: shots + "t4_preview.png" });

  // 6. outline
  const outline = await page.textContent(".mv-right .dock-page:not(.hidden)");
  check("大纲面板", outline.includes("首页") || outline.includes("(无标题)"), outline.slice(0, 40));

  // 6b. breadcrumb
  const bc = await page.textContent(".breadcrumb");
  check("面包屑导航", bc.includes("笔记本A") && bc.includes("首页"), bc);

  // 6c. backlink panel: 首页 links to 计划/随笔 -> 引用了谁
  await page.click('.mv-right .dock-tab:has-text("反链")');
  await page.waitForTimeout(800);
  const bkText = await page.textContent(".mv-right .dock-page:not(.hidden)");
  check("反链面板-引用了谁", bkText.includes("引用了谁") && bkText.includes("计划") && bkText.includes("随笔"), bkText.slice(0, 60));
  await page.screenshot({ path: shots + "t5b_backlinks.png" });

  // 7. search panel
  await page.click('.mv-right .dock-tab:has-text("搜索")');
  await page.fill('.mv-right input[type=text]', "zigzag");
  await page.click('.mv-right button:has-text("搜索")');
  await page.waitForTimeout(600);
  const searchOut = await page.textContent(".mv-right .dock-page:not(.hidden)");
  check("全文搜索", searchOut.includes("zigzag"));
  await page.screenshot({ path: shots + "t5_search.png" });

  // 8. virtual tree panel
  await page.click('.mv-dock .dock-tab:has-text("虚拟树")');
  await page.waitForTimeout(600);
  const vtText = await page.textContent(".mv-dock .dock-page:not(.hidden)");
  check("虚拟树-关系树", vtText.includes("关系树") && vtText.includes("首页") && vtText.includes("计划") && vtText.includes("随笔"));
  await page.screenshot({ path: shots + "t6_vtree.png" });

  // 9. vtree locate (open 随笔 via its label then locate)
  await page
    .locator('.mv-dock .dock-page:not(.hidden) .tree-row:has-text("随笔") .label')
    .first()
    .click();
  await page.waitForTimeout(400);
  await page.click('.mv-dock button:has-text("定位")');
  await page.waitForTimeout(300);
  const flashed = await page.locator(".tree-flash").count();
  check("虚拟树-定位闪烁", flashed > 0);
  await page.screenshot({ path: shots + "t7_locate.png" });

  // 10. vtree focus mode
  await page.click('.mv-dock button:has-text("聚焦")');
  await page.waitForTimeout(300);
  const focusText = await page.textContent(".mv-dock .dock-page:not(.hidden)");
  check("虚拟树-聚焦模式", focusText.includes("随笔"));
  await page.screenshot({ path: shots + "t8_focus.png" });

  // 10b. bookmark panel
  await page.click('.mv-right .dock-tab:has-text("书签")');
  await page.waitForTimeout(400);
  const bmText = await page.textContent(".mv-right .dock-page:not(.hidden)");
  check("书签面板", bmText.includes("example.com"), bmText.slice(0, 50));

  // 10c. clip button exists (kernel clip verified separately via API test)
  const clipBtnVisible = await page.locator('.mv-toolbar button:has-text("剪藏")').isVisible();
  check("剪藏按钮", clipBtnVisible);

  // 10d. export notebook -> writes zip into .markvault/export
  await page.click('.mv-toolbar button:has-text("导出笔记本")');
  await page.waitForTimeout(800);
  const zipExists = await page.evaluate(async () => {
    const r = await fetch("/api/file/readDir", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: ".markvault/export" }),
    });
    const j = await r.json();
    return j.code === 0 && (j.data.entries || []).some((e) => e.name.endsWith(".zip"));
  });
  check("导出笔记本 zip", zipExists);

  // 11. new doc via toolbar flow (API-backed UI) — switch back to 文件 tab first
  await page.click('.mv-dock .dock-tab:has-text("文件")');
  await page.waitForTimeout(300);
  const newDoc = page.locator('.mv-dock .dock-page:not(.hidden) button:has-text("新建文档")');
  await newDoc.click();
  await page.fill(".mv-dialog input", "E2E临时文档");
  await page.click('.mv-dialog button:has-text("确定")');
  await page.waitForTimeout(800);
  const tabCount = await page.locator(".ed-tab").count();
  check("新建文档并打开页签", tabCount >= 2, `tabs=${tabCount}`);
  await page.screenshot({ path: shots + "t9_new_doc.png" });

  // cleanup: delete the temp doc via kernel API
  await page.evaluate(async () => {
    await fetch("/api/filetree/removeDoc", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "/笔记本A/E2E临时文档.md" }),
    });
  });
  check("清理E2E临时文档", true);
} catch (e) {
  check("异常中断", false, String(e).slice(0, 300));
  await page.screenshot({ path: shots + "tx_error.png" });
} finally {
  await browser.close();
}

const failed = results.filter((r) => !r.ok);
console.log(`\n==== ${results.length - failed.length}/${results.length} passed ====`);
process.exit(failed.length ? 1 : 0);
