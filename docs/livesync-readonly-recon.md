# LiveSync 只读拉取 · 技术侦察报告

> 2026-11-11 · 承接 [TASKS.md](TASKS.md) 的「LiveSync 对接」与 v0.3 后续路线。
> **本文档不含任何口令、恢复码、访问密钥、桶名或服务端地址**；这些信息只在本机配置/会话内使用，不入库（仓库为公开远程仓库）。

## 1. 结论摘要

之前任务清单里「CouchDB 客户端」的假设**不成立**：现有 LiveSync 库用的是 **MinIO/S3 兼容对象存储 + journal sync**，不是 CouchDB。只读拉取应面向对象存储分片，而不是 CouchDB `_changes` / `_all_docs`。

用户提供的两样凭据已完成用途验证（值不落盘）：

| 凭据 | 实际用途 | 验证结果 |
|---|---|---|
| 口令 | **E2EE 数据口令**（v2 分片加密） | ✅ 成功解密一个真实 journal 分片（PBKDF2 31 万次 → HKDF → AES-GCM-256 → raw deflate → JSONL） |
| ID 派生恢复码（v1 格式） | **ID 派生密钥**（`idDerivationVersion: 1`） | 已确认字段用途；路径/文档 ID 还原阶段需要它（本轮未用到） |

> 该口令**解不开**配置层的 `encryptedPassphrase` / `encryptedIdDerivationKey` / `encryptedCouchDBConnection` / 加密 Setup URI——配置层由另一把 `configPassphrase` 保护。只读拉取不需要这些，直接用数据口令即可。

## 2. 运行环境（本机实测）

| 项 | 值 |
|---|---|
| 插件 | Self-hosted LiveSync **1.0.47**（本机 Obsidian 主库插件） |
| 后端 | `remoteType = MINIO`，S3 兼容对象存储（SigV4） |
| 加密 | `encrypt = true`、`E2EEAlgorithm = v2`、`usePathObfuscation = true` |
| 分块 | `hashAlg = xxhash64`、`chunkSplitterVersion = v3-rabin-karp`、`chunkSplitterVersion` 对应去重分块 |
| ID 派生 | `idDerivationVersion = 1`，ID key 加密存于 `encryptedIdDerivationKey` |
| 同步协议 | `protocolVersion = 2`（ADVANCED_E2EE），`pbkdf2salt` 存于桶内 `_obsidian_livesync_journal_sync_parameters.json` |

桶内对象（只读 LIST 实测约 168 个）：

- `<毫秒时间戳>-docs.jsonl.gz`：加密的 journal 分片（**后缀 .gz 但实际不是 gzip**）
- `_obsidian_livesync_journal_sync_parameters.json`：`{_id, type:"sync-parameters", protocolVersion:2, pbkdf2salt(base64 32B)}`
- `_00000000-milestone.json` 等检查点文件

## 3. 已验证的解密链路（第 1~6 步）

来源：当前插件 `main.js` 的 `encryptForUpload()` → `encryptBinary()` → `_encrypt()` / `deriveKey()` / `deriveMasterKey()`，以及 `decryptDataV2()`。

1. **S3 只读 LIST**：AWS SigV4（`AWS4-HMAC-SHA256`，region 取自配置）列出 `list-type=2` 对象，分页 `continuation-token`。本机需走本地代理。
2. **取参数**：GET `_obsidian_livesync_journal_sync_parameters.json` → `pbkdf2salt`（base64，32 字节）。
3. **分片布局**：`iv(12B) | hkdfSalt(32B) | AES-GCM 密文(含 16B tag)`。注意与配置层 `%$` 加密串不同——后者把 `pbkdf2Salt` 也放在载荷头部。
4. **派生主密钥**：`PBKDF2(passphrase_utf8, pbkdf2Salt, 310000, SHA-256)` → AES-GCM 256 位 key（导出 raw）→ 作为 **HKDF** key material 导入。
5. **派生分片密钥**：`HKDF(salt = hkdfSalt, info = 空, hash = SHA-256)` → AES-GCM 256 位 key。
6. **解密 + 解压**：`AES-GCM.decrypt(iv, tagLength=128)` → **raw deflate**（`inflateRaw`，不是 gzip/zlib/brotli）→ JSONL 文本。实测 71,292 字节分片 → 71,232 字节密文 → 205,705 字节 JSONL，约 280 行，文档 ID 形如 `~h:+<hex>` / `h:` / `i:` 前缀。

