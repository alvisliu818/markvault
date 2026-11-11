# markvault 内核 API

统一约定：
- 除静态资源外均为 `POST`，`Content-Type: application/json`，空 body 视为全默认参数（非法 JSON 也会退化成默认，并在内核日志留下 warn）。
- 响应信封 `{code, msg, data}`；`code: 0` 成功，非 0 失败；`readonly` 模式下写操作返回 `code: -1 / readonly mode`。
- 认证：`Authorization: Token <apiToken>`；伺服模式下未认证请求 401（API）或 302 → `/check-auth`（页面）。
- 所有 `path` 参数为 vault 相对路径（POSIX 斜杠）；`/` 开头会被归一化。
- `GET /petal/<name>/...` 为插件只读静态托管，不套 `{code,msg,data}` 信封。
- v0.3 新增的 history / asset / graph / repo / petal 五组端点见文末专节。

## system

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/system/version` | — | 内核版本（`CARGO_PKG_VERSION`） |
| `/api/system/health` | — | 健康检查（Tauri 等待用） |
| `/api/system/getConf` | — | `{conf, workspace}`；`conf` 为 **camelCase** 视图（kernelVersion / port / networkServe / accessAuthCode / apiToken / readonly / parentAttr / weightAttr / ignorePatterns / theme），磁盘 `conf.json` 仍是 snake_case |
| `/api/system/setConf` | `{patch:{port,networkServe,accessAuthCode,apiToken,readonly,theme,parentAttr,weightAttr}}` | 按 camelCase key 打补丁并落盘 conf.json |
| `/api/system/loginAuth` | `{auth_code}` | 校验访问码，置 mv_session cookie（该结构体未做 camelCase 重命名） |
| `/api/system/logoutAuth` | — | 清 cookie |
| `/api/system/exit` | — | 优雅退出内核 |

## notebook / filetree

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/notebook/lsNotebooks` | — | 顶层文件夹列表 + docCount |
| `/api/notebook/createNotebook` | `{name}` | 新建笔记本 |
| `/api/filetree/listDocsByPath` | `{notebook, path:"/sub"}` | 目录下 folder+doc 混合列表（folders-first 排序）；folder 的 `subFileCount` = **直接子文档数**（v0.3 起，非全递归） |
| `/api/filetree/listDocTree` | — | 全库文档平铺 |
| `/api/filetree/createDocWithMd` | `{notebook, path:"/目录/标题", markdown}` | 建档（markdown 空 → H1 模板），返回 `{path}` |
| `/api/filetree/renameDoc` | `{path, title}` | 重命名文件 + 同步虚拟树 customOrder/mount |
| `/api/filetree/removeDoc` | `{path}` | 移入 `.markvault/trash/` |
| `/api/filetree/moveDocs` | `{fromPaths:[], toPath:"/目标目录"}` | 批量移动 |
| `/api/filetree/searchDocs` | `{k}` | 标题/路径 LIKE 搜索 |

## file（原始文件读写）

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/file/getFile` | `{path}` | 返回原始字节（.md 为 text/markdown） |
| `/api/file/putFile` | `{path, data}` 或 `{path, dataBase64}` 或 `{path, isDir:true}` | 写文件/建目录；.md 触发重索引，且 **写前自动留一份旧内容快照**（v0.3） |
| `/api/file/removeFile` | `{path}` | trash 回收站 |
| `/api/file/renameFile` | `{path, newPath}` | 重命名 + 重索引前缀 |
| `/api/file/readDir` | `{path}` | 目录项 `{entries:[{name,isDir,size,mtime}]}`，已过滤点开头项 |

## block / attr / outline / search / format

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/block/getDocInfo` | `{path}` | title/notebook/hpath/created/updated/size/attrs/wordCount |
| `/api/block/getBlockMd` | `{path, line?}` | 全文或单行（文件绝对行） |
| `/api/block/insertBlock` | `{path, markdown, line?, autoId?}` | 行级插入（line=body 相对行，0=末尾；autoId 默认追加思源式 ` ^id`），返回 `{bodyLine}` |
| `/api/block/updateBlock` | `{path, line, markdown}` | 替换该行；自动保留尾部已有 ` ^id` |
| `/api/block/deleteBlock` | `{path, line}` | 删除该行 |
| `/api/block/getBlockBreadcrumb` | `{path, line}` | 回溯父级 heading 栈 `[{level,content,line}]` |
| `/api/block/getBlockInfo` | `{path, blockId}` | 按块 ID 定位（不带 `^`），返回 `{path,blockId,type,text,line}` |
| `/api/attr/getBlockAttrs` | `{path}` | frontmatter map |
| `/api/attr/setBlockAttrs` | `{path, attrs:{}}` | 合并进 frontmatter（value=null 删除），写盘重索引 |
| `/api/outline/getDocOutline` | `{path}` | `[{blockId,type:"h1".."h6",content,line}]` |
| `/api/search/fullTextSearchBlock` | `{query, limit?}` | FTS5，返回带 `<mark>` snippet |
| `/api/ref/getBacklinks` | `{path}` | 反链图：`links`(正向)/`unresolved`/`backlinks`(文档级反向)/`blockBacklinks`(锚 `^id` 的块引用)/`mentions`(伪反链) |
| `/api/format/renderMd` | `{md, path?, rewriteLinks?}` | comrak HTML；相对链接重写为 `/vault/...`，wikilink → `/#/doc/<target>`，`![[doc]]` 一级内联展开，`[[doc#^id]]` 渲染为「目标 › 块文本」，`==高亮==`→`<mark>`，脚注 |
| `/api/format/md2Text` | `{md}` | 去语法纯文本 |

