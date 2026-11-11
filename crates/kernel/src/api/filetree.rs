use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use crate::vault::sanitize_filename;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/listDocsByPath", post(list_docs_by_path))
        .route("/listDocTree", post(list_doc_tree))
        .route("/createDocWithMd", post(create_doc_with_md))
        .route("/renameDoc", post(rename_doc))
        .route("/removeDoc", post(remove_doc))
        .route("/moveDocs", post(move_docs))
        .route("/searchDocs", post(search_docs))
        .route("/getHPathByPath", post(get_hpath_by_path))
}

#[derive(Deserialize, Default)]
struct ListReq {
    #[serde(default)]
    notebook: String,
    /// hpath-style dir: "/sub/dir" ("" or "/" = notebook root)
    #[serde(default)]
    path: String,
}

/// List folders + docs directly under a directory.
pub async fn list_docs_by_path(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: ListReq = parse(&bytes);
    if req.notebook.is_empty() {
        return ApiResp::err(-1, "notebook required");
    }
    let dir_rel = format!(
        "{}{}",
        req.notebook.trim_end_matches('/'),
        if req.path.is_empty() || req.path == "/" {
            String::new()
        } else {
            req.path.trim_end_matches('/').to_string()
        }
    );
    if !st.vault.is_dir(&dir_rel) {
        return ApiResp::err(-1, "path not found");
    }

    let db = st.db.lock().unwrap();
    let mut files: Vec<Value> = Vec::new();

    // folders
    if let Ok(rd) = std::fs::read_dir(st.vault.abs(&dir_rel).unwrap()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let child_rel = format!("{}/{}", dir_rel, name);
            if e.path().is_dir() {
                files.push(json!({
                    "path": format!("/{}", child_rel),
                    "name": name,
                    "type": "folder",
                    "subFileCount": count_md_direct(&db, &child_rel),
                }));
            } else if name.ends_with(".md") && !st.is_ignored(&child_rel) {
                let doc = db
                    .query_row(
                        "SELECT title, created, updated, size FROM docs WHERE path = ?1",
                        [&child_rel],
                        |r| {
                            Ok(json!({
                                "path": format!("/{}", child_rel),
                                "name": name,
                                "title": r.get::<_, String>(0)?,
                                "created": r.get::<_, String>(1)?,
                                "updated": r.get::<_, String>(2)?,
                                "size": r.get::<_, i64>(3)?,
                                "type": "doc",
                                "subFileCount": 0,
                            }))
                        },
                    )
                    .unwrap_or_else(|_| {
                        json!({
                            "path": format!("/{}", child_rel),
                            "name": name,
                            "title": name.trim_end_matches(".md"),
                            "type": "doc",
                            "subFileCount": 0,
                        })
                    });
                files.push(doc);
            }
        }
    }

    files.sort_by(|a, b| {
        let ta = a["type"].as_str().unwrap_or("");
        let tb = b["type"].as_str().unwrap_or("");
        tb.cmp(ta).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
        })
    });

    ApiResp::ok(json!({"notebook": req.notebook, "path": req.path, "files": files}))
}

