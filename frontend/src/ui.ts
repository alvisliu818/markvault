// Shared UI primitives: context menu, dialog, toast

export function toast(msg: string) {
  const t = document.createElement("div");
  t.className = "mv-toast";
  t.textContent = msg;
  document.body.appendChild(t);
  setTimeout(() => t.remove(), 2500);
}

export interface MenuItem {
  label: string;
  danger?: boolean;
  action?: () => void;
}

export function showMenu(x: number, y: number, items: MenuItem[]) {
  closeMenu();
  const menu = document.createElement("div");
  menu.className = "mv-menu";
  menu.id = "mv-menu";
  for (const it of items) {
    const el = document.createElement("div");
    el.className = "mv-menu-item" + (it.danger ? " danger" : "");
    el.textContent = it.label;
    el.onclick = () => {
      closeMenu();
      it.action?.();
    };
    menu.appendChild(el);
  }
  document.body.appendChild(menu);
  const rect = menu.getBoundingClientRect();
  menu.style.left = Math.min(x, window.innerWidth - rect.width - 8) + "px";
  menu.style.top = Math.min(y, window.innerHeight - rect.height - 8) + "px";
  setTimeout(() => {
    document.addEventListener("mousedown", onDocDown, { once: true });
  });
  function onDocDown(e: MouseEvent) {
    if (!menu.contains(e.target as Node)) closeMenu();
    else document.addEventListener("mousedown", onDocDown, { once: true });
  }
}

export function closeMenu() {
  document.getElementById("mv-menu")?.remove();
}

export function promptDialog(title: string, placeholder: string, initial = ""): Promise<string | null> {
  return new Promise((resolve) => {
    const mask = document.createElement("div");
    mask.className = "mv-dialog-mask";
    mask.innerHTML = `
      <div class="mv-dialog">
        <h3>${escapeHtml(title)}</h3>
        <input type="text" placeholder="${escapeHtml(placeholder)}" />
        <div class="actions">
          <button class="btn cancel">取消</button>
          <button class="btn primary ok">确定</button>
        </div>
      </div>`;
    const input = mask.querySelector("input") as HTMLInputElement;
    input.value = initial;
    const done = (v: string | null) => {
      mask.remove();
      resolve(v);
    };
    (mask.querySelector(".ok") as HTMLElement).onclick = () => done(input.value.trim() || null);
    (mask.querySelector(".cancel") as HTMLElement).onclick = () => done(null);
    input.onkeydown = (e) => {
      if (e.key === "Enter") done(input.value.trim() || null);
      if (e.key === "Escape") done(null);
    };
    mask.onmousedown = (e) => {
      if (e.target === mask) done(null);
    };
    document.body.appendChild(mask);
    input.focus();
    input.select();
  });
}

export function confirmDialog(title: string, text: string): Promise<boolean> {
  return new Promise((resolve) => {
    const mask = document.createElement("div");
    mask.className = "mv-dialog-mask";
    mask.innerHTML = `
      <div class="mv-dialog">
        <h3>${escapeHtml(title)}</h3>
        <div style="color:var(--text-dim);margin-bottom:14px">${escapeHtml(text)}</div>
        <div class="actions">
          <button class="btn cancel">取消</button>
          <button class="btn primary ok">确定</button>
        </div>
      </div>`;
    const done = (v: boolean) => { mask.remove(); resolve(v); };
    (mask.querySelector(".ok") as HTMLElement).onclick = () => done(true);
    (mask.querySelector(".cancel") as HTMLElement).onclick = () => done(false);
    mask.onmousedown = (e) => { if (e.target === mask) done(false); };
    document.body.appendChild(mask);
  });
}

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[c] as string));
}

export function el<T extends HTMLElement = HTMLElement>(
  tag: string, cls?: string, text?: string
): T {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e as T;
}
