# markvault

Rust 内核 + Markdown 存储的笔记软件。版本历史见 [CHANGELOG.md](CHANGELOG.md)。参考 [思源笔记](https://github.com/siyuan-note/siyuan) 的内核分离架构与 [Obsidian](https://obsidian.md) 的纯 Markdown 存储，并整合了 [obsidian-virtual-tree](https://github.com/) 与 [siyuan-virtual-tree](https://github.com/) 两个虚拟文档树插件的功能，以及思源浏览器插件的剪藏/书签数据面。

**当前版本 v0.3**：M6 内嵌浏览器 + M11 主体（关系图谱 / 快照历史 / 多工作区 / 插件系统 / 响应式）+ watcher 地基返修已交付（内核单测 26/26、E2E 41/41、NSIS 安装包）。迭代总结见 [docs/v0.3-iteration-report.html](docs/v0.3-iteration-report.html)，后续计划见 [docs/TASKS.md](docs/TASKS.md)。

```
┌──────────────┐   HTTP API + WS   ┌─────────────────────┐
│  前端 SPA     │ ◄──────────────► │  markvault-kernel    │
│ (Vite + CM6) │   127.0.0.1:7806 │  (Rust / axum)       │
└──────────────┘                  └────────┬────────────┘
        ▲                                  │ 直接读写
        │ 伺服: 同一 URL 可从浏览器/LAN 访问   ▼
┌───────┴────────┐                ┌──────────────────┐
│ Tauri 桌面壳    │                │ <vault>/**/*.md  │ ← 唯一真相
└────────────────┘                │ (SQLite 仅索引)   │
                                  └──────────────────┘
```

## 核心特性

- **内核分离**：Rust 内核进程提供 HTTP API + WebSocket 推送 + 静态前端托管，前端可从浏览器直连（同思源"伺服"模式）。
- **Markdown 为真相**：笔记就是 `.md` 文件，元数据在 YAML frontmatter；`.markvault/` 下的 SQLite 索引随时可重建。
- **虚拟文档树**（两插件功能并集）：
  - 关系树：frontmatter `parent: ["[[父笔记]]"]` 自动派生层级，多父节点、环检测、重名消歧（`X<notes1` 风格）；
  - 挂载树：手动挂载任意笔记本/文件夹/文档，可嵌套、懒展开；
  - 三级排序：拖拽序（持久化）> weight 降序 > 名称升序；
  - 定位闪烁 / 聚焦模式 / 折叠状态持久化。
- **编辑器**：CodeMirror 6，源码 + live-preview 装饰（标题行语义字号、wikilink 高亮/未解析虚线、`==高亮==`、块 id 隐藏、整行图片内联渲染）、防抖自动保存、分屏渲染预览（嵌入块/高亮/脚注）、Ctrl+S、图片粘贴上传 assets、frontmatter 表格编辑器、面包屑导航、`[[链接#锚点]]` 跳转、未解析链接点击创建。
- **反链面板**：`/api/ref/getBacklinks` 正向引用/被谁引用/伪反链（提及）三段展示。
- **导出**：单篇 md 下载；整笔记本 zip（`.markvault/export/`）。
- **全文搜索**：SQLite FTS5，`<mark>` 高亮 snippet。
- **剪藏**：`/api/clip/clipUrl` — 内核侧拉取网页 → Readability 提取 → 转 Markdown → 建档（含 source/created/tags frontmatter）。
- **书签**：`/api/bookmark/*`，前端书签面板。
- **回收站**：删除文档/笔记本移入 `.markvault/trash/`，不直接删文件。
- **内嵌浏览器**（v0.3）：Tauri v2 多窗口页签（每页签一个 WebviewWindow）、地址栏导航、页面内注入操作栏剪藏/收藏、下载到 vault、浏览历史（10 分钟去重 / 5000 条上限）。
- **关系图谱**（v0.3）：`links` 表解析为 doc→doc 边，canvas 力导向图（hover 高亮 / 点击开文档 / 拖拽钉住 / 缩放平移）。
- **快照历史**（v0.3）：`.markvault/repo/` 内容寻址文件级快照，写文件与恢复前自动留痕，每文档 30 版上限（明文 MVP，未加密）。
- **多工作区**（v0.3）：壳侧实例池（每 vault 一个内核、最多 3 个 LRU），设置面板切换/新增工作区；切换即主窗口 navigate 到目标内核 URL。
- **插件系统**（v0.3）：petal 注册表 + `/petal/<name>/` 只读托管 + SPA 动态 `import()`，宿主暴露 api/ui/命令等原语。
- **桌面壳设置与响应式**（v0.3）：vault 选择对话框、开机自启开关、petal 启停；≤860px 时 dock/right 变互斥抽屉。
- **外部文件监听**（v0.3 修复）：watcher 精确 prefix 重索引，文件与目录的新建/删除/改名都会同步索引与面板。

## 快速开始

### 1. 构建内核 + 前端

```bash
cargo build                                    # 内核 → target/debug/markvault-kernel.exe
cd frontend && npm install && npm run build    # 前端 → frontend/dist
```

### 2. 命令行伺服（浏览器访问）

```bash
./target/debug/markvault-kernel.exe serve --workspace <你的笔记目录> --port 7806
# 浏览器打开 http://127.0.0.1:7806
# 局域网伺服: 在 UI 设置里打开 networkServe，或 --network-served
```

### 3. 桌面壳（Tauri）

```bash
cargo run --manifest-path app/src-tauri/Cargo.toml
# 自动拉起内核子进程（随机空闲端口）并打开窗口
# 工作目录: MARKVAULT_WORKSPACE 环境变量 > %APPDATA%/markvault/config.json {"workspace": "..."} > ~/markvault-vault
```

### 4. 发布打包（NSIS 安装包）

```powershell
pwsh scripts/build-release.ps1            # 一键: 内核 release → sidecar → 前端 → NSIS
pwsh scripts/build-release.ps1 -SkipInstaller   # 只构建不打包
# 产物: app/src-tauri/target/release/bundle/nsis/markvault_<ver>_x64-setup.exe
```

> ⚠️ 版本号待统一：`app/src-tauri/tauri.conf.json` 仍为 `0.2.0`、两个 `Cargo.toml` 仍为 `0.1.0`，v0.3 目前只体现在文档与功能上。正式发版前需同步为 `0.3.0` 并重新打安装包（见 [v0.3 迭代总结](docs/v0.3-iteration-report.html)「剩余打磨项」）。

### 5. 日志

内核 `--log-file` 写 `<vault>/.markvault/logs/kernel.log`（5MB 循环滚动 × 3）；stdout 模式为默认。

## 布局约定

```
<vault>/
├── .markvault/
│   ├── conf.json          # 端口/伺服/访问码/apiToken/parent 属性名等
│   ├── cache/index.db     # 可重建索引 (docs/blocks/links/FTS5)
│   ├── storage/           # vt.json(虚拟树)、bookmarks.json、browser-history.json、local/*.json(UI状态)
│   ├── repo/              # v0.3 文件级快照（内容寻址，明文 MVP）
│   ├── petal/             # v0.3 插件注册表 petals.json + 每插件目录(index.js/conf.json)
│   ├── export/            # 整笔记本导出的 zip
│   └── trash/             # 回收站
├── <笔记本>/...md          # 笔记本 = 顶层文件夹
└── <笔记本>/assets/        # 附件（预览经 /vault/* 原始文件路由读取）
```

frontmatter 约定（与 obsidian-virtual-tree 兼容）：

```yaml
---
title: 可选标题（缺省用 H1 或文件名）
parent: ["[[父笔记]]"]    # 虚拟树关系，wikilink 数组，可多个
weight: 10               # 关系树排序权重
tags: [x]
created: 20260101120000  # 缺省取文件 mtime
source: "https://..."    # 剪藏文档溯源
---
```

## API 一览（`{code,msg,data}` 信封，POST JSON；详见 docs/API.md）

- `/api/system/*` — version/health/getConf/setConf/loginAuth/exit
- `/api/notebook/*` — lsNotebooks/create/rename/remove
- `/api/filetree/*` — listDocsByPath/listDocTree/createDocWithMd/renameDoc/removeDoc/moveDocs/searchDocs
- `/api/file/*` — getFile/putFile/removeFile/renameFile/readDir
- `/api/block|attr|outline` — getDocInfo/getBlockMd/getBlockInfo · insertBlock/updateBlock/deleteBlock/getBlockBreadcrumb · getBlockAttrs/setBlockAttrs(frontmatter) · getDocOutline
- `/api/ref/getBacklinks` — 反链图（正向/反向/块引用/伪反链）
- `/api/search/fullTextSearchBlock`、`/api/format/renderMd`（含 `![[嵌入]]`/`==高亮==`/脚注/`[[doc#^id]]`）
- `/api/export/exportMd|exportNotebookMd` — 单篇/整本导出
- `/api/vt/*` — getRelationTree/addRoot/removeRoot/getMountTree/setMountTree/setNodeOrder/moveNode/settings
- `/api/clip/clipUrl|clipHtml`、`/api/bookmark/*`、`/api/network/forwardProxy`
- `/api/history/*` — list/add/remove/clear（v0.3 浏览历史，10 分钟去重 / 5000 上限）
- `/api/asset/downloadToLocal` — 内核侧抓取资源写入 `<笔记本>/assets/`（v0.3）
- `/api/graph/getGraph` — `{nodes, links}` 关系图谱数据（v0.3）
- `/api/repo/*` — createSnapshot/listSnapshots/getSnapshot/restoreSnapshot/cleanSnapshots（v0.3 文件级快照）
- `/api/petal/*` — loadPetals/setPetals/getPetalConf/setPetalConf + `GET /petal/<name>/` 只读静态托管（v0.3 插件）
- `/api/sync/*` — getStatus/getSourceState/planPull/getPlan/applyPull（v0.3+ LiveSync 只读拉取与受控导入：增量水位、后台任务、跨分片合并、`target=import` 入 vault；凭据走环境变量、远端只 GET/LIST、导入只新增不覆盖）
- `/api/storage/getVal|setVal`、`/api/index/rebuild`、`GET /ws`（推送 filetree-changed/doc-reindexed/vt-changed/progress/index-done）

> `/api/system/getConf` 的 `conf` 字段返回 **camelCase** 视图（`networkServe`/`parentAttr` 等），磁盘 `conf.json` 仍是 snake_case；`setConf` 的 patch 也用 camelCase key。

## 测试

```bash
cargo test                          # 内核单元测试 26 个（关系树/消歧/排序/frontmatter/快照/历史/资源名）
cd frontend && node e2e/check.mjs   # 无头 Edge E2E 41 项（需内核已启动；截图落 _e2e/，已 gitignore）
```

v0.3 交付口径：`cargo test` 26/26、`node e2e/check.mjs` 41/41。CI/CD 已按用户要求关闭，验证链保留在本地。

## 后续路线

- 分里程碑任务清单与打勾状态：[docs/TASKS.md](docs/TASKS.md)
- 迭代总结：[v0.3 迭代总结](docs/v0.3-iteration-report.html) · [v0.2 迭代总结](docs/v0.2-iteration-report.html)
- 剩余打磨项：版本号统一 / 桌面壳人工点验 > LiveSync 对接 > 快照与插件加密 > Tauri mobile > petal 集市

## 架构文档

见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 与 [docs/API.md](docs/API.md)。
