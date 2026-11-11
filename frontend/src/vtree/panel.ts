import { api, type VtNode, type MountItem, type FileEntry } from "../api";
import { el, showMenu, promptDialog, toast, confirmDialog } from "../ui";

interface VTreeHooks {
  openDoc: (path: string, title?: string) => void;
  locateDoc: (path: string) => void;
}

interface PanelState {
  collapsed: Set<string>;
  focusMode: boolean;
}

const DRAG_TYPE = "application/x-markvault-node";

/**
 * Virtual document tree panel — union of obsidian-virtual-tree and
 * siyuan-virtual-tree behaviors:
 *  - relation tree: derived from frontmatter `parent` wikilinks (kernel-computed)
 *  - mount tree: user-curated file/doc/notebook mounts with lazy children
 *  - three-tier sort: kernel-side (drag order > weight desc > name asc)
 *  - collapse state persisted via /api/storage, locate + focus mode
 */
export class VTreePanel {
  private container: HTMLElement;
  private relHost: HTMLElement;
  private mountHost: HTMLElement;
  private state: PanelState = { collapsed: new Set(), focusMode: false };
  private tree: VtNode[] = [];
  private mountRoots: MountItem[] = [];
  private activePath = "";
  private dragMode = false;
  private dragging: { kind: "rel" | "mount"; key?: string; path?: string } | null = null;

  constructor(container: HTMLElement, private hooks: VTreeHooks) {
    this.container = container;
    const head = el("div", "panel-head");
    const dragBtn = el("button", "btn", "拖拽排序");
    const sortBtn = el("button", "btn", "排序:权重");
    const physBtn = el("button", "btn", "物理子树");
    const collapseBtn = el("button", "btn", "折叠");
    const expandBtn = el("button", "btn", "展开");
    const focusBtn = el("button", "btn", "聚焦");
    const locateBtn = el("button", "btn", "定位");
    const mountBtn = el("button", "btn", "+挂载");
    const refreshBtn = el("button", "btn", "刷新");
    head.append(dragBtn, sortBtn, physBtn, locateBtn, collapseBtn, expandBtn, focusBtn, mountBtn, refreshBtn);

    this.relHost = el("div", "tree");
    const relTitle = el("div", "outline-row h1", "关系树");
    relTitle.style.cursor = "default";
    const mountTitle = el("div", "outline-row h1", "挂载树");
    mountTitle.style.cssText = "cursor:default;border-top:1px solid var(--border);margin-top:4px";
    this.mountHost = el("div", "tree");
    this.container.append(head, relTitle, this.relHost, mountTitle, this.mountHost);

    dragBtn.onclick = () => {
      this.dragMode = !this.dragMode;
      dragBtn.classList.toggle("primary", this.dragMode);
      toast(this.dragMode ? "拖拽排序模式已开启" : "拖拽排序模式已关闭");
    };
    sortBtn.onclick = () => void this.toggleSort(sortBtn);
    physBtn.onclick = () => void this.togglePhysical(physBtn);
    physBtn.classList.toggle("primary", this.settings.includePhysicalSubtree);
    collapseBtn.onclick = () => {
      const all = this.collectAllIds(this.tree);
      this.state.collapsed = new Set(all);
      void this.persistCollapsed();
      this.renderRel();
    };
    expandBtn.onclick = () => {
      this.state.collapsed.clear();
      void this.persistCollapsed();
      this.renderRel();
    };
    focusBtn.onclick = () => {
      this.state.focusMode = !this.state.focusMode;
      focusBtn.classList.toggle("primary", this.state.focusMode);
      this.renderRel();
    };
    locateBtn.onclick = () => this.locate();
    mountBtn.onclick = () => void this.mountFlow();
    refreshBtn.onclick = () => this.refresh();

    // Alt+L locate (obsidian-virtual-tree command parity)
    this.keyHandler = (e: KeyboardEvent) => {
      if (e.altKey && e.key.toLowerCase() === "l") {
        e.preventDefault();
        this.locate();
      }
    };
    window.addEventListener("keydown", this.keyHandler);

    void this.loadCollapsed();
  }

  private keyHandler: (e: KeyboardEvent) => void = () => {};

  destroy() {
    window.removeEventListener("keydown", this.keyHandler);
  }

  private settings: {
    sortMethod: string;
    caseSensitive: boolean;
    defaultExpandLevel: number;
    includePhysicalSubtree: boolean;
  } = { sortMethod: "weight", caseSensitive: false, defaultExpandLevel: -1, includePhysicalSubtree: false };

