use super::{parse, ApiResp};
use crate::state::AppState;
use crate::vt::{load_doc_infos, resolve_target};
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/getBacklinks", post(get_backlinks))
}

#[derive(Deserialize, Default)]
struct RefReq {
    #[serde(default)]
    path: String,
}

/// Backlink graph for one doc.
/// - links:   outgoing edges (this doc -> resolved targets)
/// - backlinks: inbound edges (docs whose wikilinks resolve to this doc)
/// - mentions: pseudo backlinks — docs whose indexed text mentions the title
///   but contain no wikilink to it (SiYuan-style 伪反链)
async fn get_backlinks(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: RefReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/').to_string();
    if rel.is_empty() {
        return ApiResp::err(-1, "path required");
    }

    // load doc infos first — it locks db internally; do NOT hold the db
    // lock across that call (same deadlock class as listDocsByPath)
    let docs = load_doc_infos(&st);
    let db = st.db.lock().unwrap();
    // doc must exist in the index
    let title: String = match db.query_row(
        "SELECT title FROM docs WHERE path = ?1",
        [&rel],
        |r| r.get(0),
    ) {
        Ok(t) => t,
        Err(_) => return ApiResp::err(-1, "doc not found"),
    };

    // outgoing: resolve each link target from this doc
    let mut out_links: Vec<Value> = Vec::new();
    let mut unresoved: Vec<Value> = Vec::new();
    if let Ok(mut stmt) =
        db.prepare("SELECT DISTINCT target, anchor FROM links WHERE src_path = ?1 ORDER BY target")
    {
        if let Ok(rows) = stmt.query_map([&rel], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        }) {
            for row in rows.flatten() {
                let (target, anchor) = row;
                match resolve_target(&target, &docs) {
                    Some(p) => out_links.push(json!({
                        "path": p,
                        "raw": target,
                        "anchor": anchor,
                    })),
                    None => unresoved.push(json!({"raw": target, "anchor": anchor})),
                }
            }
        }
    }

    // inbound: any link row whose resolved target == this doc
    let mut backlinks: Vec<Value> = Vec::new();
    if let Ok(mut stmt) =
        db.prepare("SELECT DISTINCT src_path FROM links WHERE src_path != ?1 ORDER BY src_path")
    {
        if let Ok(rows) = stmt.query_map([&rel], |r| r.get::<_, String>(0)) {
            for src in rows.flatten() {
                // resolve all targets of src once per row (small vaults: fine)
                if doc_links_to(&db, &docs, &src, &rel, &title) {
                    backlinks.push(json!({"path": src}));
                }
            }
        }
    }

    // mentions: FTS on the title, excluding docs already linking it
    let mut mentions: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = db.prepare(
        "SELECT DISTINCT path FROM doc_fts WHERE doc_fts MATCH ?1 AND path != ?2 LIMIT 50",
    ) {
        let q = format!("\"{}\"", title.replace('"', "\"\""));
        if let Ok(rows) = stmt.query_map([&q, &rel], |r| r.get::<_, String>(0)) {
            let linked: Vec<String> = backlinks
                .iter()
                .filter_map(|v| v["path"].as_str().map(|s| s.to_string()))
                .collect();
            for src in rows.flatten() {
                if !linked.contains(&src) && !mentions_body(&db, &src, &title) {
                    continue;
                }
                if !linked.contains(&src) {
                    mentions.push(json!({"path": src}));
                }
            }
        }
    }

    ApiResp::ok(json!({
        "path": rel,
        "links": out_links,
        "unresolved": unresoved,
        "backlinks": backlinks,
        "mentions": mentions,
    }))
}

/// Does doc `src` contain a wikilink that resolves to `target_path`?
fn doc_links_to(
    db: &rusqlite::Connection,
    docs: &[crate::vt::DocInfo],
    src: &str,
    target_path: &str,
    target_title: &str,
) -> bool {
    if let Ok(mut stmt) = db.prepare("SELECT target FROM links WHERE src_path = ?1") {
        if let Ok(rows) = stmt.query_map([src], |r| r.get::<_, String>(0)) {
            for t in rows.flatten() {
                if let Some(p) = resolve_target(&t, docs) {
                    if p == target_path {
                        return true;
                    }
                }
            }
        }
    }
    // frontmatter parent links are not in `links` table; check fm_json
    if let Ok(fm_json) =
        db.query_row("SELECT fm_json FROM docs WHERE path = ?1", [src], |r| {
            r.get::<_, String>(0)
        })
    {
        if let Ok(fm) = serde_json::from_str::<serde_json::Value>(&fm_json) {
            let empty: Vec<&serde_json::Value> = Vec::new();
            for v in fm.as_object().map(|m| m.values().collect::<Vec<_>>()).unwrap_or(empty) {
                let vals: Vec<String> = match v {
                    serde_json::Value::Array(a) => a
                        .iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect(),
                    serde_json::Value::String(s) => vec![s.clone()],
                    _ => vec![],
                };
                for raw in vals {
                    let t = crate::parse::split_wikilink(&raw).0;
                    if let Some(p) = resolve_target(&t, docs) {
                        if p == target_path {
                            return true;
                        }
                    }
                }
            }
        }
    }
    let _ = target_title;
    false
}

/// Verify the title actually appears in the doc body text (avoids FTS tokenization false hits).
fn mentions_body(db: &rusqlite::Connection, path: &str, title: &str) -> bool {
    if title.is_empty() {
        return false;
    }
    db.query_row(
        "SELECT COUNT(*) FROM doc_fts WHERE path = ?1 AND body LIKE ?2",
        rusqlite::params![path, format!("%{}%", title)],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}
