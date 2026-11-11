//! Block-level operations (SiYuan /api/block/* semantics) on line-based markdown.
//! Internal addressing = doc_path + 1-based line (same as the blocks table).
//! All writes go through the vault and reindex the doc afterwards.

use super::{check_writable, parse, ApiResp};
use crate::id;
use crate::state::AppState;
use crate::vault;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/insertBlock", post(insert_block))
        .route("/updateBlock", post(update_block))
        .route("/deleteBlock", post(delete_block))
        .route("/getBlockBreadcrumb", post(get_block_breadcrumb))
}

#[derive(Deserialize, Default)]
struct InsertReq {
    #[serde(default)]
    path: String,
    /// markdown content of the new block (single or multi-line)
    #[serde(default)]
    markdown: String,
    /// insert AFTER this 1-based line; 0 = append at end of doc body
    #[serde(default)]
    line: i64,
    /// auto-append a generated block id (` ^id`) to the inserted block
    #[serde(default = "default_true")]
    auto_id: bool,
}

fn default_true() -> bool {
    true
}

fn rel_of(p: &str) -> String {
    p.trim_start_matches('/').to_string()
}

/// Load body (frontmatter stripped) as lines.
fn load_body(st: &AppState, rel: &str) -> Result<(Vec<String>, String), ApiResp> {
    let raw = st
        .vault
        .read_file(rel)
        .ok_or_else(|| ApiResp::err(-1, "doc not found"))?;
    let content = String::from_utf8_lossy(&raw).to_string();
    let (_, body_offset) = vault::extract_frontmatter(&content);
    let body = content[body_offset.min(content.len())..].to_string();
    let lines: Vec<String> = body.lines().map(|l| l.to_string()).collect();
    Ok((lines, content[..body_offset.min(content.len())].to_string()))
}

fn save_body(st: &AppState, rel: &str, fm_prefix: &str, lines: &[String]) -> Result<(), ApiResp> {
    let mut out = fm_prefix.to_string();
    out.push_str(&lines.join("\n"));
    if !lines.last().map(|l| l.is_empty()).unwrap_or(true) {
        out.push('\n');
    }
    st.vault
        .write_file(rel, out.as_bytes())
        .map_err(|e| ApiResp::err(-1, &format!("write failed: {}", e)))?;
    super::ops::reindex_one(st, rel);
    super::ops::notify_tree();
    Ok(())
}

async fn insert_block(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: InsertReq = parse(&bytes);
    let rel = rel_of(&req.path);
    if req.markdown.trim().is_empty() {
        return ApiResp::err(-1, "markdown required");
    }
    let (mut lines, fm_prefix) = match load_body(&st, &rel) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let block_md = if req.auto_id {
        format!("{} ^{}", req.markdown.trim_end(), id::new_node_id())
    } else {
        req.markdown.trim_end().to_string()
    };

    // insert at anchor; 0 = append at end
    let anchor = if req.line <= 0 {
        lines.len()
    } else {
        (req.line as usize).min(lines.len())
    };
    let insert_at = anchor.min(lines.len());
    for (i, l) in block_md.lines().enumerate() {
        lines.insert(insert_at + i, l.to_string());
    }
    match save_body(&st, &rel, &fm_prefix, &lines) {
        Ok(_) => ApiResp::ok(json!({"path": rel, "bodyLine": insert_at + 1})),
        Err(e) => e,
    }
}

#[derive(Deserialize, Default)]
struct UpdateReq {
    #[serde(default)]
    path: String,
    /// 1-based line of the block to replace
    #[serde(default)]
    line: i64,
    #[serde(default)]
    markdown: String,
}

