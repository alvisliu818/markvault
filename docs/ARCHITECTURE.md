# markvault 架构

## 模块布局

```
crates/kernel/          Rust 内核（单 bin）
├── main.rs             CLI 入口（clap）：serve --workspace [--port] [--network-serve] [--readonly]
├── serve.rs            axum 启动、静态托管、文件监听、优雅关闭
├── state.rs            AppState（conf RwLock + db Mutex + vault）、忽略匹配
├── conf.rs             .markvault/conf.json 读写
├── vault.rs            路径安全解析、frontmatter 提取（LF/CRLF）、trash
├── parse.rs            行级 markdown 解析：标题/块id/wikilink/纯文本
├── id.rs               思源式 ID（14位时间戳-7位随机）
├── ws.rs               WebSocket hub（broadcast，{cmd,code,msg,data}）
├── auth.rs             apiToken + accessAuthCode 会话（HMAC cookie）+ 中间件
├── index/
│   ├── mod.rs          SQLite schema（docs/blocks/links/doc_fts）
│   ├── builder.rs      单文档 upsert / 全量重建
│   └── search.rs       FTS5 查询 + snippet
├── api/
│   ├── mod.rs          路由组装 + {code,msg,data} 信封
│   ├── system.rs notebook.rs filetree.rs file.rs block.rs search.rs format.rs storage.rs
│   ├── vt.rs           虚拟树 API
│   ├── clip.rs         剪藏（reqwest + dom_smoothie + htmd）
│   ├── bookmark.rs     书签
│   └── network.rs      forwardProxy（内核侧抓取，base64 返回）
└── vt.rs               虚拟文档树核心（关系树构建/三级排序/挂载树持久化/重命名删除同步）

frontend/               Vite + TS SPA（无框架）
├── src/api.ts          内核客户端 + WS 重连
├── src/layout/app.ts   布局壳（dock/中央/右侧面板、拖拽调宽、路由）
├── src/tabs.ts         编辑器页签管理（防抖保存、预览、属性编辑）
├── src/editor/cm.ts    CM6 装配（oneDark、wikilink 装饰、块id 隐藏、点击导航）
├── src/panels/         filetree / search / outline / bookmark
└── src/vtree/panel.ts  虚拟树面板（关系树/挂载树/拖拽排序/定位/聚焦/折叠持久化）

app/src-tauri/          Tauri v2 壳
└── src/main.rs         解析内核二进制（sidecar/dev target）→ 子进程拉起 → 等端口 → 外部 URL 窗口 → 退出杀子进程
```

## 数据流

1. **写**：前端编辑 → 防抖 900ms → `POST /api/file/putFile` → vault.rs 写文件 → `ops::reindex_one` 重索引该文件 → WS 广播 `doc-reindexed`/`filetree-changed`。
2. **外部改**：notify-debouncer-full 监听 vault → 过滤 `.` 目录与 ignore 模式 → 对 .md reindex_one / unindex → 广播。`.markvault/` 完全跳过避免自触发。
3. **关系树**：`/api/vt/getRelationTree` 时从索引读全部 docs（frontmatter `parent`/`weight`）→ 解析 wikilink（路径精确 > 标题 > 基名）→ 构树（ancestors 环检测、分支复制）→ 兄弟消歧 → 排序（customOrder > weight desc > name asc）→ 返回树 JSON。结构不持久化，只持久化意图（roots/拖拽序/设置，`vt.json`）。
4. **剪藏**：`/api/clip/clipUrl` → reqwest 拉取 → dom_smoothie(Readability) → htmd → 拼 frontmatter（source/created/tags）→ `create_doc_internal` 复用建文档路径（写入+索引+广播）。

## 设计决策与取舍

- **文件为真相，SQLite 是缓存**（仿思源 `temp/` 数据库）：`.markvault/cache/index.db` 删了就重建；启动时全量重建（vault 小时成本可接受，后续可改为增量校验 mtime）。
- **文档身份 = 相对路径**（Obsidian 风格），非思源的 `.sy`+ID。块 ID（`^id`）已入索引表但块级引用 API 留待后续。
- **关系树服务端计算**：两插件都在前端全量扫库；内核有索引后移到服务端，前端只渲染。与 siyuan-virtual-tree 的"关系树只读派生 + 拖拽序持久化"语义一致。
- **行级解析而非完整 AST**：索引用轻量行解析（frontmatter/标题/块id/wikilink），HTML 渲染用 comrak。权衡：解析快且可控；代价是嵌套块级结构（列表树）暂不作为块索引。
- **Tauri 而非 Electron**：内核分离架构下桌面壳只是"窗口 + 内核子进程管理"，Tauri 体积小；嵌入浏览器用多 WebviewWindow 方案（M6 路线）。
- **伺服安全**：默认绑 127.0.0.1；networkServe 绑 0.0.0.0 后由 accessAuthCode + HMAC cookie 保护；apiToken 走 `Authorization: Token`；`/vault/*` 静态路由拒绝任何点开头路径段（保护 `.markvault`）。

## 已知限制（MVP）

- 内核侧剪藏无法携带浏览器 cookie（登录页不可剪）——`clipHtml` 端点为嵌入浏览器剪藏预留。
- 索引启动全量重建（大库可优化为 mtime 校验）。
- 移动端布局、多窗口、同步/历史版本未实现。
- `.sy` 导入/导出未实现（思源用户需从 md 或 std markdown 迁移）。
