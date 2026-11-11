use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::sync::Arc;

/// Bookmarks — the transferable data plane of the SiYuan browser plugin.
/// Persisted at `.markvault/storage/bookmarks.json`.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/list", post(list))
        .route("/add", post(add))
        .route("/remove", post(remove))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub created: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn load(st: &AppState) -> Vec<Bookmark> {
    let path = st.storage_dir().join("bookmarks.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(st: &AppState, list: &[Bookmark]) {
    let dir = st.storage_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(
        dir.join("bookmarks.json"),
        serde_json::to_string_pretty(list).unwrap_or_default(),
    );
}

async fn list(State(st): State<Arc<AppState>>) -> ApiResp {
    ApiResp::ok(json!(load(&st)))
}

#[derive(Deserialize, Default)]
struct AddReq {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    tags: Vec<String>,
}

async fn add(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: AddReq = parse(&bytes);
    if req.url.trim().is_empty() {
        return ApiResp::err(-1, "url required");
    }
    let mut list = load(&st);
    if list.iter().any(|b| b.url == req.url) {
        return ApiResp::err(-1, "bookmark exists");
    }
    list.push(Bookmark {
        url: req.url.trim().to_string(),
        title: if req.title.is_empty() { req.url.clone() } else { req.title },
        created: chrono::Local::now().format("%Y%m%d%H%M%S").to_string(),
        tags: req.tags,
    });
    save(&st, &list);
    ApiResp::ok(json!(list))
}

#[derive(Deserialize, Default)]
struct RemoveReq {
    #[serde(default)]
    url: String,
}

async fn remove(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: RemoveReq = parse(&bytes);
    let mut list = load(&st);
    list.retain(|b| b.url != req.url);
    save(&st, &list);
    ApiResp::ok(json!(list))
}
