import { api, onWsPush, type FileEntry, type Notebook } from "../api";
import { el, showMenu, promptDialog, confirmDialog } from "../ui";

interface PanelHooks {
  openDoc: (path: string, title?: string) => void;
  onAddRoot: (path: string) => void;
  /** right-click "mount to mount tree" (wired to vtree panel via app) */
  onMountToTree?: (path: string, name: string, isDoc: boolean) => void;
}

/**
 * File tree panel: notebooks = top level, lazy-expanded folders,
 * context menus for create/rename/delete/locate.
 */
export class FileTreePanel {
  private treeEl: HTMLElement;
  private notebooks: Notebook[] = [];
  expanded = new Set<string>();

  constructor(container: HTMLElement, private hooks: PanelHooks) {
    const head = el("div", "panel-head");
    const newDocBtn = el("button", "btn", "新建文档");
    const newNbBtn = el("button", "btn", "新建笔记本");
    const refreshBtn = el("button", "btn", "刷新");
    head.append(newDocBtn, newNbBtn, refreshBtn);
    this.treeEl = el("div", "tree");
    container.append(head, this.treeEl);

    refreshBtn.onclick = () => void this.refresh();
    newNbBtn.onclick = async () => {
      const name = await promptDialog("新建笔记本", "笔记本名称");
      if (name) {
        await api.createNotebook(name);
        void this.refresh();
      }
    };
    newDocBtn.onclick = () => {
      // default to first notebook root
      if (this.notebooks.length === 0) return;
      void this.createDocFlow(this.notebooks[0].id, "");
    };

    onWsPush((cmd) => {
      if (cmd === "filetree-changed") void this.refresh();
    });
  }

  async refresh() {
    const { notebooks } = await api.lsNotebooks();
    this.notebooks = notebooks;
    this.render();
  }

  private render() {
    this.treeEl.innerHTML = "";
    for (const nb of this.notebooks) {
      const node = this.renderNode({
        path: nb.id,
        name: nb.name,
        type: "folder",
        subFileCount: nb.docCount,
      }, 0, nb.id);
      this.treeEl.appendChild(node);
    }
    // auto-expand first notebook
    if (this.notebooks.length && this.expanded.size === 0) {
      this.expanded.add(this.notebooks[0].id);
      this.render();
    }
  }

  private renderNode(entry: FileEntry, depth: number, notebook: string): HTMLElement {
    const node = el("div", "tree-node");
    const row = el("div", "tree-row");
    row.style.paddingLeft = depth * 12 + "px";
    const isDir = entry.type === "folder";
    const expanded = isDir && this.expanded.has(entry.path);
    const toggle = el("span", "toggle" + (expanded ? " expanded" : "") + (isDir ? "" : " leaf"), "▶");
    const icon = el("span", "icon", isDir ? (expanded ? "📂" : "📁") : "📄");
    const label = el("span", "label", entry.title || entry.name);
    row.append(toggle, icon, label);
    if (isDir && (entry.subFileCount ?? 0) > 0) {
      row.append(el("span", "badge", String(entry.subFileCount)));
    }
    node.appendChild(row);

    if (isDir && expanded) {
      const kids = el("div", "tree-children");
      node.appendChild(kids);
      void this.loadChildren(notebook, entry.path, depth, kids);
    }

    row.onclick = () => {
      if (!isDir) {
        this.hooks.openDoc(entry.path, entry.title || entry.name);
        row.parentElement?.parentElement?.querySelectorAll(".active").forEach((n) => n.classList.remove("active"));
        row.classList.add("active");
        return;
      }
      if (this.expanded.has(entry.path)) {
        this.expanded.delete(entry.path);
      } else {
        this.expanded.add(entry.path);
      }
      this.render();
    };

    row.oncontextmenu = (e) => {
      e.preventDefault();
      showMenu(e.clientX, e.clientY, isDir ? this.dirMenu(notebook, entry) : this.docMenu(entry));
    };
    return node;
  }

  private docMenu(entry: FileEntry) {
    const path = entry.path;
    return [
      { label: "打开", action: () => this.hooks.openDoc(path, entry.title || entry.name) },
      { label: "重命名", action: () => void this.renameFlow(path, entry.title || entry.name) },
      { label: "设为虚拟树根", action: () => this.hooks.onAddRoot(path) },
      ...(this.hooks.onMountToTree
        ? [{
            label: "挂载到挂载树",
            action: () => this.hooks.onMountToTree!(path, entry.title || entry.name, true),
          }]
        : []),
      {
        label: "删除", danger: true, action: async () => {
          if (await confirmDialog("删除文档", `移到回收站: ${path} ?`)) {
            await api.removeDoc(path);
            void this.refresh();
          }
        },
      },
    ];
  }

  private dirMenu(notebook: string, entry: FileEntry) {
    const path = entry.path;
    return [
      { label: "新建文档", action: () => void this.createDocFlow(notebook, path) },
      { label: "重命名", action: () => void this.renameFlow(path, entry.name) },
      ...(this.hooks.onMountToTree
        ? [{
            label: "挂载到挂载树",
            action: () => this.hooks.onMountToTree!(path, entry.name, false),
          }]
        : []),
      {
        label: "删除", danger: true, action: async () => {
          if (await confirmDialog("删除文件夹", `移到回收站: ${path} ?`)) {
            await api.removeDoc(path);
            void this.refresh();
          }
        },
      },
    ];
  }

  private async createDocFlow(notebook: string, dirPath: string) {
    const title = await promptDialog("新建文档", "文档标题");
    if (!title) return;
    const hpath = (dirPath.replace(new RegExp(`^/?${notebook}`), "") || "") + "/" + title;
    try {
      const { path } = await api.createDocWithMd(notebook, hpath, "");
      await this.refresh();
      this.hooks.openDoc("/" + path.replace(/^\//, ""), title);
    } catch (e) {
      alert((e as Error).message);
    }
  }

  private async renameFlow(path: string, current: string) {
    const title = await promptDialog("重命名", "新名称", current);
    if (!title) return;
    await api.renameDoc(path, title);
    void this.refresh();
  }

  private async loadChildren(notebook: string, dirPath: string, depth: number, host: HTMLElement) {
    try {
      const { files } = await api.listDocsByPath(notebook, dirPath.replace(new RegExp(`^/?${notebook}`), "") || "/");
      for (const f of files) {
        host.appendChild(this.renderNode(f, depth + 1, notebook));
      }
    } catch {
      const err = el("div", "search-row", "(加载失败)");
      host.appendChild(err);
    }
  }
}
