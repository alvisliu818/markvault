use super::{check_writable, parse, ApiResp};
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
    Router::new().route("/moveNode", post(move_node))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MoveReq {
    /// doc being moved (vault-relative)
    #[serde(default)]
    path: String,
    /// new parent doc path (vault-relative); empty = detach (top-level)
    #[serde(default)]
    new_parent: String,
}

/// Cross-parent move: rewrite the doc's frontmatter `parent` attr so the
/// relation tree derives the new hierarchy. Single-parent semantics: replaces
/// the whole parent list (multi-parent docs are managed via frontmatter edit).
async fn move_node(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: MoveReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/').to_string();
    if rel.is_empty() {
        return ApiResp::err(-1, "path required");
    }
    let raw = match st.vault.read_file(&rel) {
        Some(r) => r,
        None => return ApiResp::err(-1, "doc not found"),
    };
    let content = String::from_utf8_lossy(&raw).to_string();
    let (mut fm, body_offset) = vault::extract_frontmatter(&content);

    let parent_attr = {
        let conf = st.conf.read().unwrap();
        conf.parent_attr.clone()
    };

    if req.new_parent.trim_matches('/').is_empty() {
        fm.remove(&parent_attr);
    } else {
        let parent_rel = req.new_parent.trim_start_matches('/').trim_end_matches(".md");
        let parent_title = parent_rel
            .rsplit('/')
            .next()
            .unwrap_or(parent_rel)
            .to_string();
        fm.insert(
            parent_attr,
            Value::Array(vec![Value::String(format!("[[{}]]", parent_title))]),
        );
    }

    let body = &content[body_offset.min(content.len())..];
    let new_content = super::block::render_frontmatter(&fm, body);
    if let Err(e) = st.vault.write_file(&rel, new_content.as_bytes()) {
        return ApiResp::err(-1, &format!("write failed: {}", e));
    }
    super::ops::reindex_one(&st, &rel);
    super::ops::notify_tree();
    ApiResp::ok(json!({"path": rel}))
}
