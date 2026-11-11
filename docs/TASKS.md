# markvault 后续任务

> 状态基线：v0.3 已交付（v0.2 全量 + M6 内嵌浏览器），内核单测 22/22，E2E 24/24。
> 本文档承接 README「后续路线」与 docs/ARCHITECTURE.md「已知限制」，按里程碑组织。做完请打勾。

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
- [ ] Tauri 设置页：vault 选择对话框（代替 `%APPDATA%/markvault/config.json` 手编）、开机自启开关 —— SPA 设置面板已可改 conf，壳侧对话框缓后
- [x] SPA 设置面板：conf 项（port/networkServe/accessAuthCode/apiToken/readonly/parentAttr/weightAttr/ignorePatterns）可视化编辑（toolbar「设置」按钮，settings.ts）
- [x] 启动索引优化：`index_meta` 表存 mtime_ms+size 戳，boot 命中即跳过解析（测试库二次启动索引阶段 62ms/8 docs）；手动「重建索引」走 force 全量；boot 与 rebuild 均接 WS progress（每 20 篇上报，原已知问题一并闭环）
- [x] cargo warning 清零（id.rs 删除、doc_summary/fm_str_array/fm_number 转 cfg(test)、WsSink/header_value/ParseLink.display 删除）
- [x] CI（GitHub Actions）：kernel（test+clippy -D warnings）/ frontend（tsc+build）/ e2e（headless Edge）三 job —— v0.3 应用户要求已关闭（删除 `.github/workflows/ci.yml`，commit dfb9b25）
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
优先级建议：~~M7 块级操作 > M8 虚拟树补全 > M6 内嵌浏览器~~（均已交付）；剩余 M10 设置页对话框 > 已知问题清理 > M11。
改动前先读 docs/ARCHITECTURE.md 的数据流与设计决策节。
