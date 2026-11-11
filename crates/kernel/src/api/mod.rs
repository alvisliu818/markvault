pub mod asset;
pub mod block;
pub mod bookmark;
pub mod clip;
pub mod graph;
pub mod history;
pub mod file;
pub mod filetree;
pub mod format;
pub mod network;
pub mod notebook;
pub mod ops;
pub mod petal;
pub mod search;
pub mod snapshot;
pub mod storage;
pub mod sync;
pub mod system;
pub mod vt;
pub mod zip;
pub mod export;
pub mod refs;
pub mod blockops;
pub mod vtmove;

use crate::state::AppState;
use axum::body::Bytes;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// SiYuan-style envelope `{code, msg, data}` (code 0 = success)
pub struct ApiResp {
    pub code: i32,
    pub msg: String,
    pub data: Value,
}

impl ApiResp {
    pub fn ok(data: Value) -> Self {
        ApiResp { code: 0, msg: String::new(), data }
    }
    pub fn err(code: i32, msg: &str) -> Self {
        ApiResp { code, msg: msg.into(), data: Value::Null }
    }
}

impl IntoResponse for ApiResp {
    fn into_response(self) -> Response {
        Json(json!({"code": self.code, "msg": self.msg, "data": self.data})).into_response()
    }
}

/// Parse a request body with permissive defaults (empty body -> Default).
/// Malformed JSON also falls back to Default but logs the rejection so
/// client-side encoding bugs are visible in the kernel log.
pub fn parse<T: DeserializeOwned + Default>(bytes: &Bytes) -> T {
    if bytes.is_empty() {
        T::default()
    } else {
        match serde_json::from_slice(bytes) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "request body parse failed: {} (body: {})",
                    e,
                    String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
                );
                T::default()
            }
        }
    }
}

/// Reject mutations in readonly mode
pub fn check_writable(st: &AppState) -> Option<ApiResp> {
    if st.conf.read().unwrap().readonly {
        return Some(ApiResp::err(-1, "readonly mode"));
    }
    None
}

pub fn rebuild_flag_set(st: &AppState) -> bool {
    st.rebuilding.load(Ordering::SeqCst)
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/api/system", system::routes())
        .nest("/api/notebook", notebook::routes())
        .nest("/api/filetree", filetree::routes())
        .nest("/api/file", file::routes())
        .nest("/api/block", block::routes())
        .nest("/api/outline", block::outline_routes())
        .nest("/api/search", search::routes())
        .nest("/api/format", format::routes())
        .nest("/api/attr", block::attr_routes())
        .nest("/api/block", blockops::routes())
        .nest("/api/storage", storage::routes())
        .nest("/api/vt", vt::routes())
        .nest("/api/vt", vtmove::routes())
        .nest("/api/clip", clip::routes())
        .nest("/api/bookmark", bookmark::routes())
        .nest("/api/history", history::routes())
        .nest("/api/asset", asset::routes())
        .nest("/api/network", network::routes())
        .nest("/api/ref", refs::routes())
        .nest("/api/graph", graph::routes())
        .nest("/api/repo", snapshot::routes())
        .nest("/api/petal", petal::routes())
        .nest("/api/sync", sync::routes())
        .nest("/api/export", export::routes())
        .nest("/api/index", index_routes())
}

fn index_routes() -> Router<Arc<AppState>> {
    use axum::extract::State;
    async fn rebuild(
        State(st): State<Arc<AppState>>,
        bytes: Bytes,
    ) -> ApiResp {
        let _ = parse::<Value>(&bytes);
        if rebuild_flag_set(&st) {
            return ApiResp::err(-1, "rebuild already running");
        }
        st.rebuilding.store(true, Ordering::SeqCst);
        let st2 = st.clone();
        tokio::task::spawn_blocking(move || {
            let db = st2.db.lock().unwrap();
            let n = crate::index::builder::rebuild_all_ext(
                &db,
                &st2.vault,
                &|rel| st2.is_ignored(rel),
                true,
                &|done, total| {
                    if done % 20 == 0 || done == total {
                        crate::ws::push_progress(done as u64, total as u64);
                    }
                },
            );
            drop(db);
            crate::ws::broadcast("index-done", json!({"count": n}));
            st2.rebuilding.store(false, Ordering::SeqCst);
        });
        ApiResp::ok(json!({"msg": "rebuild started"}))
    }
    Router::new().route("/rebuild", post(rebuild))
}

pub fn ws_routes() -> Router<Arc<AppState>> {
    Router::new().route("/ws", get(crate::ws::ws_handler))
}
