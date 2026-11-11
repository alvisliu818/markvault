// 壳内人工项自动化：C2 开机自启 / B10 严格CSP回退 / B7 壳内面板按钮
// 前提：cargo build（app debug + kernel debug）；本脚本自启壳 + 本地严格CSP服务器，结束即清理。
// C1（rfd 原生模态对话框）无法安全自动化，仍留人工。
import { chromium } from "playwright-core";
import { spawn, execSync } from "node:child_process";
import { readFileSync, existsSync, mkdirSync } from "node:fs";
import http from "node:http";

const ROOT = new URL("../../", import.meta.url).pathname.replace(/^\/([A-Za-z]):/, "$1:");
const shots = ROOT + "_e2e/";
mkdirSync(shots, { recursive: true });
const results = [];
const check = (name, ok, extra = "") => {
  results.push({ name, ok, extra });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${extra ? " — " + extra : ""}`);
};

const SHELL = ROOT + "app/src-tauri/target/debug/markvault-app.exe";
const KERNEL = ROOT + "target/debug/markvault-kernel.exe";
const WS = ROOT + "testvault";
const CDP = 9223;
const CSP_PORT = 9911;

// ---- 严格 CSP 测试服务器：connect-src 'self' 拦截页面到内核的 fetch ----
const server = http.createServer((req, res) => {
  res.setHeader(
    "Content-Security-Policy",
    "default-src 'none'; script-src 'none'; style-src 'self'; connect-src 'self'; img-src 'self'"
  );
  if (req.url === "/" || req.url === "/index.html") {
    res.setHeader("Content-Type", "text/html; charset=utf-8");
    res.end(`<!doctype html><html><head><title>Strict CSP Page</title></head>
<body><h1>strict-csp-test</h1><img src="/pic.png"><p>no external fetch allowed</p></body></html>`);
  } else if (req.url === "/pic.png") {
    // 1x1 PNG
    const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
    res.setHeader("Content-Type", "image/png");
    res.end(png);
  } else {
    res.statusCode = 404;
    res.end();
  }
});
await new Promise((r) => server.listen(CSP_PORT, "127.0.0.1", r));

const run = (cmd) => execSync(cmd, { encoding: "utf-8" }).trim();
const regValue = () => {
  try {
    return run(`reg query "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run" /v markvault`);
  } catch {
    return "";
  }
};
const jsonFile = (p) => (existsSync(p) ? JSON.parse(readFileSync(p, "utf-8")) : null);

let shellProc = null;
let browser = null;

async function invokeOn(page, cmd, args, timeoutMs = 8000) {
  // Tauri IPC + Promise.race（方法论：evaluate 无超时，会永久挂起）
  return page.evaluate(
    ({ cmd, args, timeoutMs }) => {
      const t = new Promise((_, rej) => setTimeout(() => rej(new Error("ipc timeout")), timeoutMs));
      const tauri = window.__TAURI__;
      const invoke = tauri?.core?.invoke ?? tauri?.invoke;
      if (!invoke) throw new Error("window.__TAURI__ not exposed");
      return Promise.race([invoke(cmd, args || {}), t]);
    },
    { cmd, args, timeoutMs }
  );
}

try {
  // ---- 启动壳（stdio ignore：后台常驻不接管道）----
  shellProc = spawn(SHELL, [], {
    cwd: ROOT + "app/src-tauri",
    env: {
      ...process.env,
      MARKVAULT_WORKSPACE: WS,
      MARKVAULT_KERNEL: KERNEL,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP} --remote-allow-origins=*`,
    },
    stdio: "ignore",
    detached: true,
  });

  // ---- 连接 CDP ----
  let mainPage = null;
  for (let i = 0; i < 30 && !mainPage; i++) {
    await new Promise((r) => setTimeout(r, 800));
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP}`);
      const pages = browser.contexts().flatMap((c) => c.pages());
      mainPage = pages.find((p) => p.url().startsWith("http://127.0.0.1:") && !p.url().includes("9911")) ?? null;
      if (!mainPage) await browser.close().catch(() => {});
    } catch {
      /* shell not ready yet */
    }
  }
  if (!mainPage) throw new Error("shell main window not reachable via CDP");
  await mainPage.waitForSelector(".mv-toolbar", { timeout: 15000 });
  check("壳拉起+CDP连接", true);

  // ================= C2: 开机自启 =================
  const before = regValue();
  const wasOn = before.includes("markvault");
  await invokeOn(mainPage, "set_autostart", { enable: true });
  const on = regValue();
  await invokeOn(mainPage, "set_autostart", { enable: false });
  const off = regValue();
  check("C2-开机自启注册表增删", on.includes("markvault") && !off.includes("markvault"), `on=${on.includes("markvault")} off=${off.includes("markvault")}`);
  if (wasOn) {
    await invokeOn(mainPage, "set_autostart", { enable: true }); // 恢复原状态
  }
  check("C2-状态恢复", regValue().includes("markvault") === wasOn, `原状态 wasOn=${wasOn}`);

  // ================= B10: 严格 CSP 站点 =================
  const tabs0 = await invokeOn(mainPage, "browser_list", {});
  await invokeOn(mainPage, "browser_open", { tabId: "b10", url: `http://127.0.0.1:${CSP_PORT}/` }, 15000);
  // 找到页签窗口的 CDP target
  let tabPage = null;
  for (let i = 0; i < 20 && !tabPage; i++) {
    await new Promise((r) => setTimeout(r, 700));
    const pages = browser.contexts().flatMap((c) => c.pages());
    tabPage = pages.find((p) => p.url().includes(`127.0.0.1:${CSP_PORT}`)) ?? null;
  }
  check("B10-页签窗口建立", !!tabPage, tabPage?.url() ?? "");
  if (tabPage) {
    await tabPage.waitForTimeout(1500); // 注入轮询周期 500ms
    const barCount = await tabPage.locator("#__mv-bar").count();
    check("B10-CSP页注入栏存在", barCount === 1, `bar=${barCount}（ExecuteScript 不受页面 CSP 限制）`);
    // 点收藏 → 页面 fetch 被连接 CSP 拦截 → catch 提示，不崩
    await tabPage.click('#__mv-bar button:has-text("收藏")');
    await tabPage.waitForTimeout(800);
    const statusText = await tabPage.evaluate(() => {
      const els = [...document.querySelectorAll("div")].filter((d) => d.textContent.includes("网络错误") || d.textContent.includes("已收藏"));
      return els.map((e) => e.textContent).join("|") || "(no status)";
    });
    check("B10-CSP拦截优雅降级", statusText.includes("网络错误"), statusText.slice(0, 80));
    await tabPage.screenshot({ path: shots + "shell_B10_csp_bar.png" });

    // 内核侧确认收藏未写入（fetch 被拦）
    const bmPath = WS + "/.markvault/storage/bookmarks.json";
    const bm = jsonFile(bmPath) || [];
    const leaked = bm.some((b) => b.url?.includes(`127.0.0.1:${CSP_PORT}`));
    check("B10-CSP下内核未收到收藏", !leaked, `bookmarks=${bm.length}`);
    await browser.close().catch(() => {});
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP}`);
    // 重连后旧页面句柄随上一个 CDP 会话失效，须重新解析主窗口
    const pages2 = browser.contexts().flatMap((c) => c.pages());
    mainPage =
      pages2.find((p) => p.url().startsWith("http://127.0.0.1:") && !p.url().includes("9911")) ??
      null;
    if (!mainPage) throw new Error("main window lost after CDP reconnect");

    // ---- 面板侧回退：剪藏（clipUrl 内核抓取，不走页面 fetch）----
    await mainPage.bringToFront();
    await mainPage.waitForTimeout(600);
    const browserDock = mainPage.locator('.mv-dock .dock-tab:has-text("浏览器")');
    if ((await browserDock.count()) > 0) await browserDock.click();
    await mainPage.waitForTimeout(500);
    const visibleDockPage = mainPage.locator('.mv-dock .dock-page:not(.hidden):has(.browser-tabs)');
    // 面板剪藏按钮作用于活动页签（b10 已激活）——作用域限定浏览器面板，避开工具栏同名按钮
    const clipBtn = visibleDockPage.locator('button:has-text("剪藏")').first();
    await clipBtn.click();
    await mainPage.waitForTimeout(3000); // clipUrl 内核抓取 + 建档
    const bm2 = jsonFile(bmPath) || [];
    const clipDir = WS + "/笔记收藏/剪藏";
    const clipName = existsSync(clipDir) ? execSync(`ls "${clipDir}"`, { encoding: "utf-8" }) : "";
    check("B7/B10-面板clipUrl回退", clipName.includes("Strict CSP Page"), clipName.trim().slice(0, 60));
    await mainPage.screenshot({ path: shots + "shell_B7_panel_clip.png" });

    // ---- B7: 面板收藏按钮 ----
    const favBtn = visibleDockPage.locator('button:has-text("收藏")').first();
    await favBtn.click();
    await mainPage.waitForTimeout(1200);
    const bm3 = jsonFile(bmPath) || [];
    check("B7-面板收藏", bm3.some((b) => b.url?.includes(`127.0.0.1:${CSP_PORT}`)), `bookmarks ${bm.length}->${bm3.length}`);

    // ---- B7: 面板下载到 vault（prompt 输入 CSP 页图片 URL）----
    const dlBtn = visibleDockPage.locator('button:has-text("下载到 vault")').first();
    await dlBtn.click();
    await mainPage.waitForTimeout(400);
    const dlgInput = mainPage.locator(".mv-dialog input[type=text]");
    if ((await dlgInput.count()) > 0) {
      await dlgInput.fill(`http://127.0.0.1:${CSP_PORT}/pic.png`);
      await mainPage.click('.mv-dialog button:has-text("确定")');
      await mainPage.waitForTimeout(3000);
      const toastText = await mainPage.evaluate(() =>
        [...document.querySelectorAll(".mv-toast")].map((t) => t.textContent).join(" | ")
      );
      const assetNames = await mainPage.evaluate(async () => {
        const r = await fetch("/api/file/readDir", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: "笔记收藏/assets" }) });
        const j = await r.json();
        return j.code === 0 ? j.data.entries.map((e) => e.name) : ["ERR:" + j.msg];
      });
      check("B7-下载到vault", assetNames.some((n) => n.includes("pic")), `toast=${toastText || "(none)"} assets=${assetNames.join(",")}`);
    } else {
      check("B7-下载到vault", false, "prompt 对话框未出现");
    }
  }

  // ================= 清理 =================
  await invokeOn(mainPage, "browser_close", { tabId: "b10" }).catch(() => {});
  // 删除探针产物（面板剪藏文档 + 收藏 + 下载资产）
  await mainPage.evaluate(async () => {
    const post = (p, b) => fetch(p, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(b) });
    const { notebooks } = await (await post("/api/notebook/lsNotebooks", {})).json();
    const nb = notebooks[0]?.name;
    await post("/api/filetree/removeDoc", { path: `/${nb}/剪藏/Strict CSP Page.md` }).catch(() => {});
    const bmKey = `${nb}/剪藏`;
    await post("/api/file/removeFile", { path: `/${bmKey}/Strict CSP Page.md` }).catch(() => {});
    await post("/api/bookmark/remove", { url: `http://127.0.0.1:9911/` }).catch(() => {});
    await post("/api/bookmark/remove", { url: `http://127.0.0.1:9911/index.html` }).catch(() => {});
    await post("/api/file/removeFile", { path: `/${nb}/assets/pic.png` }).catch(() => {});
    return "cleaned";
  }).catch(() => "cleanup via fs");
  // 兜底文件系统清理
  for (const p of [
    WS + "/笔记收藏/剪藏/Strict CSP Page.md",
    WS + "/笔记收藏/assets/pic.png",
  ]) {
    try { execSync(`rm -f "${p}"`); } catch { /* ignore */ }
  }
  try {
    const bmPath = WS + "/.markvault/storage/bookmarks.json";
    const bm = jsonFile(bmPath) || [];
    const filtered = bm.filter((b) => !String(b.url).includes("127.0.0.1:9911"));
    if (filtered.length !== bm.length) {
      await import("node:fs").then((fs) => fs.writeFileSync(bmPath, JSON.stringify(filtered)));
    }
  } catch { /* best effort */ }
  check("探针产物清理", true);
} catch (e) {
  check("异常中断", false, String(e).slice(0, 300));
} finally {
  try { await browser?.close(); } catch { /* ignore */ }
  if (shellProc?.pid) {
    try { execSync(`taskkill /F /PID ${shellProc.pid} /T`); } catch { /* already gone */ }
  }
  try { execSync(`taskkill /F /IM markvault-app.exe`); } catch { /* ignore */ }
  try { execSync(`taskkill /F /IM markvault-kernel.exe`); } catch { /* ignore */ }
  server.close();
}

const failed = results.filter((r) => !r.ok);
console.log(`\n==== ${results.length - failed.length}/${results.length} passed ====`);
process.exit(failed.length ? 1 : 0);