  private async loadSettings() {
    try {
      const { settings } = await api.getVtSettings();
      if (settings) this.settings = settings;
    } catch { /* ignore */ }
  }

  private async toggleSort(btn: HTMLElement) {
    await this.loadSettings();
    this.settings.sortMethod = this.settings.sortMethod === "weight" ? "name" : "weight";
    await api.setVtSettings({ ...this.settings });
    btn.textContent = "排序:" + (this.settings.sortMethod === "weight" ? "权重" : "名称");
    toast("排序: " + (this.settings.sortMethod === "weight" ? "权重优先" : "名称优先") + "（拖拽序仍最优先）");
    this.refresh();
  }

  private async togglePhysical(btn: HTMLElement) {
    await this.loadSettings();
    this.settings.includePhysicalSubtree = !this.settings.includePhysicalSubtree;
    await api.setVtSettings({ ...this.settings });
    btn.classList.toggle("primary", this.settings.includePhysicalSubtree);
    toast(this.settings.includePhysicalSubtree ? "物理子树已合并" : "物理子树已关闭");
    this.refresh();
  }

  private async loadCollapsed() {
    try {
      const saved = await api.getVal<string[]>("vt-collapsed");
      this.state.collapsed = new Set(saved ?? []);
    } catch { /* ignore */ }
  }

  private async persistCollapsed() {
    await api.setVal("vt-collapsed", [...this.state.collapsed]);
  }

  refresh() {
    void this.reload();
  }

