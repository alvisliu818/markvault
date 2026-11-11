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

  // 5b. preview wikilink hash navigation (/#/doc/ links route via hashchange —
  // same Chromium webview as the Tauri shell, so this covers the known-issue)
  const hashLink = page.locator('.preview-pane a[href*="#/doc/"]').first();
  const hashCnt = await hashLink.count();
  if (hashCnt > 0) {
    await hashLink.click();
    await page.waitForTimeout(700);
    const edTabs = await page.textContent(".editor-tabs");
    check("预览hash链接导航(/#/doc/)", edTabs.includes("计划") || edTabs.includes("随笔"), edTabs.slice(0, 60));
    // return to 首页 so the following panel assertions keep their baseline
    await page.click('.editor-tabs .ed-tab:has-text("首页")');
    await page.waitForTimeout(400);
  } else {
    check("预览hash链接导航(/#/doc/)", false, "no /#/doc/ links rendered in preview");
  }

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

  // 10b-2. graph panel: canvas rendered + nodes counted from /api/graph/getGraph
  await page.click('.mv-right .dock-tab:has-text("图谱")');
  await page.waitForTimeout(1200);
  const gStat = await page.textContent('.mv-right .dock-page:not(.hidden) .graph-stat');
  const nodeN = parseInt((gStat || "").match(/^(\d+) 节点/)?.[1] || "0", 10);
  check("图谱面板-节点统计", nodeN > 0, gStat || "(empty)");
  const gCanvas = await page.locator('.mv-right .dock-page:not(.hidden) canvas.graph-canvas').isVisible();
  check("图谱面板-canvas渲染", gCanvas);
  await page.screenshot({ path: shots + "t11_graph.png" });

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

  // 10e. browser panel: kernel-side seed history, then panel shows it;
  // outside the Tauri shell the tab area degrades to a hint
  await page.evaluate(async () => {
    await fetch("/api/history/add", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url: "https://example.com/e2e-article", title: "E2E历史条目" }),
    });
  });
  await page.click('.mv-dock .dock-tab:has-text("浏览器")');
  await page.waitForTimeout(400);
  const brText = await page.textContent(".mv-dock .dock-page:not(.hidden)");
  check("浏览器面板-壳外降级提示", brText.includes("桌面壳"), brText.slice(0, 60));
  await page.click('.mv-dock .dock-page:not(.hidden) button:has-text("刷新")');
  await page.waitForTimeout(400);
  const brText2 = await page.textContent(".mv-dock .dock-page:not(.hidden)");
  check("浏览器面板-历史记录列表", brText2.includes("E2E历史条目"), brText2.slice(0, 80));
  const addrVisible = await page.locator(".browser-addr").isVisible();
  check("浏览器面板-地址栏", addrVisible);
  await page.screenshot({ path: shots + "t10_browser.png" });

  // 10f. downloadToLocal: kernel fetches the vault-served file and lands it in assets/
  const dl = await page.evaluate(async (base) => {
    const r = await fetch("/api/asset/downloadToLocal", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        url: base + "/vault/" + encodeURIComponent("笔记本A/首页.md"),
        notebook: "笔记本A",
        filename: "e2e-download.md",
      }),
    });
    return await r.json();
  }, base);
  check(
    "downloadToLocal 落地 assets",
    dl.code === 0 && String(dl.data?.path || "").includes("assets/e2e-download.md"),
    JSON.stringify(dl).slice(0, 80)
  );

  // 10g. watcher: directory create/rename -> precise prefix reindex (no manual
  // rebuild) + subFileCount direct-child semantics (Obsidian parity)
  const tv = new URL("../../testvault/", import.meta.url).pathname.replace(/^\/([A-Za-z]):/, "$1:");
  const { mkdirSync, writeFileSync, renameSync, rmSync } = await import("node:fs");
  const dir1 = tv + "笔记本A/E2E目录";
  mkdirSync(dir1 + "/子", { recursive: true });
  writeFileSync(dir1 + "/直接.md", "---\ntitle: 直接\n---\n\nmvWatcherE2Exyz 直接子文档\n");
  writeFileSync(dir1 + "/子/深层.md", "---\ntitle: 深层\n---\n\nmvWatcherE2E深层标记\n");
  await page.waitForTimeout(1600); // watcher debounce 300ms + index
  const sub = await page.evaluate(async () => {
    const r = await fetch("/api/filetree/listDocsByPath", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ notebook: "笔记本A", path: "/" }),
    });
    const j = await r.json();
    return (j.data.files || []).find((f) => f.name === "E2E目录");
  });
  check(
    "subFileCount 直接子文档数",
    sub && sub.subFileCount === 1,
    `subFileCount=${sub?.subFileCount}`
  );
  const renameDir = tv + "笔记本A/E2E目录改名";
  renameSync(dir1, renameDir);
  await page.waitForTimeout(1600);
  const renamed = await page.evaluate(async () => {
    const r = await fetch("/api/search/fullTextSearchBlock", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ query: "mvWatcherE2Exyz" }),
    });
    const j = await r.json();
    return (j.data || []).map((d) => d.path);
  });
  check(
    "watcher 目录改名-索引跟随新路径",
    renamed.length > 0 && renamed.every((p) => p.includes("E2E目录改名")),
    JSON.stringify(renamed).slice(0, 100)
  );

  // cleanup: downloaded asset + seeded history + watcher test dir
  rmSync(renameDir, { recursive: true, force: true });
  await page.evaluate(async () => {
    await fetch("/api/file/removeFile", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/assets/e2e-download.md" }),
    });
    await fetch("/api/history/clear", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });
  });
  check("清理下载文件与历史", true);

  // 10h. snapshot history: putFile auto-snapshots pre-write content,
  // restore rolls the file back (kernel /api/repo/*)
  const snapA = await page.evaluate(async () => {
    const r = await fetch("/api/file/putFile", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/E2E快照.md", data: "快照V1内容标记\n" }),
    });
    return r.json();
  });
  await page.evaluate(async () => {
    await fetch("/api/file/putFile", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/E2E快照.md", data: "快照V2内容标记\n" }),
    });
  });
  await page.evaluate(async () => {
    await fetch("/api/file/putFile", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/E2E快照.md", data: "快照V3内容标记\n" }),
    });
  });
  const snapList = await page.evaluate(async () => {
    const r = await fetch("/api/repo/listSnapshots", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/E2E快照.md" }),
    });
    return (await r.json()).data.snapshots;
  });
  check(
    "快照-putFile 自动留存",
    snapA.code === 0 && snapList.length >= 2,
    `snapshots=${snapList.length}`
  );
  const restored = await page.evaluate(async (id) => {
    const r1 = await fetch("/api/repo/restoreSnapshot", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/E2E快照.md", id }),
    });
    const j1 = await r1.json();
    const r2 = await fetch("/api/file/getFile", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "笔记本A/E2E快照.md" }),
    });
    return { restore: j1, content: await r2.text() };
  }, snapList[snapList.length - 1].id);
  check(
    "快照-恢复回滚到旧版",
    restored.restore.code === 0 && restored.content.includes("快照V1内容标记"),
    restored.content.slice(0, 40)
  );

  // UI: editor toolbar 历史 button opens the history dialog
  const histBtn = page.locator('.editor-pane:not(.hidden) button:has-text("历史")').first();
  if ((await histBtn.count()) > 0) {
    await histBtn.click();
    await page.waitForSelector(".history-dialog", { timeout: 4000 });
    await page.waitForTimeout(400);
    const histText = await page.textContent(".history-dialog");
    check("历史版本对话框", histText.includes("历史版本") && histText.includes("创建快照"), histText.slice(0, 60));
    await page.screenshot({ path: shots + "t12_history.png" });
    await page.click('.history-dialog .cancel');
  } else {
    check("历史版本对话框", false, "no 历史 button visible");
  }

  // cleanup: snapshot test doc
  await page.evaluate(async () => {
    await fetch("/api/filetree/removeDoc", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "/笔记本A/E2E快照.md" }),
    });
  });
  check("清理快照测试文档", true);

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

  // 11b. wikilink autocomplete: type [[ in editor -> suggestion list appears
  await page.click(".cm-content");
  await page.keyboard.press("Control+End");
  await page.keyboard.type("\n\n[[计划");
  await page.waitForSelector(".cm-tooltip-autocomplete", { timeout: 5000 });
  const acText = await page.textContent(".cm-tooltip-autocomplete");
  check("wikilink 自动补全", acText.includes("计划"), acText.slice(0, 40));
  await page.keyboard.press("Escape");
  // remove the typed text (no save)
  await page.keyboard.press("Control+End");
  for (let i = 0; i < 4; i++) await page.keyboard.press("Backspace");

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
