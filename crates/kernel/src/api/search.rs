use super::{parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/fullTextSearchBlock", post(full_text_search))
}

#[derive(Deserialize, Default)]
struct SearchReq {
    #[serde(default)]
    query: String,
    #[serde(default)]
    limit: i64,
}

async fn full_text_search(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: SearchReq = parse(&bytes);
    if req.query.trim().is_empty() {
        return ApiResp::ok(json!([]));
    }
    let db = st.db.lock().unwrap();
    let limit = if req.limit <= 0 { 60 } else { req.limit as usize };
    let rows = crate::index::search::full_text_search(&db, req.query.trim(), limit);
    ApiResp::ok(json!(rows))
}