> 验证全程：只读 LIST/GET，未写入、未删除任何远端对象；解密只在内存进行，未把笔记明文写入磁盘；未输出任何笔记正文。

## 4. 只读拉取 MVP 的剩余工作

| 阶段 | 内容 | 状态 |
|---|---|---|
| A | S3 SigV4 列对象 / 下载分片 / PBKDF2+HKDF+AES-GCM / raw deflate → JSONL | ✅ 已验证，已落 Rust 内核 `/api/sync/*`（S1） |
| B | 解析 JSONL 文档类型（`plain` / `leaf` / `chunkpack` / `note` / `versioninfo` 等），按 `_id`/`_rev` 取最新版 | 待做 |
| C | **文档层 E2EE v2 解密**：`decryptDataV2()` 对应实现（分片层与文档层是两层加密） | 待做 |
| D | **路径混淆还原 + chunk 合并**：用 ID 派生密钥还原文档 ID/路径，`v3-rabin-karp` 分块按 `children` 还原为完整 .md | 待做 |
| E | 集成形态定案：① 独立 CLI 导出到 vault 目录；② 内核 `/api/sync/*` 只读端点；③ 前端只读浏览面板 | 方案已出，见 [sync-architecture.md](sync-architecture.md)（默认内核 API + 导出独立目录，CLI 作薄封装）；仍待确认 |
| F | 增量策略：按时间戳只拉新分片 + milestone 检查点，避免每次全量 168 个对象 | 待做 |

**建议**：先做 B + C（把一个分片里的笔记标题/正文解出来，端到端跑通 1 篇），再定 E 的集成形态；D 的 chunk 合并可以放在「单篇可读」之后。

### S1 实机结果（2026-11-11，内核 API 形态）

集成形态已定为**内核 API**：`crates/kernel/src/api/sync.rs` 新增 `/api/sync/getStatus | getSourceState | planPull | applyPull`，全部只读远端；`ring` 提供 PBKDF2/HKDF/AES-256-GCM，`flate2` 处理 raw deflate，SigV4 用已有 hmac/sha2/hex 手写。单测 4 个（加解密 round-trip、`%=` 字段层、deflate、LIST XML），内核单测总数 26 → 30。

实机（测试库 + 环境变量）：`getSourceState` 返回 176 对象 / 174 分片 / 最新分片名；`planPull(limit=1)` 成功解密最新分片 → 65 行 / 44 个唯一文档 / 路径与 size 明文可读；`applyPull(export)` 只写 manifest 并推进同步点，未写 `.md`。

**S2 关键线索**：合并 166 个分片后共 6784 个文档，`_id` 全部为 `f:` 前缀、`type` 仅 `plain`/`newnote`，`children` 指向 `h:+` chunk 但 **chunk 文档未出现在分片中**，`eden` 为空；最新分片 65 行里有 **21 行不是 JSON**（`parseErrors`）——chunk/内联数据很可能藏在这些非 JSON 行或另一种记录编码里。S2 应先定位这 21 行的格式，再按 `children` 拼接。另：这些分片合计约 775 MB，增量水位（只拉新分片）比全量下载更重要。

## 5. 工程注意

- **绝不入库**：口令、恢复码、accessKey/secretKey、endpoint、bucket 名。实现时应从环境变量或本机 Obsidian 插件配置读取，仓库内只放格式与算法说明。
- **只读纪律**：MVP 阶段只允许 `GET`/`LIST`，不要实现 `PUT`/`DELETE`，避免误写生产同步库。
- **凭据有效性**：口令已通过真实分片解密验证；配置层口令未提供，也不需要。
- **本机代理**：访问对象存储需经本机本地 HTTP 代理（与 git push 相同），实现时从环境读取、不写死。
- 配置参考：本机另有 11-03 的旧版插件配置备份，字段与当前 1.0.47 有差异，逆向时以当前主库插件为准。