  /** Add a mount root from external flows (filetree right-click). */
  addMountRoot(path: string, name: string, kind: "doc" | "notebook" | "file") {
    const p = path.replace(/^\//, "");
    if (this.mountRoots.some((m) => m.path === p)) {
      toast("已存在相同挂载");
      return;
    }
    this.mountRoots.push({
      uid: "m" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
      kind,
      name,
      path: p,
      children: [],
    });
    void api.setMountTree(this.mountRoots).then(() => this.renderMount());
  }

  private async reload() {
    try {
      const [rel, mount] = await Promise.all([
        api.getRelationTree(),
        api.getMountTree(),
      ]);
      this.tree = rel.tree;
      this.mountRoots = mount.mountRoots ?? [];
      this.renderRel();
      this.renderMount();
    } catch (e) {
      this.relHost.innerHTML = "";
      this.relHost.appendChild(el("div", "search-row", "加载失败: " + (e as Error).message));
    }
  }

  private collectAllIds(nodes: VtNode[]): string[] {
    const out: string[] = [];
    const walk = (n: VtNode) => {
      out.push(n.id);
      n.children.forEach(walk);
    };
    nodes.forEach(walk);
    return out;
  }

  // ---------- relation tree ----------

  setActive(path: string) {
    this.activePath = path;
    this.renderRel();
    this.renderMount();
  }

  private renderRel() {
    this.relHost.innerHTML = "";
    let roots = this.tree;
    if (this.state.focusMode && this.activePath) {
      roots = this.filterToPath(this.tree);
    }
    if (roots.length === 0) {
      this.relHost.appendChild(el("div", "search-row", "无内容 — 用文件树右键“设为虚拟树根”，或在笔记 frontmatter 写 parent: [\"[[父笔记]]\"]"));
      return;
    }
    for (const n of roots) {
      this.relHost.appendChild(this.renderRelNode(n, 0, "__root__"));
    }
  }

  private filterToPath(nodes: VtNode[]): VtNode[] {
    const out: VtNode[] = [];
    for (const n of nodes) {
      if (n.path === this.activePath) {
        out.push(n);
      } else {
        const kids = this.filterToPath(n.children);
        if (kids.length > 0) {
          out.push({ ...n, children: kids });
        }
      }
    }
    return out;
  }

  private renderRelNode(node: VtNode, depth: number, parentKey: string): HTMLElement {
    const wrap = el("div", "tree-node");
    const row = el("div", "tree-row");
    row.style.paddingLeft = depth * 12 + "px";
    row.dataset.nodeId = node.id;
    row.dataset.nodePath = node.path;
    const expanded = !this.state.collapsed.has(node.id);
    const toggle = el("span", "toggle" + (expanded ? " expanded" : "") + (node.children.length ? "" : " leaf"), "▶");
    const icon = el("span", "icon", "📄");
    const label = el("span", "label", node.name);
    row.append(toggle, icon, label);
    if (node.weight !== 0) row.append(el("span", "badge", String(node.weight)));
    if (node.isRoot) row.classList.add("is-root");
    if (node.path === this.activePath) row.classList.add("active");
    wrap.appendChild(row);

    if (node.children.length && expanded) {
      const kids = el("div", "tree-children");
      wrap.appendChild(kids);
      for (const c of node.children) {
        kids.appendChild(this.renderRelNode(c, depth + 1, node.path));
      }
    }

    row.onclick = (e) => {
      if (e.target === toggle || !this.isNameArea(e, row)) {
        if (node.children.length) this.toggleNode(node.id, wrap);
        return;
      }
      this.hooks.openDoc(node.path, node.name);
      this.setActive(node.path);
    };
    row.oncontextmenu = (e) => {
      e.preventDefault();
      showMenu(e.clientX, e.clientY, [
        { label: "打开", action: () => this.hooks.openDoc(node.path, node.name) },
        { label: "定位", action: () => { this.activePath = node.path; this.locateIn(this.relHost); } },
        ...(node.isRoot
          ? [{
              label: "从虚拟树移除根", danger: true, action: async () => {
                await api.removeRoot(node.path);
                this.refresh();
              },
            }]
          : []),
      ]);
    };

    if (this.dragMode) this.enableRelDrag(row, node, parentKey);
    return wrap;
  }

  private toggleNode(id: string, wrap: HTMLElement) {
    if (this.state.collapsed.has(id)) this.state.collapsed.delete(id);
    else this.state.collapsed.add(id);
    void this.persistCollapsed();
    this.renderRel();
    void wrap;
  }

  private isNameArea(e: MouseEvent, _row: HTMLElement): boolean {
    const t = e.target as HTMLElement;
    return t.classList.contains("label") || t.classList.contains("icon");
  }

  // ---------- drag reorder (three-tier sort tier 1) ----------

  private enableRelDrag(row: HTMLElement, node: VtNode, parentKey: string) {
    row.draggable = true;
    row.ondragstart = (e) => {
      this.dragging = { kind: "rel", key: node.path };
      e.dataTransfer?.setData(DRAG_TYPE, node.path);
    };
    row.ondragover = (e) => {
      if (this.dragging?.kind !== "rel") return;
      e.preventDefault();
      const rect = row.getBoundingClientRect();
      const before = e.clientY < rect.top + rect.height / 2;
      row.style.borderTop = before ? "2px solid var(--accent)" : "";
      row.style.borderBottom = before ? "" : "2px solid var(--accent)";
    };
    row.ondragleave = () => {
      row.style.borderTop = "";
      row.style.borderBottom = "";
    };
    row.ondrop = async (e) => {
      e.preventDefault();
      row.style.borderTop = "";
      row.style.borderBottom = "";
      if (this.dragging?.kind !== "rel" || !this.dragging.key) return;
      const srcPath = this.dragging.key;
      if (srcPath === node.path) return;
      const rect = row.getBoundingClientRect();
      const before = e.clientY < rect.top + rect.height / 2;
      // cross-parent move when target's parent differs from source's
      const srcRow = [...this.relHost.querySelectorAll(".tree-row")].find(
        (r) => (r as HTMLElement).dataset.nodePath === srcPath
      ) as HTMLElement | undefined;
      const srcId = srcRow?.dataset.nodeId || "";
      const sameParent =
        parentKey === "__root__"
          ? !srcId.includes("::")
          : srcId.startsWith(parentKey + "::") && !srcId.slice(parentKey.length + 2).includes("::");
      if (!sameParent) {
        if (parentKey === "__root__") {
          const ok = await confirmDialog("移动到顶层", "将移除源文档的 frontmatter parent 属性（脱离所有父节点），继续？");
          if (!ok) return;
        }
        try {
          await api.moveNode(srcPath, parentKey === "__root__" ? "" : parentKey);
          toast(parentKey === "__root__" ? "已移到顶层（parent 已移除）" : "已移动（改写 frontmatter parent）");
        } catch (err) {
          toast("移动失败: " + (err as Error).message);
        }
        this.refresh();
        return;
      }
      await this.reorderSiblings(parentKey, node.path, srcPath, before);
    };
  }

  /** Compute sibling order from current DOM and persist via setNodeOrder. */
  private async reorderSiblings(parentKey: string, targetPath: string, srcPath: string, before: boolean) {
    // collect all doc paths in DOM order
    const order: string[] = [];
    const collect = (host: HTMLElement) => {
      for (const child of Array.from(host.children)) {
        if (child.classList.contains("tree-node")) {
          const row = child.querySelector(":scope > .tree-row") as HTMLElement | null;
          if (row?.dataset.nodePath) order.push(row.dataset.nodePath);
          const kids = child.querySelector(":scope > .tree-children") as HTMLElement | null;
          if (kids) collect(kids);
        }
      }
    };
    collect(this.relHost);
    // filter to siblings: node-id has exactly one more "::" segment than parent
    const allRows = [...this.relHost.querySelectorAll(".tree-row")] as HTMLElement[];
    const prefix = parentKey === "__root__" ? "" : parentKey + "::";
    const siblings = order.filter((p) => {
      const row = allRows.find((r) => r.dataset.nodePath === p);
      if (!row) return false;
      const id = row.dataset.nodeId || "";
      if (parentKey === "__root__") return !id.includes("::");
      return id.startsWith(prefix) && !id.slice(prefix.length).includes("::");
    });
    // rebuild with move
    const from = siblings.indexOf(srcPath);
    if (from >= 0) siblings.splice(from, 1);
    const at = siblings.indexOf(targetPath);
    if (at < 0) return;
    siblings.splice(before ? at : at + 1, 0, srcPath);
    await api.setNodeOrder(parentKey, siblings);
    this.refresh();
  }

  // ---------- locate ----------

  locate() {
    if (!this.activePath) {
      toast("当前没有活动文档");
      return;
    }
    this.locateIn(this.relHost) || this.locateIn(this.mountHost) || toast("当前文档不在树中");
  }

  private locateIn(host: HTMLElement): boolean {
    const rows = [...host.querySelectorAll(".tree-row")] as HTMLElement[];
    const target = rows.find((r) => r.dataset.nodePath === this.activePath) ?? null;
    if (!target) return false;
    // expand ancestors: walk up .tree-node wrappers, remove their ids from collapsed
    let node = target.parentElement;
    const toExpand: string[] = [];
    while (node && node !== host) {
      if (node.classList.contains("tree-node")) {
        const row = node.querySelector(":scope > .tree-row") as HTMLElement | null;
        if (row?.dataset.nodeId) toExpand.push(row.dataset.nodeId);
      }
      node = node.parentElement;
    }
    let changed = false;
    for (const id of toExpand) {
      if (this.state.collapsed.has(id)) {
        this.state.collapsed.delete(id);
        changed = true;
      }
    }
    const finish = () => {
      target.scrollIntoView({ behavior: "smooth", block: "center" });
      target.classList.add("tree-flash");
      setTimeout(() => target.classList.remove("tree-flash"), 2000);
    };
    if (changed) {
      void this.persistCollapsed().then(() => {
        this.renderRel();
        requestAnimationFrame(finish);
      });
    } else {
      finish();
    }
    return true;
  }

  // ---------- mount tree ----------

  private async mountFlow() {
    const name = await promptDialog("挂载", "输入笔记本/文件夹/文档路径 (vault 相对路径)");
    if (!name) return;
    const p = name.trim().replace(/^\//, "");
    const kind = p.includes(".md") ? "doc" : "notebook";
    this.mountRoots.push({
      uid: "m" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
      kind,
      name: p.split("/").pop() || p,
      path: p,
      children: [],
    });
    await api.setMountTree(this.mountRoots);
    this.renderMount();
  }

  private renderMount() {
    this.mountHost.innerHTML = "";
    if (this.mountRoots.length === 0) {
      this.mountHost.appendChild(el("div", "search-row", "无挂载项 — 点“+挂载”或右键文件树挂载"));
      return;
    }
    for (const m of this.mountRoots) {
      this.mountHost.appendChild(this.renderMountNode(m, 0));
    }
  }

  private renderMountNode(item: MountItem, depth: number): HTMLElement {
    const wrap = el("div", "tree-node");
    const row = el("div", "tree-row");
    row.style.paddingLeft = depth * 12 + "px";
    row.dataset.mountUid = item.uid;
    row.dataset.mountPath = item.path;
    const expanded = !this.state.collapsed.has("m:" + item.uid);
    const isDir = item.kind !== "doc";
    const toggle = el("span", "toggle" + (expanded ? " expanded" : "") + (isDir ? "" : " leaf"), "▶");
    const icon = el("span", "icon", item.kind === "notebook" ? "📚" : isDir ? "📁" : "📄");
    const label = el("span", "label", item.name);
    row.append(toggle, icon, label);
    if (!this.mountExists(item)) {
      row.classList.add("is-missing");
      label.title = "目标不存在";
    }
    wrap.appendChild(row);

    if (isDir && expanded) {
      const kids = el("div", "tree-children");
      wrap.appendChild(kids);
      for (const c of item.children) {
        kids.appendChild(this.renderMountNode(c, depth + 1));
      }
      void this.loadMountChildren(item, kids, depth);
    }

    row.onclick = (e) => {
      if (!this.isNameArea(e, row) && isDir) {
        if (this.state.collapsed.has("m:" + item.uid)) this.state.collapsed.delete("m:" + item.uid);
        else this.state.collapsed.add("m:" + item.uid);
        void this.persistCollapsed();
        this.renderMount();
        return;
      }
      if (item.kind === "doc") {
        this.hooks.openDoc(item.path, item.name);
        this.setActive(item.path);
      } else if (isDir) {
        if (this.state.collapsed.has("m:" + item.uid)) this.state.collapsed.delete("m:" + item.uid);
        else this.state.collapsed.add("m:" + item.uid);
        void this.persistCollapsed();
        this.renderMount();
      }
    };

    row.oncontextmenu = (e) => {
      e.preventDefault();
      showMenu(e.clientX, e.clientY, [
        { label: "挂载子项", action: () => void this.mountChildFlow(item) },
        { label: "重命名显示名", action: () => void this.renameMountFlow(item) },
        {
          label: "提升到顶层", action: () => {
            // remove from any nested position, re-add as a mount root
            const strip = (items: MountItem[]): MountItem[] =>
              items
                .filter((m) => m.uid !== item.uid)
                .map((m) => ({ ...m, children: strip(m.children) }));
            this.mountRoots = strip(this.mountRoots);
            this.mountRoots.push(JSON.parse(JSON.stringify(item)));
            void api.setMountTree(this.mountRoots).then(() => this.renderMount());
          },
        },
        {
          label: "移除挂载", danger: true, action: async () => {
            this.mountRoots = this.mountRoots.filter((m) => m.uid !== item.uid);
            await api.setMountTree(this.mountRoots);
            this.renderMount();
          },
        },
      ]);
    };
    return wrap;
  }

  private mountExists(item: MountItem): boolean {
    void item;
    return true; // kernel readDir would verify; optimistic for MVP
  }

  private async loadMountChildren(item: MountItem, host: HTMLElement, depth: number) {
    try {
      if (item.kind === "notebook") {
        const { files } = await api.listDocsByPath(item.path, "/");
        this.appendFsEntries(host, files, depth, item);
      } else {
        const { entries } = await api.readDir(item.path);
        const files: FileEntry[] = entries.map((e) => ({
          path: item.path + "/" + e.name,
          name: e.name,
          type: e.isDir ? "folder" : e.name.endsWith(".md") ? "doc" : "folder",
        }));
        this.appendFsEntries(host, files, depth, item);
      }
    } catch {
      host.appendChild(el("div", "search-row", "(无法加载子项)"));
    }
  }

  private appendFsEntries(host: HTMLElement, files: FileEntry[], depth: number, parent: MountItem) {
    for (const f of files) {
      if (f.name.startsWith(".")) continue;
      const row = el("div", "tree-row");
      row.style.paddingLeft = (depth + 1) * 12 + "px";
      const isDoc = f.type === "doc";
      const icon = el("span", "icon", f.type === "folder" ? "📁" : "📄");
      const label = el("span", "label", f.title || f.name);
      row.append(el("span", "toggle leaf", "▶"), icon, label);
      row.onclick = () => {
        if (isDoc) {
          this.hooks.openDoc(f.path, f.title || f.name);
          this.setActive(f.path);
        }
      };
      row.oncontextmenu = (e) => {
        e.preventDefault();
        showMenu(e.clientX, e.clientY, [
          {
            label: "挂载到挂载树", action: () => {
              parent.children.push({
                uid: "m" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
                kind: isDoc ? "doc" : "notebook",
                name: f.title || f.name,
                path: f.path,
                children: [],
              });
              void api.setMountTree(this.mountRoots).then(() => this.renderMount());
            },
          },
        ]);
      };
      host.appendChild(row);
    }
  }

  private async mountChildFlow(parent: MountItem) {
    const name = await promptDialog("挂载子项", "路径", parent.path);
    if (!name) return;
    const p = name.trim().replace(/^\//, "");
    parent.children.push({
      uid: "m" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
      kind: p.endsWith(".md") ? "doc" : "notebook",
      name: p.split("/").pop() || p,
      path: p,
      children: [],
    });
    await api.setMountTree(this.mountRoots);
    this.renderMount();
  }

  private async renameMountFlow(item: MountItem) {
    const name = await promptDialog("重命名显示名", "", item.name);
    if (!name) return;
    item.name = name;
    await api.setMountTree(this.mountRoots);
    this.renderMount();
  }
}
