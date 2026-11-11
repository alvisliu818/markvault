use crate::conf::{markvault_dir, Conf};
use crate::vault::Vault;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

/// Shared kernel state (SiYuan's model.Conf + util globals equivalent)
pub struct AppState {
    pub workspace: PathBuf,
    pub vault: Vault,
    pub conf: RwLock<Conf>,
    pub db: Mutex<Connection>,
    /// index rebuild in progress flag
    pub rebuilding: AtomicBool,
    /// bulk apply (LiveSync import) in progress — the watcher stands down
    /// because a full rebuild runs right after the writes finish
    pub bulk_apply: AtomicBool,
    /// shutdown signal for graceful exit API
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// HMAC session secret derived from accessAuthCode (stable across restarts)
    pub session_key: Vec<u8>,
}

impl AppState {
    pub fn new(
        workspace: PathBuf,
        conf: Conf,
        db: Connection,
        shutdown_tx: tokio::sync::watch::Sender<bool>,
    ) -> Arc<AppState> {
        let vault = Vault::new(workspace.clone());
        let session_key = derive_session_key(&conf, &workspace);
        Arc::new(AppState {
            workspace: workspace.clone(),
            vault,
            conf: RwLock::new(conf),
            db: Mutex::new(db),
            rebuilding: AtomicBool::new(false),
            bulk_apply: AtomicBool::new(false),
            shutdown_tx,
            session_key,
        })
    }

    pub fn conf_dir(&self) -> PathBuf {
        markvault_dir(&self.workspace)
    }

    pub fn storage_dir(&self) -> PathBuf {
        self.conf_dir().join("storage")
    }

    pub fn save_conf(&self) {
        let conf = self.conf.read().unwrap().clone();
        if let Err(e) = conf.save(&self.conf_dir()) {
            tracing::error!("save conf failed: {}", e);
        }
    }

    pub fn is_ignored(&self, rel: &str) -> bool {
        let conf = self.conf.read().unwrap();
        let p = rel.replace('\\', "/");
        conf.ignore_patterns.iter().any(|pat| match_pattern(&p, pat))
    }
}

/// Simple glob-ish match: `*` wildcard, `**` any segments, `?` single char.
/// Pattern without wildcard = prefix match (SiYuan excludeFolders semantics).
pub fn match_pattern(path: &str, pattern: &str) -> bool {
    let path = path.to_lowercase();
    let pat = pattern.to_lowercase();
    let (re_str, prefix_only) = build_regex(&pat);
    let re = once_cell_regex(re_str);
    if prefix_only {
        path.starts_with(&pat.trim_end_matches("/**"))
    } else {
        re.map(|r| r.is_match(&path)).unwrap_or(false)
    }
}

fn build_regex(pat: &str) -> (String, bool) {
    if !pat.contains('*') && !pat.contains('?') {
        return (String::new(), true);
    }
    let mut re = String::from("^");
    for c in pat.chars() {
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '.' | '(' | ')' | '[' | ']' | '{' | '}' | '+' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            _ => re.push(c),
        }
    }
    (re, false)
}

fn once_cell_regex(pattern: String) -> Option<regex::Regex> {
    regex::Regex::new(&pattern).ok()
}

fn derive_session_key(conf: &Conf, workspace: &std::path::Path) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(
        format!("{}|{}", conf.access_auth_code, workspace.display()).as_bytes(),
    )
    .expect("hmac key");
    mac.update(b"markvault-session");
    mac.finalize().into_bytes().to_vec()
}
