use crate::state::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use std::sync::Arc;

pub const SESSION_COOKIE: &str = "mv_session";

fn session_value(key: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac");
    mac.update(b"authed");
    hex::encode(mac.finalize().into_bytes())
}

pub fn login_cookie_header(key: &[u8]) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=7776000",
        SESSION_COOKIE,
        session_value(key)
    )
}

pub fn logout_cookie_header() -> String {
    format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0", SESSION_COOKIE)
}

fn header_value(s: &str) -> header::HeaderValue {
    header::HeaderValue::from_str(s).unwrap_or(header::HeaderValue::from_static(""))
}

fn has_valid_session(key: &[u8], cookies: &str) -> bool {
    let want = session_value(key);
    cookies.split(';').any(|c| {
        let c = c.trim();
        c == format!("{}={}", SESSION_COOKIE, want)
    })
}

fn is_api(req: &Request<Body>) -> bool {
    req.uri().path().starts_with("/api")
}

fn exempt(req: &Request<Body>) -> bool {
    let p = req.uri().path();
    matches!(
        p,
        "/api/system/loginAuth"
            | "/api/system/version"
            | "/api/system/health"
            | "/check-auth"
    )
}

fn local_request(req: &Request<Body>) -> bool {
    req.headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.starts_with("127.0.0.1") || h.starts_with("[::1]"))
        .unwrap_or(true)
}

pub fn api_error(code: u16, msg: &str) -> Response {
    let body = json!({"code": code, "msg": msg, "data": null});
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(body),
    )
        .into_response()
}

/// Auth middleware: token header > valid session cookie > local trust.
/// Mirrors SiYuan's model.CheckAuth flow (simplified).
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if exempt(&req) {
        return next.run(req).await;
    }

    let conf = state.conf.read().unwrap().clone();

    // 1. API token
    if !conf.api_token.is_empty() {
        if let Some(h) = req.headers().get(header::AUTHORIZATION) {
            if let Ok(h) = h.to_str() {
                let token = h
                    .strip_prefix("Token ")
                    .or_else(|| h.strip_prefix("Bearer "))
                    .unwrap_or("");
                if token == conf.api_token {
                    return next.run(req).await;
                }
            }
        }
    }

    // 2. access auth code (伺服 lock). Local requests are trusted.
    if conf.access_auth_code.is_empty() || local_request(&req) {
        return next.run(req).await;
    }

    let cookies = req
        .headers()
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .unwrap_or("");
    if has_valid_session(&state.session_key, cookies) {
        return next.run(req).await;
    }

    if is_api(&req) {
        api_error(401, "unauthenticated")
    } else {
        Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "/check-auth")
            .body(Body::empty())
            .unwrap()
    }
}

/// Simple lock-screen page served at /check-auth (SiYuan stage/auth.html equivalent)
pub async fn check_auth_page() -> impl IntoResponse {
    axum::response::Html(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><title>markvault</title>
<style>
body{display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#1e1e28;color:#dcdfe4;font-family:system-ui}
.box{width:320px;padding:32px;background:#282a36;border-radius:8px}
input{width:100%;box-sizing:border-box;padding:10px;margin:16px 0;border:1px solid #44475a;border-radius:6px;background:#1e1e28;color:#dcdfe4}
button{width:100%;padding:10px;border:0;border-radius:6px;background:#7c6f64;color:#fff;cursor:pointer}
.err{color:#f38ba8;font-size:12px;min-height:16px}
</style></head>
<body><div class="box"><h3>markvault 访问验证</h3>
<input id="code" type="password" placeholder="访问授权码" onkeydown="if(event.key==='Enter')go()">
<div class="err" id="err"></div>
<button onclick="go()">进入</button></div>
<script>
async function go(){
  const r = await fetch('/api/system/loginAuth',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({authCode:document.getElementById('code').value})});
  const j = await r.json();
  if(j.code===0){location.href='/'}else{document.getElementById('err').textContent='授权码错误'}
}
</script></body></html>"#,
    )
}
