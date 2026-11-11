use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use crate::vault::sanitize_filename;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::sync::Arc;

/// Notebook = top-level folder in the vault (SiYuan Box equivalent)
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/lsNotebooks", post(ls_notebooks))
        .route("/createNotebook", post(create_notebook))
        .route("/renameNotebook", post(rename_notebook))
        .route("/removeNotebook", post(remove_notebook))
}

fn doc_count(st: &AppState, notebook: &str) -> i64 {
    let db = st.db.lock().unwrap();
    db.query_row(
        "SELECT COUNT(*) FROM docs WHERE notebook = ?1",
        [notebook],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

async fn ls_notebooks(State(st): State<Arc<AppState>>) -> ApiResp {
    let mut out = Vec::new();
    for name in st.vault.ls_notebooks() {
        out.push(json!({
            "id": name,
            "name": name,
            "docCount": doc_count(&st, &name),
        }));
    }
    ApiResp::ok(json!({"notebooks": out}))
}

#[derive(Deserialize, Default)]
struct NameReq {
    #[serde(default)]
    name: String,
}

async fn create_notebook(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: NameReq = parse(&bytes);
    let name = sanitize_filename(req.name.trim());
    if name.is_empty() {
        return ApiResp::err(-1, "name required");
    }
    if st.vault.exists(&name) {
        return ApiResp::err(-1, "notebook exists");
    }
    if let Err(e) = st.vault.mkdir(&name) {
        return ApiResp::err(-1, &format!("create failed: {}", e));
    }
    crate::ws::push_filetree_changed();
    ApiResp::ok(json!({"id": name, "name": name}))
}

#[derive(Deserialize, Default)]
struct RenameReq {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

async fn rename_notebook(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: RenameReq = parse(&bytes);
    let new_name = sanitize_filename(req.name.trim());
    if new_name.is_empty() || req.id.is_empty() {
        return ApiResp::err(-1, "id/name required");
    }
    if st.vault.exists(&new_name) {
        return ApiResp::err(-1, "target exists");
    }
    if let Err(e) = st.vault.rename(&req.id, &new_name) {
        return ApiResp::err(-1, &format!("rename failed: {}", e));
    }
    // reindex both sides of the rename
    let _ = fs::remove_dir_all(st.workspace.join(".markvault").join("cache"));
    crate::ws::push_filetree_changed();
    ApiResp::ok(json!({"id": new_name}))
}

async fn remove_notebook(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: NameReq = parse(&bytes);
    match st.vault.trash(&req.name) {
        Ok(dest) => {
            {
                let db = st.db.lock().unwrap();
                let _ = db.execute("DELETE FROM docs WHERE notebook = ?1", [&req.name]);
            }
            crate::ws::push_filetree_changed();
            ApiResp::ok(json!({"trashPath": dest}))
        }
        Err(e) => ApiResp::err(-1, &format!("remove failed: {}", e)),
    }
}
