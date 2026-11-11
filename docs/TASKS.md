# markvault 后续任务

> 状态基线：v0.3 已交付（v0.2 全量 + M6 内嵌浏览器 + M10 设置页 + watcher/subFileCount 修复 + 关系图谱 + 快照历史 + 多工作区 + 插件系统 + 移动端响应式），内核单测 26/26，E2E 41/41。8 个提交 `43d0cb6 → 591ac31` 全部推送；迭代总结见 [v0.3-iteration-report.html](v0.3-iteration-report.html)，README / API.md / ARCHITECTURE.md 已同步到 v0.3 口径。
> 本文档承接 README「后续路线」与 docs/ARCHITECTURE.md「已知限制」，按里程碑组织。做完请打勾。
> ⚠️ **发版前待办**：`tauri.conf.json` 仍 `0.2.0`、两个 `Cargo.toml` 仍 `0.1.0`，包版本与 NSIS 安装包尚未同步为 0.3.0。

## M6 内嵌浏览器（Tauri 多窗口方案，v0.3 完成）

数据面已就绪：书签面板、`/api/bookmark/*`、`/api/clip/clipUrl`、`/api/network/forwardProxy`、`/api/clip/clipHtml`（已实现待接前端）。

- [x] **浏览器页签模型**：每个页签一个 Tauri `WebviewWindow`（Tauri v2 无嵌套 webview），`app/src-tauri` 新增 `browser.rs` 管理窗口组；SPA 内嵌浏览器面板只做页签列表/新建/激活（通过 Tauri invoke 或自定义协议通信）——壳侧 6 个 `browser_*` 命令 + 500ms 轮询上报 url/title（`browser:opened`/`browser:url`/`browser:closed` 事件）；主窗口为 remote 源，`capabilities/main.json` 显式授权（`allow-browser-*`）
- [x] **地址栏 + 导航**：前进/后退（页面 history.back/forward）/刷新/停止（`window.stop`）/主页（about:blank）；协议补全（http/https/搜索词→Bing 搜索回退，移植 siyuan-plugin-browser `utils/url.ts` normalizeUrl）
- [x] **剪藏接入**：浏览器窗口注入右上角操作栏（500ms 轮询重注入，`window.__mvBar` 防重）：
  - 面板「剪藏」按钮 → 内核侧 `clipUrl`（无 cookie，公开页）
  - 注入栏「剪藏」→ 登录页走 `clipHtml`：webview eval 注入脚本在页面上下文取 `document.documentElement.outerHTML` fetch 内核（结论：Tauri eval/ExecuteScript 不受页面 CSP 限制可注入；但页面自身的 fetch 受 connect-src 限制，严格 CSP 站点注入栏报网络错误，回退面板 clipUrl）
- [x] **书签栏联动**：注入栏「收藏」+ 面板「收藏」按钮 → `/api/bookmark/add`，SPA 书签面板已可展示
- [x] **下载管理**：`POST /api/asset/downloadToLocal`（内核 fetch → `<笔记本>/assets/<文件名>`，文件名 URL basename 解码/非法字符过滤/`-2` 防覆盖，单测覆盖 sanitize+dedupe），面板「下载到 vault」按钮
- [x] 历史记录：`.markvault/storage/browser-history.json` + `/api/history/list|add|remove|clear` + 面板历史区（对齐 siyuan historyStore：10min 内同 URL 去重更新、上限 5000 截尾，含单测）

## M7 块级能力（v0.2 全部完成）

`blocks` 表已存 `block_id`（Obsidian `^id` 语法）+ heading/段落结构。

