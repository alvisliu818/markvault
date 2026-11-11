import { api, type HistoryEntry } from "../api";
import { el, toast, showMenu, confirmDialog } from "../ui";

/**
 * Embedded browser panel (M6, Tauri multi-window scheme).
 *
 * The panel is the controller: tab list / new / activate / address bar /
 * back-forward-reload-stop / clip / bookmark / download-to-vault.
 * Each tab maps to a separate WebviewWindow managed by the Tauri shell
 * (app/src-tauri/src/browser.rs); state syncs over Tauri events.
 * Outside the desktop shell (plain browser / E2E) it degrades to a hint
 * while the history section still works over the kernel API.
 */

// ---- Tauri IPC bridge (only present inside the desktop shell) ----
interface TauriBridge {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event: { listen: (ev: string, h: (e: { payload: any }) => void) => Promise<() => void> };
}
function tauri(): TauriBridge | null {
  return (window as any).__TAURI__ ?? null;
}

/** Address-bar completion, ported from siyuan-plugin-browser utils/url.ts:
 *  scheme:// -> as-is; example.com / localhost:port -> https:// / http://;
 *  anything else -> default search engine. */
export function normalizeUrl(input: string): string {
  const t = input.trim();
  if (!t) return "about:blank";
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(t)) return t;
  if (t.startsWith("about:")) return t;
  if (/^[\w-]+(\.[\w-]+)+(\/.*)?$/.test(t) || /^localhost(:\d+)?(\/.*)?$/.test(t)) {
    return "https://" + t;
  }
  return SEARCH_URL.replace("{q}", encodeURIComponent(t));
}

const SEARCH_URL = "https://www.bing.com/search?q={q}";

interface BrowserTab {
  tabId: string;
  url: string;
  title: string;
}

function fmtTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getMonth() + 1}-${d.getDate()} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

export class BrowserPanel {
  private listEl: HTMLElement;
  private histEl: HTMLElement;
  private addr: HTMLInputElement;
  private tabs = new Map<string, BrowserTab>();
  private order: string[] = [];
  private activeId: string | null = null;
  private ta: TauriBridge | null;

  constructor(container: HTMLElement) {
    this.ta = tauri();

    // toolbar: nav buttons + address bar + page actions
    const bar = el("div", "browser-bar");
    this.addr = el("input", "browser-addr") as HTMLInputElement;
    this.addr.type = "text";
    this.addr.placeholder = "输入网址或搜索词，回车打开";
    const backBtn = el("button", "btn browser-nav", "◀");
    const fwdBtn = el("button", "btn browser-nav", "▶");
    const reloadBtn = el("button", "btn browser-nav", "⟳");
    const stopBtn = el("button", "btn browser-nav", "✕");
    const homeBtn = el("button", "btn browser-nav", "⌂");
    homeBtn.title = "主页";
    homeBtn.onclick = () => {
      if (!this.activeId) void this.openNew("about:blank");
      else void this.invoke("browser_navigate", { tabId: this.activeId, url: "about:blank" });
    };
    for (const [b, act] of [
      [backBtn, "back"],
      [fwdBtn, "forward"],
      [reloadBtn, "reload"],
      [stopBtn, "stop"],
    ] as [HTMLElement, string][]) {
      b.title = act;
      b.onclick = () => this.act(act);
    }
    const newTabBtn = el("button", "btn", "新页签");
    const clipBtn = el("button", "btn", "剪藏");
    const favBtn = el("button", "btn", "收藏");
    const dlBtn = el("button", "btn", "下载到 vault");
    bar.append(backBtn, fwdBtn, reloadBtn, stopBtn, homeBtn, this.addr, newTabBtn, clipBtn, favBtn, dlBtn);

    // tabs
    const tabsHead = el("div", "panel-head", "页签");
    this.listEl = el("div", "tree browser-tabs");

    // history
    const histHead = el("div", "panel-head");
    histHead.append(el("span", "", "历史记录"));
    const refreshBtn = el("button", "btn", "刷新");
    refreshBtn.onclick = () => void this.refreshHistory();
    const clearBtn = el("button", "btn", "清空");
    clearBtn.onclick = () => void this.clearHistory();
    histHead.append(refreshBtn, clearBtn);
    this.histEl = el("div", "tree browser-history");

    container.append(bar, tabsHead, this.listEl, histHead, this.histEl);

    this.addr.onkeydown = (e) => {
      if (e.key === "Enter") void this.go();
    };
    newTabBtn.onclick = () => void this.openNew();
    clipBtn.onclick = () => void this.clipActive();
    favBtn.onclick = () => void this.favActive();
    dlBtn.onclick = () => void this.downloadActive();

    if (!this.ta) {
      const hint = el("div", "search-row dim", "浏览器窗口需在 markvault 桌面壳（Tauri）内使用");
      this.listEl.append(hint);
    } else {
      this.bindEvents();
      void this.sync();
    }
    void this.refreshHistory();
  }

