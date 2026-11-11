use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// Raw workspace file access — heavily used by the virtual-tree panel
/// (mirrors SiYuan /api/file/*).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/getFile", post(get_file))
        .route("/putFile", post(put_file))
        .route("/removeFile", post(remove_file))
        .route("/renameFile", post(rename_file))
        .route("/readDir", post(read_dir))
}

#[derive(Deserialize, Default)]
struct PathReq {
    #[serde(default)]
    path: String,
}

pub async fn get_file(State(st): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let req: PathReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    if rel.is_empty() {
        return ApiResp::err(-1, "path required").into_response();
    }
    match st.vault.read_file(rel) {
        Some(data) => {
            let ct = if rel.ends_with(".md") {
                "text/markdown; charset=utf-8"
            } else {
                "application/octet-stream"
            };
            ([(header::CONTENT_TYPE, ct)], data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "file not found").into_response(),
    }
}

#[derive(Deserialize, Default)]
struct PutReq {
    #[serde(default)]
    path: String,
    /// text content; binary payloads use base64 in `dataBase64`
    #[serde(default)]
    data: String,
    #[serde(default)]
    data_base64: String,
    #[serde(default)]
    is_dir: bool,
}

pub async fn put_file(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: PutReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    if rel.is_empty() {
        return ApiResp::err(-1, "path required");
    }
    // auto-snapshot: keep the pre-write content of md files in the repo
    if !req.is_dir && rel.ends_with(".md") {
        if let Some(old) = st.vault.read_file(rel) {
            let _ = super::snapshot::store(&st).snapshot(rel, &old, false);
        }
    }
    let res = if req.is_dir {
        st.vault.mkdir(rel).map(|_| 0)
    } else {
        let data = if !req.data_base64.is_empty() {
            use base64_serde_help::*;
            decode_b64(&req.data_base64)
        } else {
            Ok(req.data.clone().into_bytes())
        };
        match data {
            Ok(d) => st.vault.write_file(rel, &d).map(|_| d.len()),
            Err(e) => Err(e),
        }
    };
    match res {
        Ok(_) => {
            let is_md = rel.ends_with(".md");
            if is_md {
                super::ops::reindex_one(&st, rel);
                super::ops::notify_tree();
            }
            ApiResp::ok(json!({"path": rel}))
        }
        Err(e) => ApiResp::err(-1, &format!("put failed: {}", e)),
    }
}

mod base64_serde_help {
    pub fn decode_b64(s: &str) -> std::io::Result<Vec<u8>> {
        const TBL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        fn val(c: u8) -> Option<u32> {
            TBL.iter().position(|&t| t == c).map(|p| p as u32)
        }
        let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=').collect();
        let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
        for chunk in bytes.chunks(4) {
            let mut n: u32 = 0;
            for (i, &c) in chunk.iter().enumerate() {
                match val(c) {
                    Some(v) => n |= v << (18 - 6 * i),
                    None => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad base64")),
                }
            }
            out.push((n >> 16) as u8);
            if chunk.len() > 1 {
                out.push((n >> 8) as u8);
            }
            if chunk.len() > 2 {
                out.push(n as u8);
            }
        }
        Ok(out)
    }
}

pub async fn remove_file(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: PathReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    match st.vault.trash(rel) {
        Ok(dest) => {
            if rel.ends_with(".md") {
                super::ops::unindex(&st, rel);
            }
            let mut vt = crate::vt::VtStore::load(&st);
            crate::vt::sync_delete(&mut vt, rel);
            vt.save(&st);
            super::ops::notify_tree();
            ApiResp::ok(json!({"trashPath": dest}))
        }
        Err(e) => ApiResp::err(-1, &format!("remove failed: {}", e)),
    }
}

#[derive(Deserialize, Default)]
struct RenameReq {
    #[serde(default)]
    path: String,
    #[serde(default)]
    new_path: String,
}

pub async fn rename_file(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: RenameReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    let new_rel = req.new_path.trim_start_matches('/');
    if rel.is_empty() || new_rel.is_empty() {
        return ApiResp::err(-1, "path/newPath required");
    }
    if st.vault.exists(new_rel) {
        return ApiResp::err(-1, "target exists");
    }
    if let Err(e) = st.vault.rename(rel, new_rel) {
        return ApiResp::err(-1, &format!("rename failed: {}", e));
    }
    if rel.ends_with(".md") {
        super::ops::reindex_prefix(&st, rel, new_rel);
    }
    let mut vt = crate::vt::VtStore::load(&st);
    crate::vt::sync_rename(&mut vt, rel, new_rel);
    vt.save(&st);
    super::ops::notify_tree();
    ApiResp::ok(json!({"path": new_rel}))
}

#[derive(Deserialize, Default)]
struct DirReq {
    #[serde(default)]
    path: String,
}

async fn read_dir(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: DirReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    let abs = match st.vault.abs(rel) {
        Some(p) => p,
        None => return ApiResp::err(-1, "invalid path"),
    };
    if !abs.is_dir() {
        return ApiResp::err(-1, "not a directory");
    }
    let mut entries: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&abs) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let meta = e.metadata().ok();
            entries.push(json!({
                "name": name,
                "isDir": e.path().is_dir(),
                "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
                "mtime": meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .map(crate::vault::ts_from_mtime)
                    .unwrap_or_default(),
            }));
        }
    }
    entries.sort_by(|a, b| {
        b["isDir"]
            .as_bool()
            .unwrap_or(false)
            .cmp(&a["isDir"].as_bool().unwrap_or(false))
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    ApiResp::ok(json!({"entries": entries}))
}
