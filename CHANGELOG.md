# Changelog

## 0.3.0 — 2026-11-12

**M6 内嵌浏览器 / M7 块级引用 / M8 虚拟树补全 / M9 编辑器体验 / M10 壳设置 / M11 插件系统+快照+图谱 / S1-S7 LiveSync 只读拉取 / 0.3.0 打包**

### 内嵌浏览器（M6）
- Tauri 多窗口页签（每页签一个 WebviewWindow），地址栏/前进后退/刷新/停止/主页
- 页内注入操作栏（收藏/剪藏，严格 CSP 站点回退面板 clipUrl）
- 面板：页签列表、历史记录（10min 去重、5000 上限）、剪藏/收藏/下载到 vault
- 页签窗口销毁不再误杀内核（仅 main 窗口触发退出）

### 块级引用（M7）
- `[[doc#^block-id]]` 引用语法端到端（解析/渲染/跳转）
- 块引用反链面板（`/api/ref/*`）

### 虚拟树补全（M8）
- 跨父移动（`vtmove.rs`）、物理子树合并、排序方式切换 UI
- filetree 右键挂载联动、快捷键

### 编辑器与写作（M9）
- live-preview：标题行语义装饰、`==高亮==`、wikilink 未解析虚线样式
- frontmatter 属性面板化、面包屑导航
- 图片粘贴 → notebook assets；[[doc#锚点]] 定位
- 笔记本导出 zip（`/api/export/*`）

### 插件系统（M11 petal）
- 内核 petal 托管（`.markvault/petal/<name>/index.js` 静态托管 + 注册表 + conf 存储）
- SPA 动态加载器：onload/onunload、`host.addCommand` 命令菜单、`host.ui`（toast/menu/prompt/confirm）、`host.api` 全量内核访问
- 设置面板插件启停

### 快照与图谱（M11）
- 文件级快照历史 `repo/`（内容寻址，30 版/文档；`/api/history/*`）+ 可选快照加密（`/api/repo/initKey`，口令派生）
- 关系图谱面板（canvas 力导向，`/api/graph/*`）

### 多工作区（M11）
- 壳内实例池：多 vault 同时打开、切换/关闭、`ws_list/ws_switch/ws_close`；canonicalize 路径归一修复

### LiveSync 只读拉取（S1-S7）
- `/api/sync/getStatus|getSourceState|planPull|applyPull`：MinIO/S3 journal sync 分片解密（PBKDF2+HKDF+AES-256-GCM+raw deflate）
- chunk（US 分隔符行格式）跨分片合并 → 完整 `.md` 还原；路径混淆还原（ID 恢复码可选）
- 增量水位（`latest-sync`）、后台任务 + 进度轮询（`getPlan` 含 `apply` 状态）
- 受控导入：只新增不覆盖、partial/不安全路径跳过、批量 reindex
- 全量首拉实测：174 分片 / 775MB / 32,620 chunks / 6,851 篇 100% 覆盖
- **红线保持**：远端只 GET/LIST，无 PUT/DELETE；凭据仅环境变量，不入库不入日志

### 打包（M10）
- **NSIS 安装包发布**：`markvault_0.3.0_x64-setup.exe`（6.97 MiB，sidecar 内核捆绑），静默安装/卸载实测通过
- tauri-cli v2.12.4（官方源安装，杀软排除 proc-macro DLL 后）
- 同步 UI polish：阶段中文化进度条、apply 轮询修复（getPlan.apply）、成功/失败着色
- 壳内点验：17 自动项 + shell_manual 11 项（C2/B10/B7）+ manual7 8 项全绿，**仅剩 C1 人工**（docs/v0.3-shell-verification.md）
- watcher 抑制闸（bulk_apply）：LiveSync 导入写入期间停机，消除双重索引与 WS 风暴

## 0.2.0 — 2026-11-11

- M6 内嵌浏览器首版、M7/M8 主体、M9 编辑器体验批次、M10 壳设置区
- 详见 docs/v0.2-iteration-report.html

## 0.1.0 — 2026-11-11

- 首个可用版本：Rust 内核（axum + SQLite/FTS5 + comrak + notify）、Vite+CM6 前端、
  虚拟文档树（关系树/挂载树/三级排序）、剪藏+书签、Tauri 壳、伺服 + accessAuthCode/apiToken
- 内核单测 9/9、E2E 14/14（当时口径）
