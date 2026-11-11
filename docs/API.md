# markvault 内核 API

统一约定：
- 除静态资源外均为 `POST`，`Content-Type: application/json`，空 body 视为全默认参数。
- 响应信封 `{code, msg, data}`；`code: 0` 成功，非 0 失败。
- 认证：`Authorization: Token <apiToken>`；伺服模式下未认证请求 401（API）或 302 → `/check-auth`（页面）。
- 所有 `path` 参数为 vault 相对路径（POSIX 斜杠）；`/` 开头会被归一化。

## system

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/system/version` | — | 内核版本 |
| `/api/system/health` | — | 健康检查（Tauri 等待用） |
| `/api/system/getConf` | — | 完整 conf + workspace 路径 |
| `/api/system/setConf` | `{patch:{port,networkServe,accessAuthCode,apiToken,readonly,theme,parentAttr,weightAttr}}` | 更新并落盘 conf.json |
| `/api/system/loginAuth` | `{authCode}` | 校验访问码，置 mv_session cookie |
| `/api/system/logoutAuth` | — | 清 cookie |
| `/api/system/exit` | — | 优雅退出内核 |

## notebook / filetree

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/notebook/lsNotebooks` | — | 顶层文件夹列表 + docCount |
| `/api/notebook/createNotebook` | `{name}` | 新建笔记本 |
| `/api/filetree/listDocsByPath` | `{notebook, path:"/sub"}` | 目录下 folder+doc 混合列表（folders-first 排序） |
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
| `/api/file/putFile` | `{path, data}` 或 `{path, dataBase64}` 或 `{path, isDir:true}` | 写文件/建目录；.md 触发重索引 |
| `/api/file/removeFile` | `{path}` | trash 回收站 |
| `/api/file/renameFile` | `{path, newPath}` | 重命名 + 重索引前缀 |
| `/api/file/readDir` | `{path}` | 目录项 `{entries:[{name,isDir,size,mtime}]}` |

## block / attr / outline / search / format

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/block/getDocInfo` | `{path}` | title/notebook/hpath/created/updated/size/attrs/wordCount |
| `/api/block/getBlockMd` | `{path, line?}` | 全文或单行 |
| `/api/block/insertBlock` | `{path, markdown, line?, autoId?}` | 行级插入（line=body 相对行，0=末尾；autoId 默认追加思源式 ` ^id`），返回 `{bodyLine}` |
| `/api/block/updateBlock` | `{path, line, markdown}` | 替换该行；自动保留尾部已有 ` ^id` |
| `/api/block/deleteBlock` | `{path, line}` | 删除该行 |
| `/api/block/getBlockBreadcrumb` | `{path, line}` | 回溯父级 heading 栈 `[{level,content,line}]` |
| `/api/attr/getBlockAttrs` | `{path}` | frontmatter map |
| `/api/attr/setBlockAttrs` | `{path, attrs:{}}` | 合并进 frontmatter（value=null 删除），写盘重索引 |
| `/api/outline/getDocOutline` | `{path}` | `[{blockId,type:"h1".."h6",content,line}]` |
| `/api/search/fullTextSearchBlock` | `{query, limit?}` | FTS5，返回带 `<mark>` snippet |
| `/api/ref/getBacklinks` | `{path}` | 反链图：`links`(正向)/`unresolved`/`backlinks`(反向)/`mentions`(伪反链) |
| `/api/format/renderMd` | `{md, path?, rewriteLinks?}` | comrak HTML；相对链接重写为 `/vault/...`，wikilink → `/#/doc/<target>`，`![[doc]]` 一级内联展开，`==高亮==`→`<mark>`，脚注 |
| `/api/format/md2Text` | `{md}` | 去语法纯文本 |

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
| `/api/clip/clipHtml` | `{html, url?, title?, notebook?, path?, tags?}` | 客户端供 HTML（预留嵌入浏览器剪藏） |
| `/api/bookmark/list` / `add{url,title}` / `remove{url}` | — | `.markvault/storage/bookmarks.json` |
| `/api/network/forwardProxy` | `{url, headers?, timeout?}` | 内核侧抓取 → `{status, contentType, bodyBase64, len}` |

## storage / index / ws

| 端点 | 参数 | 说明 |
|---|---|---|
| `/api/storage/getVal` / `setVal` | `{key}` / `{key, val}` | `.markvault/storage/local/<key>.json`（折叠状态等 UI 态） |
| `/api/index/rebuild` | — | 后台全量重建，WS 广播 `index-done` |
| `/api/export/exportMd` | `{path}` | 单篇 md 下载（`Content-Disposition: attachment`） |
| `/api/export/exportNotebookMd` | `{notebook}` | 整笔记本打 zip（STORE + UTF-8 名）写入 `.markvault/export/<名>.zip`，返回 `{path,count,bytes}` |

`GET /ws`：推送 `{cmd,code,msg,data}`；`cmd` ∈ `filetree-changed` / `doc-reindexed{path}` / `vt-changed` / `progress` / `index-done`。
