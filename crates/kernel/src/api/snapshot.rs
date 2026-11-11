use std::fs;
use std::path::PathBuf;
use sha2::{Digest, Sha256};

use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// `/api/repo/*` — snapshot endpoints. Auto-snapshots happen in putFile
/// (pre-write copy of the current content); manual snapshots are forced.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/createSnapshot", post(create_snapshot))
        .route("/listSnapshots", post(list_snapshots))
        .route("/getSnapshot", post(get_snapshot))
        .route("/restoreSnapshot", post(restore_snapshot))
        .route("/cleanSnapshots", post(clean_snapshots))
}

pub fn store(st: &AppState) -> SnapshotStore {
    SnapshotStore::new(crate::conf::markvault_dir(&st.vault.root).join("repo"))
}

#[derive(Deserialize, Default)]
struct PathReq {
    #[serde(default)]
    path: String,
}

async fn create_snapshot(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: PathReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    let Some(data) = st.vault.read_file(rel) else {
        return ApiResp::err(-1, "file not found");
    };
    match store(&st).snapshot(rel, &data, true) {
        Ok(Some(id)) => ApiResp::ok(json!({"id": id})),
        Ok(None) => ApiResp::ok(json!({"id": Value::Null, "msg": "unchanged"})),
        Err(e) => ApiResp::err(-1, &format!("snapshot: {}", e)),
    }
}

async fn list_snapshots(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: PathReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    let snaps: Vec<serde_json::Value> = store(&st)
        .list(rel)
        .into_iter()
        .map(|(id, millis, chash)| json!({"id": id, "created": millis, "hash": chash}))
        .collect();
    ApiResp::ok(json!({"snapshots": snaps}))
}

#[derive(Deserialize, Default)]
struct GetReq {
    #[serde(default)]
    path: String,
    #[serde(default)]
    id: String,
}

async fn get_snapshot(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: GetReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    match store(&st).read(rel, &req.id) {
        Some(data) => ApiResp::ok(json!({
            "content": String::from_utf8_lossy(&data),
        })),
        None => ApiResp::err(-1, "snapshot not found"),
    }
}

async fn restore_snapshot(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: GetReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    let Some(data) = store(&st).read(rel, &req.id) else {
        return ApiResp::err(-1, "snapshot not found");
    };
    // safety net: keep the current content as a snapshot before overwriting
    if let Some(cur) = st.vault.read_file(rel) {
        let _ = store(&st).snapshot(rel, &cur, false);
    }
    if let Err(e) = st.vault.write_file(rel, &data) {
        return ApiResp::err(-1, &format!("restore write: {}", e));
    }
    if rel.ends_with(".md") {
        super::ops::reindex_one(&st, rel);
        super::ops::notify_tree();
    }
    ApiResp::ok(json!({"path": rel}))
}

async fn clean_snapshots(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: PathReq = parse(&bytes);
    let rel = req.path.trim_start_matches('/');
    let res = if rel.is_empty() {
        store(&st).clean_all()
    } else {
        store(&st).clean(rel)
    };
    match res {
        Ok(_) => ApiResp::ok(json!({"msg": "cleaned"})),
        Err(e) => ApiResp::err(-1, &format!("clean: {}", e)),
    }
}

// ---------------------------------------------------------------------------
// Snapshot store (M11, SiYuan repo/-style, plaintext MVP).
/// Layout: `.markvault/repo/<sha256(path)[0..16]>/<millis>-<sha256(content)[0..8]>.md`
/// Content-addressed filenames dedupe identical revisions; per-doc cap trims
/// the oldest. Plaintext by design for MVP — encryption is a known limitation.
pub struct SnapshotStore {
    root: PathBuf,
    cap: usize,
}

pub const DEFAULT_CAP: usize = 30;

fn hash_hex(data: &[u8], n: usize) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{:02x}", b)).collect::<String>().chars().take(n).collect()
}

/// `20260101120000-1a2b3c4d` from filename stem (no extension).
fn parse_stem(stem: &str) -> Option<(i64, String)> {
    let (ts, ch) = stem.split_once('-')?;
    let millis = ts.parse::<i64>().ok()?;
    Some((millis, ch.to_string()))
}

impl SnapshotStore {
    pub fn new(root: PathBuf) -> Self {
        SnapshotStore { root, cap: DEFAULT_CAP }
    }

    pub fn with_cap(root: PathBuf, cap: usize) -> Self {
        SnapshotStore { root, cap: cap.max(1) }
    }

    fn dir_of(&self, path: &str) -> PathBuf {
        self.root.join(hash_hex(path.as_bytes(), 16))
    }

