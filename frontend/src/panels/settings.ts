import { api } from "../api";
import { el, toast, escapeHtml } from "../ui";

const SYNC_PHASE_ZH: Record<string, string> = {
  preparing: "准备中",
  listing: "列清单",
  downloading: "下载分片",
  parsing: "解析",
  done: "完成",
  error: "失败",
  writing: "写入 vault",
  reindexing: "重建索引",
};

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
    repoEncryption?: boolean;
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

  // ---- LiveSync read-only pull (kernel API; credentials live in kernel env) ----
  void (async () => {
    let status: Awaited<ReturnType<typeof api.syncGetStatus>>;
    try {
      status = await api.syncGetStatus();
    } catch {
      return; // kernel without sync support
    }
    const syncBody = status.configured
      ? `<div class="set-hint" id="sync-state">状态加载中…</div>
         <div class="sync-progress" id="sync-progress" hidden><div class="sync-progress-fill" id="sync-progress-fill"></div></div>
         <div style="display:flex;gap:6px;margin-top:6px;flex-wrap:wrap">
           <button class="btn" id="sync-refresh">刷新远端状态</button>
           <button class="btn" id="sync-plan">拉取计划（增量）</button>
           <button class="btn" id="sync-plan-full">拉取计划（全量）</button>
         </div>
         <div style="display:flex;gap:6px;margin-top:6px;align-items:center;flex-wrap:wrap">
           <input type="text" id="sync-notebook" placeholder="导入笔记本名（默认 livesync）" style="flex:1;min-width:160px" />
           <label style="font-size:12px"><input type="checkbox" id="sync-partial" /> 含部分命中的笔记</label>
         </div>
         <div style="display:flex;gap:6px;margin-top:6px">
           <button class="btn primary" id="sync-import">导入到 vault</button>
           <button class="btn" id="sync-export">导出到目录</button>
         </div>
         <div class="set-hint">远端只读（GET/LIST）；导入只新增、绝不覆盖已有笔记；进度经 WS sync-progress 推送</div>`
      : `<div class="set-hint">未配置：需以 MARKVAULT_SYNC_* 环境变量启动内核（键名见 docs/API.md 的 sync 节）</div>`;
    (dialog.querySelector(".actions") as HTMLElement).insertAdjacentHTML(
      "beforebegin",
      `<div class="set-row">
        <label class="set-label">同步</label>
        <div class="set-input" id="set-sync">${syncBody}</div>
      </div>`
    );
    if (!status.configured) return;
    const stateEl = () => dialog.querySelector("#sync-state") as HTMLElement | null;
    const setState = (msg: string, kind: "info" | "ok" | "error" = "info") => {
      const el = stateEl();
      if (!el) return;
      el.textContent = msg;
      el.classList.toggle("sync-error", kind === "error");
      el.classList.toggle("sync-ok", kind === "ok");
    };
    const progressEl = () => dialog.querySelector("#sync-progress") as HTMLElement | null;
    const fillEl = () => dialog.querySelector("#sync-progress-fill") as HTMLElement | null;
    const setProgress = (done: number, total: number, phase: string) => {
      const bar = progressEl();
      if (!bar) return;
      bar.hidden = false;
      const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
      (fillEl() as HTMLElement).style.width = pct + "%";
      setState(`${SYNC_PHASE_ZH[phase] || phase} ${done}/${total}（${pct}%）`);
    };
    const hideProgress = () => {
      const el = progressEl();
      if (el) el.hidden = true;
    };
    let lastPlan: { planId: string; token: string } | null = null;
    const busy = (msg: string) => {
      setState(msg);
      for (const id of ["sync-refresh", "sync-plan", "sync-plan-full", "sync-import", "sync-export"]) {
        const b = dialog.querySelector(`#${id}`) as HTMLButtonElement | null;
        if (b) b.disabled = true;
      }
    };
    const idle = () => {
      for (const id of ["sync-refresh", "sync-plan", "sync-plan-full", "sync-import", "sync-export"]) {
        const b = dialog.querySelector(`#${id}`) as HTMLButtonElement | null;
        if (b) b.disabled = false;
      }
    };
    (dialog.querySelector("#sync-refresh") as HTMLElement).onclick = () => {
      void (async () => {
        try {
          busy("刷新远端状态…");
          const s = await api.syncGetSourceState();
          setState(`远端 ${s.shards} 分片 / ${(s.bytes / 1048576).toFixed(1)} MB · 最新 ${s.latestShard.slice(0, 24)}… · 本地水位 ${s.localSyncPoint || "无"}`, "ok");
        } catch (e) {
          setState("刷新失败: " + (e as Error).message, "error");
        } finally {
          idle();
        }
      })();
    };
    const runPlan = (full: boolean) => {
      void (async () => {
        try {
          busy(full ? "全量拉取中（分片较多，请耐心等待）…" : "增量拉取中…");
          const p = await api.syncPlanPull({ full, limit: 0 });
          lastPlan = { planId: p.planId, token: p.token };
          for (;;) {
            const s = await api.syncGetPlan(p.planId);
            if (s.done) {
              hideProgress();
              if (s.error) {
                setState("拉取失败: " + s.error, "error");
                toast("拉取失败: " + s.error);
              } else {
                setState(`拉取完成：${s.shards} 分片（跳过 ${s.skippedShards}）· ${s.lines} 行 · ${s.parseErrors} 解析错误 · 笔记 ${s.docsTotal}（完整 ${s.docsComplete} / 部分 ${s.docsPartial} / 未覆盖 ${s.docsUncovered}）`, "ok");
                toast(`拉取完成：笔记 ${s.docsTotal} 篇`);
              }
              break;
            }
            setProgress(s.progress, s.progressTotal, s.phase);
            await new Promise((r) => setTimeout(r, 1000));
          }
        } catch (e) {
          hideProgress();
          setState("拉取失败: " + (e as Error).message, "error");
        } finally {
          idle();
        }
      })();
    };
    (dialog.querySelector("#sync-plan") as HTMLElement).onclick = () => runPlan(false);
    (dialog.querySelector("#sync-plan-full") as HTMLElement).onclick = () => runPlan(true);
    const apply = (target: "import" | "export") => {
      void (async () => {
        if (!lastPlan) {
          toast("请先拉取计划");
          return;
        }
        const notebook = (dialog.querySelector("#sync-notebook") as HTMLInputElement).value.trim() || "livesync";
        const includePartial = (dialog.querySelector("#sync-partial") as HTMLInputElement).checked;
        try {
          busy(target === "import" ? "导入中…" : "导出中…");
          await api.syncApplyPull({ planId: lastPlan.planId, token: lastPlan.token, target, notebook, includePartial });
          // applyPull 已后台化（内核 spawn_blocking）：轮询 getPlan.apply 直到 done/error
          for (;;) {
            const s = await api.syncGetPlan(lastPlan.planId);
            const a = s.apply;
            if (!a) throw new Error("apply 状态缺失（内核版本过旧或已被清理）");
            if (a.done || a.phase === "error") {
              hideProgress();
              if (a.phase === "error" || a.error) {
                setState(`${target === "import" ? "导入" : "导出"}失败: ${a.error || "unknown"}`, "error");
                toast("同步应用失败: " + (a.error || "unknown"));
                break;
              }
              if (target === "import") {
                setState(`导入完成：新增 ${a.imported} · 已存在跳过 ${a.skippedExisting} · 部分跳过 ${a.skippedPartial} · 空 ${a.skippedEmpty} · 不安全路径 ${a.skippedUnsafe} · 错误 ${a.errors}`, "ok");
                toast(`导入完成：新增 ${a.imported} 篇笔记`);
              } else {
                setState(`导出完成：${a.imported} 篇 → ${String(a.exportDir || "").slice(0, 60)}`, "ok");
                toast(`导出完成：${a.imported} 篇`);
              }
              break;
            }
            setProgress(a.progress, a.total, a.phase);
            await new Promise((r) => setTimeout(r, 1000));
          }
        } catch (e) {
          hideProgress();
          setState(target + " 失败: " + (e as Error).message, "error");
        } finally {
          idle();
        }
      })();
    };
    (dialog.querySelector("#sync-import") as HTMLElement).onclick = () => apply("import");
    (dialog.querySelector("#sync-export") as HTMLElement).onclick = () => apply("export");
  })();

  // ---- snapshot repo encryption (/api/repo/initKey, SiYuan Conf.Repo.Key style) ----
  (dialog.querySelector(".actions") as HTMLElement).insertAdjacentHTML(
    "beforebegin",
    `<div class="set-row">
      <label class="set-label">快照加密</label>
      <div class="set-input">
        <div style="display:flex;gap:6px;align-items:center">
          <input type="password" id="repo-pass" placeholder="${c.repoEncryption ? "输入新口令更换 / 留空提交即关闭" : "设置口令以加密快照"}" style="flex:1" />
          <button class="btn" id="repo-init">应用</button>
        </div>
        <div class="set-hint" id="repo-state">${c.repoEncryption ? "已开启（密文快照；S6 之前的明文快照仍可读）" : "未开启（快照明文存储）"}</div>
      </div>
    </div>`
  );
  (dialog.querySelector("#repo-init") as HTMLElement).onclick = () => {
    void (async () => {
      const pass = (dialog.querySelector("#repo-pass") as HTMLInputElement).value;
      try {
        const r = await api.repoInitKey(pass);
        const el = dialog.querySelector("#repo-state") as HTMLElement | null;
        if (el) el.textContent = r.enabled ? `已开启（verifier ${r.verifier ?? ""}；旧明文快照仍可读）` : "未开启（快照明文存储）";
        toast(r.enabled ? "快照加密已开启" : "快照加密已关闭");
      } catch (e) {
        toast("设置失败: " + (e as Error).message);
      }
    })();
  };

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