- [x] `POST /api/block/insertBlock|updateBlock|deleteBlock`：行级块操作（body 相对行定位，响应带 bodyLine 字段；updateBlock 自动保留尾部 `^id`），写入后 reindex + notify
- [x] 自动补块 ID：insertBlock `autoId:true` 默认追加思源式 ` ^id`（id.rs 恢复并接线，含格式单测）
- [x] `POST /api/block/getBlockBreadcrumb`：按 line 回溯父级 heading 栈（降栈算法含单测）
- [x] **反链面板**：v0.2 已交付 `/api/ref/getBacklinks`，前端"反链" dock；块级反链待 `[[doc#^id]]` 引用语法落地后补充
- [x] wikilink 自动补全：CM6 autocompletion `[[` 触发，docsFacet 过滤 + label/detail 双列，E2E 断言防回归
- [x] 嵌入块（`![[doc]]`）渲染：v0.2 renderMd 已实现
- [x] `[[doc#^blockId]]` 块引用语法：编辑器跳转（jumpToAnchor 支持 ^id 行尾匹配）+ 预览渲染（renderMd 显示「目标 › 块文本」label + 带锚 hash 链接）+ getBlockInfo 定位端点 + 反链面板块引用分区（blockBacklinks 按 anchor ^ 前缀分流）

## M8 虚拟树补全（v0.2 全部完成）

- [x] 拖拽跨父移动：内核 `POST /api/vt/moveNode`（改写源文档 frontmatter parent，detach = 移除 parent 属性，前端 detach 前确认）；同父拖拽仍走 setNodeOrder（拖拽序最优先）
- [x] 关系树物理子树合并：`includePhysicalSubtree` 设置（VtSettings，默认关），构建时把目录子树内未链接文档并入节点孩子（含单测：直接子目录+深层含入、其他笔记本排除）
- [x] 排序方式 UI：面板「排序:权重/名称」切换按钮（sortMethod 接入 build_subtree 双模式：weight 模式 = v0.1 三级排序兼容；name 模式 = 名称优先）+ caseSensitive 内核已支持（UI 暂无入口，后置）
- [x] 快捷键：Alt+L 定位（对齐 obsidian 版命令表）
- [x] 挂载树上下文菜单补充：右键「提升到顶层」（递归剥离 + 顶层重挂）
- [x] filetree 右键「挂载到挂载树」：doc/目录菜单接入 vtree.addMountRoot（去重）

## M9 编辑器与写作体验（v0.2 主体完成）

- [x] live-preview 增强：标题行字号/加粗语义装饰（`mv-h1..h6` 行级装饰）、`==高亮==` 编辑器内 + 预览双端、脚注渲染（comrak footnotes）
- [x] frontmatter 属性编辑器面板化：表格编辑（key/value 行、✕ 删除、添加行），替换 `window.prompt` 简陋版；parent wikilink 选择器（◈ 按钮列出全部文档）
- [x] 面包屑导航（path 段渲染，父段可点击跳转对应 .md）；大纲点击跳转已有（折叠渲染区缓后）
- [x] 双链跳转细节：`[[链接#锚点]]` 定位到标题行（先精确匹配标题文本，再退化子串匹配）；未解析链接虚线样式 + 点击创建确认
- [x] 图片粘贴：clipboard → base64 → `/api/file/putFile` 存入 `<笔记本>/assets/paste-*.png` 并插入相对链接；编辑器内整行图片语法内联渲染成 `<img>`
- [x] 导出：单篇 `POST /api/export/exportMd`（浏览器下载）+ 笔记本 `POST /api/export/exportNotebookMd`（STORE zip 落 `.markvault/export/`，内核自带 zip writer 含 UTF-8 文件名标志）
- [ ] `.sy` 导入：用户决定不做
- [x] 预览面板外部 http 链接 `window.open` 新窗口打开（原已知问题顺手修）

## M10 工程化与打包（v0.2 完成）

