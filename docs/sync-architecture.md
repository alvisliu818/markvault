# markvault 同步架构（参考思源内核）

> 2026-11-11 · 承接 [TASKS.md](TASKS.md)、[livesync-readonly-recon.md](livesync-readonly-recon.md) 与 [ARCHITECTURE.md](ARCHITECTURE.md)。
> 参考源码：思源笔记 Go 内核 `kernel/`（本仓库之外，AGPL-3.0-or-later）。**本文档不含任何口令、恢复码、密钥、桶名或服务端地址。**

## 0. 结论先行

思源把同步做成四件事：**方向显式化、内容寻址仓库、三方合并、checkout 原子落盘**。markvault 可以完整借鉴前两件与「完整性校验/失败治理」，但**绝不能照搬 checkout 覆盖语义**——思源的工作区可被 repo 反向改写，markvault 是「Markdown 文件为唯一真相、SQLite 仅索引」。

一句话红线：**只读拉取阶段只重建索引 / 导出到独立目录，永不回写 `.md`**。

两条同步面要分开，不共用密钥与命名：

| 同步面 | 现状 | 目标 |
|---|---|---|
| 本地快照 `repo/` | v0.3 已交付，明文内容寻址、每文档 30 版 | 借鉴思源：同步点 `latest-sync`、保留策略、hash 校验、可选加密 |
| 外部 LiveSync | 侦察完成：MinIO/S3 journal sync、E2EE v2 分片已能解密到 JSONL | 先只读拉取（S1-S5），双向留到最后（S7） |

> **证据分级**：下文标【内核】的结论可在思源 `kernel/` 源码直接复核；标【外部库】的来自其依赖 `dejavu` / `encryption` 的上游实现（本地无 vendor 与 module cache 副本）；标【未确认】的不作为设计依据。

## 1. 思源同步骨架（提炼）

### 1.1 模块布局【内核】

- `kernel/model/repository.go`：同步总编排（快照/列举/对比/恢复/密钥/上下行/合并结果/repo 构造/事件订阅）
- `kernel/model/sync.go`：驱动与锁——`SyncDataDownload`(:48)、`SyncDataUpload`(:123)、`SyncData`(:272)、`BootSyncData`(:179)、`SyncDataJob`(:168)、`lockSync`(:304)、`IncSync`、`planSyncAfter`
- `kernel/model/sync_source.go` / `sync_checkout*.go` / `sync_perception.go` / `sync_ignore.go` / `sync_auto_pause.go` / `sync_dedup.go` / `sync_pending.go`：同步源预览、下行应用状态机、感知、忽略规则、失败退避、请求去重、待同步提示
- `kernel/conf/sync.go` / `conf/repo.go`：`Mode(1自动/2手动/3完全手动)`、`Interval(30s)`、`GenerateConflictDoc`、四种 Provider（官方云/S3/WebDAV/本地）；`IndexRetentionDays=180`、`RetentionIndexesDaily=2`、`SyncIndexTiming=12s`
- `kernel/api/router.go` 的 `/api/sync/*`、`/api/repo/*`；前端 `app/src/sync/syncGuide.ts`、`checkoutProgress.ts`
- 定时入口 `kernel/job/cron.go`（5s 轮询 + 30s 门控）；启动/退出同步；云端 WebSocket `cmd:"synced"` 感知

### 1.2 内容寻址仓库【外部库 dejavu，内核调用点佐证】

```
<workspace>/repo/
├── indexes/<40位 SHA-1>          # 每次 Index 生成一个快照索引
├── objects/<前2位>/<后38位>       # 按 hash 分桶的 chunk/file 对象
├── refs/latest                   # 最新索引
└── refs/latest-sync              # 【内核】本地同步点（三方合并的 base）
```

- `Index` → `File(path,size,updated,chunks[])` → `Chunk(data)`，所有 ID = 内容 hash（SHA-1）；大文件用 `restic/chunker` CDC 切块，只传差量 chunk。
- 变更检测默认靠 `size/mtime/stamp` + 本地状态缓存，`FullCheck` 才全量读盘；同步前 `indexRepoBeforeCloudSync0()` 比对前后 index ID，相同即跳过。
- 对象名即内容 hash → 重跑天然幂等（等效断点续传）；`GetChunks()` 返回云端缺失分块。
- 保留策略：`autoPurgeRepo()` 按日期分组，>180 天整批淘汰，历史每天最多 2 个，少于 3 个不清。

### 1.3 密钥与完整性【内核 + 外部库】

- **repo 层**：`Conf.Repo.Key` 为 32 字节 AES-256 密钥，随配置落盘；口令模式 salt = `sha256(passphrase) 十六进制前 16 字符`，`encryption.KDF` 用 **scrypt(N=32768,r=8,p=1,32B)**；另有随机密钥模式，须靠导出的 base64 key 恢复。
- **加密笔记本层**：`util/kdf.go` 用 **Argon2id(64MB,3 iter,4 线程,32B)**，`SENC` 信封 + AES-256-GCM，HKDF-SHA256 派生子密钥，路径/用途绑定 AAD。
- 校验：`hash(data)==对象ID`、下载 index 的 `ID` 一致性、`VerifyAESKey`、GCM tag、`checkChunks/FullCheck`。
- ⚠️ `util/crypt.go` 的 AES-CBC（硬编码 SK/IV）只是配置字段混淆，**不是 repo 加密**，不要误用。

