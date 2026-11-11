//! LiveSync read-only pull (S1-S3).
//!
//! The remote is a MinIO/S3-compatible **journal sync** store, not CouchDB:
//! `<millis>-docs.jsonl.gz` shards are AES-256-GCM encrypted (E2EE v2), raw-deflate
//! compressed JSONL. See `docs/livesync-readonly-recon.md` and
//! `docs/sync-architecture.md` for the format and the L2-L4 design.
//!
//! Discipline (see sync-architecture.md §3.2):
//! * **Only GET / LIST** — no PUT/DELETE is implemented anywhere in this module.
//! * Credentials come from environment variables only; they are never written to
//!   `conf.json`, logs, responses or this repository.
//! * `applyPull` writes reconstructed `.md` bodies into an isolated export
//!   directory under `.markvault/export/` — never into the vault itself.
//!
//! Stage status:
//! * S1 — shard decryption + deflate + JSONL parsing (verified against a real remote).
//! * S2 — journal chunk rows (`~h:+<64hex>\u{1f}<body>`, plaintext after the shard
//!   layer) are parsed and `applyPull(export)` writes real `.md` files.
//! * S3 — cross-shard merge + incremental watermark: `planPull` runs as a
//!   background job (progress via `getPlan`), pulls only shards newer than the
//!   `latest-sync` watermark unless `full` is requested, and stores decrypted
//!   chunks in a content-addressed temp dir that is removed on apply.

use super::{parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chrono::Utc;
use flate2::read::DeflateDecoder;
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use regex::Regex;
use ring::{aead, hkdf, pbkdf2};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

const PBKDF2_ITERATIONS: u32 = 310_000;
const SYNC_PARAMS_KEY: &str = "_obsidian_livesync_journal_sync_parameters.json";
const SHARD_SUFFIX: &str = "-docs.jsonl.gz";
const HKDF_FIELD_PREFIX: &str = "%=";
const TMP_TTL_HOURS: u64 = 24;

// ---------------------------------------------------------------- config

#[derive(Clone)]
struct SyncConfig {
    endpoint: String,
    host: String,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    passphrase: String,
    #[allow(dead_code)] // S3+: ID derivation key for chunk-id / path verification
    id_key: String,
}

fn env_required(key: &str) -> Result<String, String> {
    std::env::var(key).map_err(|_| format!("missing env {}", key))
}

impl SyncConfig {
    fn from_env() -> Result<Self, String> {
        let endpoint = env_required("MARKVAULT_SYNC_ENDPOINT")?
            .trim_end_matches('/')
            .to_string();
        let host = endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        if host.is_empty() {
            return Err("MARKVAULT_SYNC_ENDPOINT must be an http(s) URL".into());
        }
        Ok(Self {
            endpoint,
            host,
            bucket: env_required("MARKVAULT_SYNC_BUCKET")?,
            region: env_required("MARKVAULT_SYNC_REGION")?,
            access_key: env_required("MARKVAULT_SYNC_ACCESS_KEY")?,
            secret_key: env_required("MARKVAULT_SYNC_SECRET_KEY")?,
            passphrase: env_required("MARKVAULT_SYNC_PASSPHRASE")?,
            id_key: std::env::var("MARKVAULT_SYNC_ID_KEY").unwrap_or_default(),
        })
    }
}

// ---------------------------------------------------------------- crypto

struct KeyLen32;
impl hkdf::KeyType for KeyLen32 {
    fn len(&self) -> usize {
        32
    }
}

fn hmac_sha256(key: &[u8], msg: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac accepts any key length");
    m.update(msg.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// PBKDF2-HMAC-SHA256(310k) -> HKDF-SHA256(salt = hkdfSalt, info = empty) -> AES-256 key.
fn derive_aes_key(passphrase: &str, pbkdf2_salt: &[u8], hkdf_salt: &[u8]) -> Result<[u8; 32], String> {
    let mut master = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PBKDF2_ITERATIONS).unwrap(),
        pbkdf2_salt,
        passphrase.as_bytes(),
        &mut master,
    );
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, hkdf_salt);
    let prk = salt.extract(&master);
    let info: &[&[u8]] = &[];
    let mut out = [0u8; 32];
    prk.expand(info, KeyLen32)
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| "HKDF expand failed".to_string())?;
    Ok(out)
}

/// Shared primitive for both the shard layer (raw bytes) and the document
/// layer (`%=` + base64). Layout: iv(12) | hkdfSalt(32) | AES-GCM ct+tag(16).
fn decrypt_blob(blob: &[u8], passphrase: &str, pbkdf2_salt: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < 12 + 32 + 16 {
        return Err("encrypted blob too short".into());
    }
    let iv = &blob[..12];
    let hkdf_salt = &blob[12..44];
    let ct = &blob[44..];
    let key = derive_aes_key(passphrase, pbkdf2_salt, hkdf_salt)?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key)
        .map_err(|_| "AES key init failed".to_string())?;
    let key = aead::LessSafeKey::new(unbound);
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(iv);
    let mut buf = ct.to_vec();
    let len = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::empty(),
            &mut buf,
        )
        .map_err(|_| "AES-GCM auth failed (wrong passphrase or corrupt shard)")?
        .len();
    buf.truncate(len);
    Ok(buf)
}

/// Document-layer field decryption. Unencrypted fields pass through unchanged.
fn decrypt_field(value: &str, passphrase: &str, pbkdf2_salt: &[u8]) -> Result<String, String> {
    if !value.starts_with(HKDF_FIELD_PREFIX) {
        return Ok(value.to_string());
    }
    let raw = B64
        .decode(&value[HKDF_FIELD_PREFIX.len()..])
        .map_err(|e| format!("field base64: {}", e))?;
    let plain = decrypt_blob(&raw, passphrase, pbkdf2_salt)?;
    String::from_utf8(plain).map_err(|e| format!("field utf8: {}", e))
}

