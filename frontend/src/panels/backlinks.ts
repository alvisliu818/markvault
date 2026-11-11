import { api, type BacklinksData } from "../api";
import { el } from "../ui";

export interface BacklinkHooks {
  openDoc: (path: string, title?: string) => void;
}

/**
 * Backlinks dock panel: 引用了谁 (outgoing) / 被谁引用 (wikilink backlinks)
 * / 提及 (pseudo-backlinks via FTS title match).
 */
export class BacklinkPanel {
  private listEl: HTMLElement;
  private currentPath = "";
  private timer: number | null = null;

  constructor(container: HTMLElement, private hooks: BacklinkHooks) {
    const head = el("div", "panel-head");
    head.append(el("span", "", "反链"));
    this.listEl = el("div", "tree");
    container.append(head, this.listEl);
  }

  /** Debounced refresh keyed to the active doc path. */
  schedule(path: string) {
    if (this.timer) window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.refresh(path), 500);
  }

  async refresh(path: string) {
    if (!path || path === this.currentPath) {
      if (!path) return;
    }
    this.currentPath = path;
    this.listEl.innerHTML = "";
    try {
      const data = await api.getBacklinks(path);
      this.renderData(data);
    } catch {
      this.listEl.appendChild(el("div", "outline-row", "(加载失败)"));
    }
  }

  private section(title: string, rows: { path: string; label: string }[]): boolean {
    if (rows.length === 0) return false;
    this.listEl.appendChild(el("div", "bk-section-title", `${title} (${rows.length})`));
    for (const r of rows) {
      const row = el("div", "outline-row", r.label);
      row.onclick = () => this.hooks.openDoc(r.path, r.label);
      this.listEl.appendChild(row);
    }
    return true;
  }

  private renderData(data: BacklinksData) {
    const any =
      this.section("被谁引用", data.backlinks.map((b) => ({ path: b.path, label: this.base(b.path) }))) ||
      this.section("引用了谁", data.links.map((l) => ({ path: l.path, label: this.base(l.path) }))) ||
      this.section("未解析链接", data.unresolved.map((u) => ({ path: "", label: u.raw }))) ||
      this.section("提及", data.mentions.map((m) => ({ path: m.path, label: this.base(m.path) })));
    if (!any) {
      this.listEl.appendChild(el("div", "outline-row", "(无)"));
    }
  }

  private base(p: string) {
    return p.split("/").pop()?.replace(/\.md$/, "") ?? p;
  }
}
