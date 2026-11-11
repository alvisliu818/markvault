# markvault 后续任务

> 状态基线：v0.2 已交付（v0.1 五里程碑 + M9 编辑器体验 + 部分已知问题修复），内核单测 10/10，E2E 18/18。
> 本文档承接 README「后续路线」与 docs/ARCHITECTURE.md「已知限制」，按里程碑组织。做完请打勾。

## M6 内嵌浏览器（Tauri 多窗口方案）

数据面已就绪：书签面板、`/api/bookmark/*`、`/api/clip/clipUrl`、`/api/network/forwardProxy`、`/api/clip/clipHtml`（已实现待接前端）。

- [ ] **浏览器页签模型**：每个页签一个 Tauri `WebviewWindow`（Tauri v2 无嵌套 webview），`app/src-tauri` 新增 `browser.rs` 管理窗口组；SPA 内嵌浏览器面板只做页签列表/新建/激活（通过 Tauri invoke 或自定义协议通信）
- [ ] **地址栏 + 导航**：前进/后退/刷新/停止/主页；协议补全（http/https/搜索词→搜索引擎），参考 siyuan-plugin-browser `src/browser/Toolbar.ts` 的搜索引擎回退逻辑
- [ ] **剪藏接入**：浏览器窗口监听剪藏动作 → 两种路径：
  - 内核侧 `clipUrl`（无 cookie，公开页）
  - 登录页走 `clipHtml`：在 webview 窗口内取 `document.documentElement.outerHTML` 传给内核（需评估 Tauri 对外部 URL 页面的 JS 注入限制；受限则回退 `clipUrl`）
- [ ] **书签栏联动**：浏览器窗口收藏 → `/api/bookmark/add`，SPA 书签面板已可展示
- [ ] **下载管理**：`/api/network/forwardProxy` 已可内核侧抓取；落地"保存图片/文件到 vault assets"端点（`/api/asset/downloadToLocal`），参考 siyuan-plugin-browser `kernel.ts` 的 putFile multipart 契约
- [ ] 历史记录：`.markvault/storage/browser-history.json` + 历史面板（对齐 bookmarksStore 的 10min 去重/上限策略）

## M7 块级能力（索引已备好地基）

`blocks` 表已存 `block_id`（Obsidian `^id` 语法）+ heading/段落结构，无 API 暴露。

- [ ] `POST /api/block/insertBlock|updateBlock|deleteBlock`：行级块操作（内部定位 = doc_path + line），写入后 `ops::reindex_one`
- [ ] 自动补块 ID：新建块写入时用 `id::new_node_id()` 生成（思源式 14 位时间戳-7 位随机，`id.rs` 已备）
- [x] `GET /api/block/getBlockBreadcrumb`（v0.2 以面包屑组件实现：前端按 path 段渲染，父段可点击跳转）
- [x] **反链面板**：v0.2 已交付 `/api/ref/getBacklinks`（正向 links/未解析 + 反向 backlinks + 伪反链 mentions），前端"反链" dock；块级反链待 M7 块操作落地后补充
- [x] wikilink 自动补全：v0.2 用 CM6 docsFacet 实现**未解析链接虚线样式 + 点击创建提示**；下拉补全列表仍开放（接 `listDocTree` 数据即可）
- [x] 嵌入块（`![[doc]]`）渲染：renderMd 端点已实现（一级展开、带标题栏和"打开"链接、相对链接按目标文档位置重写）

## M8 虚拟树补全（对照两插件残余差异）

