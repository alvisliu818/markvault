use super::{parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Kernel-side fetch for CORS-bound frontends (browser plugin data plane:
/// SiYuan /api/network/forwardProxy equivalent). Returns base64 body.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/forwardProxy", post(forward_proxy))
}

#[derive(Deserialize, Default)]
struct ProxyReq {
    #[serde(default)]
    url: String,
    #[serde(default)]
    headers: Map<String, Value>,
    /// seconds, default 30
    #[serde(default)]
    timeout: i64,
}

async fn forward_proxy(State(_st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: ProxyReq = parse(&bytes);
    if req.url.trim().is_empty() {
        return ApiResp::err(-1, "url required");
    }
    let timeout = if req.timeout <= 0 || req.timeout > 120 {
        30
    } else {
        req.timeout as u64
    };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ApiResp::err(-1, &format!("client: {}", e)),
    };
    let mut rb = client.get(req.url.trim());
    for (k, v) in &req.headers {
        if let (Ok(_), Some(vs)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            v.as_str(),
        ) {
            rb = rb.header(k.as_str(), vs);
        }
    }
    match rb.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            match resp.bytes().await {
                Ok(body) => {
                    use base64::Engine as _;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&body);
                    ApiResp::ok(json!({
                        "status": status,
                        "contentType": content_type,
                        "bodyBase64": b64,
                        "len": body.len(),
                    }))
                }
                Err(e) => ApiResp::err(-1, &format!("body: {}", e)),
            }
        }
        Err(e) => ApiResp::err(-1, &format!("fetch failed: {}", e)),
    }
}

#[allow(dead_code)]
fn _unused(_: HashMap<String, String>) {}