### 1.4 方向与「只下载」语义【内核 + 外部库】

| 模式 | 入口 | 语义 |
|---|---|---|
| 只下载 | `SyncDataDownload`(:48) → `repo.SyncDownload` | **不发布本地变更、保留 pending 提示**；纯下载不发 `synced` 信号（`sync.go:77`, `:433`） |
| 只上传 | `SyncDataUpload`(:123) → `repo.SyncUpload` | 只上传 `refs/latest` |
| 双向 | `SyncData`(:272) → `repo.Sync` | 三方合并 |

- 拉取前 `getSyncSourceState` 预计算 `UploadFiles/OverwriteFiles/RemoveFiles`，前端弹「上传 / 下载」方向选择；`Source + LocalID + CloudID` 锁定预览版本防 TOCTOU。
- `DownloadCloudSnapshot` 更纯：只把云端索引下到本地 repo，**不 checkout 到工作区**。

### 1.5 避免覆盖本地【外部库 dejavu】

1. 三方合并：base = `refs/latest-sync`，local = `repo.Latest()`，cloud = 云端 `refs/latest`；仅 `Winner==cloud` 才写 `Upserts/Removes`，local winner 跳过。
2. 双方都变 → 生成**合并索引**而非采用云端索引；云端未初始化时按「云端=本地」处理，避免误判「删光」。
3. 传输后 recheck：应用前重新索引磁盘，若 index 变化则递归重跑合并，保留期间的编辑。
4. `syncignore` 规则过滤 `Removes`；冲突可生成副本 + 历史目录。
5. 原子落盘：临时文件 → `fsync` → 文件锁 → `rename` → `Chtimes` 恢复 mtime（一次只替换一个文件）。
6. 治理：`syncLock` + 请求合并去重 + 云端锁、连续失败 >7 次暂停 64 分钟、DNS 错误刷新重试、200ms 节流的 `syncProgress` 事件、`syncPending` 提示。

## 2. 对照 markvault：能学什么 / 不能学什么

| 维度 | 思源 | markvault | 决策 |
|---|---|---|---|
| 真相源 | repo 对象可 checkout **反向改写工作区** | `<vault>/**/*.md` 唯一真相，SQLite 仅缓存 | **红线**：只读拉取不写 `.md`；只重建索引或导出到独立目录 |
| 本地仓库 | 加密内容寻址 index/file/chunk + refs | 明文 `<pathHash>/<millis>-<chash8>.md`，30 版 | 可加 `latest-sync`、保留策略、hash 校验；加密另做，不与 LiveSync 混用 |
| 远端格式 | S3 上 `indexes/objects/refs` 对象 | MinIO 上 `<millis>-docs.jsonl.gz` 加密分片 + journal 参数 | 增量语义按 **journal 水位**，不是索引引用图 |
| 加密/KDF | scrypt / Argon2id + HKDF 子密钥 + AAD | LiveSync PBKDF2(310k) + HKDF + AES-GCM，两层 E2EE | 各自独立实现，**不能拿 dejavu Store 读 LiveSync 数据** |
| 分块/压缩 | `restic/chunker` CDC + zstd | `v3-rabin-karp` + raw deflate | chunk 边界/ID 不互认，去重逻辑重写 |
| 冲突 UI | 多窗口 `confirmSyncCheckout`、15s 超时、token、phase 状态机 | 单实例 SPA + 只读优先 | 简化为「单锁 + 预览确认 + 进度」 |
| 生态 | 订阅门禁、官方云配额、WebSocket 感知、mDNS P2P | 自建/第三方对象存储 | 全部不引入 |

**可借鉴 5 条**：①方向显式化 + 拉取前预览清单；②「只下载不改变本地真相」的语义边界与持久化同步点；③内容寻址/幂等增量（对象名即 hash、缺失分块差量）；④完整性校验（hash==ID、GCM tag、校验失败即拒收）；⑤失败治理（互斥锁、请求去重、退避暂停、进度事件）。

**不照搬 5 条**：①checkout 覆盖工作区；②dejavu 的 AESKey/KDF/对象命名直接套用到 LiveSync；③restic chunker 与 zstd 假设；④多窗口 checkout 阻断状态机；⑤订阅/配额/官方云/WS 感知/P2P/引用回收 GC。

## 3. markvault 同步分层（落地设计）

```
L0 真相与索引   <vault>/**/*.md + docs/blocks/links/FTS5 + watcher          【已有】
L1 本地快照     .markvault/repo/  + 同步点 .markvault/sync/<src>/latest-sync 【部分已有】
L2 远端适配     S3 SigV4：LIST/GET only；Provider 抽象（S3 → 后续 WebDAV）   【侦察已验证】
L3 解码         分片 E2EE v2 → raw deflate → JSONL → 文档层 E2EE v2
                → ID key 路径还原 → rabin-karp chunk 合并                   【S1/S2】
L4 应用策略     预览(加入/覆盖/删除) → 显式确认 → 导出独立目录 / 重建索引
                默认不写 .md；同步点记录已应用分片水位                       【S3/S4】
L5 状态治理     单锁 + 幂等跳过 + 退避 + WS 进度 + 校验失败拒收               【S5】
L6 双向(远期)   三方合并 + 冲突副本 + 原子落盘 + 忽略规则                    【S7】
```