> 行号语义：`insertBlock/updateBlock/deleteBlock/getBlockBreadcrumb` 的 `line` 是 **body 相对行**，响应 `bodyLine` 与 `getBlockMd(line)` 的 **文件绝对行** 不同，两者相差 frontmatter 行数。

## vt（虚拟文档树）

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/vt/getRelationTree` | — | `{roots, tree, settings}`；无显式根时自动推导 |
| `/api/vt/addRoot` / `removeRoot` | `{path}` | 根管理 |
| `/api/vt/getMountTree` / `setMountTree` | `{mountRoots}` | 挂载树整存整取（前端负责编辑） |
| `/api/vt/setNodeOrder` | `{parentKey, orderedPaths}` | 拖拽序；顶层用 `parentKey:"__root__"` |
| `/api/vt/getSettings` / `setSettings` | `{settings}` | sortMethod("weight"默认/"name")、caseSensitive、defaultExpandLevel、includePhysicalSubtree |
| `/api/vt/moveNode` | `{path, newParent}` | 跨父移动：改写 frontmatter parent；newParent 空 = detach（移除 parent 属性） |

VtNode 结构：`{id(祖先链"::"连接), path, name(消歧后), weight, isRoot, children[]}`。
MountItem：`{uid, kind:"file"|"doc"|"notebook", name, path, children[]}`。

## clip / bookmark / network

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/clip/clipUrl` | `{url, notebook?, path?, tags?}` | 内核拉取 → Readability → markdown → 建档；返回 `{path,title}` |
| `/api/clip/clipHtml` | `{html, url?, title?, notebook?, path?, tags?}` | 客户端供 HTML（内嵌浏览器注入栏在页面上下文取 DOM 后调用），用于内核 fetch 不到/无 cookie 的登录页 |
| `/api/bookmark/list` / `add{url,title}` / `remove{url}` | — | `.markvault/storage/bookmarks.json` |
| `/api/network/forwardProxy` | `{url, headers?, timeout?}` | 内核侧抓取 → `{status, contentType, bodyBase64, len}` |

## history / asset（v0.3 · 内嵌浏览器数据面）

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/history/list` | — | `HistoryEntry[]`（`{url,title,created}`），最新在前；存 `.markvault/storage/browser-history.json` |
| `/api/history/add` | `{url, title}` | 10 分钟内同 URL 原地更新、新条目插到头部、超 5000 条截尾；返回更新后的列表 |
| `/api/history/remove` | `{url}` | 删除该 URL，返回更新后的列表 |
| `/api/history/clear` | — | 清空，返回 `[]` |
| `/api/asset/downloadToLocal` | `{url, notebook, filename}` | 内核 fetch → `<notebook>/assets/<name>`；文件名 = 显式 `filename` > URL basename 百分号解码 > `asset-<时间戳>`，非法字符过滤 + `-2/-3` 防覆盖；返回 `{path,filename,len}`；`notebook` 空取第一个笔记本 |

## graph / repo / petal（v0.3 · M11）

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/graph/getGraph` | — | `{nodes:[{id,title,notebook}], links:[{source,target}]}`；links 经 `resolve_target` 解析，跳过未解析/自环/重复边，孤立文档也在 nodes 中 |
| `/api/repo/createSnapshot` | `{path}` | 强制建快照，返回 `{id}`；内容未变时 `{id:null,msg:"unchanged"}` |
| `/api/repo/listSnapshots` | `{path}` | `{snapshots:[{id,created,hash}]}`，按时间倒序 |
| `/api/repo/getSnapshot` | `{path, id}` | `{content}` 快照正文（UTF-8 lossy） |
| `/api/repo/restoreSnapshot` | `{path, id}` | **先快照当前内容**再覆写，`.md` 触发重索引 + 广播；返回 `{path}` |
| `/api/repo/cleanSnapshots` | `{path}` | 删该文档的快照目录；`path` 为空则清整个 repo（前端未暴露空 path 入口） |
| `/api/petal/loadPetals` | — | `{petals:[{name,enabled}]}`，读 `.markvault/petal/petals.json` |
| `/api/petal/setPetals` | `{petals:[{name,enabled}]}` | 归一化 + 插件名白名单 `[a-z0-9-]`（≤64、不以 `-` 开头）校验后写回 |
| `/api/petal/getPetalConf` | `{name}` | 读 `.markvault/petal/<name>/conf.json`，不存在返回 `null` |
| `/api/petal/setPetalConf` | `{name, conf}` | 写该插件 conf.json |
| `GET /petal/<name>/index.js` | — | ServeDir 只读托管；SPA 用 `import("/petal/<name>/index.js")` 动态加载 |

