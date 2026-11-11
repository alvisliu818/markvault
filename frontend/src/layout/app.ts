import { api, onWsPush } from "../api";
import { post } from "../api";
import { el, toast } from "../ui";
import { TabManager } from "../tabs";
import { FileTreePanel } from "../panels/filetree";
import { SearchPanel } from "../panels/search";
import { OutlinePanel } from "../panels/outline";
import { BacklinkPanel } from "../panels/backlinks";
import { BookmarkPanel } from "../panels/bookmark";
import { BrowserPanel } from "../panels/browser";
import { VTreePanel } from "../vtree/panel";
import { settingsButton } from "../panels/settings";
import { editorTheme } from "../editor/cm";

/**
 * Main application shell:
 *   toolbar / [dock | center | right-panel]
 * dock has tabs: 文件 / 虚拟树
 * right has tabs: 大纲 / 反链 / 搜索 / 书签
 */
export class App {
  tabs!: TabManager;
  private outline!: OutlinePanel;
  private backlinks!: BacklinkPanel;
  private filetree!: FileTreePanel;
  private vtree!: VTreePanel;
  private statusEl!: HTMLElement;
  private breadcrumbEl!: HTMLElement;
  private dockPages: Record<string, HTMLElement> = {};
  private docIndex: { path: string; title: string }[] = [];