fn inflate_raw(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = DeflateDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("raw deflate: {}", e))?;
    Ok(out)
}

// ---------------------------------------------------------------- S3 (read only)

fn s3_authorization(cfg: &SyncConfig, path: &str, query: &str) -> (String, String, String, String) {
    let now = Utc::now();
    let amz = now.format("%Y%m%dT%H%M%SZ").to_string();
    let ds = now.format("%Y%m%d").to_string();
    let payload = sha256_hex(b"");
    let signed = "host;x-amz-content-sha256;x-amz-date";
    let canonical = format!(
        "GET\n{}\n{}\nhost:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n\n{}\n{}",
        path, query, cfg.host, payload, amz, signed, payload
    );
    let scope = format!("{}/{}/s3/aws4_request", ds, cfg.region);
    let sts = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz,
        scope,
        sha256_hex(canonical.as_bytes())
    );
    let k = hmac_sha256(format!("AWS4{}", cfg.secret_key).as_bytes(), &ds);
    let k = hmac_sha256(&k, &cfg.region);
    let k = hmac_sha256(&k, "s3");
    let k = hmac_sha256(&k, "aws4_request");
    let sig = hex::encode(hmac_sha256(&k, &sts));
    let auth = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        cfg.access_key, scope, signed, sig
    );
    (auth, amz, ds, payload)
}

/// Only `GET` is ever issued. `query` must already be URL-encoded exactly as it
/// appears in the canonical request.
async fn s3_get(cfg: &SyncConfig, path: &str, query: &str) -> Result<Bytes, String> {
    let (auth, amz, _ds, payload) = s3_authorization(cfg, path, query);
    let url = if query.is_empty() {
        format!("{}{}", cfg.endpoint, path)
    } else {
        format!("{}{}?{}", cfg.endpoint, path, query)
    };
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Host", &cfg.host)
        .header("X-Amz-Date", amz)
        .header("X-Amz-Content-Sha256", payload)
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|e| format!("S3 request failed: {}", e))?;
    let status = resp.status();
    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("S3 body read failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("S3 GET {} -> HTTP {}", path, status.as_u16()));
    }
    Ok(body)
}

fn parse_list_xml(xml: &str) -> (Vec<(String, u64)>, Option<String>, bool) {
    let key_re = Regex::new(r"<Key>([^<]+)</Key>").unwrap();
    let size_re = Regex::new(r"<Size>(\d+)</Size>").unwrap();
    let token_re = Regex::new(r"<NextContinuationToken>([^<]+)</NextContinuationToken>").unwrap();
    let keys: Vec<String> = key_re
        .captures_iter(xml)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect();
    let sizes: Vec<u64> = size_re
        .captures_iter(xml)
        .filter_map(|c| c.get(1).and_then(|m| m.as_str().parse().ok()))
        .collect();
    let pairs = keys
        .into_iter()
        .enumerate()
        .map(|(i, k)| (k, sizes.get(i).copied().unwrap_or(0)))
        .collect();
    let token = token_re
        .captures(xml)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()));
    let truncated = xml.to_ascii_lowercase().contains("<istruncated>true</istruncated>");
    (pairs, token, truncated)
}

async fn list_objects(cfg: &SyncConfig) -> Result<Vec<(String, u64)>, String> {
    let path = format!("/{}", cfg.bucket);
    let mut all = Vec::new();
    let mut marker: Option<String> = None;
    for _ in 0..10 {
        let query = match &marker {
            Some(t) => format!(
                "list-type=2&max-keys=1000&continuation-token={}",
                utf8_percent_encode(t, NON_ALPHANUMERIC)
            ),
            None => "list-type=2&max-keys=1000".to_string(),
        };
        let body = s3_get(cfg, &path, &query).await?;
        let xml = String::from_utf8_lossy(&body);
        let (mut entries, token, truncated) = parse_list_xml(&xml);
        all.append(&mut entries);
        if !truncated || token.is_none() {
            break;
        }
        marker = token;
    }
    Ok(all)
}

async fn fetch_pbkdf2_salt(cfg: &SyncConfig) -> Result<Vec<u8>, String> {
    let path = format!("/{}/{}", cfg.bucket, SYNC_PARAMS_KEY);
    let body = s3_get(cfg, &path, "").await?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| format!("sync-parameters json: {}", e))?;
    let s = v
        .get("pbkdf2salt")
        .and_then(Value::as_str)
        .ok_or("sync-parameters.pbkdf2salt missing")?;
    let salt = B64.decode(s).map_err(|e| format!("pbkdf2salt base64: {}", e))?;
    if salt.len() != 32 {
        return Err(format!("pbkdf2salt must be 32 bytes, got {}", salt.len()));
    }
    Ok(salt)
}

// ---------------------------------------------------------------- state / plans

#[derive(Clone, serde::Serialize)]
struct SyncStatus {
    phase: String,
    last_result: i32,
    last_error: String,
    last_sync: u64,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            last_result: 0,
            last_error: String::new(),
            last_sync: 0,
        }
    }
}

fn status() -> &'static Mutex<SyncStatus> {
    static S: OnceLock<Mutex<SyncStatus>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(SyncStatus::default()))
}

fn set_status(phase: &str, result: i32, error: &str) {
    if let Ok(mut s) = status().lock() {
        s.phase = phase.to_string();
        s.last_result = result;
        s.last_error = error.to_string();
        if result == 1 {
            s.last_sync = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
        }
    }
}

