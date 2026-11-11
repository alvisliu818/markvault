use rusqlite::Connection;
use serde_json::{json, Value};

/// Full-text search over doc_fts. Query is sanitized to a safe FTS5 MATCH
/// expression (quoted tokens joined by AND, each with prefix match).
pub fn full_text_search(conn: &Connection, query: &str, limit: usize) -> Vec<Value> {
    let q = sanitize_fts_query(query);
    if q.is_empty() {
        return Vec::new();
    }
    let sql = format!(
        "SELECT f.path, d.title, d.hpath, d.notebook, d.updated,
                snippet(doc_fts, 2, '<mark>', '</mark>', '…', 12) AS snip
         FROM doc_fts f
         LEFT JOIN docs d ON d.path = f.path
         WHERE doc_fts MATCH ?1
         ORDER BY bm25(doc_fts)
         LIMIT {limit}"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([&q], |r| {
        Ok(json!({
            "path": r.get::<_, String>(0)?,
            "title": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            "hpath": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            "notebook": r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            "updated": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            "snippet": r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        }))
    });
    match rows {
        Ok(it) => it.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| {
            let t = t.replace('"', "");
            if t.is_empty() {
                String::new()
            } else {
                format!("\"{}\"*", t)
            }
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Title/path search for filetree.searchDocs
pub fn search_docs(conn: &Connection, kw: &str, limit: usize) -> Vec<Value> {
    let like = format!("%{}%", kw.replace('%', ""));
    let mut stmt = match conn.prepare(
        "SELECT path, title, hpath, notebook FROM docs
         WHERE title LIKE ?1 OR path LIKE ?1
         ORDER BY title LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![like, limit as i64], |r| {
        Ok(json!({
            "path": r.get::<_, String>(0)?,
            "title": r.get::<_, String>(1)?,
            "hpath": r.get::<_, String>(2)?,
            "notebook": r.get::<_, String>(3)?,
        }))
    });
    match rows {
        Ok(it) => it.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}
