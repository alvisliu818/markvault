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
├── src/panels/         filetree / search / outline / bookmark / graph（力导向 canvas）
└── src/vtree/panel.ts  虚拟树面板（关系树/挂载树/拖拽排序/定位/聚焦/折叠持久化）

app/src-tauri/          Tauri v2 壳
├── src/main.rs         解析内核二进制（sidecar/dev target）→ 子进程拉起 → 等端口 → 外部 URL 窗口 → 退出杀子进程
└── src/browser.rs      内嵌浏览器多窗口（M6）：每页签一个 WebviewWindow（label browser-<tabId>），
                        6 个 browser_* IPC 命令（open/navigate/act/close/focus/list）+ 500ms 轮询
                        （url/title 变化 → 事件广播到主窗口 + 注入收藏/剪藏操作栏）；capabilities/main.json
                        为 remote 主窗口授权（allow-browser-*）
└── src/settings.rs     壳侧设置（M10）：pick_vault_dir（rfd 原生目录对话框）→ set_workspace 写
                        %APPDATA%/markvault/config.json；set_autostart 写 HKCU Run 注册表键；get_shell_conf
```

## 数据流

1. **写**：前端编辑 → 防抖 900ms → `POST /api/file/putFile` → vault.rs 写文件 → `ops::reindex_one` 重索引该文件 → WS 广播 `doc-reindexed`/`filetree-changed`。
2. **外部改**：notify-debouncer-full 监听 vault → 过滤 `.` 目录与 ignore 模式 → 对 .md reindex_one / unindex；**目录级事件（新建/删除/改名）走精确 prefix 重索引**（旧路径 `unindex("<dir>/")` 防兄弟目录误删，新路径 `reindex_prefix` 扫描重建）→ 广播。`.markvault/` 完全跳过避免自触发。注意 `vault.rel` 需容忍 debouncer-full 的 forward-slash verbatim 路径（`//?/E:/...`）——字符串级前缀回退在 vault.rs normalize_win。
3. **关系树**：`/api/vt/getRelationTree` 时从索引读全部 docs（frontmatter `parent`/`weight`）→ 解析 wikilink（路径精确 > 标题 > 基名）→ 构树（ancestors 环检测、分支复制）→ 兄弟消歧 → 排序（customOrder > weight desc > name asc）→ 返回树 JSON。结构不持久化，只持久化意图（roots/拖拽序/设置，`vt.json`）。
4. **剪藏**：`/api/clip/clipUrl` → reqwest 拉取 → dom_smoothie(Readability) → htmd → 拼 frontmatter（source/created/tags）→ `create_doc_internal` 复用建文档路径（写入+索引+广播）。
5. **内嵌浏览器（M6）**：SPA 浏览器面板 = 控制器（页签列表/地址栏/前进后退/剪藏/收藏/下载），壳 `browser.rs` = 窗口组管理。页签窗口 URL/title 由壳轮询并以 `browser:*` 事件推回主窗口；SPA 收到 url 变化即 `POST /api/history/add`（内核 10min 去重+5000 上限）。页面内操作（收藏/剪 HTML）由壳 eval 注入的操作栏完成——页面上下文 fetch 内核（本地 Host 信任放行 + CORS very_permissive），登录页 DOM 内核拿不到，这正是 `clipHtml` 存在的理由。
6. **关系图谱（M11）**：`/api/graph/getGraph` 返回 docs 全量节点 + links 表经 resolve_target 解析的 doc→doc 边（未解析目标跳过）；前端 panels/graph.ts 手写力导向（斥力/弹簧/中心引力/alpha 衰减 + 画布软边界），节点点击打开文档、拖拽钉住、滚轮缩放。hidden dock 页 canvas rect=0 → IntersectionObserver 首次可见才加载。
7. **快照历史（M11）**：`/api/repo/*` + `.markvault/repo/<sha256(path)16>/<millis>-<chash8>.md` 内容寻址存储（同内容去重、每文档 30 版上限）。putFile 写前自动留存旧内容；restore 前自动留存当前内容。前端「历史版本」对话框（文件树右键 / 编辑器工具行）列表+预览+恢复。plaintext MVP，加密未做。

## 设计决策与取舍

- **文件为真相，SQLite 是缓存**（仿思源 `temp/` 数据库）：`.markvault/cache/index.db` 删了就重建；启动时全量重建（vault 小时成本可接受，后续可改为增量校验 mtime）。
- **文档身份 = 相对路径**（Obsidian 风格），非思源的 `.sy`+ID。块 ID（`^id`）已入索引表但块级引用 API 留待后续。
- **关系树服务端计算**：两插件都在前端全量扫库；内核有索引后移到服务端，前端只渲染。与 siyuan-virtual-tree 的"关系树只读派生 + 拖拽序持久化"语义一致。
- **行级解析而非完整 AST**：索引用轻量行解析（frontmatter/标题/块id/wikilink），HTML 渲染用 comrak。权衡：解析快且可控；代价是嵌套块级结构（列表树）暂不作为块索引。
- **Tauri 而非 Electron**：内核分离架构下桌面壳只是"窗口 + 内核子进程管理"，Tauri 体积小；嵌入浏览器用多 WebviewWindow 方案（M6 路线）。
- **伺服安全**：默认绑 127.0.0.1；networkServe 绑 0.0.0.0 后由 accessAuthCode + HMAC cookie 保护；apiToken 走 `Authorization: Token`；`/vault/*` 静态路由拒绝任何点开头路径段（保护 `.markvault`）。

## 已知限制（MVP）

- 内核侧剪藏无法携带浏览器 cookie（登录页不可剪）——嵌入浏览器注入栏走 `clipHtml` 在页面上下文取 DOM 解决；但严格 CSP 站点（connect-src 收紧）会拦截注入栏的 fetch，回退面板「剪藏」（clipUrl）。
- networkServe 伺服模式下，浏览器窗口内注入栏 fetch 内核不带访问码 cookie（外部页面无登录态），剪藏/收藏会 401；本地模式不受影响。
- 注入操作栏的样式受页面 CSP style-src 影响（个别站点可能裸样式），功能不受影响。
- 索引启动全量重建已优化为 mtime 增量（index_meta），rebuild 强制全量。
- 移动端布局、同步/历史版本未实现。
- `.sy` 导入/导出未实现（思源用户需从 md 或 std markdown 迁移）。