快照存储：`.markvault/repo/<sha256(path)[0..16]>/<millis>-<sha256(content)[0..8]>.md`，内容寻址去重、每文档 30 版上限，**明文 MVP 未加密**；读取时拒绝 `/ \ ..` 防目录穿越。

## storage / index / export / ws

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/storage/getVal` / `setVal` | `{key}` / `{key, val}` | `.markvault/storage/local/<key>.json`（折叠状态等 UI 态） |
| `/api/index/rebuild` | — | 后台全量重建（重建中重复调用返回 -1），每 20 篇上报 WS progress，结束广播 `index-done` |
| `/api/export/exportMd` | `{path}` | 单篇 md 下载（`Content-Disposition: attachment`） |
| `/api/export/exportNotebookMd` | `{notebook}` | 整笔记本打 zip（STORE + UTF-8 名）写入 `.markvault/export/<名>.zip`，返回 `{path,count,bytes}` |

`GET /ws`：推送 `{cmd,code,msg,data}`；`cmd` ∈ `filetree-changed` / `doc-reindexed{path}` / `vt-changed` / `progress` / `index-done`。
外部文件改动由 watcher 监听：.md 走 `reindex_one`/`unindex`，目录级事件走精确 prefix 重索引（`unindex("<dir>/")` + `reindex_prefix`），`.markvault/` 完全跳过避免自触发。

## sync（v0.3+ · LiveSync 只读拉取，S1 阶段）

凭据**只从环境变量读取**（`MARKVAULT_SYNC_ENDPOINT / BUCKET / REGION / ACCESS_KEY / SECRET_KEY / PASSPHRASE / ID_KEY`），不落 `conf.json`、不写入响应与日志。整个模块对远端只发 `GET`/`LIST`，**没有 PUT/DELETE**。

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/sync/getStatus` | — | `{configured, phase, lastResult, lastError, lastSync, idKeyPresent}`；`lastResult` 0 未知 / 1 成功 / 2 失败 |
| `/api/sync/getSourceState` | — | SigV4 只读列对象：`{source, remoteObjects, shards, bytes, latestShard, localSyncPoint, readonly:true}` |
| `/api/sync/planPull` | `{limit?: 1..20}` | 下载最新 N 个分片 → PBKDF2/HKDF/AES-GCM 解密 → raw deflate → JSONL 解析；返回 `{planId, token, shards, lines, uniqueIds, parseErrors, docs[], note}`。`docs` 为元数据预览（id/type/path/size/children/dataPrefix），**不写任何文件** |
| `/api/sync/getPlan` | `{planId}` | 后台任务的进度轮询：`{phase, progress, progressTotal, error, done, shards, skippedShards, lines, parseErrors, docsTotal, docsComplete, docsPartial, docsUncovered, chunks, docsPreview[≤50]}` |
| `/api/sync/applyPull` | `{planId, token, target:"export"\|"index"}` | 校验 plan/token 且 phase=done；`export` 按 `children` 拼接正文写出真实 `.md` 到 `.markvault/export/livesync-<时间戳>/`（防穿越过滤），推进同步点并清理临时 chunk；`index` 返回「后续阶段」。**不覆盖 vault** |

> S3 起行为：`planPull` 是**后台任务**，立即返回 `{planId, token, phase:"preparing"}`，进度轮询 `getPlan`；默认**增量水位**（只拉 `latest-sync` 之后的分片，`skippedShards` 记录跳过数），`{full:true}` 强制全量；解密后的 chunk 写入内容寻址临时目录（`.markvault/tmp/livesync-<id>/chunks/`），applyPull 成功后清理。单分片只覆盖部分笔记，跨分片合并靠 plan 聚合多个分片的 chunk。

> journal chunk 行格式：`~h:+<64hex>` + `\u{1f}` 分隔符 + 内容（分片层解密后为明文，未加密字段原样返回）。详见 [livesync-readonly-recon.md](livesync-readonly-recon.md) 与 [sync-architecture.md](sync-architecture.md)。

## Tauri 壳 IPC（非 HTTP，v0.3）

桌面前端经 `window.__TAURI__` 调用（`withGlobalTauri` + `capabilities/main.json` 的 `allow-*` 授权，remote 主窗口必需）：

| 命令 | 参数 | 说明 |
|---|---|---|
| `browser_open` / `browser_navigate` / `browser_act` / `browser_close` / `browser_focus` / `browser_list` | `tabId,url` / `action` | 每页签一个 WebviewWindow；`action` ∈ back/forward/reload/stop；500ms 轮询 emit `browser:opened|url|closed` |
| `pick_vault_dir` / `set_workspace` / `set_autostart` / `get_shell_conf` | `path` / `enable` | 原生目录对话框、写 `%APPDATA%/markvault/config.json`、HKCU Run 开机自启 |
| `ws_list` / `ws_switch` / `ws_close` | `path` | 多工作区实例池（≤3 LRU）；切换 = 主窗口 navigate 到目标内核 URL，同步注入栏 kernel_origin |
