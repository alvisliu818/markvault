use once_cell::sync::Lazy;
use rand::Rng;
use regex::Regex;

/// SiYuan-style ID: `20240510103024-abc1234` = 14-digit timestamp + 7 random alnum.
/// Used for generated block IDs.
pub fn new_node_id() -> String {
    let ts = chrono::Local::now().format("%Y%m%d%H%M%S");
    let suffix: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(7)
        .map(|c| (c as char).to_ascii_lowercase())
        .collect();
    format!("{}-{}", ts, suffix)
}

pub static ID_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d{14}-[a-z0-9]{7}$").unwrap());

pub fn is_node_id(s: &str) -> bool {
    ID_PATTERN.is_match(s)
}

/// `created` time derived from the ID prefix (SiYuan util.TimeFromID)
pub fn time_from_id(id: &str) -> Option<&str> {
    id.get(..14)
}