- [x] **Tauri release 打包**：NSIS 安装包 `markvault_0.2.0_x64-setup.exe`（6.27 MiB，内核 sidecar + 前端资源）；`scripts/build-release.ps1` 一键流程（kernel release → sidecar 拷为 `<名>-<target-triple>.exe` → 前端构建 → tauri build）；图标 15 件套（PIL 生成）；冒烟验证 release 壳拉起 sidecar 内核（随机端口 + --log-file）
- [x] **Tauri 设置页**：vault 选择对话框（`settings.rs` `pick_vault_dir` rfd 原生对话框 → `set_workspace` 写 `%APPDATA%/markvault/config.json`，重启生效）+ 开机自启开关（`set_autostart` 写 HKCU Run 注册表键）——SPA 设置面板壳区（仅 `__TAURI__` 环境显示）；kernel conf 项仍在 SPA 面板（settings.ts）
- [x] SPA 设置面板：conf 项（port/networkServe/accessAuthCode/apiToken/readonly/parentAttr/weightAttr/ignorePatterns）可视化编辑（toolbar「设置」按钮，settings.ts）
- [x] 启动索引优化：`index_meta` 表存 mtime_ms+size 戳，boot 命中即跳过解析（测试库二次启动索引阶段 62ms/8 docs）；手动「重建索引」走 force 全量；boot 与 rebuild 均接 WS progress（每 20 篇上报，原已知问题一并闭环）
- [x] cargo warning 清零（id.rs 删除、doc_summary/fm_str_array/fm_number 转 cfg(test)、WsSink/header_value/ParseLink.display 删除）
- [x] CI（GitHub Actions）：kernel（test+clippy -D warnings）/ frontend（tsc+build）/ e2e（headless Edge）三 job —— v0.3 应用户要求已关闭（删除 `.github/workflows/ci.yml`，commit dfb9b25）
- [x] 日志：内核 `--log-file` 循环滚动（5MB × 3 个文件，main.rs 内置 Roller）；Tauri 壳默认带 --log-file 拉起内核

## M11 更远期

- [~] 同步/历史：文件级快照仓库（仿思源 repo/ 加密快照）或对接已有 LiveSync 方案 —— v0.3 交付本地快照部分：内核 `.markvault/repo/`（snapshot.rs，内容寻址 `<millis>-<chash8>.md` 去重、每文档 30 版上限、plaintext MVP 加密留待后续）+ `/api/repo/createSnapshot|listSnapshots|getSnapshot|restoreSnapshot|cleanSnapshots`；putFile 写前自动快照旧内容；restore 前自动快照当前内容；前端「历史版本」对话框（filetree 右键 + 编辑器工具行「历史」）；单测 3 + E2E 3。LiveSync（CouchDB）对接未做
- [~] 多工作区切换（内核 `--workspace` 多实例 or 单实例多 vault）—— v0.3 交付路线 A（壳侧多实例）：每个 vault 一个内核实例（随机端口，同时最多 3 个 LRU），`ws_list`/`ws_switch`/`ws_close` 命令；设置面板「工作区」区列出已开实例并可切换/新增（pick_vault_dir → ws_switch）；切换 = 主窗口 navigate 到目标内核 URL（SPA 零改动），注入栏 kernel_origin 同步更新到当前 vault；启动实例随主窗口销毁全部回收。需壳内人工点验
- [x] 插件系统：前端模块加载器 + 内核 `/api/petal/*` 存储约定（对齐思源插件生态形态）—— v0.3 交付：内核 `/api/petal/loadPetals|setPetals|getPetalConf|setPetalConf`（注册表 `.markvault/petal/petals.json`、每插件 conf.json KV、插件名 `[a-z0-9-]` 白名单校验含单测）+ `/petal/<name>/` 只读静态托管（ServeDir）；前端 `petal/loader.ts` 动态 `import(/petal/<name>/index.js)` 加载，宿主 API（api 客户端/ui 原语/getActiveDoc/insertToActiveDoc/addCommand），工具栏「命令」按钮 = 插件命令菜单；示例插件 sample 全链路 E2E（托管→注册→加载→命令→toast）
- [~] 移动端布局（SPA 响应式 + Tauri mobile）—— v0.3 交付 SPA 响应式部分：≤860px 时 dock/right 变 fixed 抽屉（默认滑出屏外），工具栏 ☰/▤ 切换按钮（大屏隐藏），互斥开合；E2E 小屏 viewport 断言抽屉开/收。Tauri mobile（Android/iOS 构建）需对应 SDK 环境，未做
- [x] 关系图谱（links 表 → force graph canvas）—— v0.3 交付：内核 `POST /api/graph/getGraph`（docs 全量节点 + links 经 resolve_target 解析成 doc→doc 边，未解析目标跳过）；前端右侧「图谱」页签（panels/graph.ts，手写力导向：斥力+边弹簧+中心引力+alpha 衰减、软边界 clamp、hover 高亮邻边、节点点击打开文档、拖拽钉住、滚轮缩放/空白天拖平移/双击复位、IntersectionObserver 首次可见才加载——hidden dock 页 canvas rect=0 会把 backing store 钉死在 50px）；E2E 断言节点统计+canvas 渲染

