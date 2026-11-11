use crate::parse::{self, ParseBlock};
use crate::vault::{self, Vault};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

/// Parsed doc record ready for insertion
pub struct IndexedDoc {
    pub path: String,
    pub notebook: String,
    pub title: String,
    pub hpath: String,
    pub created: String,
    pub updated: String,
    pub size: i64,
    pub fm: Map<String, Value>,
    pub blocks: Vec<ParseBlock>,
    pub links: Vec<parse::ParseLink>,
    pub body_text: String,
}

/// Read + parse one markdown file into an IndexedDoc.
/// `title` falls back to first h1, then file basename.
pub fn index_doc(vault: &Vault, path: &str) -> Option<IndexedDoc> {
    let raw = vault.read_file(path)?;
    let content = String::from_utf8_lossy(&raw).to_string();
    let (fm, body_offset) = vault::extract_frontmatter(&content);
    let body = &content[body_offset.min(content.len())..];
    // line base = number of newlines before the body (byte offset ≠ line number)
    let line_base = content[..body_offset.min(content.len())].matches('\n').count();

    let blocks = parse::parse_blocks(body, line_base);
    let links = parse::parse_wikilinks(body);

    let title_fm = vault::fm_str(&fm, "title");
    let title = title_fm
        .or_else(|| {
            blocks
                .iter()
                .find(|b| b.r#type == "h1")
                .map(|b| b.text.clone())
        })
        .unwrap_or_else(|| {
            Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });

    let notebook = path.split('/').next().unwrap_or("").to_string();
    let hpath = format!("/{}", path.trim_end_matches(".md"));

    let meta = fs::metadata(vault.abs(path)?).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let created = vault::fm_str(&fm, "created")
        .or_else(|| mtime.map(vault::ts_from_mtime))
        .unwrap_or_default();
    let updated = vault::fm_str(&fm, "updated")
        .or_else(|| mtime.map(vault::ts_from_mtime))
        .unwrap_or_default();

    Some(IndexedDoc {
        path: path.to_string(),
        notebook,
        title,
        hpath,
        created,
        updated,
        size: raw.len() as i64,
        fm,
        blocks,
        links,
        body_text: parse::plain_text(body),
    })
}

/// Replace a doc's rows in all index tables (single transaction).
pub fn upsert_doc(conn: &Connection, doc: &IndexedDoc) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM docs WHERE path = ?1", [&doc.path])?;
    tx.execute("DELETE FROM blocks WHERE doc_path = ?1", [&doc.path])?;
    tx.execute("DELETE FROM links WHERE src_path = ?1", [&doc.path])?;
    tx.execute("DELETE FROM doc_fts WHERE path = ?1", [&doc.path])?;

    tx.execute(
        "INSERT INTO docs(path,notebook,title,hpath,created,updated,size,fm_json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![
            doc.path,
            doc.notebook,
            doc.title,
            doc.hpath,
            doc.created,
            doc.updated,
            doc.size,
            serde_json::to_string(&doc.fm).unwrap_or_default()
        ],
    )?;

    for (i, b) in doc.blocks.iter().enumerate() {
        tx.execute(
            "INSERT INTO blocks(doc_path,block_id,type,text,line,sort) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![doc.path, b.block_id, b.r#type, b.text, b.line as i64, i as i64],
        )?;
    }
    for l in &doc.links {
        tx.execute(
            "INSERT INTO links(src_path,target,anchor,kind) VALUES(?1,?2,?3,?4)",
            rusqlite::params![doc.path, l.target, l.anchor, "wiki"],
        )?;
    }
    tx.execute(
        "INSERT INTO doc_fts(path,title,body) VALUES(?1,?2,?3)",
        rusqlite::params![doc.path, doc.title, doc.body_text],
    )?;
    tx.commit()
}

pub fn remove_doc(conn: &Connection, path: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM docs WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM blocks WHERE doc_path = ?1", [path])?;
    conn.execute("DELETE FROM links WHERE src_path = ?1", [path])?;
    conn.execute("DELETE FROM doc_fts WHERE path = ?1", [path])?;
    Ok(())
}

/// Walk the vault and rebuild everything. Returns number of docs indexed.
pub fn rebuild_all(conn: &Connection, vault: &Vault, is_ignored: &dyn Fn(&str) -> bool) -> usize {
    conn.execute_batch("DELETE FROM docs; DELETE FROM blocks; DELETE FROM links; DELETE FROM doc_fts;")
        .ok();
    let mut count = 0usize;
    walk(vault, vault.root.clone(), "", &mut |rel| {
        if !rel.ends_with(".md") || is_ignored(rel) {
            return;
        }
        if let Some(doc) = index_doc(vault, &rel) {
            if upsert_doc(conn, &doc).is_ok() {
                count += 1;
            }
        }
    });
    count
}

fn walk(vault: &Vault, dir: std::path::PathBuf, rel_prefix: &str, f: &mut dyn FnMut(&str)) {
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let rel = if rel_prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", rel_prefix, name)
            };
            let p = e.path();
            if p.is_dir() {
                walk(vault, p, &rel, f);
            } else {
                f(&rel);
            }
        }
    }
}

/// Serialize doc row summary used by filetree / vt APIs
pub fn doc_summary(doc: &IndexedDoc) -> Value {
    json!({
        "path": doc.path,
        "notebook": doc.notebook,
        "title": doc.title,
        "hpath": doc.hpath,
        "created": doc.created,
        "updated": doc.updated,
        "size": doc.size,
        "attrs": doc.fm,
    })
}
