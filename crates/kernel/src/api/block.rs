use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use crate::vault;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/getDocInfo", post(get_doc_info))
        .route("/getBlockMd", post(get_block_md))
}

/// SiYuan-style outline domain
pub fn outline_routes() -> Router<Arc<AppState>> {
    Router::new().route("/getDocOutline", post(get_outline))
}

/// Frontmatter read/write exposed in SiYuan attr style
pub fn attr_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/getBlockAttrs", post(get_block_attrs))
        .route("/setBlockAttrs", post(set_block_attrs))
}

#[derive(Deserialize, Default)]
struct PathReq {
    #[serde(default)]
    path: String,
}

fn rel_of(p: &str) -> String {
    p.trim_start_matches('/').to_string()
}

async fn get_doc_info(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: PathReq = parse(&bytes);
    let rel = rel_of(&req.path);
    let db = st.db.lock().unwrap();
    match db.query_row(
        "SELECT title, notebook, hpath, created, updated, size, fm_json FROM docs WHERE path = ?1",
        [&rel],
        |r| {
            Ok(json!({
                "path": req.path,
                "title": r.get::<_, String>(0)?,
                "notebook": r.get::<_, String>(1)?,
                "hpath": r.get::<_, String>(2)?,
                "created": r.get::<_, String>(3)?,
                "updated": r.get::<_, String>(4)?,
                "size": r.get::<_, i64>(5)?,
                "attrs": serde_json::from_str::<Value>(&r.get::<_, String>(6)?).unwrap_or(Value::Null),
            }))
        },
    ) {
        Ok(v) => {
            let mut v = v;
            if let Value::Object(o) = &mut v {
                let word_count = count_words(&rel, &st);
                o.insert("wordCount".into(), json!(word_count));
            }
            ApiResp::ok(v)
        }
        Err(_) => ApiResp::err(-1, "doc not found"),
    }
}

fn count_words(rel: &str, st: &AppState) -> i64 {
    let raw = st.vault.read_file(rel).unwrap_or_default();
    let text = String::from_utf8_lossy(&raw);
    text.chars().filter(|c| !c.is_whitespace()).count() as i64
}

#[derive(Deserialize, Default)]
struct BlockReq {
    #[serde(default)]
    path: String,
    /// 1-based line for block-specific read (0 = whole doc)
    #[serde(default)]
    line: i64,
}

async fn get_block_md(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: BlockReq = parse(&bytes);
    let rel = rel_of(&req.path);
    match st.vault.read_file(&rel) {
        Some(raw) => {
            let content = String::from_utf8_lossy(&raw).to_string();
            if req.line <= 0 {
                ApiResp::ok(json!({"content": content}))
            } else {
                // single-line block read (heading / paragraph line)
                let seg: String = content
                    .lines()
                    .nth(req.line.saturating_sub(1) as usize)
                    .unwrap_or("")
                    .to_string();
                ApiResp::ok(json!({"content": seg}))
            }
        }
        None => ApiResp::err(-1, "doc not found"),
    }
}

async fn get_outline(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: PathReq = parse(&bytes);
    let rel = rel_of(&req.path);
    let db = st.db.lock().unwrap();
    let mut out = Vec::new();
    if let Ok(mut stmt) = db.prepare(
        "SELECT block_id, type, text, line FROM blocks WHERE doc_path = ?1 AND type LIKE 'h%' ORDER BY sort",
    ) {
        if let Ok(rows) = stmt.query_map([&rel], |r| {
            Ok(json!({
                "blockId": r.get::<_, Option<String>>(0)?,
                "type": r.get::<_, String>(1)?,
                "content": r.get::<_, String>(2)?,
                "line": r.get::<_, i64>(3)?,
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    ApiResp::ok(json!({"outline": out}))
}

async fn get_block_attrs(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: PathReq = parse(&bytes);
    let rel = rel_of(&req.path);
    let raw = match st.vault.read_file(&rel) {
        Some(r) => r,
        None => return ApiResp::err(-1, "doc not found"),
    };
    let content = String::from_utf8_lossy(&raw).to_string();
    let (fm, _) = vault::extract_frontmatter(&content);
    ApiResp::ok(Value::Object(fm))
}

#[derive(Deserialize, Default)]
struct SetAttrsReq {
    #[serde(default)]
    path: String,
    #[serde(default)]
    attrs: Map<String, Value>,
}

/// Merge attrs into frontmatter and rewrite the file.
/// Values: null deletes the key. Parent links keep their `[[...]]` string form.
async fn set_block_attrs(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: SetAttrsReq = parse(&bytes);
    let rel = rel_of(&req.path);
    let raw = match st.vault.read_file(&rel) {
        Some(r) => r,
        None => return ApiResp::err(-1, "doc not found"),
    };
    let content = String::from_utf8_lossy(&raw).to_string();
    let (mut fm, body_offset) = vault::extract_frontmatter(&content);
    for (k, v) in &req.attrs {
        if v.is_null() {
            fm.remove(k);
        } else {
            fm.insert(k.clone(), v.clone());
        }
    }
    let body = &content[body_offset.min(content.len())..];
    let new_content = render_frontmatter(&fm, body);
    if let Err(e) = st.vault.write_file(&rel, new_content.as_bytes()) {
        return ApiResp::err(-1, &format!("write failed: {}", e));
    }
    super::ops::reindex_one(&st, &rel);
    super::ops::notify_tree();
    ApiResp::ok(json!({}))
}

/// Serialize frontmatter map + body back to a full markdown file
pub fn render_frontmatter(fm: &Map<String, Value>, body: &str) -> String {
    if fm.is_empty() {
        return body.to_string();
    }
    let mut out = String::from("---\n");
    for (k, v) in fm {
        match v {
            Value::String(s) => out.push_str(&format!("{}: {}\n", k, s)),
            Value::Array(a) => {
                out.push_str(&format!("{}:\n", k));
                for item in a {
                    out.push_str(&format!("  - {}\n", item));
                }
            }
            other => out.push_str(&format!("{}: {}\n", k, other)),
        }
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}