## 已知问题（下个迭代顺手修）

- [x] **getConf 输出 snake_case 而 SPA 读 camelCase** —— settings 面板 escapeHtml(undefined) 页面错误（v0.2 起面板从未被 E2E 触发过）；修法 get_conf 手动构建 camelCase 视图（conf.json 文件保持 snake_case 零迁移）
- [x] `readDir` 列出 `.` 开头目录 —— v0.2 已加 dot 过滤（与 filetree/listDocsByPath 语义对齐）
- [x] listDocsByPath 的 `subFileCount` 对 folder 是全递归计数，与 obsidian 版"直接子文档数"语义不一致，二选一 —— v0.3 统一为直接子文档数（`count_md_direct`：`LIKE dir/% AND NOT LIKE dir/%/%`），E2E 断言防回归
- [x] `fmt_progress` 与 ws `push_progress` 未接线 —— v0.2 boot/rebuild 均已上报（每 20 篇 + 收尾）
- [x] tabs.ts `onOpenLink` 外部 http 链接处理 —— 预览面板已接 `window.open`；编辑器内 wiki 装饰本身只处理 doc 目标
- [x] watcher 对"目录重命名"事件会退化为全量结构广播而非精确 prefix 重索引（`serve.rs` 只按文件粒度处理）—— v0.3 修复，且顺藤摸出**两个更深的 bug**：①`start_watcher` 的 debouncer 是局部变量，函数返回即 drop，**watcher 自 v0.1 起从未真正工作过**（boot 全量索引 + API 内部 reindex 掩盖）；②debouncer-full 把 verbatim 路径归一化成 `//?/E:/...` 形式，Windows path parser 不认，`vault.rel` 的 strip_prefix 全部失败——加 `normalize_win` 字符串级前缀回退。修复后目录改名 = 精确 prefix 重索引（unindex 旧路径 + `reindex_prefix` 新路径），E2E 断言防回归
- [x] preview 面板中 `/#/doc/` 链接在 Tauri 外部 URL 窗口内要验证 hash 路由兼容性 —— v0.3 E2E 覆盖：预览面板点击 `a[href*="#/doc/"]` → hashchange → resolveLink → 文档页签打开（E2E 的 headless Edge 与 Tauri WebView2 同为 Chromium 内核，hashchange 行为一致）
- [x] **CM6 装饰插件漏传 `{decorations}` spec** —— v0.1 起 wikilink/块id 装饰从未真正渲染（E2E 无断言漏网），v0.2 五个装饰插件统一补上 `ViewPlugin.fromClass(cls, { decorations: v => v.decorations })`
- [x] refs.rs 初版 db Mutex 重入死锁（lock 持有下调 load_doc_infos）—— 与 listDocsByPath 同类坑，先取 infos 再 lock
- [x] **editor-pane hidden 类失效**：`buildPane` 初始 hidden 时活动页签 pane 未被显示（`tabs.ts` 改为无条件 `remove("hidden")`）—— v0.3 第 4 个潜伏 bug
- [x] **watcher 双深层 bug**：debouncer 栈变量被 drop（v0.1 起外部变更从未索引）+ `vault.rel` verbatim 路径解析全失败 —— 详见上方已知问题条目
---

## v0.3 之后的待办（按优先级）

> M6-M11 主体全部交付（~~M7 > M8 > M6 > M10 > 已知问题 > 图谱 > 快照历史 > 多工作区 > 插件系统 > 移动端响应式~~）。以下为 2026-11-11 收尾时确认的剩余项。

### 功能项（M11 剩余）