- [ ] 拖拽跨父移动：目前 `reorderSiblings` 仅支持同级（siyuan 版仅 sibling 语义，一致；obsidian 版无拖拽）；如需移动语义要在内核加 `moveNode`（改源文件 frontmatter `parent`）
- [ ] 关系树物理子树合并：对应 siyuan-virtual-tree `includePhysicalSubtree`（SQL path-prefix 拉入真实层级），内核 `vt.rs` 加开关
- [ ] 排序方式 UI：`sortMethod`("name"/"weight") 已支持内核存储，面板缺切换入口 + `caseSensitive` 开关
- [ ] 折叠态与 focus/locate 的快捷键（Alt+L 定位、Ctrl+Shift+R 刷新等，对齐 obsidian 版命令表）
- [ ] 挂载树上下文菜单补充：提升子挂载到顶层、右键空白菜单 = 挂载选择器（对齐 obsidian 版 `view.ts`）
- [ ] filetree 右键"挂载到挂载树"（面板已有 menu 入口，未与 filetree.ts 联通）

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
- [ ] Tauri 设置页：vault 选择对话框（代替 `%APPDATA%/markvault/config.json` 手编）、开机自启开关 —— SPA 设置面板已可改 conf，壳侧对话框缓后
- [x] SPA 设置面板：conf 项（port/networkServe/accessAuthCode/apiToken/readonly/parentAttr/weightAttr/ignorePatterns）可视化编辑（toolbar「设置」按钮，settings.ts）
- [x] 启动索引优化：`index_meta` 表存 mtime_ms+size 戳，boot 命中即跳过解析（测试库二次启动索引阶段 62ms/8 docs）；手动「重建索引」走 force 全量；boot 与 rebuild 均接 WS progress（每 20 篇上报，原已知问题一并闭环）
- [x] cargo warning 清零（id.rs 删除、doc_summary/fm_str_array/fm_number 转 cfg(test)、WsSink/header_value/ParseLink.display 删除）
- [x] CI（GitHub Actions）：kernel（test+clippy -D warnings）/ frontend（tsc+build）/ e2e（headless Edge）三 job
- [x] 日志：内核 `--log-file` 循环滚动（5MB × 3 个文件，main.rs 内置 Roller）；Tauri 壳默认带 --log-file 拉起内核

## M11 更远期

- [ ] 同步/历史：文件级快照仓库（仿思源 repo/ 加密快照）或对接已有 LiveSync 方案
- [ ] 多工作区切换（内核 `--workspace` 多实例 or 单实例多 vault）
- [ ] 移动端布局（SPA 响应式 + Tauri mobile）
- [ ] 插件系统：前端模块加载器 + 内核 `/api/petal/*` 存储约定（对齐思源插件生态形态）
- [ ] 关系图谱（links 表 → force graph canvas）

## 已知问题（下个迭代顺手修）

- [x] `readDir` 列出 `.` 开头目录 —— v0.2 已加 dot 过滤（与 filetree/listDocsByPath 语义对齐）
- [ ] listDocsByPath 的 `subFileCount` 对 folder 是全递归计数，与 obsidian 版"直接子文档数"语义不一致，二选一
- [x] `fmt_progress` 与 ws `push_progress` 未接线 —— v0.2 boot/rebuild 均已上报（每 20 篇 + 收尾）
- [x] tabs.ts `onOpenLink` 外部 http 链接处理 —— 预览面板已接 `window.open`；编辑器内 wiki 装饰本身只处理 doc 目标
- [ ] watcher 对"目录重命名"事件会退化为全量结构广播而非精确 prefix 重索引（`serve.rs` 只按文件粒度处理）
- [ ] preview 面板中 `/#/doc/` 链接在 Tauri 外部 URL 窗口内要验证 hash 路由兼容性
- [x] **CM6 装饰插件漏传 `{decorations}` spec** —— v0.1 起 wikilink/块id 装饰从未真正渲染（E2E 无断言漏网），v0.2 五个装饰插件统一补上 `ViewPlugin.fromClass(cls, { decorations: v => v.decorations })`
- [x] refs.rs 初版 db Mutex 重入死锁（lock 持有下调 load_doc_infos）—— 与 listDocsByPath 同类坑，先取 infos 再 lock
---
优先级建议：M7 块级操作 > M8 虚拟树补全 > M6 内嵌浏览器。
改动前先读 docs/ARCHITECTURE.md 的数据流与设计决策节。
