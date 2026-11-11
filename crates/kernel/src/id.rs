use once_cell::sync::Lazy;
use rand::Rng;
use regex::Regex;

/// SiYuan-style ID: `20240510103024-abc1234` = 14-digit timestamp + 7 random alnum.
/// Used for generated block IDs (` ^id` trailing syntax, Obsidian-compatible).
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

/// Recognize generated IDs (tests now; block-ref resolution next).
#[allow(dead_code)]
pub fn is_node_id(s: &str) -> bool {
    ID_PATTERN.is_match(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_format() {
        let a = new_node_id();
        assert!(is_node_id(&a), "bad id: {}", a);
        assert_eq!(a.len(), 22); // 14 + '-' + 7
        assert_ne!(a, new_node_id());
    }
}