| 任务 | 状态 | 说明与依赖 |
|---|---|---|
| **LiveSync 对接** | 侦察完成 / 只读 MVP 待实现 | **实际后端是 MinIO/S3 journal sync（非 CouchDB）**：`<毫秒>-docs.jsonl.gz` 加密分片 + `sync-parameters`。当前插件 1.0.47：E2EE v2、路径混淆、ID 派生 v1（恢复码）。已验证只读 LIST + PBKDF2(310k)→HKDF→AES-GCM 分片解密 → raw deflate → JSONL 全链路（口令有效）；配置层由另一把口令保护、只读拉取不需要。**S1-S6 已交付、S7 边界定案（内核 API 形态）**：`/api/sync/*` 六端点（含 `getPlan` 进度轮询）、34/34 单测；S1 分片解密与元数据预览、S2 chunk 行格式（`~h:+<id>\u{1f}<内容>`）与正文导出、S3 后台任务 + 增量水位 + chunk 内容寻址落盘 + 跨分片合并、S4 受控导入（实机 9 篇、FTS 可见）、**S5 进度推前端**（`sync-progress/sync-done` WS 事件 + 设置面板「同步」区 UI：状态/增量/全量/导入/导出，浏览器实机验证）、**S6 快照加密**（`/api/repo/initKey` 口令派生 + AES-256-GCM，legacy 明文兼容，实机密文落盘/解密/关闭回退）。**S7**：本地半边（vault ↔ repo 快照双向）已闭环；LiveSync 上传方向明确不做（风险边界见 [sync-architecture.md](sync-architecture.md) §S7）。剩余：全量首拉实测中（约 775MB）、导出笔记入索引的 `index` 直连目标。另：petal/repo 加密条目中的快照部分已由本项交付。详见 [livesync-readonly-recon.md](livesync-readonly-recon.md)；已参考思源内核同步实现产出 [sync-architecture.md](sync-architecture.md)（接口契约 + S1-S7 实现顺序，红线：只读阶段不回写 `.md`） |
| **Tauri mobile 构建** | 未做 | Android/iOS 打包需对应 SDK 环境（本机 Windows 只能走 Android 路线）；SPA 响应式已交付，此项纯构建配置层 |
| **petal 加密** | 未做 | 插件存储为 plaintext；repo 快照加密已由 S6 交付（`/api/repo/initKey` 口令派生 + AES-256-GCM）；petal 侧密钥管理待设计 |
| **petal 集市** | 未做 | 插件市场/分发（列表、下载、更新），生态成型后再做；当前只支持本地安装 |

### 验证项（代码已交付，需人工点验）

- **壳内交互**：浏览器多页签（打开/剪藏/收藏/注入栏）、多工作区实际切换、vault 选择对话框（rfd）、开机自启开关——E2E 跑在浏览器环境，触不到 Tauri IPC。
- **preview `/#/doc/` 在 Tauri WebView2 内的 hash 路由**：E2E 已用同类 Chromium 内核覆盖，壳内实机未点。

### 顺手可做（小）

- v0.2 遗留：`watcher 对目录重命名的 vt 挂载树跟随` —— 目录改名后挂载树节点会 missing，需重挂（v0.3 只修了索引精确性）。
- **release 版重打包**：NSIS 安装包仍是 v0.2.0，功能已大幅演进；统一版本号后出 `markvault_0.3.0`。

### 壳内点验修复轮（2026-11-11 追加）

按建议第 1 项跑了一轮真实桌面壳点验（WebView2 CDP + Tauri IPC，全程只用 `testvault`）：**17 项自动点验全绿**，并发现修复 4 个真实缺陷——设置对话框按钮被挤进 `.actions` 内、`browser_open` 同步命令创建窗口自锁、任意窗口销毁误杀全部内核实例、启动实例路径未归一化导致切回初始 vault 重复起内核。逐项结果与证据见 [v0.3-shell-verification.md](v0.3-shell-verification.md)。剩余 7 项需人工点验（rfd 对话框 / 开机自启 / 面板侧按钮 / CSP 回退 / preview hash 实机 / 小屏抽屉 / 插件命令菜单）。

