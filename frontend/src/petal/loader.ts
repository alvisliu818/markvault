import { api } from "../api";
import { toast, showMenu, promptDialog, confirmDialog } from "../ui";
import type { TabManager } from "../tabs";

/**
 * Petal plugin loader (M11, SiYuan-style):
 * `.markvault/petal/<name>/index.js` is served by the kernel at
 * `/petal/<name>/index.js` and imported dynamically. A petal's default
 * export is a class receiving a host object; lifecycle = onload/onunload.
 * Commands registered via host.addCommand show up in the toolbar 命令 menu.
 */

export interface PetalCommand {
  id: string;
  title: string;
  cb: () => void;
}

export interface PetalHost {
  name: string;
  api: typeof api;
  ui: {
    toast(msg: string): void;
    showMenu(
      x: number,
      y: number,
      items: { label: string; danger?: boolean; action?: () => void }[]
    ): void;
    promptDialog(title: string, placeholder: string, initial?: string): Promise<string | null>;
    confirmDialog(title: string, text: string): Promise<boolean>;
  };
  getActiveDoc(): { path: string; title: string } | null;
  insertToActiveDoc(text: string): boolean;
  addCommand(id: string, title: string, cb: () => void): void;
}

interface PetalInstance {
  name: string;
  inst: { onload?: () => void; onunload?: () => void };
}

const commands: PetalCommand[] = [];
const instances: PetalInstance[] = [];

export function petalCommands(): PetalCommand[] {
  return [...commands];
}

export function unloadPetals() {
  for (const i of instances) {
    try {
      i.inst.onunload?.();
    } catch {
      /* petal unload errors are non-fatal */
    }
  }
  instances.length = 0;
  commands.length = 0;
}

function makeHost(name: string, tabs: TabManager): PetalHost {
  return {
    name,
    api,
    ui: { toast, showMenu, promptDialog, confirmDialog },
    getActiveDoc: () =>
      tabs.active ? { path: tabs.active.path, title: tabs.active.title } : null,
    insertToActiveDoc: (text: string) => {
      const v = tabs.active?.view;
      if (!v) return false;
      const pos = v.state.selection.main.to;
      v.dispatch({ changes: { from: pos, insert: text } });
      return true;
    },
    addCommand: (id, title, cb) =>
      commands.push({ id: `${name}:${id}`, title: `${name}: ${title}`, cb }),
  };
}

export async function loadPetals(tabs: TabManager) {
  unloadPetals();
  let petals: { name: string; enabled: boolean }[] = [];
  try {
    petals = (await api.loadPetals()).petals;
  } catch {
    return; // kernel without petal support / offline
  }
  for (const p of petals.filter((x) => x.enabled)) {
    try {
      const mod = await import(/* @vite-ignore */ `/petal/${p.name}/index.js`);
      const inst = new mod.default(makeHost(p.name, tabs));
      inst.onload?.();
      instances.push({ name: p.name, inst });
    } catch (e) {
      toast(`插件 ${p.name} 加载失败: ${(e as Error).message}`);
    }
  }
}

/** Open the 命令 menu at an anchor element (toolbar 命令 button). */
export function openCommandMenu(anchor: HTMLElement) {
  const cmds = petalCommands();
  if (!cmds.length) {
    toast("无插件命令（安装并启用 petal 插件后出现）");
    return;
  }
  const r = anchor.getBoundingClientRect();
  showMenu(r.left, r.bottom + 4, cmds.map((c) => ({ label: c.title, action: c.cb })));
}
