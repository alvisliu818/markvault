# markvault

Rust 内核 + Markdown 存储的笔记软件。参考 [思源笔记](https://github.com/siyuan-note/siyuan) 的内核分离架构与 [Obsidian](https://obsidian.md) 的纯 Markdown 存储，并整合了 [obsidian-virtual-tree](https://github.com/) 与 [siyuan-virtual-tree](https://github.com/) 两个虚拟文档树插件的功能，以及思源浏览器插件的剪藏/书签数据面。

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
- **编辑器**：CodeMirror 6，源码 + live-preview 装饰（wikilink 高亮、块 id 隐藏）、防抖自动保存、分屏渲染预览、Ctrl+S。
- **全文搜索**：SQLite FTS5，`<mark>` 高亮 snippet。
- **剪藏**：`/api/clip/clipUrl` — 内核侧拉取网页 → Readability 提取 → 转 Markdown → 建档（含 source/created/tags frontmatter）。
- **书签**：`/api/bookmark/*`，前端书签面板。
- **回收站**：删除文档/笔记本移入 `.markvault/trash/`，不直接删文件。

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

## 布局约定

```
<vault>/
├── .markvault/
│   ├── conf.json          # 端口/伺服/访问码/apiToken/parent 属性名等
│   ├── cache/index.db     # 可重建索引 (docs/blocks/links/FTS5)
│   ├── storage/           # vt.json(虚拟树)、bookmarks.json、local/*.json(UI状态)
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
- `/api/block|attr|outline` — getDocInfo/getBlockMd · getBlockAttrs/setBlockAttrs(frontmatter) · getDocOutline
- `/api/search/fullTextSearchBlock`、`/api/format/renderMd`
- `/api/vt/*` — getRelationTree/addRoot/removeRoot/getMountTree/setMountTree/setNodeOrder/settings
- `/api/clip/clipUrl|clipHtml`、`/api/bookmark/*`、`/api/network/forwardProxy`
- `/api/storage/getVal|setVal`、`/api/index/rebuild`、`GET /ws`（推送 filetree-changed/vt-changed/progress）

## 测试

```bash
cargo test                    # 内核单元测试（关系树/消歧/排序/frontmatter）
cd frontend && node e2e/check.mjs   # 无头 Edge E2E 冒烟（需内核已启动）
```

## 后续路线

- 内嵌浏览器（Tauri 多窗口方案，每页签一个 WebviewWindow + 内核书签/剪藏数据面）
- 块级引用（`^block-id` 已在索引中）、反链面板、关系图
- 发布分享（publish 服务）、多工作区切换

## 架构文档

见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 与 [docs/API.md](docs/API.md)。
