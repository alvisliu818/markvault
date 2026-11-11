import { api } from "../api";
import { el, escapeHtml, toast, confirmDialog } from "../ui";

/**
 * Snapshot history dialog (M11): list versions of one doc, preview content,
 * restore. putFile auto-snapshots the pre-write content, so every save
 * leaves a trail; manual snapshots are forced even when unchanged.
 */

function fmtMillis(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

export function showHistoryDialog(path: string, title: string) {
  const mask = el("div", "mv-dialog-mask");
  const dialog = el("div", "mv-dialog history-dialog");
  dialog.innerHTML = `
    <h3>历史版本 — ${escapeHtml(title)}</h3>
    <div class="hist-body">
      <div class="hist-list"></div>
      <pre class="hist-view"></pre>
    </div>
    <div class="hist-foot">
      <span class="hist-hint">保存时自动留存修改前内容；恢复前当前内容也会先快照</span>
      <div class="actions">
        <button class="btn snap">创建快照</button>
        <button class="btn primary restore" disabled>恢复此版本</button>
        <button class="btn cancel">关闭</button>
      </div>
    </div>`;
  const listEl = dialog.querySelector(".hist-list") as HTMLElement;
  const viewEl = dialog.querySelector(".hist-view") as HTMLElement;
  const restoreBtn = dialog.querySelector(".restore") as HTMLButtonElement;

  let selected: string | null = null;
  let items: { id: string; created: number; hash: string }[] = [];

  const render = () => {
    listEl.innerHTML = "";
    if (!items.length) {
      listEl.append(el("div", "hist-empty", "暂无快照（编辑保存后自动生成）"));
      return;
    }
    for (const s of items) {
      const row = el("div", "hist-row" + (s.id === selected ? " active" : ""));
      const t = el("div", "", fmtMillis(s.created));
      const meta = el("div", "path", `#${s.hash}`);
      row.append(t, meta);
      row.onclick = () => void select(s.id);
      listEl.append(row);
    }
  };

  const select = async (id: string) => {
    selected = id;
    render();
    restoreBtn.disabled = false;
    try {
      const { content } = await api.getSnapshot(path, id);
      viewEl.textContent = content;
    } catch (e) {
      viewEl.textContent = "读取失败: " + (e as Error).message;
    }
  };

  const reload = async () => {
    try {
      items = (await api.listSnapshots(path)).snapshots;
    } catch (e) {
      toast("读取历史失败: " + (e as Error).message);
      items = [];
    }
    selected = null;
    restoreBtn.disabled = true;
    viewEl.textContent = "";
    render();
  };

  restoreBtn.onclick = () => {
    void (async () => {
      if (!selected) return;
      if (!(await confirmDialog("恢复版本", `将「${title}」恢复到此版本？当前内容会先自动快照。`))) return;
      try {
        await api.restoreSnapshot(path, selected);
        toast("已恢复，重新打开文档可见最新内容");
        await reload();
      } catch (e) {
        toast("恢复失败: " + (e as Error).message);
      }
    })();
  };
  (dialog.querySelector(".snap") as HTMLElement).onclick = () => {
    void (async () => {
      try {
        await api.createSnapshot(path);
        toast("已创建快照");
        await reload();
      } catch (e) {
        toast("快照失败: " + (e as Error).message);
      }
    })();
  };
  (dialog.querySelector(".cancel") as HTMLElement).onclick = () => mask.remove();
  mask.onmousedown = (e) => {
    if (e.target === mask) mask.remove();
  };

  mask.appendChild(dialog);
  document.body.appendChild(mask);
  void reload();
}
