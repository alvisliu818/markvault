import { api, readText } from "./api";
import { createEditor } from "./editor/cm";
import type { EditorView } from "@codemirror/view";
import { el } from "./ui";

export interface EdTab {
  path: string;
  title: string;
  view: EditorView | null;
  dirty: boolean;
  hostEl: HTMLElement | null;
  previewEl: HTMLElement | null;
  previewVisible: boolean;
  saveTimer: number | null;
}

interface TabHooks {
  onTitleChange: () => void;
  onOutlineRefresh: (path: string) => void;
  onStatus: (s: string) => void;
  resolveLink: (target: string) => Promise<string | null>;
}

/**
 * Editor tab manager: one CM6 instance per open doc, debounced save via putFile.
 */
export class TabManager {
  tabs: EdTab[] = [];
  active: EdTab | null = null;
  private bar: HTMLElement;
  private area: HTMLElement;
  private status: (s: string) => void;

  constructor(bar: HTMLElement, area: HTMLElement, statusFn: (s: string) => void, private hooks: TabHooks) {
    this.bar = bar;
    this.area = area;
    this.status = statusFn;
  }

  async open(path: string, title?: string) {
    const existing = this.tabs.find((t) => t.path === path);
    if (existing) {
      this.activate(existing);
      return;
    }
    let content = "";
    try {
      content = await readText(path);
    } catch (e) {
      this.status(`打开失败: ${path}`);
      return;
    }
    const tab: EdTab = {
      path,
      title: title || path.split("/").pop()!.replace(/\.md$/, ""),
      view: null,
      dirty: false,
      hostEl: null,
      previewEl: null,
      previewVisible: false,
      saveTimer: null,
    };
    this.tabs.push(tab);
    this.renderBar();
    this.activate(tab, content);
  }

  activate(tab: EdTab, content?: string) {
    this.active = tab;
    // hide others
    for (const t of this.tabs) {
      if (t !== tab) t.hostEl?.classList.add("hidden");
    }
    this.renderBar();
    if (!tab.hostEl) {
      this.buildPane(tab, content ?? "");
    } else {
      tab.hostEl.classList.remove("hidden");
    }
    this.hooks.onOutlineRefresh(tab.path);
    this.hooks.onStatus(`${tab.path}`);
    history.replaceState(null, "", `#/doc/${encodeURIComponent(tab.path)}`);
  }