/// A pull plan. Long-running jobs stay in this map and are driven by a
/// background task; clients poll `getPlan` for phase/progress.
struct Plan {
    plan_id: String,
    token: String,
    mode: String, // "incremental" | "full"
    phase: String,
    progress: usize,
    progress_total: usize,
    error: Option<String>,
    done: bool,
    shards: Vec<String>,
    skipped_shards: usize,
    /// main docs deduped by `_id` (latest shard wins), after filtering deleted
    docs: Vec<Value>,
    /// decrypted chunk bodies written to content-addressed files: id -> file
    chunks: HashMap<String, PathBuf>,
    tmp_dir: Option<PathBuf>,
    lines: usize,
    parse_errors: usize,
    docs_complete: usize,
    docs_partial: usize,
    docs_uncovered: usize,
    pbkdf2_salt: Option<Vec<u8>>,
    apply: Option<ApplyState>,
    created: u64,
}

impl Plan {
    fn new(plan_id: String, token: String, mode: String) -> Self {
        Self {
            plan_id,
            token,
            mode,
            phase: "preparing".into(),
            progress: 0,
            progress_total: 0,
            error: None,
            done: false,
            shards: Vec::new(),
            skipped_shards: 0,
            docs: Vec::new(),
            chunks: HashMap::new(),
            tmp_dir: None,
            lines: 0,
            parse_errors: 0,
            docs_complete: 0,
            docs_partial: 0,
            docs_uncovered: 0,
            pbkdf2_salt: None,
            apply: None,
            created: Utc::now().timestamp() as u64,
        }
    }
}

type SharedPlan = Arc<Mutex<Plan>>;

/// Background apply (export/import) state — surfaced via getPlan.
#[derive(Clone, serde::Serialize)]
struct ApplyState {
    target: String,
    notebook: String,
    include_partial: bool,
    phase: String, // writing|reindexing|done|error
    progress: usize,
    total: usize,
    imported: usize,
    partial: usize,
    skipped_existing: usize,
    skipped_partial: usize,
    skipped_empty: usize,
    skipped_unsafe: usize,
    errors: usize,
    body_bytes_total: usize,
    export_dir: Option<String>,
    error: Option<String>,
    done: bool,
}

fn plans() -> &'static Mutex<HashMap<String, SharedPlan>> {
    static P: OnceLock<Mutex<HashMap<String, SharedPlan>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sync_point_path(st: &AppState) -> PathBuf {
    st.workspace.join(".markvault/sync/livesync/latest-sync")
}

fn read_sync_point(st: &AppState) -> String {
    std::fs::read_to_string(sync_point_path(st)).unwrap_or_default()
}

fn shard_millis(key: &str) -> Option<u64> {
    key.split('-').next()?.parse().ok()
}

/// Remove leftover temp dirs from plans that were never applied (older than TTL).
fn cleanup_old_tmp(st: &AppState) {
    let tmp_root = st.workspace.join(".markvault/tmp");
    let Ok(entries) = std::fs::read_dir(&tmp_root) else {
        return;
    };
    let ttl = std::time::Duration::from_secs(TMP_TTL_HOURS * 3600);
    for entry in entries.flatten() {
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .map(|e| e > ttl)
            .unwrap_or(false);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn write_chunk_file(tmp_dir: &std::path::Path, id: &str, body: &str) -> Result<PathBuf, String> {
    let hex = id
        .strip_prefix("h:+")
        .ok_or_else(|| format!("unexpected chunk id prefix: {}", &id[..id.len().min(6)]))?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("unexpected chunk id length: {}", hex.len()));
    }
    let dir = tmp_dir.join("chunks").join(&hex[..2]);
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir chunk: {}", e))?;
    let file = dir.join(hex);
    std::fs::write(&file, body).map_err(|e| format!("write chunk: {}", e))?;
    Ok(file)
}

// ---------------------------------------------------------------- handlers

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/getStatus", post(get_status))
        .route("/getSourceState", post(get_source_state))
        .route("/planPull", post(plan_pull))
        .route("/getPlan", post(get_plan))
        .route("/applyPull", post(apply_pull))
}

async fn get_status(State(_st): State<Arc<AppState>>, _bytes: Bytes) -> ApiResp {
    let s = status().lock().map(|s| s.clone()).unwrap_or_default();
    let configured = SyncConfig::from_env().is_ok();
    ApiResp::ok(json!({
        "configured": configured,
        "phase": s.phase,
        "lastResult": s.last_result,
        "lastError": s.last_error,
        "lastSync": s.last_sync,
        "idKeyPresent": std::env::var("MARKVAULT_SYNC_ID_KEY").is_ok(),
    }))
}

