use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

/// Petal plugin system (M11, SiYuan-style).
/// Registry:  `.markvault/petal/petals.json`  = [{name, enabled}]
/// Conf:      `.markvault/petal/<name>/conf.json`
/// Static:    `.markvault/petal/<name>/` served read-only at `/petal/<name>/`
///            (index.js entry; loaded by the SPA's dynamic module loader).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/loadPetals", post(load_petals))
        .route("/setPetals", post(set_petals))
        .route("/getPetalConf", post(get_petal_conf))
        .route("/setPetalConf", post(set_petal_conf))
}

fn petal_root(st: &AppState) -> PathBuf {
    crate::conf::markvault_dir(&st.vault.root).join("petal")
}

/// Plugin dir names are restricted to a safe charset (they become URL/path segments).
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

fn read_registry(st: &AppState) -> Vec<Value> {
    let path = petal_root(st).join("petals.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

fn write_registry(st: &AppState, list: &[Value]) -> Result<(), String> {
    let dir = petal_root(st);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(
        dir.join("petals.json"),
        serde_json::to_string_pretty(list).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())
}

async fn load_petals(State(st): State<Arc<AppState>>, _bytes: Bytes) -> ApiResp {
    ApiResp::ok(json!({ "petals": read_registry(&st) }))
}

#[derive(Deserialize, Default)]
struct SetPetalsReq {
    #[serde(default)]
    petals: Vec<Value>,
}

async fn set_petals(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: SetPetalsReq = parse(&bytes);
    // normalize + validate entries
    let mut list: Vec<Value> = Vec::new();
    for p in &req.petals {
        let Some(name) = p.get("name").and_then(|v| v.as_str()) else { continue };
        if !valid_name(name) {
            return ApiResp::err(-1, &format!("invalid petal name: {}", name));
        }
        list.push(json!({
            "name": name,
            "enabled": p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
        }));
    }
    match write_registry(&st, &list) {
        Ok(_) => ApiResp::ok(json!({ "petals": list })),
        Err(e) => ApiResp::err(-1, &e),
    }
}

#[derive(Deserialize, Default)]
struct NameReq {
    #[serde(default)]
    name: String,
}

async fn get_petal_conf(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: NameReq = parse(&bytes);
    if !valid_name(&req.name) {
        return ApiResp::err(-1, "invalid petal name");
    }
    let path = petal_root(&st).join(&req.name).join("conf.json");
    match fs::read_to_string(path) {
        Ok(s) => ApiResp::ok(serde_json::from_str::<Value>(&s).unwrap_or(Value::Null)),
        Err(_) => ApiResp::ok(Value::Null),
    }
}

#[derive(Deserialize, Default)]
struct SetConfReq {
    #[serde(default)]
    name: String,
    #[serde(default)]
    conf: Value,
}

async fn set_petal_conf(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: SetConfReq = parse(&bytes);
    if !valid_name(&req.name) {
        return ApiResp::err(-1, "invalid petal name");
    }
    let dir = petal_root(&st).join(&req.name);
    fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| {
        fs::write(dir.join("conf.json"), serde_json::to_string_pretty(&req.conf).unwrap_or_default())
            .map_err(|e| e.to_string())
    })
    .map(|_| ApiResp::ok(json!({})))
    .unwrap_or_else(|e| ApiResp::err(-1, &e))
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    #[test]
    fn test_valid_name() {
        assert!(valid_name("sample-petal"));
        assert!(valid_name("a1"));
        assert!(valid_name("upper"));
        assert!(!valid_name(""));
        assert!(!valid_name("-lead"));
        assert!(!valid_name("has space"));
        assert!(!valid_name("Upp"));
        assert!(!valid_name("../escape"));
        assert!(!valid_name(&"a".repeat(65)));
    }
}
