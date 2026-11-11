use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::sync::Arc;

/// Browser history — data plane of the embedded browser (M6).
/// Persisted at `.markvault/storage/browser-history.json`, newest first.
/// Dedup strategy aligned with siyuan-plugin-browser historyStore:
/// same URL within 10 min updates the existing entry instead of appending,
/// hard cap trims the tail.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/list", post(list))
        .route("/add", post(add))
        .route("/remove", post(remove))
        .route("/clear", post(clear))
}

/// 10-minute dedup window (ms)
pub const DEDUP_WINDOW_MS: i64 = 10 * 60 * 1000;
/// hard cap of entries
pub const HISTORY_CAP: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// epoch millis
    pub created: i64,
}

fn load(st: &AppState) -> Vec<HistoryEntry> {
    let path = st.storage_dir().join("browser-history.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(st: &AppState, list: &[HistoryEntry]) {
    let dir = st.storage_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(
        dir.join("browser-history.json"),
        serde_json::to_string_pretty(list).unwrap_or_default(),
    );
}

/// Pure record logic (unit-testable): dedup within window / cap / newest first.
/// Mirrors siyuan-plugin-browser historyStore.record: revisited entries update
/// `created` in place; new entries unshift; overflow trims the tail.
pub fn record_at(list: &mut Vec<HistoryEntry>, url: &str, title: &str, now: i64) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    if let Some(e) = list
        .iter_mut()
        .find(|e| e.url == url && now - e.created < DEDUP_WINDOW_MS)
    {
        e.created = now;
        if !title.is_empty() {
            e.title = title.to_string();
        }
        return;
    }
    list.insert(
        0,
        HistoryEntry {
            url: url.to_string(),
            title: if title.is_empty() { url.to_string() } else { title.to_string() },
            created: now,
        },
    );
    if list.len() > HISTORY_CAP {
        list.truncate(HISTORY_CAP);
    }
}

async fn list(State(st): State<Arc<AppState>>) -> ApiResp {
    ApiResp::ok(json!(load(&st)))
}

#[derive(Deserialize, Default)]
struct AddReq {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
}

async fn add(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: AddReq = parse(&bytes);
    let mut list = load(&st);
    let now = chrono::Utc::now().timestamp_millis();
    record_at(&mut list, &req.url, &req.title, now);
    save(&st, &list);
    ApiResp::ok(json!(list))
}

#[derive(Deserialize, Default)]
struct RemoveReq {
    #[serde(default)]
    url: String,
}

async fn remove(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: RemoveReq = parse(&bytes);
    let mut list = load(&st);
    list.retain(|e| e.url != req.url);
    save(&st, &list);
    ApiResp::ok(json!(list))
}

async fn clear(State(st): State<Arc<AppState>>, _bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    save(&st, &[]);
    ApiResp::ok(json!([]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_new_entry_unshift() {
        let mut l = vec![];
        record_at(&mut l, "https://a.com", "A", 1000);
        record_at(&mut l, "https://b.com", "", 2000);
        assert_eq!(l.len(), 2);
        // newest first
        assert_eq!(l[0].url, "https://b.com");
        // empty title falls back to url
        assert_eq!(l[0].title, "https://b.com");
    }

    #[test]
    fn test_record_dedup_within_10min() {
        let mut l = vec![];
        record_at(&mut l, "https://a.com", "old", 1000);
        record_at(&mut l, "https://b.com", "B", 1500);
        // revisit within 10min: update in place, no append
        record_at(&mut l, "https://a.com", "new", 60 * 1000);
        assert_eq!(l.len(), 2);
        let a = l.iter().find(|e| e.url == "https://a.com").unwrap();
        assert_eq!(a.created, 60 * 1000);
        assert_eq!(a.title, "new");
        // in-place update keeps insertion position (b.com still first)
        assert_eq!(l[0].url, "https://b.com");
    }

    #[test]
    fn test_record_beyond_10min_appends() {
        let mut l = vec![];
        record_at(&mut l, "https://a.com", "A", 0);
        record_at(&mut l, "https://a.com", "A2", DEDUP_WINDOW_MS + 1);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].title, "A2");
    }

    #[test]
    fn test_record_cap_trim_tail() {
        let mut l = vec![];
        for i in 0..(HISTORY_CAP + 5) {
            record_at(&mut l, &format!("https://x.com/{}", i), "", i as i64);
        }
        assert_eq!(l.len(), HISTORY_CAP);
        // newest kept, oldest trimmed
        assert_eq!(l[0].url, format!("https://x.com/{}", HISTORY_CAP + 4));
        assert_eq!(l.last().unwrap().url, format!("https://x.com/{}", 5));
    }

    #[test]
    fn test_record_empty_url_ignored() {
        let mut l = vec![];
        record_at(&mut l, "  ", "x", 1);
        assert!(l.is_empty());
    }
}