async fn update_block(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: UpdateReq = parse(&bytes);
    let rel = rel_of(&req.path);
    if req.line <= 0 {
        return ApiResp::err(-1, "line required");
    }
    let (mut lines, fm_prefix) = match load_body(&st, &rel) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let idx = (req.line as usize).saturating_sub(1);
    if idx >= lines.len() {
        return ApiResp::err(-1, "line out of range");
    }
    // preserve an existing trailing block id unless the new md carries one
    let old = &lines[idx];
    let new_md = req.markdown.trim_end().to_string();
    let has_id = new_md.contains(" ^") && !new_md.ends_with(" ^");
    let preserved_id = if has_id {
        None
    } else if let Some(cap) = regex::Regex::new(r"\s(\^[A-Za-z0-9][A-Za-z0-9-]*)\s*$")
        .ok()
        .and_then(|re| re.captures(old))
    {
        Some(cap[1].to_string())
    } else {
        None
    };
    let final_md = match preserved_id {
        Some(pid) if !new_md.is_empty() => format!("{} {}", new_md, pid),
        _ => new_md,
    };
    lines[idx] = final_md;
    match save_body(&st, &rel, &fm_prefix, &lines) {
        Ok(_) => ApiResp::ok(json!({"path": rel, "bodyLine": req.line})),
        Err(e) => e,
    }
}

#[derive(Deserialize, Default)]
struct DeleteReq {
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: i64,
}

async fn delete_block(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: DeleteReq = parse(&bytes);
    let rel = rel_of(&req.path);
    if req.line <= 0 {
        return ApiResp::err(-1, "line required");
    }
    let (mut lines, fm_prefix) = match load_body(&st, &rel) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let idx = (req.line as usize).saturating_sub(1);
    if idx >= lines.len() {
        return ApiResp::err(-1, "line out of range");
    }
    lines.remove(idx);
    match save_body(&st, &rel, &fm_prefix, &lines) {
        Ok(_) => ApiResp::ok(json!({"path": rel})),
        Err(e) => e,
    }
}

#[derive(Deserialize, Default)]
struct BreadcrumbReq {
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: i64,
}

/// Heading stack above the given line: [{level, content, line}]
async fn get_block_breadcrumb(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: BreadcrumbReq = parse(&bytes);
    let rel = rel_of(&req.path);
    let db = st.db.lock().unwrap();
    let mut out: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = db.prepare(
        "SELECT type, text, line FROM blocks
         WHERE doc_path = ?1 AND type LIKE 'h%' AND line <= ?2
         ORDER BY line",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![rel, req.line], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        }) {
            // keep a descending-level stack: pop deeper-or-equal headings
            let mut level = 0u8;
            for (t, text, line) in rows.flatten() {
                let l: u8 = t.get(1..).and_then(|s| s.parse().ok()).unwrap_or(1);
                if out.is_empty() || l > level {
                    out.push(json!({"level": l, "content": text, "line": line}));
                    level = l;
                } else {
                    while level >= l && !out.is_empty() {
                        out.pop();
                        level = out
                            .last()
                            .and_then(|v| v["level"].as_u64())
                            .map(|n| n as u8)
                            .unwrap_or(0);
                    }
                    out.push(json!({"level": l, "content": text, "line": line}));
                    level = l;
                }
            }
        }
    }
    ApiResp::ok(json!({"path": rel, "breadcrumb": out}))
}

#[cfg(test)]
mod tests {
    #[test]
    fn breadcrumb_stack_logic_lines() {
        // Simulated heading flow: h1, h2, h2, h3, h1
        // expected final breadcrumb for a line under the last h1: [h1(last)]
        let seq = [(1u8, "A"), (2, "B"), (2, "C"), (3, "D"), (1, "E")];
        let mut out: Vec<(u8, &str)> = Vec::new();
        let mut level = 0u8;
        for (l, t) in seq {
            if out.is_empty() || l > level {
                out.push((l, t));
                level = l;
            } else {
                while level >= l && !out.is_empty() {
                    out.pop();
                    level = out.last().map(|x| x.0).unwrap_or(0);
                }
                out.push((l, t));
                level = l;
            }
        }
        assert_eq!(out, vec![(1, "E")]);
    }
}