  // ---- shell state sync ----

  private bindEvents() {
    const ta = this.ta!;
    void ta.event.listen("browser:opened", (e) => {
      const p = e.payload as { tabId: string; url: string };
      if (!this.tabs.has(p.tabId)) {
        this.tabs.set(p.tabId, { tabId: p.tabId, url: p.url, title: "" });
        this.order.push(p.tabId);
      }
      this.setActive(p.tabId);
      this.renderTabs();
    });
    void ta.event.listen("browser:url", (e) => {
      const p = e.payload as { tabId: string; url: string; title: string };
      const t = this.tabs.get(p.tabId);
      if (t) {
        t.url = p.url;
        t.title = p.title || hostOf(p.url);
      }
      if (p.tabId === this.activeId) this.addr.value = p.url;
      this.renderTabs();
      // kernel-side 10min dedup / cap
      void api.historyAdd(p.url, p.title).catch(() => {});
      void this.refreshHistory();
    });
    void ta.event.listen("browser:closed", (e) => {
      const id = (e.payload as { tabId: string }).tabId;
      this.tabs.delete(id);
      this.order = this.order.filter((x) => x !== id);
      if (this.activeId === id) {
        this.activeId = this.order[this.order.length - 1] ?? null;
        if (this.activeId) this.addr.value = this.tabs.get(this.activeId)?.url ?? "";
      }
      this.renderTabs();
    });
  }

  private async sync() {
    try {
      const list = (await this.ta!.core.invoke("browser_list")) as {
        tabId: string;
        url: string;
        title: string;
      }[];
      this.tabs.clear();
      this.order = [];
      for (const t of list) {
        this.tabs.set(t.tabId, { tabId: t.tabId, url: t.url, title: t.title || hostOf(t.url) });
        this.order.push(t.tabId);
      }
      if (!this.activeId && this.order.length) this.activeId = this.order[this.order.length - 1];
      this.renderTabs();
    } catch {
      /* shell not ready */
    }
  }

  private setActive(id: string | null) {
    this.activeId = id;
    if (id) this.addr.value = this.tabs.get(id)?.url ?? "";
  }

  // ---- actions ----

