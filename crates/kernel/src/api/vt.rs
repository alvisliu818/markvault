use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use crate::vt::{self, MountItem, VtStore, VtSettings};
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Virtual document tree API domain
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/getRelationTree", post(get_relation_tree))
        .route("/addRoot", post(add_root))
        .route("/removeRoot", post(remove_root))
        .route("/getMountTree", post(get_mount_tree))
        .route("/setMountTree", post(set_mount_tree))
        .route("/setNodeOrder", post(set_node_order))
        .route("/getSettings", post(get_settings))
        .route("/setSettings", post(set_settings))
}

async fn get_relation_tree(State(st): State<Arc<AppState>>) -> ApiResp {
    let store = VtStore::load(&st);
    let docs = vt::load_doc_infos(&st);
    let tree = vt::build_relation_tree(&docs, &store.roots, &store.settings, &store.custom_order);
    ApiResp::ok(json!({
        "roots": store.roots,
        "tree": tree,
        "settings": store.settings,
    }))
}

#[derive(Deserialize, Default)]
struct PathOnlyReq {
    #[serde(default)]
    path: String,
}

async fn add_root(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: PathOnlyReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    if rel.is_empty() {
        return ApiResp::err(-1, "path required");
    }
    let mut store = VtStore::load(&st);
    let rel = rel.to_string();
    if !store.roots.contains(&rel) {
        store.roots.push(rel);
        store.save(&st);
    }
    crate::ws::push_vt_changed();
    ApiResp::ok(json!({"roots": store.roots}))
}

async fn remove_root(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: PathOnlyReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/').to_string();
    let mut store = VtStore::load(&st);
    store.roots.retain(|r| *r != rel);
    store.save(&st);
    crate::ws::push_vt_changed();
    ApiResp::ok(json!({"roots": store.roots}))
}

async fn get_mount_tree(State(st): State<Arc<AppState>>) -> ApiResp {
    let store = VtStore::load(&st);
    ApiResp::ok(json!({"mountRoots": store.mount_roots}))
}

#[derive(Deserialize, Default)]
struct SetMountReq {
    #[serde(default)]
    mount_roots: Vec<MountItem>,
}

async fn set_mount_tree(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: SetMountReq = parse(&bytes);
    let mut store = VtStore::load(&st);
    store.mount_roots = req.mount_roots;
    store.save(&st);
    crate::ws::push_vt_changed();
    ApiResp::ok(json!({"mountRoots": store.mount_roots}))
}

#[derive(Deserialize, Default)]
struct OrderReq {
    /// parent key: doc path, or "__root__" for top level
    #[serde(default)]
    parent_key: String,
    #[serde(default)]
    ordered_paths: Vec<String>,
}

async fn set_node_order(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: OrderReq = parse(&bytes);
    let mut store = VtStore::load(&st);
    let key = if req.parent_key == "__root__" {
        "__root__".to_string()
    } else {
        req.parent_key.trim_start_matches('/').to_string()
    };
    let ordered: Vec<String> = req
        .ordered_paths
        .iter()
        .map(|p| p.trim_start_matches('/').to_string())
        .collect();
    store.custom_order.insert(key, ordered);
    store.save(&st);
    crate::ws::push_vt_changed();
    ApiResp::ok(json!({}))
}

async fn get_settings(State(st): State<Arc<AppState>>) -> ApiResp {
    let store = VtStore::load(&st);
    ApiResp::ok(json!({"settings": store.settings}))
}

#[derive(Deserialize, Default)]
struct SettingsReq {
    #[serde(default)]
    settings: Option<VtSettings>,
}

async fn set_settings(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: SettingsReq = parse(&bytes);
    let mut store = VtStore::load(&st);
    if let Some(s) = req.settings {
        store.settings = s;
        store.save(&st);
    }
    crate::ws::push_vt_changed();
    ApiResp::ok(json!({"settings": store.settings}))
}
