use crate::state::AppState;

/// Reindex a single doc after a file write.
pub fn reindex_one(st: &AppState, path: &str) {
    let db = st.db.lock().unwrap();
    match crate::index::builder::index_doc(&st.vault, path) {
        Some(doc) => {
            let _ = crate::index::builder::upsert_doc(&db, &doc);
        }
        None => {
            let _ = crate::index::builder::remove_doc(&db, path);
        }
    }
    crate::ws::push_doc_reindexed(path);
}

/// Remove index rows for a deleted doc (and its sub-tree if directory).
pub fn unindex(st: &AppState, path: &str) {
    let db = st.db.lock().unwrap();
    let prefix = format!("{}%", path.trim_end_matches(".md"));
    let _ = db.execute(
        "DELETE FROM docs WHERE path = ?1 OR path LIKE ?2",
        rusqlite::params![path, prefix],
    );
    let _ = db.execute(
        "DELETE FROM blocks WHERE doc_path = ?1 OR doc_path LIKE ?2",
        rusqlite::params![path, prefix],
    );
    let _ = db.execute(
        "DELETE FROM links WHERE src_path = ?1 OR src_path LIKE ?2",
        rusqlite::params![path, prefix],
    );
    let _ = db.execute(
        "DELETE FROM doc_fts WHERE path = ?1 OR path LIKE ?2",
        rusqlite::params![path, prefix],
    );
}

/// Reindex every .md under a prefix after rename/move.
pub fn reindex_prefix(st: &AppState, old_prefix: &str, new_prefix: &str) {
    unindex(st, old_prefix);
    let db = st.db.lock().unwrap();
    let scan_prefix = new_prefix.trim_end_matches(".md").to_string();
    let mut paths: Vec<String> = Vec::new();
    collect_md_paths(&st.vault, &scan_prefix, &mut paths);
    for p in &paths {
        if let Some(doc) = crate::index::builder::index_doc(&st.vault, p) {
            let _ = crate::index::builder::upsert_doc(&db, &doc);
        }
    }
}

fn collect_md_paths(vault: &crate::vault::Vault, rel_prefix: &str, out: &mut Vec<String>) {
    let abs = match vault.abs(rel_prefix) {
        Some(p) => p,
        None => return,
    };
    if abs.is_file() {
        out.push(rel_prefix.to_string());
        return;
    }
    if let Ok(rd) = std::fs::read_dir(&abs) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let child = format!("{}/{}", rel_prefix.trim_end_matches('/'), name);
            if e.path().is_dir() {
                collect_md_paths(vault, &child, out);
            } else if name.ends_with(".md") {
                out.push(child);
            }
        }
    }
}

/// Broadcast filetree + vt change events
pub fn notify_tree() {
    crate::ws::push_filetree_changed();
    crate::ws::push_vt_changed();
}