### 3.1 只读阶段的接口契约（草案，对齐思源 `/api/sync/*` 形态）

| 端点 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `POST /api/sync/getSourceState` | — | `{source, localId, cloudId, adds, overwrites, removes, bytes, checkedAt}` | 拉取前预览；`source` 为凭据/目标的指纹（不回传凭据），`cloudId` 为远端最新分片水位 |
| `POST /api/sync/planPull` | `{direction:"download", source, localId, cloudId}` | `{planId, token, items[]}` | 锁定预览版本防 TOCTOU；只生成计划，不动文件 |
| `POST /api/sync/applyPull` | `{planId, token, target:"export"\|"index", exportDir?}` | `{applied, skipped, conflicts}` | `export` 写到独立目录；`index` 只重建 SQLite 索引 |
| `POST /api/sync/getStatus` | — | `{phase, current, total, lastResult, paused}` | `phase`: idle/preparing/applying/done，`lastResult` 不持久化「进行中」 |
| `GET /ws` | — | `sync-progress` / `sync-done` | 进度节流推送（对齐现有 progress 机制） |

**设置项**（对应思源 `conf/sync.go` / `conf/repo.go`）：`sync.enabled`、`sync.mode(manual/auto)`、`sync.interval`、`sync.conflictDoc`、`repo.retentionDays`、`repo.retentionPerDay`；凭据只从环境变量或本机配置读取，**不落 `.markvault/conf.json` 明文、不入库**。

### 3.2 应用策略的三条硬约束

1. **默认不覆盖**：`applyPull` 的 `target` 默认 `export`，输出到 `.markvault/export/livesync-<时间戳>/`；`target:"index"` 只更新索引，不写 Markdown。
2. **同步点**：`.markvault/sync/<source>/latest-sync` 记录已应用的最后一个分片时间戳/ID；重跑跳过已应用分片（幂等），对应思源 `refs/latest-sync`。
3. **校验失败即拒收**：分片 GCM tag、JSONL 结构、文档 hash、chunk ID 任一不通过则整批中止并广播错误，不做部分应用；对应思源 `ErrRepoFatal` / `VerifyAESKey` 的处理。

## 4. 实现顺序（写入 TASKS.md 的 S1-S7）

| 阶段 | 内容 | 依赖 |
|---|---|---|
| **S1** | JSONL 文档类型解析、按 `_id/_rev` 取最新、**文档层 E2EE v2 解密**，端到端读出 1 篇笔记 | 侦察报告的 A 阶段 |
| **S2** | ID 派生密钥还原路径/文档 ID、`v3-rabin-karp` chunk 按 `children` 合并成完整 `.md` | S1 |
| **S3** | `getSourceState` / `planPull` 预览契约 + `latest-sync` 同步点（纯只读） | S2 |
| **S4** | `applyPull` 的 `export` / `index` 两种应用目标 + 原子写（临时文件 + rename） | S3 |
| **S5** | 增量水位（按时间戳只拉新分片）、单锁、幂等跳过、退避、WS 进度 | S4 |
| **S6** | 本地 `repo/` 加密（借鉴 scrypt KDF + 密钥校验值）与 180 天/每日保留策略 | 独立，可与 S1-S5 并行 |
| **S7** | 双向同步：三方合并、冲突副本、`syncignore`、原子落盘 | S1-S6 + 明确的冲突策略评审 |

> S1-S5 全程只读远端；**S7 之前不实现任何 PUT/DELETE**。

## 5. 参考与证据

- 思源内核（本地可复核）：`kernel/conf/sync.go`、`kernel/conf/repo.go`、`kernel/model/sync.go`、`kernel/model/repository.go`、`kernel/model/sync_source.go`、`kernel/model/sync_checkout*.go`、`kernel/model/sync_auto_pause.go`、`kernel/util/kdf.go`、`kernel/api/router.go`、`app/src/sync/syncGuide.ts`
- 外部依赖（上游在线实现，本地无副本）：`github.com/siyuan-note/dejavu`（Index/File/Chunk/Ref、cloud.Cloud 接口、三方合并）、`github.com/siyuan-note/encryption`（scrypt KDF、AES-GCM）
- 【未确认】dejavu `Store` 对 index/file 对象逐字段加密范围、`SyncDownload/SyncUpload` 在库内定义位置、S3 multipart 显式断点续传、是否存在独立签名——均不作为本方案依据
- markvault 侧：[livesync-readonly-recon.md](livesync-readonly-recon.md)（已验证解密链路）、[ARCHITECTURE.md](ARCHITECTURE.md)（数据流与已知限制）、[TASKS.md](TASKS.md)（任务状态）
