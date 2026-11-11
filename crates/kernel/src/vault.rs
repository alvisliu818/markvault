use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Vault filesystem helpers. All API paths are vault-relative, POSIX-slashed.
pub struct Vault {
    pub root: PathBuf,
}

impl Vault {
    pub fn new(root: PathBuf) -> Self {
        Vault { root }
    }

    /// Resolve a vault-relative path to an absolute path, rejecting escapes.
    pub fn abs(&self, rel: &str) -> Option<PathBuf> {
        let rel = rel.trim().replace('\\', "/");
        if rel.contains("..") {
            return None;
        }
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            return Some(self.root.clone());
        }
        let p = self.root.join(rel);
        Some(p)
    }

    /// Vault-relative POSIX path from an absolute path under root.
    pub fn rel(&self, abs: &Path) -> Option<String> {
        abs.strip_prefix(&self.root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.abs(rel).map(|p| p.exists()).unwrap_or(false)
    }

    pub fn is_dir(&self, rel: &str) -> bool {
        self.abs(rel).map(|p| p.is_dir()).unwrap_or(false)
    }

    pub fn read_file(&self, rel: &str) -> Option<Vec<u8>> {
        fs::read(self.abs(rel)?).ok()
    }

    /// Write file, creating parent dirs. Returns bytes written.
    pub fn write_file(&self, rel: &str, data: &[u8]) -> std::io::Result<()> {
        let p = self
            .abs(rel)
            .ok_or_else(|| io_err("invalid path"))?;
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(p, data)
    }

    pub fn mkdir(&self, rel: &str) -> std::io::Result<()> {
        let p = self.abs(rel).ok_or_else(|| io_err("invalid path"))?;
        fs::create_dir_all(p)
    }

    /// Move to `.markvault/trash/<ts>_<name>` instead of deleting (trash > rm).
    pub fn trash(&self, rel: &str) -> std::io::Result<String> {
        let p = self.abs(rel).ok_or_else(|| io_err("invalid path"))?;
        if !p.exists() {
            return Ok(String::new());
        }
        let ts = chrono::Local::now().format("%Y%m%d%H%M%S");
        let trash_dir = self.root.join(".markvault").join("trash");
        fs::create_dir_all(&trash_dir)?;
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "item".into());
        let dest = trash_dir.join(format!("{}_{}", ts, name));
        fs::rename(&p, &dest)?;
        Ok(dest
            .strip_prefix(&self.root)
            .map(|x| x.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default())
    }

    pub fn rename(&self, rel: &str, new_rel: &str) -> std::io::Result<()> {
        let from = self.abs(rel).ok_or_else(|| io_err("invalid path"))?;
        let to = self.abs(new_rel).ok_or_else(|| io_err("invalid path"))?;
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(from, to)
    }

    /// Top-level folders = notebooks. Hidden dirs and ignored names excluded.
    pub fn ls_notebooks(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(&self.root) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    out.push(name);
                }
            }
        }
        out.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        out
    }
}

fn io_err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, msg)
}

/// Extract YAML frontmatter map from raw markdown. Returns (map, body_offset)
/// where body_offset is the exact byte offset of the first body line.
/// A block is frontmatter only when the file starts with a `---` line.
/// Handles both LF and CRLF line endings.
pub fn extract_frontmatter(md: &str) -> (Map<String, Value>, usize) {
    let mut map = Map::new();
    let mut chunks = md.split_inclusive('\n').peekable();
    let first = match chunks.peek() {
        Some(c) => c.trim_end_matches(['\n', '\r']),
        None => return (map, 0),
    };
    if first != "---" {
        return (map, 0);
    }
    let first_chunk = chunks.next().unwrap();
    let mut offset_so_far = first_chunk.len();
    let mut fm_lines: Vec<String> = Vec::new();
    for chunk in chunks {
        let line = chunk.trim_end_matches(['\n', '\r']);
        if line == "---" {
            offset_so_far += chunk.len();
            let yaml: Result<serde_yaml::Value, _> = serde_yaml::from_str(&fm_lines.join("\n"));
            if let Ok(serde_yaml::Value::Mapping(m)) = yaml {
                for (k, v) in m {
                    map.insert(
                        match &k {
                            serde_yaml::Value::String(s) => s.clone(),
                            other => serde_yaml::to_string(other)
                                .unwrap_or_default()
                                .trim()
                                .to_string(),
                        },
                        yaml_to_json(v),
                    );
                }
            }
            return (map, offset_so_far);
        }
        fm_lines.push(line.to_string());
        offset_so_far += chunk.len();
    }
    // opening fence without closing fence: treat whole file as body
    (Map::new(), 0)
}

pub fn yaml_to_json(v: serde_yaml::Value) -> Value {
    serde_json::to_value(&v).unwrap_or(Value::Null)
}

/// Read a string field from frontmatter
pub fn fm_str(fm: &Map<String, Value>, key: &str) -> Option<String> {
    fm.get(key).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

/// Read a string array field (frontmatter `parent: ["[[X]]"]` etc.)
/// Used by tests today; upcoming /api/attr batch APIs will consume it.
#[cfg(test)]
pub(crate) fn fm_str_array(fm: &Map<String, Value>, key: &str) -> Vec<String> {
    match fm.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Read a numeric frontmatter field (weight etc.); tests + upcoming APIs.
#[cfg(test)]
pub(crate) fn fm_number(fm: &Map<String, Value>, key: &str) -> f64 {
    fm.get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

/// Sanitize a filename component (used when creating/renaming docs)
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "untitled".into()
    } else {
        trimmed.to_string()
    }
}

/// Format a unix mtime as SiYuan-style timestamp `20240510103024`
pub fn ts_from_mtime(mtime: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = mtime.into();
    dt.format("%Y%m%d%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frontmatter_lf() {
        let md = "---\ntitle: T\nparent:\n  - \"[[A]]\"\n---\n# Body\n";
        let (fm, off) = extract_frontmatter(md);
        assert_eq!(fm_str(&fm, "title").as_deref(), Some("T"));
        assert_eq!(fm_str_array(&fm, "parent"), vec!["[[A]]"]);
        assert_eq!(&md[off..], "# Body\n");
    }

    #[test]
    fn test_frontmatter_crlf() {
        let md = "---\r\nweight: 3\r\n---\r\nbody line\r\n";
        let (fm, off) = extract_frontmatter(md);
        assert_eq!(fm_number(&fm, "weight"), 3.0);
        assert_eq!(&md[off..], "body line\r\n");
    }

    #[test]
    fn test_frontmatter_unclosed() {
        let md = "---\ntitle: T\nno closing fence\n";
        let (fm, off) = extract_frontmatter(md);
        assert!(fm.is_empty());
        assert_eq!(off, 0);
    }

    #[test]
    fn test_no_frontmatter() {
        let (fm, off) = extract_frontmatter("# plain\n---\nnot frontmatter\n");
        assert!(fm.is_empty());
        assert_eq!(off, 0);
    }
}
