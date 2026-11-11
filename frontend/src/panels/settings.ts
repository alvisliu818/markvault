import { api } from "../api";
import { el, toast, escapeHtml } from "../ui";

interface ConfData {
  conf: {
    port: number;
    networkServe: boolean;
    accessAuthCode: string;
    apiToken: string;
    readonly: boolean;
    parentAttr: string;
    weightAttr: string;
    ignorePatterns: string[];
    theme: number;
  };
  workspace: string;
}

/**
 * Settings dialog: visual editor for conf fields (kernel setConf API).
 * Opened from the toolbar gear button.
 */
export async function showSettings() {
  let data: ConfData;
  try {
    data = (await api.getConf()) as unknown as ConfData;
  } catch (e) {
    toast("读取配置失败: " + (e as Error).message);
    return;
  }
  const c = data.conf;

  const mask = el("div", "mv-dialog-mask");
  const dialog = el("div", "mv-dialog settings-dialog");
  const row = (
    label: string,
    inputHtml: string,
    hint = ""
  ) => `
    <div class="set-row">
      <label class="set-label">${escapeHtml(label)}</label>
      <div class="set-input">${inputHtml}</div>
      ${hint ? `<div class="set-hint">${escapeHtml(hint)}</div>` : ""}
    </div>`;

  dialog.innerHTML = `
    <h3>设置</h3>
    <div class="set-workspace">${escapeHtml(data.workspace || "")}</div>
    ${row("端口", `<input type="text" id="set-port" value="${c.port}" />`, "修改后需重启内核生效")}
    ${row("局域网伺服", `<input type="checkbox" id="set-network" ${c.networkServe ? "checked" : ""} />`, "绑定 0.0.0.0，需访问码保护")}
    ${row("访问码", `<input type="text" id="set-authcode" value="${escapeHtml(c.accessAuthCode)}" />`, "伺服模式的锁屏口令；空 = 不启用")}
    ${row("API Token", `<input type="text" id="set-token" value="${escapeHtml(c.apiToken)}" />`, "程序化访问令牌；空 = 不校验")}
    ${row("只读模式", `<input type="checkbox" id="set-readonly" ${c.readonly ? "checked" : ""} />`, "禁止所有写操作")}
    ${row("parent 属性名", `<input type="text" id="set-parent" value="${escapeHtml(c.parentAttr)}" />`, "frontmatter 中虚拟树父链字段")}
    ${row("weight 属性名", `<input type="text" id="set-weight" value="${escapeHtml(c.weightAttr)}" />`, "frontmatter 排序权重字段")}
    ${row("忽略模式", `<textarea id="set-ignore" rows="4">${escapeHtml((c.ignorePatterns || []).join("\n"))}</textarea>`, "每行一个 glob 前缀")}
    <div class="actions">
      <button class="btn cancel">取消</button>
      <button class="btn primary ok">保存</button>
    </div>`;

  (dialog.querySelector(".cancel") as HTMLElement).onclick = () => mask.remove();

  // ---- shell-only section: vault picker + autostart (Tauri IPC) ----
  const ta = (window as any).__TAURI__ as
    | { core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<any> } }
    | undefined;
  if (ta) {
    void (async () => {
      let shell: { workspace: string; autostart: boolean } | null = null;
      try {
        shell = await ta.core.invoke("get_shell_conf");
      } catch {
        return;
      }
      (dialog.querySelector(".actions") as HTMLElement).insertAdjacentHTML(
        "beforebegin",
        `<div class="set-row">
          <label class="set-label">vault 目录</label>
          <div class="set-input">
            <span class="set-shell-ws">${escapeHtml(shell?.workspace || "")}</span>
            <button class="btn" id="set-pick-vault">更改…</button>
            <div class="set-hint">写入 %APPDATA%/markvault/config.json，重启 markvault 生效</div>
          </div>
        </div>
        <div class="set-row">
          <label class="set-label">开机自启</label>
          <div class="set-input">
            <input type="checkbox" id="set-autostart" ${shell?.autostart ? "checked" : ""} />
          </div>
        </div>`
      );
      (dialog.querySelector("#set-pick-vault") as HTMLElement).onclick = () => {
        void (async () => {
          try {
            const dir = await ta.core.invoke("pick_vault_dir");
            if (!dir) return;
            await ta.core.invoke("set_workspace", { path: dir });
            const wsEl = dialog.querySelector(".set-shell-ws");
            if (wsEl) wsEl.textContent = dir;
            toast("vault 已切换，重启 markvault 生效");
          } catch (e) {
            toast("设置失败: " + (e as Error).message);
          }
        })();
      };
      (dialog.querySelector("#set-autostart") as HTMLInputElement).onchange = (e) => {
        void (async () => {
          const enable = (e.target as HTMLInputElement).checked;
          try {
            const now = await ta.core.invoke("set_autostart", { enable });
            (dialog.querySelector("#set-autostart") as HTMLInputElement).checked = now;
            toast(now ? "已开启开机自启" : "已关闭开机自启");
          } catch (err) {
            (e.target as HTMLInputElement).checked = !enable;
            toast("设置失败: " + (err as Error).message);
          }
        })();
      };

      // ---- multi-workspace pool (ws_list / ws_switch) ----
      try {
        const list = (await ta.core.invoke("ws_list")) as {
          workspace: string;
          port: number;
          active: boolean;
        }[];
        const rows = list
          .map(
            (w) => `<div class="ws-row${w.active ? " active" : ""}">
              <span class="ws-name">${escapeHtml(w.workspace.split(/[\\\\/]/).pop() || w.workspace)}</span>
              <span class="ws-port">:${w.port}</span>
              ${w.active ? '<span class="ws-badge">当前</span>' : `<button class="btn ws-go" data-ws="${escapeHtml(w.workspace)}">切换</button>`}
            </div>`
          )
          .join("");
        (dialog.querySelector(".actions") as HTMLElement).insertAdjacentHTML(
          "beforebegin",
          `<div class="set-row">
            <label class="set-label">工作区</label>
            <div class="set-input">
              ${rows || '<div class="ws-empty">无打开的工作区</div>'}
              <button class="btn" id="set-ws-add" style="margin-top:6px">新增工作区…</button>
              <div class="set-hint">每个 vault 一个内核实例（同时最多 3 个），切换时主窗口直接跳转</div>
            </div>
          </div>`
        );
        dialog.querySelectorAll<HTMLButtonElement>(".ws-go").forEach((b) => {
          b.onclick = () => {
            void (async () => {
              try {
                const info = await ta.core.invoke("ws_switch", { path: b.dataset.ws });
                toast(`已切换: ${info.workspace.split(/[\\\\/]/).pop()} (:${info.port})`);
              } catch (err) {
                toast("切换失败: " + (err as Error).message);
              }
            })();
          };
        });
        (dialog.querySelector("#set-ws-add") as HTMLElement).onclick = () => {
          void (async () => {
            try {
              const dir = await ta.core.invoke("pick_vault_dir");
              if (!dir) return;
              const info = await ta.core.invoke("ws_switch", { path: dir });
              toast(`已打开: ${info.workspace.split(/[\\\\/]/).pop()} (:${info.port})`);
            } catch (err) {
              toast("打开失败: " + (err as Error).message);
            }
          })();
        };
      } catch {
        /* ws commands unavailable (older shell) */
      }
    })();
  }

  // ---- petal management (kernel API, works outside the shell too) ----
  void (async () => {
    let petals: { name: string; enabled: boolean }[] = [];
    try {
      petals = (await api.loadPetals()).petals;
    } catch {
      return; // kernel without petal support
    }
    const rows = petals
      .map(
        (p) => `<div class="petal-row">
          <span class="petal-name">${escapeHtml(p.name)}</span>
          <label class="petal-toggle"><input type="checkbox" data-petal="${escapeHtml(p.name)}" ${p.enabled ? "checked" : ""} />启用</label>
        </div>`
      )
      .join("");
    (dialog.querySelector(".actions") as HTMLElement).insertAdjacentHTML(
      "beforebegin",
      `<div class="set-row">
        <label class="set-label">插件</label>
        <div class="set-input" id="set-petals">
          ${rows || '<div class="petal-empty">无已安装插件（.markvault/petal/&lt;name&gt;/index.js）</div>'}
          <div class="set-hint">启停后重启 markvault 生效；插件代码目录由内核 /petal/ 托管</div>
        </div>
      </div>`
    );
    const savePetals = () => {
      void (async () => {
        const next = Array.from(dialog.querySelectorAll<HTMLInputElement>("#set-petals input[data-petal]")).map(
          (c) => ({ name: c.dataset.petal!, enabled: c.checked })
        );
        try {
          await api.setPetals(next);
          toast("插件配置已保存，重启 markvault 后生效");
        } catch (e) {
          toast("保存失败: " + (e as Error).message);
        }
      })();
    };
    dialog.querySelectorAll<HTMLInputElement>("#set-petals input[data-petal]").forEach((c) => {
      c.onchange = savePetals;
    });
  })();

  (dialog.querySelector(".ok") as HTMLElement).onclick = () => {
    void (async () => {
      const port = parseInt((dialog.querySelector("#set-port") as HTMLInputElement).value, 10);
      const patch: Record<string, unknown> = {
        networkServe: (dialog.querySelector("#set-network") as HTMLInputElement).checked,
        accessAuthCode: (dialog.querySelector("#set-authcode") as HTMLInputElement).value,
        apiToken: (dialog.querySelector("#set-token") as HTMLInputElement).value,
        readonly: (dialog.querySelector("#set-readonly") as HTMLInputElement).checked,
        parentAttr: (dialog.querySelector("#set-parent") as HTMLInputElement).value.trim() || "parent",
        weightAttr: (dialog.querySelector("#set-weight") as HTMLInputElement).value.trim() || "weight",
        ignorePatterns: (dialog.querySelector("#set-ignore") as HTMLTextAreaElement).value
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean),
      };
      if (!isNaN(port) && port > 0 && port !== c.port) patch.port = port;
      try {
        await api.setConf(patch);
        toast("配置已保存" + (patch.port ? "（端口需重启内核生效）" : ""));
        mask.remove();
        if (patch.readonly !== undefined || patch.ignorePatterns) {
          toast("索引/权限变更即时生效");
        }
      } catch (e) {
        toast("保存失败: " + (e as Error).message);
      }
    })();
  };
  mask.onmousedown = (e) => {
    if (e.target === mask) mask.remove();
  };
  mask.appendChild(dialog);
  document.body.appendChild(mask);
}

export function settingsButton(): HTMLElement {
  const btn = el("button", "btn", "设置");
  btn.onclick = () => void showSettings();
  return btn;
}
