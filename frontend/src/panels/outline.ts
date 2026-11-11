import { api } from "../api";
import { el } from "../ui";

export class OutlinePanel {
  private listEl: HTMLElement;

  constructor(container: HTMLElement, private jumpToLine: (line: number) => void) {
    const head = el("div", "panel-head");
    head.append(el("span", "", "大纲"));
    this.listEl = el("div", "tree");
    container.append(head, this.listEl);
  }

  async refresh(path: string) {
    this.listEl.innerHTML = "";
    if (!path) return;
    try {
      const { outline } = await api.getOutline(path);
      for (const item of outline) {
        const row = el("div", `outline-row ${item.type}`, item.content);
        row.title = `${item.type} · line ${item.line}`;
        row.onclick = () => this.jumpToLine(item.line);
        this.listEl.appendChild(row);
      }
      if (outline.length === 0) {
        this.listEl.appendChild(el("div", "outline-row", "(无标题)"));
      }
    } catch {
      this.listEl.appendChild(el("div", "outline-row", "(加载失败)"));
    }
  }
}