  async mount(root: HTMLElement) {
    const toolbar = el("div", "mv-toolbar");
    const brand = el("span", "brand", "markvault");
    const themeBtn = el("button", "btn", "主题");
    const settingsBtn = settingsButton();
    const rebuildBtn = el("button", "btn", "重建索引");
    const clipBtn = el("button", "btn", "剪藏");
    const exportNbBtn = el("button", "btn", "导出笔记本");
    toolbar.append(brand, el("span", "", ""), rebuildBtn, clipBtn, exportNbBtn, settingsBtn, themeBtn);

    const main = el("div", "mv-main");
    const dock = el("div", "mv-dock");
    const dockResizer = el("div", "mv-dock-resizer");
    const center = el("div", "mv-center");
    const right = el("div", "mv-right");

    // dock tabs
    const dockTabs = el("div", "dock-tabs");
    const filePage = el("div", "dock-page");
    const vtPage = el("div", "dock-page hidden");
    const browserPage = el("div", "dock-page hidden");
    this.dockPages = { file: filePage, vt: vtPage, browser: browserPage };
    this.addTabBar(dockTabs, [
      { key: "file", label: "文件" },
      { key: "vt", label: "虚拟树" },
      { key: "browser", label: "浏览器" },
    ], { file: filePage, vt: vtPage, browser: browserPage });
    dock.append(dockTabs, filePage, vtPage, browserPage);

    // editor area (breadcrumb above tabs)
    this.breadcrumbEl = el("div", "breadcrumb hidden");
    const edBar = el("div", "editor-tabs");
    const edArea = el("div", "editor-area");
    this.statusEl = el("div", "status-bar", "就绪");
    center.append(this.breadcrumbEl, edBar, edArea, this.statusEl);

    // right panel
    const rightTabs = el("div", "dock-tabs");
    const outlinePage = el("div", "dock-page");
    const backlinkPage = el("div", "dock-page hidden");
    const searchPage = el("div", "dock-page hidden");
    const bookmarkPage = el("div", "dock-page hidden");
    this.addTabBar(rightTabs, [
      { key: "outline", label: "大纲" },
      { key: "backlink", label: "反链" },
      { key: "search", label: "搜索" },
      { key: "bookmark", label: "书签" },
    ], { outline: outlinePage, backlink: backlinkPage, search: searchPage, bookmark: bookmarkPage });
    right.append(rightTabs, outlinePage, backlinkPage, searchPage, bookmarkPage);

    main.append(dock, dockResizer, center, right);
    root.append(toolbar, main);

    // wire components
    this.tabs = new TabManager(edBar, edArea, (s) => (this.statusEl.textContent = s), {
      onTitleChange: () => void this.refreshTitle(),
      onOutlineRefresh: (p) => void this.outline.refresh(p),
      onBacklinksRefresh: (p) => this.backlinks.schedule(p),
      onStatus: (s) => (this.statusEl.textContent = s),
      onBreadcrumb: (p) => this.renderBreadcrumb(p),
      resolveLink: (target, anchor) => this.resolveLink(target, anchor),
      docsProvider: () => this.docIndex,
    });
    this.outline = new OutlinePanel(outlinePage, (line) => this.jumpToLine(line));
    this.backlinks = new BacklinkPanel(backlinkPage, {
      openDoc: (p, t) => void this.tabs.open(p, t),
    });
    this.filetree = new FileTreePanel(filePage, {
      openDoc: (p, t) => void this.tabs.open(p, t),
      onAddRoot: async (p) => {
        await api.addRoot(p);
        toast("已设为虚拟树根");
        this.vtree.refresh();
        this.switchDock("vt");
      },
      onMountToTree: (p, name, isDoc) => {
        this.vtree.addMountRoot(p, name, isDoc ? "doc" : "notebook");
        toast("已挂载到挂载树");
      },
    });
    this.vtree = new VTreePanel(vtPage, {
      openDoc: (p, t) => void this.tabs.open(p, t),
      locateDoc: (p) => void this.tabs.open(p),
    });
    new SearchPanel(searchPage, (p, t) => void this.tabs.open(p, t));
    new BookmarkPanel(bookmarkPage);
    new BrowserPanel(browserPage);

    // dock resize
    let dragging = false;
    dockResizer.onmousedown = () => (dragging = true);
    window.addEventListener("mousemove", (e) => {
      if (!dragging) return;
      dock.style.width = Math.min(480, Math.max(180, e.clientX)) + "px";
    });
    window.addEventListener("mouseup", () => (dragging = false));

    // toolbar actions
    rebuildBtn.onclick = async () => {
      await post("/api/index/rebuild");
      toast("索引重建已开始");
    };
    clipBtn.onclick = async () => {
      const { promptDialog } = await import("../ui");
      const url = await promptDialog("剪藏网页", "输入网页 URL");
      if (!url) return;
      try {
        const { notebooks } = await api.lsNotebooks();
        const nb = notebooks[0]?.name || "";
        const r = await api.clipUrl(url, nb, []);
        toast(`剪藏成功: ${r.title}`);
        await this.filetree.refresh();
        void this.tabs.open(r.path, r.title);
      } catch (e) {
        toast("剪藏失败: " + (e as Error).message);
      }
    };
    exportNbBtn.onclick = async () => {
      const nb = this.docIndex[0]?.path.split("/")[0];
      if (!nb) {
        toast("无笔记本可导出");
        return;
      }
      try {
        const r = (await post<{ path: string; count: number }>("/api/export/exportNotebookMd", { notebook: nb })) as any;
        toast(`已导出 ${r.count} 个文件 → ${r.path}`);
      } catch (e) {
        toast("导出失败: " + (e as Error).message);
      }
    };
    themeBtn.onclick = () => {
      const cur = document.documentElement.getAttribute("data-theme") ?? "dark";
      const next = cur === "dark" ? "light" : "dark";
      document.documentElement.setAttribute("data-theme", next);
      void api.setConf({ theme: next === "dark" ? 1 : 0 });
    };
    void editorTheme; // keep theme helper referenced

    // ws events
    onWsPush((cmd, data) => {
      if (cmd === "vt-changed") this.vtree.refresh();
      if (cmd === "filetree-changed") {
        void this.refreshDocIndex();
        // sync open tabs on rename/delete
        void this.syncTabsWithTree(data);
      }
    });

    await this.refreshDocIndex();
    await this.filetree.refresh();
    this.vtree.refresh();
    this.applyTheme();
  }

  private addTabBar(
    bar: HTMLElement,
    tabs: { key: string; label: string }[],
    pages: Record<string, HTMLElement>
  ) {
    tabs.forEach((t, i) => {
      const tabEl = el("div", "dock-tab" + (i === 0 ? " active" : ""), t.label);
      tabEl.onclick = () => this.switchTabIn(bar, tabs, pages, t.key);
      bar.appendChild(tabEl);
    });
  }

