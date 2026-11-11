//! LiveSync read-only pull (S1 stage).
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
//! * `applyPull` defaults to an isolated export directory and never writes `.md`
//!   into the vault; the Markdown truth source stays untouched.
//!
//! S1 status: shard decryption + deflate + JSONL parsing are implemented and
//! verified against a real remote. **S2 TODO**: the observed journal shards
//! contain only `f:` metadata docs whose `children` point at `h:+...` chunk IDs;
//! the chunk documents themselves were not present in the shards sampled, so
//! reassembling Markdown bodies is deliberately NOT faked. `decrypt_field()`
//! (the `%=` document-layer E2EE v2 primitive) is already in place for S2.

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
use std::sync::{Arc, Mutex, OnceLock};

const PBKDF2_ITERATIONS: u32 = 310_000;
const SYNC_PARAMS_KEY: &str = "_obsidian_livesync_journal_sync_parameters.json";
const SHARD_SUFFIX: &str = "-docs.jsonl.gz";
const HKDF_FIELD_PREFIX: &str = "%=";

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
    #[allow(dead_code)] // S2: ID derivation key for chunk-id / path verification
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

/// Document-layer field decryption (S2 will use this for `data` / encrypted meta).
#[allow(dead_code)]
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

struct Plan {
    token: String,
    shards: Vec<String>,
    docs: Value,
    created: u64,
}

fn plans() -> &'static Mutex<HashMap<String, Plan>> {
    static P: OnceLock<Mutex<HashMap<String, Plan>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sync_point_path(st: &AppState) -> std::path::PathBuf {
    st.workspace.join(".markvault/sync/livesync/latest-sync")
}

fn read_sync_point(st: &AppState) -> String {
    std::fs::read_to_string(sync_point_path(st)).unwrap_or_default()
}

// ---------------------------------------------------------------- handlers

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/getStatus", post(get_status))
        .route("/getSourceState", post(get_source_state))
        .route("/planPull", post(plan_pull))
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
    #[serde(default)]
    limit: usize,
}

/// Decrypt the newest N shards, inflate and parse JSONL; returns a preview only.
/// No files are written and no Markdown body is reconstructed yet (S2).
async fn plan_pull(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: PlanReq = parse(&bytes);
    let limit = req.limit.clamp(1, 20);
    set_status("preparing", 0, "");
    let cfg = match SyncConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            set_status("idle", 2, &e);
            return ApiResp::err(-1, &e);
        }
    };
    let result = async {
        let pbk = fetch_pbkdf2_salt(&cfg).await?;
        let all = list_objects(&cfg).await?;
        let mut shards: Vec<String> = all
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| k.ends_with(SHARD_SUFFIX))
            .collect();
        shards.sort();
        let take: Vec<String> = shards.iter().rev().take(limit).cloned().collect();
        let mut docs: Vec<Value> = Vec::new();
        let mut lines_total = 0usize;
        let mut parse_errors = 0usize;
        let mut by_id = HashMap::new();
        for key in &take {
            let path = format!("/{}/{}", cfg.bucket, key);
            let raw = s3_get(&cfg, &path, "").await?;
            let plain = decrypt_blob(&raw, &cfg.passphrase, &pbk)?;
            let text = String::from_utf8(inflate_raw(&plain)?)
                .map_err(|_| "shard is not UTF-8 JSONL".to_string())?;
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                lines_total += 1;
                match serde_json::from_str::<Value>(line) {
                    Ok(v) => {
                        if let Some(id) = v.get("_id").and_then(Value::as_str) {
                            by_id.insert(id.to_string(), ());
                        }
                        if docs.len() < 200 {
                            docs.push(json!({
                                "id": v.get("_id").and_then(Value::as_str).unwrap_or(""),
                                "type": v.get("type").and_then(Value::as_str).unwrap_or(""),
                                "path": v.get("path").and_then(Value::as_str).unwrap_or(""),
                                "size": v.get("size").and_then(Value::as_u64).unwrap_or(0),
                                "children": v.get("children").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0),
                                "dataPrefix": v.get("data").and_then(Value::as_str).map(|s| s.chars().take(2).collect::<String>()).unwrap_or_default(),
                            }));
                        }
                    }
                    Err(_) => parse_errors += 1,
                }
            }
        }
        let plan_id = format!("mv-{}", Utc::now().timestamp_millis());
        let token = format!("{:016x}", rand::random::<u64>());
        let doc_value = json!(docs);
        if let Ok(mut p) = plans().lock() {
            p.insert(
                plan_id.clone(),
                Plan {
                    token: token.clone(),
                    shards: take.clone(),
                    docs: doc_value.clone(),
                    created: Utc::now().timestamp() as u64,
                },
            );
            if p.len() > 8 {
                let oldest = p
                    .iter()
                    .min_by_key(|(_, v)| v.created)
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    p.remove(&k);
                }
            }
        }
        Ok::<_, String>(json!({
            "planId": plan_id,
            "token": token,
            "direction": "download",
            "shards": take,
            "lines": lines_total,
            "uniqueIds": by_id.len(),
            "parseErrors": parse_errors,
            "docs": doc_value,
            "note": "S1 preview only: chunk documents were not present in the sampled shards, so Markdown bodies are not reconstructed yet (S2).",
            "localSyncPoint": read_sync_point(&st),
        }))
    }
    .await;
    match result {
        Ok(v) => {
            set_status("idle", 1, "");
            ApiResp::ok(v)
        }
        Err(e) => {
            set_status("idle", 2, &e);
            ApiResp::err(-1, &e)
        }
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
}

/// Apply a preview plan. `export` writes only a manifest into an isolated
/// directory; `index` is deferred until S2 can rebuild Markdown bodies.
async fn apply_pull(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: ApplyReq = parse(&bytes);
    let plan = plans().lock().ok().and_then(|p| {
        p.get(&req.plan_id)
            .map(|v| (v.token.clone(), v.shards.clone(), v.docs.clone()))
    });
    let (expected_token, shards, docs) = match plan {
        Some(p) => p,
        None => return ApiResp::err(-1, "unknown or expired planId"),
    };
    if req.token != expected_token {
        return ApiResp::err(-1, "token mismatch");
    }
    let target = if req.target.is_empty() {
        "export"
    } else {
        &req.target
    };
    if target == "index" {
        return ApiResp::err(-1, "target=index is deferred until S2 (Markdown body reconstruction)");
    }
    if target != "export" {
        return ApiResp::err(-1, "target must be export or index");
    }
    let dir = st
        .workspace
        .join(".markvault/export")
        .join(format!("livesync-{}", Utc::now().timestamp_millis()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return ApiResp::err(-1, &format!("mkdir export: {}", e));
    }
    let manifest = json!({
        "source": "livesync-s3",
        "direction": "download",
        "shards": shards,
        "docs": docs,
        "note": "metadata preview only; .md files are intentionally NOT written (read-only discipline)",
    });
    let file = dir.join("manifest.json");
    if let Err(e) = std::fs::write(
        &file,
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    ) {
        return ApiResp::err(-1, &format!("write manifest: {}", e));
    }
    if let Some(latest) = shards.iter().max() {
        let p = sync_point_path(&st);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&p, latest);
    }
    set_status("done", 1, "");
    ApiResp::ok(json!({
        "applied": 0,
        "skipped": 0,
        "conflicts": 0,
        "manifest": file.to_string_lossy(),
        "note": "no .md written; S2 will reconstruct bodies",
    }))
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
}
