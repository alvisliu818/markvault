use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::sync::Arc;

/// Key-value JSON storage under `.markvault/storage/local/` —
/// frontend UI state (collapsed nodes, toolbar config, recents…).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/getVal", post(get_val))
        .route("/setVal", post(set_val))
}

fn sanitize_key(k: &str) -> String {
    k.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect()
}

#[derive(Deserialize, Default)]
struct GetValReq {
    #[serde(default)]
    key: String,
}

async fn get_val(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: GetValReq = parse(&bytes);
    let key = sanitize_key(req.key.trim());
    if key.is_empty() {
        return ApiResp::err(-1, "key required");
    }
    let path = st.storage_dir().join("local").join(format!("{}.json", key));
    match fs::read_to_string(&path) {
        Ok(s) => ApiResp::ok(serde_json::from_str::<Value>(&s).unwrap_or(Value::Null)),
        Err(_) => ApiResp::ok(Value::Null),
    }
}

#[derive(Deserialize, Default)]
struct SetValReq {
    #[serde(default)]
    key: String,
    #[serde(default)]
    val: Value,
}

async fn set_val(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: SetValReq = parse(&bytes);
    let key = sanitize_key(req.key.trim());
    if key.is_empty() {
        return ApiResp::err(-1, "key required");
    }
    let dir = st.storage_dir().join("local");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", key));
    match fs::write(&path, serde_json::to_string(&req.val).unwrap_or_default()) {
        Ok(_) => ApiResp::ok(json!({})),
        Err(e) => ApiResp::err(-1, &format!("write failed: {}", e)),
    }
}