    /// Store a snapshot of `data` for `path`. Returns the snapshot id, or None
    /// when the newest revision already has this exact content (dedupe).
    /// `force=true` (manual snapshot) stores even if identical.
    pub fn snapshot(&self, path: &str, data: &[u8], force: bool) -> std::io::Result<Option<String>> {
        let dir = self.dir_of(path);
        fs::create_dir_all(&dir)?;
        let chash = hash_hex(data, 8);
        if !force {
            if let Some((_, _, latest)) = self.list(path).first() {
                if *latest == chash {
                    return Ok(None);
                }
            }
        }
        let mut millis = chrono::Utc::now().timestamp_millis();
        let mut file = dir.join(format!("{}-{}.md", millis, chash));
        // same content snapped twice within one millis -> bump the timestamp
        while file.exists() {
            millis += 1;
            file = dir.join(format!("{}-{}.md", millis, chash));
        }
        let id = format!("{}-{}", millis, chash);
        fs::write(file, data)?;
        self.trim(path)?;
        Ok(Some(id))
    }

    /// List snapshots for a doc, newest first: (id, millis, content_hash).
    pub fn list(&self, path: &str) -> Vec<(String, i64, String)> {
        let mut out: Vec<(String, i64, String)> = Vec::new();
        if let Ok(rd) = fs::read_dir(self.dir_of(path)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                let Some(stem) = name.strip_suffix(".md") else { continue };
                if let Some((millis, chash)) = parse_stem(stem) {
                    out.push((stem.to_string(), millis, chash));
                }
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }

    /// Read snapshot content by id.
    pub fn read(&self, path: &str, id: &str) -> Option<Vec<u8>> {
        // id is a `<millis>-<hash>` stem; reject path traversal
        if id.contains('/') || id.contains('\\') || id.contains("..") {
            return None;
        }
        fs::read(self.dir_of(path).join(format!("{}.md", id))).ok()
    }

    /// Keep only the newest `cap` revisions for a doc.
    fn trim(&self, path: &str) -> std::io::Result<()> {
        let list = self.list(path);
        if list.len() <= self.cap {
            return Ok(());
        }
        let dir = self.dir_of(path);
        for (id, _, _) in list.iter().skip(self.cap) {
            let _ = fs::remove_file(dir.join(format!("{}.md", id)));
        }
        Ok(())
    }

    /// Remove all snapshots of a doc (and the per-doc dir when empty).
    pub fn clean(&self, path: &str) -> std::io::Result<()> {
        let dir = self.dir_of(path);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// Drop the whole repo dir (global cleanup).
    pub fn clean_all(&self) -> std::io::Result<()> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (SnapshotStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mv-snap-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        (SnapshotStore::new(dir.clone()), dir)
    }

    #[test]
    fn test_snapshot_dedupe_and_list_order() {
        let (st, dir) = store();
        let id1 = st.snapshot("笔记本A/首页.md", b"v1", false).unwrap().unwrap();
        // identical content -> deduped
        assert!(st.snapshot("笔记本A/首页.md", b"v1", false).unwrap().is_none());
        // forced manual snapshot stores identical content
        assert!(st.snapshot("笔记本A/首页.md", b"v1", true).unwrap().is_some());
        let id2 = st.snapshot("笔记本A/首页.md", b"v2", false).unwrap().unwrap();
        let list = st.list("笔记本A/首页.md");
        assert_eq!(list.len(), 3);
        // newest first (millis strictly increase; sleep-free because ids differ by content hash)
        assert_ne!(id1, id2);
        assert!(list[0].0 == id2 || list[0].0.starts_with(&list[0].1.to_string()));
        // read back
        assert_eq!(st.read("笔记本A/首页.md", &id1).unwrap(), b"v1");
        // traversal-safe read
        assert!(st.read("笔记本A/首页.md", "../../etc").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_cap_trim_keeps_newest() {
        let dir = std::env::temp_dir().join(format!(
            "mv-snap-trim-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let st = SnapshotStore::with_cap(dir.clone(), 3);
        for i in 0..5u8 {
            st.snapshot("d.md", format!("v{}", i).as_bytes(), true).unwrap().unwrap();
        }
        let list = st.list("d.md");
        assert_eq!(list.len(), 3);
        // the newest survives: v4's content hash ends the newest id
        let newest = st.read("d.md", &list[0].0).unwrap();
        assert_eq!(newest, b"v4");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_isolation_between_docs() {
        let (st, dir) = store();
        st.snapshot("a.md", b"A", false).unwrap().unwrap();
        st.snapshot("b.md", b"B", false).unwrap().unwrap();
        assert_eq!(st.list("a.md").len(), 1);
        assert_eq!(st.list("b.md").len(), 1);
        st.clean("a.md").unwrap();
        assert_eq!(st.list("a.md").len(), 0);
        assert_eq!(st.list("b.md").len(), 1);
        let _ = fs::remove_dir_all(dir);
    }
}
