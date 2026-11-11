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
/// Also stamps index_meta with the file's current mtime+size for fast boot.
pub fn upsert_doc(conn: &Connection, doc: &IndexedDoc, stamp: Option<(i64, i64)>) -> rusqlite::Result<()> {
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
    if let Some((mtime_ms, size)) = stamp {
        tx.execute(
            "INSERT INTO index_meta(path, mtime_ms, size) VALUES(?1,?2,?3)
             ON CONFLICT(path) DO UPDATE SET mtime_ms=excluded.mtime_ms, size=excluded.size",
            rusqlite::params![doc.path, mtime_ms, size],
        )?;
    }
    tx.commit()
}

pub fn remove_doc(conn: &Connection, path: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM docs WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM blocks WHERE doc_path = ?1", [path])?;
    conn.execute("DELETE FROM links WHERE src_path = ?1", [path])?;
    conn.execute("DELETE FROM doc_fts WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM index_meta WHERE path = ?1", [path])?;
    Ok(())
}

/// Walk the vault and rebuild everything. Returns number of docs indexed.
/// Incremental: files whose mtime+size match index_meta are skipped.
/// `force` = full rebuild (wipes index_meta first).
/// `progress` = optional callback (done, total) for WS progress reporting.
/// Walk the vault and rebuild everything.
/// Incremental: files whose mtime+size match index_meta are skipped.
/// `force` = full rebuild (wipes index_meta first).
/// `progress` = callback (done, total) for WS progress reporting.
pub fn rebuild_all_ext(
    conn: &Connection,
    vault: &Vault,
    is_ignored: &dyn Fn(&str) -> bool,
    force: bool,
    progress: &dyn Fn(usize, usize),
) -> usize {
    if force {
        conn.execute_batch(
            "DELETE FROM docs; DELETE FROM blocks; DELETE FROM links; DELETE FROM doc_fts; DELETE FROM index_meta;",
        )
        .ok();
    } else {
        // drop rows for files that vanished while we were away
        prune_missing(conn, vault);
    }
    // collect first for a deterministic total
    let mut targets: Vec<String> = Vec::new();
    walk(vault, vault.root.clone(), "", &mut |rel| {
        if !rel.ends_with(".md") || is_ignored(rel) {
            return;
        }
        targets.push(rel.to_string());
    });
    let total = targets.len();
    let mut count = 0usize;
    for (i, rel) in targets.iter().enumerate() {
        if !force {
            if let Some(stamp) = file_stamp(vault, rel) {
                let fresh: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM index_meta WHERE path = ?1 AND mtime_ms = ?2 AND size = ?3",
                        rusqlite::params![rel, stamp.0, stamp.1],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if fresh > 0 {
                    count += 1;
                    progress(i + 1, total);
                    continue;
                }
            }
        }
        if let Some(doc) = index_doc(vault, rel) {
            let stamp = file_stamp(vault, rel);
            if upsert_doc(conn, &doc, stamp).is_ok() {
                count += 1;
            }
        }
        progress(i + 1, total);
    }
    count
}

fn file_stamp(vault: &Vault, rel: &str) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(vault.abs(rel)?).ok()?;
    let mtime = meta.modified().ok()?;
    let ms = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some((ms, meta.len() as i64))
}

/// Public wrapper for ops.rs single-doc stamping.
pub fn file_stamp_pub(vault: &Vault, rel: &str) -> Option<(i64, i64)> {
    file_stamp(vault, rel)
}

/// Remove index rows for markdown files that no longer exist on disk.
fn prune_missing(conn: &Connection, vault: &Vault) {
    let stale: Vec<String> = {
        let mut stmt = match conn.prepare("SELECT path FROM docs") {
            Ok(s) => s,
            Err(_) => return,
        };
        let rows: Vec<String> = match stmt.query_map([], |r| r.get::<_, String>(0)) {
            Ok(m) => m.flatten().collect(),
            Err(_) => return,
        };
        rows
    };
    for p in stale {
        if !vault.exists(&p) {
            let _ = remove_doc(conn, &p);
        }
    }
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
/// (kept for upcoming /api/filetree/getDoc APIs; silence dead_code until then)
#[allow(dead_code)]
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