/// Read-only source preview (mirrors SiYuan's `getSyncSourceState`).
async fn get_source_state(State(st): State<Arc<AppState>>, _bytes: Bytes) -> ApiResp {
    set_status("preparing", 0, "");
    let cfg = match SyncConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            set_status("idle", 2, &e);
            return ApiResp::err(-1, &e);
        }
    };
    match list_objects(&cfg).await {
        Ok(all) => {
            let mut shards: Vec<(String, u64)> = all
                .iter()
                .filter(|(k, _)| k.ends_with(SHARD_SUFFIX))
                .cloned()
                .collect();
            shards.sort_by(|a, b| a.0.cmp(&b.0));
            let bytes: u64 = shards.iter().map(|(_, n)| *n).sum();
            let latest = shards.last().map(|(k, _)| k.clone()).unwrap_or_default();
            set_status("idle", 1, "");
            ApiResp::ok(json!({
                "source": "livesync-s3",
                "remoteObjects": all.len(),
                "shards": shards.len(),
                "bytes": bytes,
                "latestShard": latest,
                "localSyncPoint": read_sync_point(&st),
                "readonly": true,
            }))
        }
        Err(e) => {
            set_status("idle", 2, &e);
            ApiResp::err(-1, &e)
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PlanReq {
    /// max shards to pull; 0 = no limit
    #[serde(default)]
    limit: usize,
    /// ignore the local watermark and pull everything
    #[serde(default)]
    full: bool,
}

/// Journal chunk row: `~h:+<64hex>` + US (`\u{1f}`) separator + body.
/// Returns (`h:+<64hex>`, body). The body may be plaintext or a `%=`-prefixed
/// encrypted field — callers decide via `decrypt_field`.
fn parse_chunk_row(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("~h:+")?;
    let hex_end = rest.find(|c: char| !c.is_ascii_hexdigit())?;
    if hex_end == 0 {
        return None;
    }
    if &rest[hex_end..hex_end + 1] != "\u{1f}" {
        return None;
    }
    Some((format!("h:+{}", &rest[..hex_end]), rest[hex_end + 1..].to_string()))
}

fn is_main_doc(t: &str) -> bool {
    matches!(t, "plain" | "newnote" | "notes")
}

/// Start a background pull job and return its plan id immediately.
/// Progress is polled via `getPlan`; applyPull requires phase=done.
async fn plan_pull(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: PlanReq = parse(&bytes);
    let cfg = match SyncConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            set_status("idle", 2, &e);
            return ApiResp::err(-1, &e);
        }
    };
    let plan_id = format!("mv-{}", Utc::now().timestamp_millis());
    let token = format!("{:016x}", rand::random::<u64>());
    let mode = if req.full { "full" } else { "incremental" }.to_string();
    let plan: SharedPlan = Arc::new(Mutex::new(Plan::new(plan_id.clone(), token.clone(), mode.clone())));
    if let Ok(mut p) = plans().lock() {
        p.insert(plan_id.clone(), plan.clone());
        if p.len() > 8 {
            let oldest = p
                .iter()
                .min_by_key(|(_, v)| v.lock().unwrap().created)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                p.remove(&k);
            }
        }
    }
    set_status("preparing", 0, "");
    let job = tokio::spawn(run_pull_job(st.clone(), cfg, plan, req.limit, req.full));
    // keep the JoinHandle alive but detached; failures are recorded in the plan
    tokio::spawn(async move {
        let _ = job.await;
    });
    ApiResp::ok(json!({
        "planId": plan_id,
        "token": token,
        "mode": mode,
        "phase": "preparing",
        "note": "background job started; poll getPlan until phase=done, then applyPull(target=export)",
    }))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GetPlanReq {
    #[serde(default)]
    plan_id: String,
}

