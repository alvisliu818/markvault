use once_cell::sync::Lazy;
use regex::Regex;

/// One outline/block entry extracted from markdown
#[derive(Debug, Clone)]
pub struct ParseBlock {
    /// "h1".."h6", "p", "code", "quote"
    pub r#type: String,
    pub text: String,
    /// 1-based line number in the file (0 = frontmatter)
    pub line: usize,
    /// Obsidian-style trailing block id (` ^abc123`)
    pub block_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParseLink {
    pub target: String,
    pub anchor: Option<String>,
}

static HEADING_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(#{1,6})\s+(.*)$").unwrap());
static BLOCK_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s\^([A-Za-z0-9][A-Za-z0-9-]*)\s*$").unwrap());
static WIKILINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\[([^\]\|#]+)(?:#([^\]\|]+))?(?:\|([^\]]+))?\]\]").unwrap());

/// Parse markdown body (frontmatter already stripped at `body_start`).
/// Tracks code fences so headings inside fences are ignored.
pub fn parse_blocks(md_body: &str, body_start: usize) -> Vec<ParseBlock> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_marker = String::new();
    let mut in_frontmatter_like = false;

    for (i, line) in md_body.lines().enumerate() {
        let line_no = body_start + i + 1; // 1-based file line
        let trimmed = line.trim_start();

        if !in_fence {
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = true;
                fence_marker = trimmed.chars().take(3).collect();
                out.push(ParseBlock {
                    r#type: "code".into(),
                    text: line.trim_start_matches(['`', '~']).trim().to_string(),
                    line: line_no,
                    block_id: None,
                });
                continue;
            }
        } else {
            if trimmed.starts_with(&fence_marker) {
                in_fence = false;
            }
            continue;
        }

        if trimmed.starts_with("---") {
            in_frontmatter_like = !in_frontmatter_like;
            continue;
        }
        if in_frontmatter_like {
            continue;
        }

        if let Some(cap) = HEADING_RE.captures(line) {
            let level = cap[1].len();
            let mut text = cap[2].trim().to_string();
            let mut block_id = None;
            if let Some(idcap) = BLOCK_ID_RE.captures(&text) {
                block_id = Some(idcap[1].to_string());
                text = BLOCK_ID_RE.replace(&text, "").trim_end().to_string();
            }
            out.push(ParseBlock {
                r#type: format!("h{}", level),
                text,
                line: line_no,
                block_id,
            });
            continue;
        }

        // paragraph lines (non-empty) — keep last contiguous paragraph as one block
        if !trimmed.is_empty() {
            let mut text = trimmed.to_string();
            let mut block_id = None;
            if let Some(idcap) = BLOCK_ID_RE.captures(&text) {
                block_id = Some(idcap[1].to_string());
                text = BLOCK_ID_RE.replace(&text, "").trim_end().to_string();
            }
            out.push(ParseBlock {
                r#type: "p".into(),
                text,
                line: line_no,
                block_id,
            });
        }
    }
    out
}

/// Extract all wikilinks from a markdown body
pub fn parse_wikilinks(md_body: &str) -> Vec<ParseLink> {
    let mut out = Vec::new();
    for cap in WIKILINK_RE.captures_iter(md_body) {
        out.push(ParseLink {
            target: cap[1].trim().to_string(),
            anchor: cap.get(2).map(|m| m.as_str().to_string()),
        });
    }
    out
}

/// Plain text of a markdown body (light stripping for FTS).
pub fn plain_text(md_body: &str) -> String {
    let mut out = String::with_capacity(md_body.len());
    let mut in_fence = false;
    let mut fence_marker = String::new();
    for line in md_body.lines() {
        let trimmed = line.trim_start();
        if in_fence {
            if trimmed.starts_with(&fence_marker) {
                in_fence = false;
            } else {
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = true;
            fence_marker = trimmed.chars().take(3).collect();
            continue;
        }
        let stripped = HEADING_RE.replace(line, "$2");
        let stripped = stripped
            .replace("**", "")
            .replace('*', "")
            .replace("==", "");
        out.push_str(stripped.trim_start());
        out.push('\n');
    }
    out
}

/// Parse a wikilink into (target, anchor, alias)
pub fn split_wikilink(raw: &str) -> (String, Option<String>, Option<String>) {
    let raw = raw.trim();
    let inner = raw
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(raw);
    if let Some(cap) = WIKILINK_RE.captures(inner) {
        return (
            cap[1].trim().to_string(),
            cap.get(2).map(|m| m.as_str().to_string()),
            cap.get(3).map(|m| m.as_str().to_string()),
        );
    }
    // plain text target (frontmatter parents may be written without [[]])
    if let Some((t, a)) = inner.split_once('#') {
        return (t.trim().to_string(), Some(a.to_string()), None);
    }
    if let Some((t, al)) = inner.split_once('|') {
        return (t.trim().to_string(), None, Some(al.to_string()));
    }
    (inner.trim().to_string(), None, None)
}
