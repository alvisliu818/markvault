use super::ApiResp;
use crate::state::AppState;
use crate::vt::{load_doc_infos, resolve_target};
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Global link graph for the force-graph canvas panel (M11).
/// Nodes = all indexed docs (isolated ones included); links = wikilinks
/// resolved to actual docs (unresolved targets are skipped).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/getGraph", post(get_graph))
}

async fn get_graph(State(st): State<Arc<AppState>>, _bytes: Bytes) -> ApiResp {
    // load_doc_infos locks db internally — take our copy first (deadlock rule)
    let docs = load_doc_infos(&st);
    let db = st.db.lock().unwrap();

    let mut nodes: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = db.prepare("SELECT path, title, notebook FROM docs ORDER BY path") {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) {
            for (path, title, notebook) in rows.flatten() {
                nodes.push(json!({
                    "id": path,
                    "title": title,
                    "notebook": notebook,
                }));
            }
        }
    }

    // resolve DISTINCT (src,target) pairs into doc->doc edges
    let mut edges: Vec<Value> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    if let Ok(mut stmt) = db.prepare("SELECT DISTINCT src_path, target FROM links") {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
            ))
        }) {
            for (src, target) in rows.flatten() {
                if let Some(dst) = resolve_target(&target, &docs) {
                    if dst == src || !seen.insert((src.clone(), dst.clone())) {
                        continue;
                    }
                    edges.push(json!({"source": src, "target": dst}));
                }
            }
        }
    }

    ApiResp::ok(json!({
        "nodes": nodes,
        "links": edges,
    }))
}