/// Poll a plan's phase / progress / per-doc coverage preview.
async fn get_plan(State(_st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: GetPlanReq = parse(&bytes);
    let plan = plans()
        .lock()
        .ok()
        .and_then(|p| p.get(&req.plan_id).cloned());
    let Some(plan) = plan else {
        return ApiResp::err(-1, "unknown or expired planId");
    };
    ApiResp::ok(plan_summary(&req.plan_id, &plan))
}

/// getPlan response assembly (kept public-visible fields in one place;
/// `apply` must be present or import-progress polling reads None).
fn plan_summary(plan_id: &str, plan: &SharedPlan) -> Value {
    let inner = plan.lock().unwrap();
    let preview: Vec<Value> = inner
        .docs
        .iter()
        .take(50)
        .map(|d| {
            json!({
                "id": d.get("_id").and_then(Value::as_str).unwrap_or(""),
                "type": d.get("type").and_then(Value::as_str).unwrap_or(""),
                "path": d.get("path").and_then(Value::as_str).unwrap_or(""),
                "size": d.get("size").and_then(Value::as_u64).unwrap_or(0),
                "children": d.get("children").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0),
                "chunksHit": d.get("chunksHit").and_then(Value::as_u64).unwrap_or(0),
                "chunksMissing": d.get("chunksMissing").and_then(Value::as_u64).unwrap_or(0),
                "bodyBytes": d.get("bodyBytes").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect();
    json!({
        "planId": plan_id,
        "mode": inner.mode,
        "phase": inner.phase,
        "progress": inner.progress,
        "progressTotal": inner.progress_total,
        "error": inner.error,
        "done": inner.done,
        "shards": inner.shards.len(),
        "skippedShards": inner.skipped_shards,
        "lines": inner.lines,
        "parseErrors": inner.parse_errors,
        "docsTotal": inner.docs.len(),
        "docsComplete": inner.docs_complete,
        "docsPartial": inner.docs_partial,
        "docsUncovered": inner.docs_uncovered,
        "chunks": inner.chunks.len(),
        "docsPreview": preview,
        "apply": inner.apply.clone(),
    })
}

async fn run_pull_job(st: Arc<AppState>, cfg: SyncConfig, plan: SharedPlan, limit: usize, full: bool) {
    let result = async {
        set_phase(&plan, "listing", None);
        let pbk = fetch_pbkdf2_salt(&cfg).await?;
        plan.lock().unwrap().pbkdf2_salt = Some(pbk.clone());
        let all = list_objects(&cfg).await?;
        let mut shards: Vec<String> = all
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| k.ends_with(SHARD_SUFFIX))
            .collect();
        shards.sort();
        // incremental watermark: only shards newer than latest-sync
        if !full {
            let latest = read_sync_point(&st);
            let from = shard_millis(&latest);
            if let Some(from) = from {
                let before = shards.len();
                shards.retain(|k| shard_millis(k).map(|m| m > from).unwrap_or(false));
                plan.lock().unwrap().skipped_shards = before - shards.len();
            }
        }
        // newest-first limit (0 = unlimited)
        if limit > 0 && shards.len() > limit {
            let cut = shards.split_off(shards.len() - limit);
            plan.lock().unwrap().skipped_shards += shards.len();
            shards = cut;
        }
        {
            let mut p = plan.lock().unwrap();
            p.shards = shards.clone();
            p.progress_total = shards.len();
            p.phase = "downloading".into();
        }
        // temp dir for content-addressed chunks
        cleanup_old_tmp(&st);
        let tmp_dir = st.workspace.join(".markvault/tmp").join(format!(
            "livesync-{}",
            Utc::now().timestamp_millis()
        ));
        std::fs::create_dir_all(tmp_dir.join("chunks"))
            .map_err(|e| format!("mkdir tmp: {}", e))?;
        plan.lock().unwrap().tmp_dir = Some(tmp_dir.clone());

        // concurrent downloads (4), sequential parsing afterwards in shard order
        set_phase(&plan, "downloading", None);
        let mut jobs = Vec::new();
        for key in &shards {
            let cfg2 = cfg.clone();
            let key2 = key.clone();
            let pbk2 = pbk.clone();
            jobs.push(tokio::spawn(async move {
                let path = format!("/{}/{}", cfg2.bucket, key2);
                let raw = s3_get(&cfg2, &path, "").await?;
                let plain = decrypt_blob(&raw, &cfg2.passphrase, &pbk2)?;
                let text = inflate_raw(&plain)?;
                String::from_utf8(text).map_err(|_| "shard is not UTF-8 JSONL".to_string())
            }));
        }
        let mut shard_texts: Vec<(String, String)> = Vec::new();
        for (i, job) in jobs.into_iter().enumerate() {
            let res = job
                .await
                .map_err(|e| format!("download task: {}", e))?
                .map_err(|e| e)?;
            shard_texts.push((shards[i].clone(), res));
            if let Ok(mut p) = plan.lock() {
                p.progress = i + 1;
            }
            push_plan_progress(&plan);
        }
        shard_texts.sort_by(|a, b| a.0.cmp(&b.0)); // apply latest shard last

        set_phase(&plan, "parsing", None);
        let mut raw_docs: HashMap<String, Value> = HashMap::new();
        let mut chunks: HashMap<String, PathBuf> = HashMap::new();
        let mut lines_total = 0usize;
        let mut parse_errors = 0usize;
        for (_key, text) in &shard_texts {
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                lines_total += 1;
                match serde_json::from_str::<Value>(line) {
                    Ok(v) => {
                        if let Some(id) = v.get("_id").and_then(Value::as_str) {
                            raw_docs.insert(id.to_string(), v);
                        }
                    }
                    Err(_) => match parse_chunk_row(line) {
                        Some((id, body)) => {
                            match write_chunk_file(&tmp_dir, &id, &body) {
                                Ok(file) => {
                                    chunks.insert(id, file);
                                }
                                Err(_) => parse_errors += 1,
                            }
                        }
                        None => parse_errors += 1,
                    },
                }
            }
        }
        // dedupe main docs (latest shard wins) and compute coverage
        let mut docs: Vec<Value> = Vec::new();
        let mut docs_complete = 0usize;
        let mut docs_partial = 0usize;
        let mut docs_uncovered = 0usize;
        for v in raw_docs.values() {
            let t = v.get("type").and_then(Value::as_str).unwrap_or("");
            if !is_main_doc(t) {
                continue;
            }
            let deleted = v
                .get("_deleted")
                .or_else(|| v.get("deleted"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if deleted {
                continue;
            }
            let children = v
                .get("children")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut hit = 0usize;
            let mut body_bytes = 0usize;
            for c in &children {
                let cid = c.as_str().unwrap_or("");
                if let Some(f) = chunks.get(cid) {
                    if let Ok(meta) = std::fs::metadata(f) {
                        hit += 1;
                        body_bytes += meta.len() as usize;
                    }
                }
            }
            let missing = children.len() - hit;
            if children.is_empty() || missing == 0 {
                docs_complete += 1;
            } else if hit > 0 {
                docs_partial += 1;
            } else {
                docs_uncovered += 1;
            }
            docs.push(json!({
                "id": v.get("_id").and_then(Value::as_str).unwrap_or(""),
                "type": t,
                "path": v.get("path").and_then(Value::as_str).unwrap_or(""),
                "size": v.get("size").and_then(Value::as_u64).unwrap_or(0),
                "children": children,
                "childrenCount": children.len(),
                "chunksHit": hit,
                "chunksMissing": missing,
                "bodyBytes": body_bytes,
            }));
        }
        docs.sort_by(|a, b| {
            a.get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(b.get("path").and_then(Value::as_str).unwrap_or(""))
        });
        {
            let mut p = plan.lock().unwrap();
            p.docs = docs;
            p.chunks = chunks;
            p.tmp_dir = Some(tmp_dir);
            p.lines = lines_total;
            p.parse_errors = parse_errors;
            p.docs_complete = docs_complete;
            p.docs_partial = docs_partial;
            p.docs_uncovered = docs_uncovered;
            p.phase = "done".into();
            p.done = true;
        }
        Ok::<_, String>(())
    }
    .await;
    match result {
        Ok(()) => {
            set_phase(&plan, "done", None);
            plan.lock().unwrap().done = true;
        }
        Err(e) => {
            set_phase(&plan, "error", Some(e.clone()));
            plan.lock().unwrap().done = true;
            set_status("idle", 2, &e);
        }
    }
}

fn set_phase(plan: &SharedPlan, phase: &str, error: Option<String>) {
    let (plan_id, progress, progress_total) = {
        let mut p = plan.lock().unwrap();
        p.phase = phase.to_string();
        if let Some(e) = error {
            p.error = Some(e);
        }
        (p.plan_id.clone(), p.progress, p.progress_total)
    };
    crate::ws::broadcast(
        "sync-progress",
        json!({"planId": plan_id, "phase": phase, "progress": progress, "progressTotal": progress_total}),
    );
}

fn push_plan_progress(plan: &SharedPlan) {
    if let Ok(p) = plan.lock() {
        crate::ws::broadcast(
            "sync-progress",
            json!({
                "planId": p.plan_id, "phase": p.phase,
                "progress": p.progress, "progressTotal": p.progress_total,
            }),
        );
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApplyReq {
    #[serde(default)]
    plan_id: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    target: String,
    /// import only: notebook under the vault that receives the notes
    #[serde(default)]
    notebook: String,
    /// import only: also import notes whose chunks are only partially covered
    #[serde(default)]
    include_partial: bool,
}

/// Sanitize a remote path into a safe relative export path.
/// Rejects traversal, absolute paths and drive prefixes; strips the
/// LiveSync namespace prefixes (`i:` / `ix:` / `ps:`) component-wise.
fn safe_relative_path(raw: &str) -> Option<PathBuf> {
    let norm = raw.replace('\\', "/");
    let trimmed = norm.trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in trimmed.split('/') {
        let comp = comp.trim();
        if comp.is_empty() || comp == "." || comp.contains("..") {
            return None;
        }
        let comp = comp
            .strip_prefix("ix:")
            .or_else(|| comp.strip_prefix("i:"))
            .or_else(|| comp.strip_prefix("ps:"))
            .unwrap_or(comp);
        if comp.is_empty() {
            return None;
        }
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Apply a finished plan. Long runs (full import = thousands of notes) run as a
/// background job; progress goes to `getPlan` (apply.*) and WS `sync-progress`.
async fn apply_pull(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: ApplyReq = parse(&bytes);
    let plan = plans()
        .lock()
        .ok()
        .and_then(|p| p.get(&req.plan_id).cloned());
    let Some(plan) = plan else {
        return ApiResp::err(-1, "unknown or expired planId");
    };
    let target = if req.target.is_empty() {
        "export".to_string()
    } else {
        req.target.clone()
    };
    if target != "export" && target != "import" {
        return ApiResp::err(-1, "target must be export or import");
    }
    let notebook = req.notebook.trim().trim_matches('/').to_string();
    if target == "import" && notebook.is_empty() {
        return ApiResp::err(-1, "notebook required for target=import");
    }
    let (token, ready, ready_err, already) = {
        let p = plan.lock().unwrap();
        let already = p
            .apply
            .as_ref()
            .map(|a| serde_json::to_value(a).unwrap_or_default());
        (
            p.token.clone(),
            done_and_ok(&p.phase, &p.error),
            p.error.clone(),
            already,
        )
    };
    if req.token != token {
        return ApiResp::err(-1, "token mismatch");
    }
    if !ready {
        return ApiResp::err(-1, &format!("plan not ready: {}", ready_err.unwrap_or_default()));
    }
    if let Some(a) = already {
        if a.get("done") != Some(&json!(true)) {
            // a previous apply is still running — report progress instead
            return ApiResp::ok(json!({
                "planId": req.plan_id,
                "phase": a.get("phase").cloned().unwrap_or(json!("writing")),
                "progress": a.get("progress").cloned().unwrap_or(json!(0)),
                "total": a.get("total").cloned().unwrap_or(json!(0)),
            }));
        }
        return ApiResp::err(-1, "plan already applied; planPull again for a fresh plan");
    }
    {
        let mut p = plan.lock().unwrap();
        p.apply = Some(ApplyState {
            target: target.clone(),
            notebook: notebook.clone(),
            include_partial: req.include_partial,
            phase: "writing".into(),
            progress: 0,
            total: p.docs.len(),
            imported: 0,
            partial: 0,
            skipped_existing: 0,
            skipped_partial: 0,
            skipped_empty: 0,
            skipped_unsafe: 0,
            errors: 0,
            body_bytes_total: 0,
            export_dir: None,
            error: None,
            done: false,
        });
    }
    let job = {
        let st = st.clone();
        let plan = plan.clone();
        let target = target.clone();
        let notebook = notebook.clone();
        tokio::task::spawn_blocking(move || {
            run_apply_job(&st, &plan, &target, &notebook, req.include_partial);
        })
    };
    // surface panics as plan errors
    let watcher = plan.clone();
    tokio::spawn(async move {
        if job.await.is_err() {
            if let Ok(mut p) = watcher.lock() {
                if let Some(a) = p.apply.as_mut() {
                    a.phase = "error".into();
                    a.error = Some("apply job panicked".into());
                    a.done = true;
                }
            }
        }
    });
    ApiResp::ok(json!({
        "planId": req.plan_id,
        "phase": "writing",
        "total": docs_len_of(&plan),
        "note": "background job started; poll getPlan (apply.*) until done",
    }))
}

fn docs_len_of(plan: &SharedPlan) -> usize {
    plan.lock().unwrap().docs.len()
}

fn apply_bump(plan: &SharedPlan, f: impl FnOnce(&mut ApplyState)) {
    let payload = {
        let mut p = plan.lock().unwrap();
        if let Some(a) = p.apply.as_mut() {
            f(a);
        }
        p.apply
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok())
            .map(|mut v| {
                v["planId"] = json!(p.plan_id);
                v
            })
    };
    if let Some(v) = payload {
        crate::ws::broadcast("sync-progress", v);
    }
}

/// Stands the watcher down while bulk apply writes files; clears the flag on
/// drop (including panics) so the watcher can never stay silenced.
struct BulkApplyGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for BulkApplyGuard<'_> {
    fn drop(&mut self) {
        self.0
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Synchronous apply body (runs on a blocking thread). `import` writes notes
/// add-only into the vault and rebuilds the index once; `export` writes bodies
/// into an isolated directory. Both push progress and finish the plan.
fn run_apply_job(st: &AppState, plan: &SharedPlan, target: &str, notebook: &str, include_partial: bool) {
    let _bulk_guard = if target == "import" {
        st.bulk_apply
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Some(BulkApplyGuard(&st.bulk_apply))
    } else {
        None
    };
    let result = (|| -> Result<(), String> {
        let (passphrase, pbk, docs, chunks, shards) = {
            let p = plan.lock().unwrap();
            (
                cfg_passphrase(),
                p.pbkdf2_salt.clone().unwrap_or_default(),
                p.docs.clone(),
                p.chunks.clone(),
                p.shards.clone(),
            )
        };
        let export_dir = st
            .workspace
            .join(".markvault/export")
            .join(format!("livesync-{}", Utc::now().timestamp_millis()));
        if target == "export" {
            std::fs::create_dir_all(&export_dir).map_err(|e| format!("mkdir export: {}", e))?;
        }
        let mut imported_paths: Vec<String> = Vec::new();
        for (idx, d) in docs.iter().enumerate() {
            let children = d
                .get("children")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut pieces: Vec<String> = Vec::new();
            let mut miss = 0usize;
            for c in &children {
                let cid = c.as_str().unwrap_or("");
                match chunks.get(cid) {
                    Some(file) => match std::fs::read_to_string(file) {
                        Ok(body) => match decrypt_field(&body, &passphrase, &pbk) {
                            Ok(piece) => pieces.push(piece),
                            Err(_) => miss += 1,
                        },
                        Err(_) => miss += 1,
                    },
                    None => miss += 1,
                }
            }
            apply_bump(plan, |a| {
                a.phase = "writing".into();
                a.progress = idx + 1;
            });
            if pieces.is_empty() {
                apply_bump(plan, |a| a.skipped_empty += 1);
                continue;
            }
            if miss > 0 && !include_partial {
                apply_bump(plan, |a| a.skipped_partial += 1);
                continue;
            }
            let raw_path = d.get("path").and_then(Value::as_str).unwrap_or("");
            let Some(rel) = safe_relative_path(raw_path) else {
                apply_bump(plan, |a| a.skipped_unsafe += 1);
                continue;
            };
            let body = pieces.join("");
            match target {
                "import" => {
                    let rel_full = format!(
                        "{}/{}",
                        notebook.trim_matches('/'),
                        rel.to_string_lossy().replace('\\', "/")
                    );
                    // add-only: never overwrite an existing note
                    if st.vault.read_file(&rel_full).is_some() {
                        apply_bump(plan, |a| a.skipped_existing += 1);
                        continue;
                    }
                    match st.vault.write_file(&rel_full, body.as_bytes()) {
                        Ok(_) => {
                            imported_paths.push(rel_full.clone());
                            apply_bump(plan, |a| {
                                a.imported += 1;
                                a.body_bytes_total += body.len();
                                if miss > 0 {
                                    a.partial += 1;
                                }
                            });
                        }
                        Err(_) => apply_bump(plan, |a| a.errors += 1),
                    }
                }
                _ => {
                    let file = export_dir.join(&rel);
                    if let Some(parent) = file.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&file, &body) {
                        Ok(_) => apply_bump(plan, |a| {
                            a.imported += 1;
                            a.body_bytes_total += body.len();
                            a.export_dir = Some(export_dir.to_string_lossy().to_string());
                        }),
                        Err(_) => apply_bump(plan, |a| a.errors += 1),
                    }
                }
            }
        }
        // index: import relies on a single full rebuild (one pass over the
        // vault, far faster than thousands of per-doc transactions)
        if target == "import" && !imported_paths.is_empty() {
            apply_bump(plan, |a| {
                a.phase = "reindexing".into();
                a.progress = imported_paths.len();
            });
            {
                let db = st.db.lock().unwrap();
                let _ = crate::index::builder::rebuild_all_ext(
                    &db,
                    &st.vault,
                    &|rel| st.is_ignored(rel),
                    true,
                    &|done, _total| {
                        apply_bump(plan, |a| {
                            a.phase = "reindexing".into();
                            a.progress = done;
                        });
                    },
                );
            }
            super::ops::notify_tree();
        }
        if target == "export" {
            let stats = plan.lock().unwrap().apply.clone();
            let manifest = json!({
                "source": "livesync-s3",
                "direction": "download",
                "target": target,
                "shards": shards,
                "apply": stats,
                "note": "exported into an isolated directory; the vault itself is never touched",
            });
            let _ = std::fs::write(
                export_dir.join("manifest.json"),
                serde_json::to_string_pretty(&manifest).unwrap_or_default(),
            );
        }
        if let Some(latest) = shards.iter().max() {
            let p = sync_point_path(st);
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&p, latest);
        }
        Ok(())
    })();
    let (phase, err) = match result {
        Ok(()) => ("done", None),
        Err(e) => ("error", Some(e)),
    };
    let summary = {
        let mut p = plan.lock().unwrap();
        if let Some(a) = p.apply.as_mut() {
            a.phase = phase.to_string();
            a.error = err.clone();
            a.done = true;
        }
        p.apply
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok())
            .unwrap_or(json!({}))
    };
    if target == "import" {
        let tmp = plan.lock().unwrap().tmp_dir.clone();
        if let Some(tmp) = tmp {
            let _ = std::fs::remove_dir_all(tmp);
        }
    }
    set_status(
        "done",
        if err.is_some() { 2 } else { 1 },
        err.as_deref().unwrap_or(""),
    );
    crate::ws::broadcast("sync-done", json!({"target": target, "apply": summary}));
}

fn done_and_ok(phase: &str, error: &Option<String>) -> bool {
    phase == "done" && error.is_none()
}

fn cfg_passphrase() -> String {
    std::env::var("MARKVAULT_SYNC_PASSPHRASE").unwrap_or_default()
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hkdf_decrypt_roundtrip() {
        let pass = "unit-test-passphrase";
        let pbkdf2_salt = [7u8; 32];
        let hkdf_salt = [3u8; 32];
        let iv = [9u8; 12];
        let key = derive_aes_key(pass, &pbkdf2_salt, &hkdf_salt).unwrap();
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap();
        let key = aead::LessSafeKey::new(unbound);
        let mut ct = b"hello livesync".to_vec();
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(iv),
            aead::Aad::empty(),
            &mut ct,
        )
        .unwrap();
        let mut blob = iv.to_vec();
        blob.extend_from_slice(&hkdf_salt);
        blob.extend_from_slice(&ct);
        assert_eq!(decrypt_blob(&blob, pass, &pbkdf2_salt).unwrap(), b"hello livesync");
        assert!(decrypt_blob(&blob, "wrong-pass", &pbkdf2_salt).is_err());
    }

    #[test]
    fn field_layer_decrypts_hkdf_prefixed_value() {
        let pass = "field-test";
        let pbkdf2_salt = [1u8; 32];
        let hkdf_salt = [2u8; 32];
        let iv = [4u8; 12];
        let key = derive_aes_key(pass, &pbkdf2_salt, &hkdf_salt).unwrap();
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap();
        let key = aead::LessSafeKey::new(unbound);
        let mut ct = b"chunk body".to_vec();
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(iv),
            aead::Aad::empty(),
            &mut ct,
        )
        .unwrap();
        let mut blob = iv.to_vec();
        blob.extend_from_slice(&hkdf_salt);
        blob.extend_from_slice(&ct);
        let field = format!("{}{}", HKDF_FIELD_PREFIX, B64.encode(&blob));
        assert_eq!(decrypt_field(&field, pass, &pbkdf2_salt).unwrap(), "chunk body");
        assert_eq!(decrypt_field("plain", pass, &pbkdf2_salt).unwrap(), "plain");
    }

    #[test]
    fn raw_deflate_roundtrip() {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"{\"type\":\"plain\"}\n").unwrap();
        let compressed = enc.finish().unwrap();
        assert_eq!(inflate_raw(&compressed).unwrap(), b"{\"type\":\"plain\"}\n");
    }

    #[test]
    fn list_xml_parses_keys_sizes_and_truncation() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated>\
          <NextContinuationToken>abc+/=</NextContinuationToken>\
          <Contents><Key>1-docs.jsonl.gz</Key><Size>123</Size></Contents>\
          <Contents><Key>2-docs.jsonl.gz</Key><Size>456</Size></Contents></ListBucketResult>";
        let (entries, token, truncated) = parse_list_xml(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].1, 456);
        assert_eq!(token.as_deref(), Some("abc+/="));
        assert!(truncated);
    }

    #[test]
    fn chunk_row_parses_us_separator() {
        let row = "~h:+a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6\u{1f}hello chunk";
        let (id, body) = parse_chunk_row(row).unwrap();
        assert_eq!(id, "h:+a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6");
        assert_eq!(body, "hello chunk");
        assert!(parse_chunk_row("{\"type\":\"plain\"}").is_none());
        assert!(parse_chunk_row("~h:+abc other-sep").is_none());
    }

    #[test]
    fn safe_relative_path_rejects_traversal() {
        assert_eq!(
            safe_relative_path("260817/笔记/首页.md"),
            Some(PathBuf::from("260817/笔记/首页.md"))
        );
        assert!(safe_relative_path("../evil.md").is_none());
        assert!(safe_relative_path("a/../../b.md").is_none());
        assert!(safe_relative_path("").is_none());
        assert_eq!(
            safe_relative_path("/260817/a.md"),
            Some(PathBuf::from("260817/a.md"))
        );
    }

    #[test]
    fn shard_millis_extracts_timestamp() {
        assert_eq!(shard_millis("1794397462713-docs.jsonl.gz"), Some(1794397462713));
        assert_eq!(shard_millis("nope"), None);
    }

    #[test]
    fn plan_summary_includes_apply_state() {
        let plan: SharedPlan = std::sync::Arc::new(std::sync::Mutex::new(Plan::new(
            "p1".into(),
            "tok".into(),
            "full".into(),
        )));
        // no apply yet — field must still be present (null), not missing
        let v = plan_summary("p1", &plan);
        assert!(v.as_object().unwrap().contains_key("apply"));

        {
            let mut p = plan.lock().unwrap();
            p.apply = Some(ApplyState {
                target: "import".into(),
                notebook: "livesync".into(),
                include_partial: false,
                phase: "reindexing".into(),
                progress: 3,
                total: 30,
                imported: 3,
                partial: 0,
                skipped_existing: 0,
                skipped_partial: 0,
                skipped_empty: 0,
                skipped_unsafe: 0,
                errors: 0,
                body_bytes_total: 123,
                export_dir: None,
                error: None,
                done: false,
            });
        }
        let v = plan_summary("p1", &plan);
        let apply = v.get("apply").and_then(|a| a.as_object()).expect("apply object");
        assert_eq!(apply.get("phase").and_then(|x| x.as_str()), Some("reindexing"));
        assert_eq!(apply.get("progress").and_then(|x| x.as_u64()), Some(3));
        assert_eq!(apply.get("done").and_then(|x| x.as_bool()), Some(false));
    }
}
