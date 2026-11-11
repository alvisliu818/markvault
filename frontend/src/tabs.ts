import { api, readText } from "./api";
import { createEditor, updateEditorContext, type EditorOpts } from "./editor/cm";
import { showHistoryDialog } from "./panels/snapshots";
import type { EditorView } from "@codemirror/view";
import { el, toast, confirmDialog, escapeHtml } from "./ui";

export interface EdTab {
  path: string;
  title: string;
  view: EditorView | null;
  dirty: boolean;
  hostEl: HTMLElement | null;
  previewEl: HTMLElement | null;
  previewVisible: boolean;
  saveTimer: number | null;
  docsComp?: ReturnType<typeof createEditor>["docsComp"];
  pathComp?: ReturnType<typeof createEditor>["pathComp"];
}

interface TabHooks {
  onTitleChange: () => void;
  onOutlineRefresh: (path: string) => void;
  onBacklinksRefresh: (path: string) => void;
  onStatus: (s: string) => void;
  onBreadcrumb: (path: string | null) => void;
  /** resolve + open a doc target; returns resolved path or null */
  resolveLink: (target: string, anchor?: string) => Promise<string | null>;
  /** current doc index for wikilink resolution styling */
  docsProvider: () => { path: string; title: string }[];
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

  async open(path: string, title?: string, anchor?: string) {
    const existing = this.tabs.find((t) => t.path === path);
    if (existing) {
      this.activate(existing);
      if (anchor) this.jumpToAnchor(existing, anchor);
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
    if (anchor) this.jumpToAnchor(tab, anchor);
  }

  /** Scroll to `^block-id` line, else heading anchor, else first text match. */
  private jumpToAnchor(tab: EdTab, anchor: string) {
    if (!tab.view) return;
    const doc = tab.view.state.doc;
    const a = anchor.trim();
    // block ref: ^id at end of line
    if (a.startsWith("^")) {
      const re = new RegExp("\\" + a + "\\s*$");
      for (let i = 1; i <= doc.lines; i++) {
        const line = doc.line(i);
        if (re.test(line.text)) {
          this.jumpLine(tab, i);
          return;
        }
      }
      this.status(`块引用未找到: ${a}`);
      return;
    }
    const lower = a.toLowerCase();
    for (let i = 1; i <= doc.lines; i++) {
      const line = doc.line(i);
      const m = /^#{1,6}\s+(.*)$/.exec(line.text);
      if (m && m[1].trim().toLowerCase() === lower) {
        this.jumpLine(tab, i);
        return;
      }
    }
    for (let i = 1; i <= doc.lines; i++) {
      const line = doc.line(i);
      if (line.text.toLowerCase().includes(lower)) {
        this.jumpLine(tab, i);
        return;
      }
    }
    this.status(`锚点未找到: #${anchor}`);
  }

  private jumpLine(tab: EdTab, lineNo: number) {
    if (!tab.view) return;
    const doc = tab.view.state.doc;
    const li = doc.line(Math.min(Math.max(1, lineNo), doc.lines));
    tab.view.dispatch({ selection: { anchor: li.from }, scrollIntoView: true });
    tab.view.focus();
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
    }
    // buildPane starts hidden; the active tab's pane is now shown
    tab.hostEl?.classList.remove("hidden");
    this.hooks.onOutlineRefresh(tab.path);
    this.hooks.onBacklinksRefresh(tab.path);
    this.hooks.onBreadcrumb(tab.path);
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
    const exportBtn = el("button", "btn", "导出");
    const histBtn = el("button", "btn", "历史");
    row.append(previewBtn, saveBtn, attrsBtn, exportBtn, histBtn);
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
        this.hooks.onBacklinksRefresh(tab.path);
      } catch (e) {
        this.status("保存失败: " + (e as Error).message);
      }
    };

    const opts: EditorOpts = {
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
      onOpenLink: (target, anchor) => {
        void (async () => {
          const resolved = await this.hooks.resolveLink(target, anchor);
          if (!resolved) {
            const ok = await confirmDialog("创建文档", `链接目标不存在，创建「${target.replace(/^\[\[|\]\]$/g, "")}.md」？`);
            if (ok) {
              try {
                const notebook = tab.path.split("/")[0];
                const title = target.replace(/^\[\[|\]\]$/g, "").trim();
                const { path } = await api.createDocWithMd(notebook, "/" + title, "");
                await this.hooks.resolveLink(path.replace(/^\//, "").replace(/\.md$/, ""));
              } catch (e) {
                toast("创建失败: " + (e as Error).message);
              }
            }
          }
        })();
      },
      docsProvider: () => this.hooks.docsProvider(),
      onPasteFile: (file, view) => {
        if (!file.type.startsWith("image/")) return false;
        void this.pasteImage(tab, file, view);
        return true;
      },
    };
    const handle = createEditor(host, opts);
    tab.view = handle.view;
    tab.docsComp = handle.docsComp;
    tab.pathComp = handle.pathComp;
    // keep doc-index facet fresh (path facet is fixed at creation)
    this.refreshEditorContext(tab);

    saveBtn.onclick = () => void save();
    previewBtn.onclick = () => {
      tab.previewVisible = !tab.previewVisible;
      preview.classList.toggle("hidden", !tab.previewVisible);
      previewBtn.textContent = tab.previewVisible ? "关闭预览" : "预览";
      if (tab.previewVisible) this.renderPreview(tab);
    };
    attrsBtn.onclick = () => void this.showAttrs(tab);
    exportBtn.onclick = () => void this.exportDoc(tab);
    histBtn.onclick = () => showHistoryDialog(tab.path, tab.title);
    // Ctrl+S
    handle.view.dom.addEventListener("keydown", (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      }
    });
  }

  /** POST /api/export/exportMd expects JSON body, not query — use fetch+anchor */
  async exportDoc(tab: EdTab) {
    try {
      const r = await fetch("/api/export/exportMd", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path: "/" + tab.path }),
      });
      if (!r.ok) throw new Error(String(r.status));
      const blob = await r.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = tab.title + ".md";
      a.click();
      URL.revokeObjectURL(url);
      toast("已导出 " + tab.title + ".md");
    } catch (e) {
      toast("导出失败: " + (e as Error).message);
    }
  }

  /** Image paste: upload to <notebook>/assets/ then insert a relative link. */
  private async pasteImage(tab: EdTab, file: File, view: EditorView) {
    const notebook = tab.path.split("/")[0];
    const ext = (file.name.split(".").pop() || "png").toLowerCase();
    const name = `paste-${Date.now()}-${Math.random().toString(36).slice(2, 6)}.${ext}`;
    const assetPath = `${notebook}/assets/${name}`;
    this.status("上传图片中…");
    try {
      const buf = await file.arrayBuffer();
      const b64 = arrayBufferToBase64(buf);
      await api.putFileBinary(assetPath, b64);
      const dir = tab.path.split("/").slice(0, -1).join("/");
      // relative link from the doc's directory to the notebook-root assets folder
      const rel = dir === notebook ? `assets/${name}` : `../${notebook}/assets/${name}`;
      view.dispatch({
        changes: { from: view.state.doc.length, insert: `![${file.name || name}](${rel})\n` },
      });
      this.status(`图片已上传 ${assetPath}`);
    } catch (e) {
      toast("图片上传失败: " + (e as Error).message);
    }
  }

  /** Push the latest doc index into the editor's docs facet. */
  refreshEditorContext(tab: EdTab) {
    if (tab.docsComp && tab.view) {
      updateEditorContext(tab.view, tab.docsComp, this.hooks.docsProvider(), null);
    }
  }

  syncEditorPath(tab: EdTab) {
    if (tab.pathComp && tab.view) {
      updateEditorContext(tab.view, tab.docsComp ?? (null as never), [], tab.pathComp, tab.path);
    }
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
      // external links open in a new window/tab (Tauri-safe)
      tab.previewEl.querySelectorAll('a[href^="http"]').forEach((a) => {
        a.addEventListener("click", (e) => {
          e.preventDefault();
          window.open((a as HTMLAnchorElement).href, "_blank");
        });
      });
    } catch {
      /* ignore */
    }
  }

  /**
   * Frontmatter attribute editor: table rows (key/value), delete on ✕,
   * parent wikilink picker via prompt; saved with setBlockAttrs.
   */
  private async showAttrs(tab: EdTab) {
    let attrs: Record<string, unknown> = {};
    try {
      attrs = await api.getAttrs(tab.path);
    } catch {
      /* new file without fm */
    }
    const mask = el("div", "mv-dialog-mask");
    const dialog = el("div", "mv-dialog attrs-dialog");
    dialog.innerHTML = `
      <h3>属性 — ${escapeHtml(tab.title)}</h3>
      <div class="attrs-table"></div>
      <div class="attrs-actions-row">
        <button class="btn add-row">＋ 添加属性</button>
        <span class="attrs-hint">parent 值用 [[标题]] 形式</span>
      </div>
      <div class="actions">
        <button class="btn cancel">取消</button>
        <button class="btn primary ok">保存</button>
      </div>`;
    const table = dialog.querySelector(".attrs-table") as HTMLElement;

    const addRow = (key = "", value = "") => {
      const tr = el("div", "attrs-row");
      const keyInput = document.createElement("input");
      keyInput.type = "text";
      keyInput.className = "attr-key";
      keyInput.placeholder = "key";
      keyInput.value = key;
      const valInput = document.createElement("input");
      valInput.type = "text";
      valInput.className = "attr-val";
      valInput.placeholder = "value";
      valInput.value = value;
      // parent picker
      const pick = el("button", "btn attr-pick", "◈");
      pick.title = "从文档列表选择 (parent)";
      pick.onclick = () => {
        const docs = this.hooks.docsProvider();
        const names = docs.map((d) => `[[${d.title || d.path.split("/").pop()?.replace(/\.md$/, "")}]]`);
        const pick_ = window.prompt("输入 parent 值（可从列表复制）:\n" + names.slice(0, 40).join("\n"), valInput.value);
        if (pick_ !== null) valInput.value = pick_;
      };
      const del = el("span", "attr-del", "✕");
      del.title = "删除该属性";
      del.onclick = () => tr.remove();
      tr.append(keyInput, valInput, pick, del);
      table.appendChild(tr);
    };

    for (const [k, v] of Object.entries(attrs)) {
      const vs = typeof v === "string" ? v : JSON.stringify(v);
      addRow(k, vs);
    }
    (dialog.querySelector(".add-row") as HTMLElement).onclick = () => addRow();

    const done = (save: boolean) => {
      if (save) void (async () => {
        const next: Record<string, unknown> = {};
        const toDelete: string[] = [];
        const origKeys = new Set(Object.keys(attrs));
        for (const rowEl of Array.from(table.children)) {
          const k = (rowEl.querySelector(".attr-key") as HTMLInputElement).value.trim();
          const raw = (rowEl.querySelector(".attr-val") as HTMLInputElement).value.trim();
          if (!k) continue;
          origKeys.delete(k);
          let v: unknown = raw;
          try { v = JSON.parse(raw); } catch { /* keep string */ }
          next[k] = v;
        }
        for (const k of origKeys) toDelete.push(k);
        // delete removed keys first, then set
        for (const k of toDelete) {
          try { await api.setAttrs(tab.path, { [k]: null }); } catch { /* ignore */ }
        }
        if (Object.keys(next).length) {
          await api.setAttrs(tab.path, next);
        }
        toast("属性已更新");
        this.hooks.onTitleChange();
        this.hooks.onBacklinksRefresh(tab.path);
      })();
      mask.remove();
    };
    (dialog.querySelector(".ok") as HTMLElement).onclick = () => done(true);
    (dialog.querySelector(".cancel") as HTMLElement).onclick = () => done(false);
    mask.onmousedown = (e) => {
      if (e.target === mask) done(false);
    };
    mask.appendChild(dialog);
    document.body.appendChild(mask);
  }

  async close(tab: EdTab) {
    if (tab.dirty) {
      const ok = await confirmDialog("关闭页签", "有未保存修改，保存并关闭？(取消=仍关闭但不保存)");
      if (ok && tab.view) {
        await api.putFile(tab.path, tab.view.state.doc.toString());
        tab.dirty = false;
      }
    }
    if (tab.saveTimer) window.clearTimeout(tab.saveTimer);
    tab.hostEl?.remove();
    this.tabs = this.tabs.filter((t) => t !== tab);
    if (this.active === tab) {
      this.active = this.tabs[this.tabs.length - 1] ?? null;
      if (this.active) this.activate(this.active);
      else this.hooks.onBreadcrumb(null);
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
        this.syncEditorPath(t);
        this.refreshEditorContext(t);
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

function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}