  private buildPane(tab: EdTab, content: string) {
    const pane = el("div", "editor-pane");
    const row = el("div");
    row.style.cssText = "display:flex;align-items:center;gap:8px;padding:6px 12px;border-bottom:1px solid var(--border);flex-shrink:0";
    const previewBtn = el("button", "btn", "预览");
    const saveBtn = el("button", "btn", "保存");
    const attrsBtn = el("button", "btn", "属性");
    row.append(previewBtn, saveBtn, attrsBtn);
    const host = el("div", "cm-host");
    const preview = el("div", "preview-pane hidden");
    pane.append(row, host, preview);
    pane.classList.add("hidden");
    this.area.appendChild(pane);
    tab.hostEl = pane;
    tab.previewEl = preview;

    const save = async () => {
      if (!tab.view) return;
      const text = tab.view.state.doc.toString();
      try {
        await api.putFile(tab.path, text);
        tab.dirty = false;
        this.renderBar();
        this.status("已保存 " + tab.path);
        this.hooks.onOutlineRefresh(tab.path);
      } catch (e) {
        this.status("保存失败: " + (e as Error).message);
      }
    };

    const view = createEditor(host, {
      doc: content,
      path: tab.path,
      onDirty: () => {
        tab.dirty = true;
        this.renderBar();
        if (tab.saveTimer) window.clearTimeout(tab.saveTimer);
        tab.saveTimer = window.setTimeout(save, 900);
        if (tab.previewVisible) this.renderPreview(tab);
      },
      onSave: () => void save(),
      onOpenLink: (target) => void this.hooks.resolveLink(target),
      docsProvider: () => [],
    });
    tab.view = view;

    saveBtn.onclick = () => void save();
    previewBtn.onclick = () => {
      tab.previewVisible = !tab.previewVisible;
      preview.classList.toggle("hidden", !tab.previewVisible);
      previewBtn.textContent = tab.previewVisible ? "关闭预览" : "预览";
      if (tab.previewVisible) this.renderPreview(tab);
    };
    attrsBtn.onclick = () => this.showAttrs(tab);
    // Ctrl+S
    view.dom.addEventListener("keydown", (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      }
    });
  }

  private async renderPreview(tab: EdTab) {
    if (!tab.view || !tab.previewEl) return;
    const md = tab.view.state.doc.toString();
    try {
      const { content } = await api.renderMd(md, tab.path);
      tab.previewEl.innerHTML = content;
      // in-preview doc links
      tab.previewEl.querySelectorAll('a[href^="/#/doc/"]').forEach((a) => {
        a.addEventListener("click", (e) => {
          e.preventDefault();
          const target = decodeURIComponent((a as HTMLAnchorElement).getAttribute("href")!.replace("/#/doc/", ""));
          void this.hooks.resolveLink(target);
        });
      });
    } catch {
      /* ignore */
    }
  }

  private async showAttrs(tab: EdTab) {
    const attrs = await api.getAttrs(tab.path);
    const lines = Object.entries(attrs)
      .map(([k, v]) => `${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`)
      .join("\n");
    const edit = window.prompt("frontmatter 属性 (key: value 每行, 删除行=移除)", lines || "");
    if (edit === null) return;
    const next: Record<string, unknown> = {};
    for (const line of edit.split("\n")) {
      const i = line.indexOf(":");
      if (i <= 0) continue;
      const k = line.slice(0, i).trim();
      let v: unknown = line.slice(i + 1).trim();
      if (v === "") continue;
      // arrays like ["[[A]]"] stay strings; try JSON parse for numbers/arrays
      try { v = JSON.parse(v as string); } catch { /* keep string */ }
      next[k] = v;
    }
    await api.setAttrs(tab.path, next);
    this.status("属性已更新");
    this.hooks.onTitleChange();
  }

  async close(tab: EdTab) {
    if (tab.dirty) {
      // flush save
      if (tab.view) await api.putFile(tab.path, tab.view.state.doc.toString());
    }
    if (tab.saveTimer) window.clearTimeout(tab.saveTimer);
    tab.hostEl?.remove();
    this.tabs = this.tabs.filter((t) => t !== tab);
    if (this.active === tab) {
      this.active = this.tabs[this.tabs.length - 1] ?? null;
      if (this.active) this.activate(this.active);
    }
    this.renderBar();
  }

  renderBar() {
    this.bar.innerHTML = "";
    for (const t of this.tabs) {
      const tabEl = el("div", "ed-tab" + (t === this.active ? " active" : ""));
      const title = el("span", "title", t.title);
      tabEl.append(title);
      if (t.dirty) tabEl.append(el("span", "dirty", "●"));
      const close = el("span", "close", "✕");
      close.onclick = (e) => {
        e.stopPropagation();
        void this.close(t);
      };
      tabEl.append(close);
      tabEl.onclick = () => this.activate(t);
      this.bar.appendChild(tabEl);
    }
  }

  // Called when kernel renamed/deleted a doc
  async syncRename(oldPath: string, newPath: string, newTitle?: string) {
    const t = this.tabs.find((x) => x.path === oldPath);
    if (!t) return;
    t.path = newPath;
    t.title = newTitle || newPath.split("/").pop()!.replace(/\.md$/, "");
    if (t.view) {
      // reload content from new location
      try {
        const text = await readText(newPath);
        t.view.dispatch({ changes: { from: 0, to: t.view.state.doc.length, insert: text } });
        t.dirty = false;
      } catch { /* ignore */ }
    }
    if (this.active === t) history.replaceState(null, "", `#/doc/${encodeURIComponent(newPath)}`);
    this.renderBar();
  }

  syncDelete(path: string) {
    const t = this.tabs.find((x) => x.path === path);
    if (t) void this.close(t);
  }
}
