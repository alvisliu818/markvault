use super::{parse, ApiResp};
use crate::auth;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/version", post(version))
        .route("/health", post(health))
        .route("/getConf", post(get_conf))
        .route("/setConf", post(set_conf))
        .route("/loginAuth", post(login_auth))
        .route("/logoutAuth", post(logout_auth))
        .route("/exit", post(exit))
        .route("/ping", post(ping))
}

async fn version() -> ApiResp {
    ApiResp::ok(json!({"version": env!("CARGO_PKG_VERSION"), "mode": "kernel"}))
}

async fn health() -> ApiResp {
    ApiResp::ok(json!({"ok": true}))
}

async fn ping() -> ApiResp {
    ApiResp::ok(json!("pong"))
}

async fn get_conf(State(st): State<Arc<AppState>>) -> ApiResp {
    let conf = st.conf.read().unwrap().clone();
    let data = serde_json::to_value(&conf).unwrap_or(Value::Null);
    ApiResp::ok(json!({"conf": data, "workspace": st.workspace.display().to_string()}))
}

#[derive(Deserialize, Default)]
struct SetConfReq {
    #[serde(default)]
    patch: Value,
}

async fn set_conf(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: SetConfReq = parse(&bytes);
    let patch = match req.patch.as_object() {
        Some(o) => o.clone(),
        None => return ApiResp::err(-1, "patch object required"),
    };
    {
        let mut conf = st.conf.write().unwrap();
        if let Some(v) = patch.get("port").and_then(|v| v.as_u64()) {
            conf.port = v as u16;
        }
        if let Some(v) = patch.get("networkServe").and_then(|v| v.as_bool()) {
            conf.network_serve = v;
        }
        if let Some(v) = patch.get("accessAuthCode") {
            conf.access_auth_code = v.as_str().unwrap_or("").to_string();
        }
        if let Some(v) = patch.get("apiToken") {
            conf.api_token = v.as_str().unwrap_or("").to_string();
        }
        if let Some(v) = patch.get("readonly").and_then(|v| v.as_bool()) {
            conf.readonly = v;
        }
        if let Some(v) = patch.get("theme").and_then(|v| v.as_u64()) {
            conf.theme = v as u8;
        }
        if let Some(v) = patch.get("parentAttr") {
            conf.parent_attr = v.as_str().unwrap_or("parent").to_string();
        }
        if let Some(v) = patch.get("weightAttr") {
            conf.weight_attr = v.as_str().unwrap_or("weight").to_string();
        }
    }
    st.save_conf();
    ApiResp::ok(json!({}))
}

#[derive(Deserialize, Default)]
struct LoginReq {
    #[serde(default)]
    auth_code: String,
}

async fn login_auth(State(st): State<Arc<AppState>>, bytes: Bytes) -> Response {
    use axum::response::IntoResponse;
    let req: LoginReq = parse(&bytes);
    let conf = st.conf.read().unwrap().clone();
    let mut resp = ApiResp::ok(json!({})).into_response();
    if conf.access_auth_code.is_empty() || req.auth_code == conf.access_auth_code {
        if let Ok(hv) = header::HeaderValue::from_str(&auth::login_cookie_header(&st.session_key)) {
            resp.headers_mut().append(header::SET_COOKIE, hv);
        }
        resp
    } else {
        use axum::response::IntoResponse;
        (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(json!({"code": -1, "msg": "wrong auth code", "data": null})),
        )
            .into_response()
    }
}

async fn logout_auth() -> Response {
    use axum::response::IntoResponse;
    let mut resp = ApiResp::ok(json!({})).into_response();
    if let Ok(hv) = header::HeaderValue::from_str(&auth::logout_cookie_header()) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

async fn exit(State(st): State<Arc<AppState>>) -> ApiResp {
    let _ = st.shutdown_tx.send(true);
    ApiResp::ok(json!({}))
}
