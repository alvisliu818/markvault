use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Asset download — kernel-side fetch + save into the vault
/// (`<notebook>/assets/<filename>`), the embedded browser's
/// "save image/file to vault" endpoint (M6).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/downloadToLocal", post(download_to_local))
}

#[derive(Deserialize, Default)]
struct DlReq {
    #[serde(default)]
    url: String,
    #[serde(default)]
    notebook: String,
    /// target filename; empty = derive from URL basename
    #[serde(default)]
    filename: String,
}

/// Sanitize a filename for local storage: strip separators/control chars,
/// trim dots, cap length. Returns None if nothing usable remains.
fn sanitize_asset_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            _ => c,
        })
        .filter(|c| !c.is_control())
        .collect();
    let trimmed: String = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        return None;
    }
    // keep extension readable: cap total length at 120 chars
    let capped: String = trimmed.chars().take(120).collect();
    Some(capped)
}

/// Filename: explicit request name > decoded URL basename > timestamp fallback.
fn derive_filename(req_name: &str, url: &str) -> String {
    if let Some(n) = sanitize_asset_name(req_name) {
        return n;
    }
    // URL basename (decode %xx), mirroring siyuan-plugin-browser getFilenameFromUrl
    if let Some(last) = url.split("://").nth(1).and_then(|rest| {
        let path = rest.split_once('/').map(|(_, p)| p).unwrap_or("");
        let segs: Vec<&str> = path
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        segs.last().copied()
    }) {
        let decoded = percent_encoding::percent_decode_str(last)
            .decode_utf8()
            .map(|c| c.to_string())
            .unwrap_or_else(|_| last.to_string());
        if let Some(n) = sanitize_asset_name(&decoded) {
            return n;
        }
    }
    format!(
        "asset-{}",
        chrono::Local::now().format("%Y%m%d%H%M%S%3f")
    )
}

/// Avoid overwriting: insert -2, -3 ... before the extension.
fn dedupe_path(existing: impl Fn(&str) -> bool, name: &str) -> String {
    if !existing(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{}", e)),
        _ => (name.to_string(), String::new()),
    };
    for i in 2..10000 {
        let candidate = format!("{}-{}{}", stem, i, ext);
        if !existing(&candidate) {
            return candidate;
        }
    }
    format!("{}-{}{}", stem, chrono::Local::now().timestamp_millis(), ext)
}

async fn download_to_local(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: DlReq = parse(&bytes);
    if req.url.trim().is_empty() {
        return ApiResp::err(-1, "url required");
    }
    let notebook = if req.notebook.trim().is_empty() {
        match st.vault.ls_notebooks().first() {
            Some(n) => n.clone(),
            None => return ApiResp::err(-1, "no notebook available"),
        }
    } else {
        req.notebook.trim().to_string()
    };
    let name = derive_filename(&req.filename, req.url.trim());
    let rel = format!("{}/assets/{}", notebook, name);
    let final_rel = dedupe_path(
        |p| {
            let probe = format!("{}/assets/{}", notebook, p);
            st.vault.exists(&probe)
        },
        &name,
    );
    let rel = if final_rel == name {
        rel
    } else {
        format!("{}/assets/{}", notebook, final_rel)
    };

    let client = match reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) markvault-downloader")
        .timeout(std::time::Duration::from_secs(60))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ApiResp::err(-1, &format!("client: {}", e)),
    };
    let resp = match client.get(req.url.trim()).send().await {
        Ok(r) => r,
        Err(e) => return ApiResp::err(-1, &format!("fetch failed: {}", e)),
    };
    if !resp.status().is_success() {
        return ApiResp::err(-1, &format!("HTTP {}", resp.status()));
    }
    let data = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return ApiResp::err(-1, &format!("body: {}", e)),
    };
    match st.vault.write_file(&rel, &data) {
        Ok(_) => ApiResp::ok(json!({
            "path": rel,
            "filename": final_rel,
            "len": data.len(),
        })),
        Err(e) => ApiResp::err(-1, &format!("write: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_strips_separators_and_dots() {
        assert_eq!(sanitize_asset_name("a/b\\c:d.png").unwrap(), "a_b_c_d.png");
        assert_eq!(sanitize_asset_name("  ok file.jpg ").unwrap(), "ok file.jpg");
        // leading/trailing dots trimmed
        assert_eq!(sanitize_asset_name("..hidden.png..").unwrap(), "hidden.png");
        assert!(sanitize_asset_name("...").is_none());
        assert!(sanitize_asset_name("  ").is_none());
        // control chars removed
        assert_eq!(sanitize_asset_name("a\u{0000}b.png").unwrap(), "ab.png");
    }

    #[test]
    fn test_derive_filename_priority() {
        // explicit name wins
        assert_eq!(derive_filename("pic.png", "https://x.com/a/b.jpg"), "pic.png");
        // url basename with query/hash stripped, percent-decoded
        assert_eq!(
            derive_filename("", "https://x.com/dir/my%20pic.png?w=100#frag"),
            "my pic.png"
        );
        // no usable basename -> timestamp fallback
        let f = derive_filename("", "https://x.com/");
        assert!(f.starts_with("asset-"));
    }

    #[test]
    fn test_dedupe_path() {
        let names = vec!["a.png".to_string(), "a-2.png".to_string()];
        let exists = |p: &str| names.iter().any(|n| n == p);
        assert_eq!(dedupe_path(exists, "b.png"), "b.png");
        // a.png and a-2.png both exist -> next free suffix
        assert_eq!(dedupe_path(exists, "a.png"), "a-3.png");
        // no extension: suffix appended to stem
        let taken = ["file", "file-2"];
        assert_eq!(dedupe_path(|p| taken.contains(&p), "file"), "file-3");
    }
}