  private newTabId(): string {
    return (
      "t" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6)
    );
  }

  private async invoke(cmd: string, args?: Record<string, unknown>) {
    if (!this.ta) {
      toast("需在 markvault 桌面壳内使用");
      return;
    }
    try {
      await this.ta.core.invoke(cmd, args);
    } catch (e) {
      toast((e as Error).message || String(e));
    }
  }

  private async openNew(url?: string) {
    const raw = url ?? this.addr.value;
    const target = normalizeUrl(raw || "about:blank");
    const tabId = this.newTabId();
    await this.invoke("browser_open", { tabId, url: target });
    // optimistic add; browser:opened event reconciles
    this.tabs.set(tabId, { tabId, url: target, title: hostOf(target) });
    this.order.push(tabId);
    this.setActive(tabId);
    this.renderTabs();
  }

  private async go() {
    if (!this.addr.value.trim()) return;
    if (!this.activeId) {
      await this.openNew();
      return;
    }
    await this.invoke("browser_navigate", {
      tabId: this.activeId,
      url: normalizeUrl(this.addr.value),
    });
  }

  private async act(action: string) {
    if (!this.activeId) {
      toast("无活动页签");
      return;
    }
    await this.invoke("browser_act", { tabId: this.activeId, action });
  }

  private async closeTab(tabId: string) {
    await this.invoke("browser_close", { tabId });
  }

  private firstNotebook = async (): Promise<string> => {
    const { notebooks } = await api.lsNotebooks();
    return notebooks[0]?.name || "";
  };

  private async clipActive() {
    const t = this.tabs.get(this.activeId ?? "");
    if (!t || !t.url || t.url === "about:blank") {
      toast("无活动页签");
      return;
    }
    try {
      const nb = await this.firstNotebook();
      const r = await api.clipUrl(t.url, nb, []);
      toast(`剪藏成功: ${r.title}`);
    } catch (e) {
      toast("剪藏失败: " + (e as Error).message);
    }
  }

  private async favActive() {
    const t = this.tabs.get(this.activeId ?? "");
    if (!t || !t.url || t.url === "about:blank") {
      toast("无活动页签");
      return;
    }
    try {
      await api.bookmarkAdd(t.url, t.title || t.url);
      toast("已收藏");
    } catch (e) {
      toast("收藏失败: " + (e as Error).message);
    }
  }

  private async downloadActive() {
    const t = this.tabs.get(this.activeId ?? "");
    const url = await promptDefault("下载资源到 vault", "资源 URL", t?.url ?? "");
    if (!url) return;
    let nb = "";
    try {
      nb = await this.firstNotebook();
    } catch {
      /* keep empty, kernel falls back */
    }
    try {
      const r = await api.downloadToLocal(url, nb);
      toast(`已保存: ${r.path} (${r.len} B)`);
    } catch (e) {
      toast("下载失败: " + (e as Error).message);
    }
  }

  // ---- rendering ----

  private renderTabs() {
    this.listEl.innerHTML = "";
    if (!this.order.length) {
      this.listEl.append(
        el("div", "search-row dim", this.ta ? "无打开的页签，地址栏回车或点「新页签」" : "浏览器窗口需在 markvault 桌面壳（Tauri）内使用")
      );
      return;
    }
    for (const id of this.order) {
      const t = this.tabs.get(id)!;
      const row = el("div", "tree-row browser-tab-row" + (id === this.activeId ? " active" : ""));
      const label = el("div", "label", t.title || hostOf(t.url));
      const path = el("div", "path", t.url);
      const main = el("div", "browser-tab-main");
      main.append(label, path);
      const close = el("span", "browser-tab-close", "✕");
      close.onclick = (e) => {
        e.stopPropagation();
        void this.closeTab(id);
      };
      const activate = () => {
        this.setActive(id);
        this.renderTabs();
        void this.invoke("browser_focus", { tabId: id });
      };
      row.append(main, close);
      row.onclick = activate;
      row.oncontextmenu = (e) => {
        e.preventDefault();
        showMenu(e.clientX, e.clientY, [
          { label: "激活", action: activate },
          {
            label: "关闭页签",
            danger: true,
            action: () => void this.closeTab(id),
          },
        ]);
      };
      this.listEl.append(row);
    }
  }

  private async refreshHistory() {
    try {
      const items = await api.historyList();
      this.renderHistory(items);
    } catch {
      /* kernel offline */
    }
  }

  private renderHistory(items: HistoryEntry[]) {
    this.histEl.innerHTML = "";
    if (!items.length) {
      this.histEl.append(el("div", "search-row dim", "无历史记录"));
      return;
    }
    const sorted = [...items].sort((a, b) => b.created - a.created).slice(0, 100);
    for (const h of sorted) {
      const row = el("div", "search-row browser-hist-row");
      const title = el("div", "", h.title || h.url);
      const meta = el("div", "path", `${fmtTime(h.created)} · ${h.url}`);
      row.append(title, meta);
      row.onclick = () => void this.openNew(h.url);
      row.oncontextmenu = (e) => {
        e.preventDefault();
        showMenu(e.clientX, e.clientY, [
          { label: "新页签打开", action: () => void this.openNew(h.url) },
          {
            label: "删除该条",
            danger: true,
            action: async () => {
              await api.historyRemove(h.url);
              void this.refreshHistory();
            },
          },
        ]);
      };
      this.histEl.append(row);
    }
  }

  private async clearHistory() {
    if (!(await confirmDialog("清空历史", "确定清空全部浏览器历史记录？"))) return;
    await api.historyClear();
    void this.refreshHistory();
  }
}

/** promptDialog wrapper with a default value + non-null trim */
async function promptDefault(title: string, placeholder: string, initial: string): Promise<string | null> {
  const { promptDialog } = await import("../ui");
  const v = await promptDialog(title, placeholder, initial);
  return v && v.trim() ? v.trim() : null;
}
