# markvault 后续任务

> 状态基线：v0.1 已交付（内核 M1 / 前端 M2 / 虚拟树 M3 / Tauri M4 / 剪藏+书签 M5），单测 9/9，E2E 14/14。
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
- [ ] `GET /api/block/getBlockBreadcrumb`（按 line 回溯父级 heading 栈）
- [ ] **反链面板**：`links` 表已有 wikilink 边；补 `/api/ref/getBacklinks`（正向：引用了谁/被谁引用），前端加"反链" dock；伪反链（搜索关键词提及）可后置
- [ ] wikilink 自动补全（M2 遗留）：CM6 `autocompletion` 源接 `listDocTree` 数据（`editor/cm.ts` 预留位）
- [ ] 嵌入块（`![[doc]]`）渲染：renderMd 端点解析 embed 语法并内联目标文档 HTML

## M8 虚拟树补全（对照两插件残余差异）

- [ ] 拖拽跨父移动：目前 `reorderSiblings` 仅支持同级（siyuan 版仅 sibling 语义，一致；obsidian 版无拖拽）；如需移动语义要在内核加 `moveNode`（改源文件 frontmatter `parent`）
- [ ] 关系树物理子树合并：对应 siyuan-virtual-tree `includePhysicalSubtree`（SQL path-prefix 拉入真实层级），内核 `vt.rs` 加开关
- [ ] 排序方式 UI：`sortMethod`("name"/"weight") 已支持内核存储，面板缺切换入口 + `caseSensitive` 开关
- [ ] 折叠态与 focus/locate 的快捷键（Alt+L 定位、Ctrl+Shift+R 刷新等，对齐 obsidian 版命令表）
- [ ] 挂载树上下文菜单补充：提升子挂载到顶层、右键空白菜单 = 挂载选择器（对齐 obsidian 版 `view.ts`）
- [ ] filetree 右键"挂载到挂载树"（面板已有 menu 入口，未与 filetree.ts 联通）

## M9 编辑器与写作体验

- [ ] live-preview 增强：标题行字号/加粗语义装饰（现在仅语法高亮）、`==高亮==`、脚注渲染
- [ ] frontmatter 属性编辑器面板化（现在是 `window.prompt` 简陋版，`tabs.ts showAttrs`）：表格编辑 + parent wikilink 选择器
- [ ] 大纲点击折叠/展开渲染区；面包屑导航（hpath 链）
- [ ] 双链跳转细节：`[[链接#锚点]]` 定位到标题行；未解析链接虚线样式 + 创建提示
- [ ] 图片粘贴：clipboard → `/api/file/putFile`(base64) 存入 notebook assets 并插入相对链接
- [ ] 导出：单篇/笔记本导出 md zip（`/api/export/*`，对齐思源 exportMd/exportNotebookMd 语义）
- [ ] `.sy` 导入：思源库迁移（解析 `.sy` JSON AST → markdown），放 `crates/kernel/src/import_sy.rs`

## M10 工程化与打包

- [ ] **Tauri release 打包**：tauri-cli + `bundle.externalBin` sidecar（需 `markvault-kernel-x86_64-pc-windows-msvc.exe` 命名拷贝）、NSIS 安装包、图标各尺寸生成；`cargo tauri build`
- [ ] Tauri 设置页：vault 选择对话框（代替 `%APPDATA%/markvault/config.json` 手编）、开机自启开关
- [ ] SPA 设置面板：conf 项（port/networkServe/accessAuthCode/apiToken/parentAttr/weightAttr/ignorePatterns）可视化编辑（现在只有 getConf/setConf API）
- [ ] 启动索引优化：全量重建改为 mtime+size 校验增量（docs 表已有 size/updated，加 `indexed_mtime` 列即可），大 vault 冷启动提速
- [ ] 11 个 cargo warning 清理（unused id.rs/doc_summary/fm_str_array 等要么接入要么删除）；`cargo clippy --fix`
- [ ] E2E 扩充：剪藏全链路（mock URL）、拖拽排序、forwardProxy；CI（GitHub Actions：cargo test + npm build + e2e headless）
- [ ] 日志：内核 `--log-file` 选项 + 循环滚动，替代 stdout-only

## M11 更远期

- [ ] 同步/历史：文件级快照仓库（仿思源 repo/ 加密快照）或对接已有 LiveSync 方案
- [ ] 多工作区切换（内核 `--workspace` 多实例 or 单实例多 vault）
- [ ] 移动端布局（SPA 响应式 + Tauri mobile）
- [ ] 插件系统：前端模块加载器 + 内核 `/api/petal/*` 存储约定（对齐思源插件生态形态）
- [ ] 关系图谱（links 表 → force graph canvas）

## 已知问题（下个迭代顺手修）

- [ ] `readDir` 会列出 `.` 开头目录（含 .markvault）——加 dot 过滤或保持"原始访问"语义并文档化
- [ ] listDocsByPath 的 `subFileCount` 对 folder 是全递归计数，与 obsidian 版"直接子文档数"语义不一致，二选一
- [ ] `fmt_progress`（api/ops.rs）与 ws `push_progress` 未接线（rebuild 时上报进度）
- [ ] tabs.ts `onOpenLink` 只处理 doc 跳转，外部 http 链接应 `window.open`
- [ ] watcher 对"目录重命名"事件会退化为全量结构广播而非精确 prefix 重索引（`serve.rs` 只按文件粒度处理）
- [ ] preview 面板中 `/#/doc/` 链接在 Tauri 外部 URL 窗口内要验证 hash 路由兼容性

---
优先级建议：M9 编辑器体验 > M7 块级反链 > M8 虚拟树补全 > M6 内嵌浏览器 > M10 打包。
改动前先读 docs/ARCHITECTURE.md 的数据流与设计决策节。
