use super::{check_writable, parse, zip::ZipWriter, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/exportMd", post(export_md))
        .route("/exportNotebookMd", post(export_notebook_md))
}

#[derive(Deserialize, Default)]
struct ExportDocReq {
    #[serde(default)]
    path: String,
}

fn md_response(rel: &str, data: Vec<u8>) -> Response {
    let name = rel
        .rsplit('/')
        .next()
        .unwrap_or("doc.md")
        .trim_end_matches(".md")
        .to_string();
    (
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{}.md\"", encode_star(&name)),
            ),
        ],
        data,
    )
        .into_response()
}

/// RFC 5987 filename* for non-ASCII names; ASCII fallback keeps curl happy.
fn encode_star(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii() && !b.is_ascii_control() && b != b'"' && b != b'\\' && b != b' ' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

async fn export_md(State(st): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let req: ExportDocReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    if rel.is_empty() || !rel.ends_with(".md") {
        return ApiResp::err(-1, "path (.md) required").into_response();
    }
    match st.vault.read_file(rel) {
        Some(data) => md_response(rel, data),
        None => (StatusCode::NOT_FOUND, "file not found").into_response(),
    }
}

#[derive(Deserialize, Default)]
struct ExportNbReq {
    #[serde(default)]
    notebook: String,
}

fn sanitize_zip_entry(rel: &str) -> Option<String> {
    // refuse traversal and absolute components
    if rel.split('/').any(|seg| seg == ".." || seg.is_empty() && !rel.ends_with('/')) {
        return None;
    }
    Some(rel.to_string())
}

async fn export_notebook_md(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: ExportNbReq = parse(&bytes);
    let nb = req.notebook.trim_matches('/').to_string();
    if nb.is_empty() {
        return ApiResp::err(-1, "notebook required");
    }
    if !st.vault.is_dir(&nb) {
        return ApiResp::err(-1, "notebook not found");
    }
    let root = match st.vault.abs(&nb) {
        Some(p) => p,
        None => return ApiResp::err(-1, "invalid notebook path"),
    };

    let mut zw = ZipWriter::new();
    let mut count = 0usize;
    fn walk(dir: &std::path::Path, prefix: &str, zw: &mut ZipWriter, count: &mut usize) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let entry_rel = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                let p = e.path();
                if p.is_dir() {
                    walk(&p, &entry_rel, zw, count);
                } else if let Some(rel) = sanitize_zip_entry(&entry_rel) {
                    if let Ok(data) = std::fs::read(&p) {
                        zw.add(&format!("{}/{}", "export", rel), &data);
                        *count += 1;
                    }
                }
            }
        }
    }
    walk(&root, "", &mut zw, &mut count);

    // write into .markvault/export/
    let out_rel = format!(".markvault/export/{}.zip", sanitize_zip_name(&nb));
    let _ = st.vault.mkdir(".markvault/export");
    let payload = zw.finish();
    match st.vault.write_file(&out_rel, &payload) {
        Ok(_) => ApiResp::ok(json!({
            "path": format!("/{}", out_rel),
            "count": count,
            "bytes": payload.len(),
        })),
        Err(e) => ApiResp::err(-1, &format!("write zip failed: {}", e)),
    }
}

fn sanitize_zip_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    if cleaned.is_empty() {
        "notebook".to_string()
    } else {
        cleaned
    }
}
