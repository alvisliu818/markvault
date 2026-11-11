pub mod builder;
pub mod search;

use rusqlite::Connection;

/// Schema modeled after SiYuan's sql/database.go but keyed on file paths.
/// Everything here is derived from markdown files and rebuildable.
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS docs(
  path TEXT PRIMARY KEY,
  notebook TEXT NOT NULL,
  title TEXT NOT NULL,
  hpath TEXT NOT NULL,
  created TEXT NOT NULL,
  updated TEXT NOT NULL,
  size INTEGER NOT NULL,
  fm_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_docs_notebook ON docs(notebook);

CREATE TABLE IF NOT EXISTS blocks(
  doc_path TEXT NOT NULL,
  block_id TEXT,
  type TEXT NOT NULL,
  text TEXT NOT NULL,
  line INTEGER NOT NULL,
  sort INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_blocks_doc ON blocks(doc_path);
CREATE INDEX IF NOT EXISTS idx_blocks_id ON blocks(block_id);

CREATE TABLE IF NOT EXISTS links(
  src_path TEXT NOT NULL,
  target TEXT NOT NULL,
  anchor TEXT,
  kind TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_links_src ON links(src_path);

CREATE VIRTUAL TABLE IF NOT EXISTS doc_fts USING fts5(
  path UNINDEXED, title, body, tokenize='unicode61 remove_diacritics 2'
);
"#;

pub fn open(db_path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}
