import { api, type Bookmark } from "../api";
import { el, showMenu, promptDialog } from "../ui";

/** Bookmarks panel — collected links open in a new browser tab. */
export class BookmarkPanel {
  private listEl: HTMLElement;
  private items: Bookmark[] = [];

  constructor(container: HTMLElement) {
    const head = el("div", "panel-head");
    const addBtn = el("button", "btn", "添加书签");
    head.append(addBtn);
    this.listEl = el("div", "tree");
    container.append(head, this.listEl);

    addBtn.onclick = () => void this.addFlow();
    void this.refresh();
  }

  async refresh() {
    try {
      this.items = await api.bookmarkList();
      this.render();
    } catch {
      this.items = [];
      this.render();
    }
  }

  private render() {
    this.listEl.innerHTML = "";
    if (this.items.length === 0) {
      this.listEl.appendChild(el("div", "search-row", "无书签"));
      return;
    }
    for (const b of this.items) {
      const row = el("div", "search-row");
      const title = el("div", "", b.title);
      const url = el("div", "path", b.url);
      row.append(title, url);
      row.onclick = () => window.open(b.url, "_blank");
      row.oncontextmenu = (e) => {
        e.preventDefault();
        showMenu(e.clientX, e.clientY, [
          { label: "打开", action: () => window.open(b.url, "_blank") },
          {
            label: "删除书签", danger: true, action: async () => {
              await api.bookmarkRemove(b.url);
              void this.refresh();
            },
          },
        ]);
      };
      this.listEl.appendChild(row);
    }
  }

  private async addFlow() {
    const url = await promptDialog("添加书签", "URL");
    if (!url) return;
    const title = await promptDialog("标题", "显示名", url);
    await api.bookmarkAdd(url, title || url);
    void this.refresh();
  }
}