/// Direct-child doc count (Obsidian file-explorer semantics): only .md docs
/// one level below the folder, not the whole recursive subtree.
fn count_md_direct(db: &rusqlite::Connection, rel_dir: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM docs WHERE path LIKE ?1 AND path NOT LIKE ?2",
        [format!("{}/%", rel_dir), format!("{}/%/%", rel_dir)],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

async fn list_doc_tree(State(st): State<Arc<AppState>>) -> ApiResp {
    let db = st.db.lock().unwrap();
    let mut out = Vec::new();
    if let Ok(mut stmt) = db.prepare(
        "SELECT path, title, notebook, hpath, created, updated FROM docs ORDER BY path",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(json!({
                "path": format!("/{}", r.get::<_, String>(0)?),
                "title": r.get::<_, String>(1)?,
                "notebook": r.get::<_, String>(2)?,
                "hpath": r.get::<_, String>(3)?,
                "created": r.get::<_, String>(4)?,
                "updated": r.get::<_, String>(5)?,
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    ApiResp::ok(json!({"docs": out}))
}

#[derive(Deserialize, Default)]
struct CreateDocReq {
    #[serde(default)]
    notebook: String,
    /// hpath like "/dir/title"; the last segment becomes the filename
    #[serde(default)]
    path: String,
    #[serde(default)]
    markdown: String,
}

pub async fn create_doc_with_md(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: CreateDocReq = parse(&bytes);
    if req.notebook.is_empty() {
        return ApiResp::err(-1, "notebook required");
    }
    match create_doc_internal(&st, &req.notebook, &req.path, &req.markdown) {
        Ok(p) => {
            super::ops::notify_tree();
            ApiResp::ok(json!({"path": p}))
        }
        Err(e) => ApiResp::err(-1, &e),
    }
}

/// Shared doc-creation logic (API handler + clipper). hpath like "/dir/title".
pub fn create_doc_internal(
    st: &Arc<AppState>,
    notebook: &str,
    hpath: &str,
    markdown: &str,
) -> Result<String, String> {
    let parts: Vec<&str> = hpath.split('/').filter(|s| !s.is_empty()).collect();
    let title = parts
        .last()
        .map(|t| sanitize_filename(t))
        .ok_or_else(|| "path required".to_string())?;
    let dir_segments = &parts[..parts.len().saturating_sub(1)];
    let mut rel = notebook.trim_end_matches('/').to_string();
    for seg in dir_segments {
        rel.push('/');
        rel.push_str(&sanitize_filename(seg));
    }
    rel.push('/');
    rel.push_str(&title);
    rel.push_str(".md");

    if st.vault.exists(&rel) {
        return Err("doc exists".into());
    }
    let md = if markdown.trim().is_empty() {
        format!("# {}\n\n", title)
    } else {
        markdown.to_string()
    };
    st.vault
        .write_file(&rel, md.as_bytes())
        .map_err(|e| format!("write failed: {}", e))?;
    super::ops::reindex_one(st, &rel);
    Ok(format!("/{}", rel))
}

#[derive(Deserialize, Default)]
struct RenameDocReq {
    #[serde(default)]
    path: String,
    #[serde(default)]
    title: String,
}

pub async fn rename_doc(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: RenameDocReq = parse(&bytes);
    let old_rel = req.path.trim_start_matches('/');
    let title = sanitize_filename(req.title.trim());
    if old_rel.is_empty() || title.is_empty() {
        return ApiResp::err(-1, "path/title required");
    }
    if !st.vault.exists(old_rel) {
        return ApiResp::err(-1, "doc not found");
    }
    let parent = match old_rel.rsplit_once('/') {
        Some((p, _)) => p.to_string(),
        None => String::new(),
    };
    let mut new_rel = if parent.is_empty() {
        title.clone()
    } else {
        format!("{}/{}", parent, title)
    };
    new_rel.push_str(".md");
    if new_rel != old_rel {
        if st.vault.exists(&new_rel) {
            return ApiResp::err(-1, "target exists");
        }
        if let Err(e) = st.vault.rename(old_rel, &new_rel) {
            return ApiResp::err(-1, &format!("rename failed: {}", e));
        }
        super::ops::reindex_prefix(&st, old_rel, &new_rel);
        let mut vt = crate::vt::VtStore::load(&st);
        crate::vt::sync_rename(&mut vt, old_rel, &new_rel);
        vt.save(&st);
    }
    super::ops::notify_tree();
    ApiResp::ok(json!({"path": format!("/{}", new_rel)}))
}

#[derive(Deserialize, Default)]
struct RemoveDocReq {
    #[serde(default)]
    path: String,
}

pub async fn remove_doc(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: RemoveDocReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/').to_string();
    if rel.is_empty() {
        return ApiResp::err(-1, "path required");
    }
    match st.vault.trash(&rel) {
        Ok(dest) => {
            super::ops::unindex(&st, &rel);
            let mut vt = crate::vt::VtStore::load(&st);
            crate::vt::sync_delete(&mut vt, &rel);
            vt.save(&st);
            super::ops::notify_tree();
            ApiResp::ok(json!({"trashPath": dest}))
        }
        Err(e) => ApiResp::err(-1, &format!("remove failed: {}", e)),
    }
}

#[derive(Deserialize, Default)]
struct MoveReq {
    #[serde(default)]
    from_paths: Vec<String>,
    /// target directory hpath ("" = move to notebook root handled by full path form)
    #[serde(default)]
    to_path: String,
}

pub async fn move_docs(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: MoveReq = parse(&bytes);
    if req.from_paths.is_empty() {
        return ApiResp::err(-1, "fromPaths required");
    }
    let to_dir = req.to_path.trim_start_matches('/').trim_end_matches('/');
    for from in &req.from_paths {
        let from_rel = from.trim_start_matches('/');
        let name = match from_rel.rsplit('/').next() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let new_rel = if to_dir.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", to_dir, name)
        };
        if new_rel == from_rel {
            continue;
        }
        if st.vault.exists(&new_rel) {
            return ApiResp::err(-1, &format!("target exists: {}", new_rel));
        }
        if let Err(e) = st.vault.rename(from_rel, &new_rel) {
            return ApiResp::err(-1, &format!("move failed: {}", e));
        }
        super::ops::reindex_prefix(&st, from_rel, &new_rel);
        let mut vt = crate::vt::VtStore::load(&st);
        crate::vt::sync_rename(&mut vt, from_rel, &new_rel);
        vt.save(&st);
    }
    super::ops::notify_tree();
    ApiResp::ok(json!({}))
}

#[derive(Deserialize, Default)]
struct KwReq {
    #[serde(default)]
    k: String,
}

async fn search_docs(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: KwReq = parse(&bytes);
    if req.k.trim().is_empty() {
        return ApiResp::ok(json!({"docs": []}));
    }
    let db = st.db.lock().unwrap();
    let docs = crate::index::search::search_docs(&db, req.k.trim(), 100);
    ApiResp::ok(json!({"docs": docs}))
}

#[derive(Deserialize, Default)]
struct HPathReq {
    #[serde(default)]
    path: String,
}

async fn get_hpath_by_path(State(_st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: HPathReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    let hpath = format!("/{}", rel.trim_end_matches(".md"));
    ApiResp::ok(json!({"hpath": hpath}))
}
