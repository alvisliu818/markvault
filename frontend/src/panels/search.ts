import { api } from "../api";
import { el } from "../ui";

export class SearchPanel {
  private resultsEl: HTMLElement;
  private input: HTMLInputElement;

  constructor(container: HTMLElement, private openDoc: (path: string, title?: string) => void) {
    const head = el("div", "panel-head");
    this.input = document.createElement("input");
    this.input.type = "text";
    this.input.placeholder = "全文搜索…";
    const btn = el("button", "btn", "搜索");
    head.append(this.input, btn);
    this.resultsEl = el("div", "tree");
    container.append(head, this.resultsEl);

    btn.onclick = () => void this.run();
    this.input.onkeydown = (e) => {
      if (e.key === "Enter") void this.run();
    };
  }

  private async run() {
    const q = this.input.value.trim();
    this.resultsEl.innerHTML = "";
    if (!q) return;
    const rows = await api.fullTextSearch(q);
    for (const r of rows) {
      const row = el("div", "search-row");
      const title = el("div", "", r.title);
      const snip = el("div");
      snip.innerHTML = r.snippet;
      const path = el("div", "path", r.hpath);
      row.append(title, snip, path);
      row.onclick = () => this.openDoc("/" + r.path.replace(/^\//, ""), r.title);
      this.resultsEl.appendChild(row);
    }
    if (rows.length === 0) {
      this.resultsEl.appendChild(el("div", "search-row", "无结果"));
    }
  }
}