  private switchTabIn(
    bar: HTMLElement,
    tabs: { key: string; label: string }[],
    pages: Record<string, HTMLElement>,
    key: string
  ) {
    for (const [k, page] of Object.entries(pages)) {
      page.classList.toggle("hidden", k !== key);
    }
    Array.from(bar.children).forEach((c, i) => {
      c.classList.toggle("active", tabs[i]?.key === key);
    });
  }

  switchDock(key: string) {
    const dockTabBar = document.querySelector(".mv-dock .dock-tabs") as HTMLElement;
    if (dockTabBar) {
      const tabs = [
        { key: "file", label: "文件" },
        { key: "vt", label: "虚拟树" },
        { key: "browser", label: "浏览器" },
      ];
      this.switchTabIn(dockTabBar, tabs, this.dockPages, key);
    }
  }

  async refreshDocIndex() {
    try {
      const { docs } = await api.listDocTree();
      this.docIndex = docs.map((d) => ({ path: d.path.replace(/^\//, ""), title: d.title }));
      // push fresh index into all open editors (wikilink resolved styling)
      for (const t of this.tabs.tabs) this.tabs.refreshEditorContext(t);
    } catch {
      /* ignore */
    }
  }

  async refreshTitle() {
    await this.refreshDocIndex();
  }

  async resolveLink(target: string, anchor?: string): Promise<string | null> {
    // path (with/without .md) > title > basename
    const t = target.replace(/^\[\[|\]\]$/g, "").trim();
    const withMd = t.endsWith(".md") ? t : t + ".md";
    const hit =
      this.docIndex.find((d) => d.path === withMd) ||
      this.docIndex.find((d) => d.path === t) ||
      this.docIndex.find((d) => d.title === t) ||
      this.docIndex.find((d) => d.path.split("/").pop()?.replace(/\.md$/, "") === t);
    if (hit) {
      await this.tabs.open(hit.path, hit.title, anchor);
      return hit.path;
    }
    toast(`链接未找到: ${t}`);
    return null;
  }

  private renderBreadcrumb(path: string | null) {
    this.breadcrumbEl.innerHTML = "";
    if (!path) {
      this.breadcrumbEl.classList.add("hidden");
      return;
    }
    this.breadcrumbEl.classList.remove("hidden");
    const segs = path.replace(/\.md$/, "").split("/").filter(Boolean);
    segs.forEach((seg, i) => {
      if (i > 0) this.breadcrumbEl.append(el("span", "bc-sep", "›"));
      const crumb = el("span", "bc-item" + (i === segs.length - 1 ? " current" : ""), seg);
      if (i < segs.length - 1) {
        // click a parent segment -> open that doc if it exists as .md
        const prefix = segs.slice(0, i + 1).join("/") + ".md";
        crumb.onclick = () => {
          const hit = this.docIndex.find((d) => d.path === prefix);
          if (hit) void this.tabs.open(hit.path, hit.title);
        };
      }
      this.breadcrumbEl.append(crumb);
    });
  }

  jumpToLine(line: number) {
    const tab = this.tabs.active;
    if (!tab?.view) return;
    const doc = tab.view.state.doc;
    const lineInfo = doc.line(Math.min(Math.max(1, line), doc.lines));
    tab.view.dispatch({
      selection: { anchor: lineInfo.from },
      scrollIntoView: true,
    });
    tab.view.focus();
  }

  private async syncTabsWithTree(_data: unknown) {
    // rename/delete of open tabs is handled via filetree panel flows + kernel ws
  }

  private applyTheme() {
    // kernel conf theme: 0 light 1 dark; default dark
    api.getConf().then((c: any) => {
      const theme = c?.conf?.theme ?? 1;
      document.documentElement.setAttribute("data-theme", theme === 0 ? "light" : "dark");
    }).catch(() => document.documentElement.setAttribute("data-theme", "dark"));
  }
}