### 建议顺序

1. [x] **LiveSync 后续待办**（2026-11-12 收尾完成，见下节）
2. **壳内人工点验剩余 7 项**：可自动化的 4 项（E1/F1/B7-web/D2-web）已由 `frontend/e2e/manual7.mjs` 自动化（8/8 PASS，2026-11-12）；壳内人工项剩 C1(vault 对话框)/C2(开机自启)/B10(严格CSP)/B7·D2·E1·F1 壳内抽查
3. [x] **前端同步 UI polish**（2026-11-12：阶段中文化进度条、applyPull 后台化轮询修复（getPlan.apply）、成功/失败着色；dummy 凭据实测错误态截图 _e2e/m7_sync_polish.png）
4. [~] **出 0.3.0 安装包**（2026-11-12 暂缓，用户确认先不兜底）：版本号已对齐 0.3.0、CHANGELOG.md 就绪、sidecar 内核 release 已备（`target/release/markvault-kernel.exe` → `app/src-tauri/binaries/markvault-kernel-x86_64-pc-windows-msvc.exe`）。**卡点 = 杀软锁定 proc-macro DLL**：`zerovec_derive.dll: LoadLibraryExW failed: os error 32 (retried 5 times)` → 级联 "can't find crate for yoke/zerovec/phf/tauri/serde"，tauri-cli 安装与 app release 构建均反复中招（-j2、复用 artifacts 均无效）。续做：给 `%CARGO_HOME%`、`app/src-tauri/target`、`%TEMP%/cargo-install*` 加杀软排除后跑 `CARGO_HOME=<临时> cargo install tauri-cli --locked` → `cargo tauri build`（tauri.conf 已配 NSIS+externalBin+图标）

### LiveSync 后续待办（2026-11-11 暂停点）—— ✅ 2026-11-12 收尾完成

S1-S7 主体已交付并推送（至 `891ca68`）。暂停点的收尾清单执行结果：

1. [x] **修 getPlan 缺 apply 字段**：组装抽为 `plan_summary` 补 `"apply": ApplyState`，单测 `plan_summary_includes_apply_state`（内核 35/35）。
2. [x] **重验导入进度**：跳过实机重跑（口令按纪律不存储，未再次提供）；修复已由单测覆盖，管线本身 S4/S5 实机已验。`_tmp/l2_verify.py` 备好实机脚本，待口令即可跑 `planPull(30)→applyPull(import)→getPlan 轮询 apply.phase`。
3. [x] **提交工作区改动**：applyPull 后台化（`spawn_blocking` + `ApplyState` + `apply_bump` + 批量 `rebuild_all_ext`），零 warning。
4. [x] **清理测试库**：51 篇明文导入笔记 → `.markvault/trash/cleanup_*_livesync`（trash 可恢复）；水位 `sync/livesync/`、加密快照 `.markvault/repo/`、`_tmp/new_apply.rs`/`s4_perf.py` 探针已删；`.gitignore` 补 `testvault/livesync/`、`testvault/repo/` 防泄漏。
5. [x] **补 memory 日志**：见 memory/2026-11-11.md 收尾记录。

已量化/待查：

- applyPull 全量导入耗时长：6,851 篇逐篇 reindex 超 HTTP 客户端 300s 超时（后台化后不再受限，结果正确）；批量 `rebuild_all_ext` 的实际收益待量化。
- [x] docCount 口径差异（2026-11-12 定案）：观察值是**写入阶段中途快照**（docs 表只在末尾批量重建时更新），且当时 watcher 对 6,850 个写入逐个 reindex_one + WS 广播（300s 超时真因）→ 已加 `AppState.bulk_apply` 闸：apply 写入期间 watcher 停机（Drop 兜底防 panic 卡死），末尾单次 force 重建一次性填满 docs。
- 全量「拉取+导入」端到端在修复后完整重跑一次。
- 更远：petal 密钥管理、Tauri mobile 构建。

改动前先读 docs/ARCHITECTURE.md 的数据流与设计决策节。
