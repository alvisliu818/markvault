use super::{parse, ApiResp};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use comrak::Options;
use once_cell::sync::Lazy;
use percent_encoding::AsciiSet;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/renderMd", post(render_md))
        .route("/md2Text", post(md2text))
}

#[derive(Deserialize, Default)]
struct RenderReq {
    #[serde(default)]
    md: String,
    /// rewrite relative asset/img links to /vault/... for browser preview
    #[serde(default = "default_true")]
    rewrite_links: bool,
    /// source doc path (vault-relative) for correct relative-link rewriting
    #[serde(default)]
    path: String,
}

fn default_true() -> bool {
    true
}

fn comrak_options() -> Options<'static> {
    let mut opts = Options::default();
    opts.extension.table = true;
    opts.extension.strikethrough = true;
    opts.extension.tasklist = true;
    opts.extension.footnotes = true;
    opts.extension.front_matter_delimiter = Some("---".into());
    opts.render.unsafe_ = true;
    opts
}

const PATH_SAFE: &AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'#');

fn enc(p: &str) -> String {
    percent_encoding::utf8_percent_encode(p, PATH_SAFE).to_string()
}

static IMG_MD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(!\[[^\]]*\]\()([^)\s]+)([^)]*\))").unwrap());
static LINK_MD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"([^!]\[[^\]]*\]\()([^)\s]+)([^)]*\))").unwrap());
static WIKI: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\[([^\]\|#]+)(#[^\]\|]*)?(\|[^\]]*)?\]\]").unwrap());
static EMBED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"!\[\[([^\]\|#]+)(#[^\]\|]*)?\]\]").unwrap());
static HIGHLIGHT: Lazy<Regex> = Lazy::new(|| Regex::new(r"==([^=\n]+)==").unwrap());

async fn render_md(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let req: RenderReq = parse(&bytes);
    let mut md = req.md.clone();
    if req.rewrite_links {
        let (_, offset) = crate::vault::extract_frontmatter(&md);
        let body = md[offset.min(md.len())..].to_string();
        // Obsidian embeds: ![[target]] → inline the target doc's rendered HTML
        let body = expand_embeds(&st, &body);
        let body = rewrite_relative_links(&body, &req.path);
        // ==mark== highlight (comrak has no native support)
        let body = HIGHLIGHT.replace_all(&body, "<mark>$1</mark>").to_string();
        md = body;
    }
    let html = comrak::markdown_to_html(&md, &comrak_options());
    ApiResp::ok(json!({"content": html}))
}

/// Expand ![[target]] embeds one level deep (no recursion inside embedded content).
fn expand_embeds(st: &Arc<AppState>, md: &str) -> String {
    let has_embed = EMBED.is_match(md);
    if !has_embed {
        return md.to_string();
    }
    let docs = crate::vt::load_doc_infos(st);
    EMBED.replace_all(md, |c: &regex::Captures| {
        let target = c[1].trim();
        if let Some(p) = crate::vt::resolve_target(target, &docs) {
            if let Some(raw) = st.vault.read_file(&p) {
                let s = String::from_utf8_lossy(&raw).to_string();
                let (_, off) = crate::vault::extract_frontmatter(&s);
                let body = &s[off.min(s.len())..];
                // rewrite embedded content relative to its own location
                let body = rewrite_relative_links(body, &p);
                return format!(
                    "<div class=\"mv-embed\" data-target=\"{}\"><div class=\"mv-embed-title\">{}<a href=\"/#/doc/{}\">打开</a></div>{}</div>",
                    enc(&p),
                    docs
                        .iter()
                        .find(|d| d.path == p)
                        .map(|d| d.title.as_str())
                        .unwrap_or(target),
                    enc(&p),
                    body
                );
            }
        }
        format!(
            "<div class=\"mv-embed mv-embed-missing\">未找到嵌入文档: {}</div>",
            target
        )
    })
    .to_string()
}

/// Rewrite relative links so the SPA can fetch them from /vault/*;
/// wikilinks become hash routes the frontend resolves (path then title).
fn rewrite_relative_links(md: &str, doc_path: &str) -> String {
    let dir = match doc_path.rsplit_once('/') {
        Some((d, _)) if !d.is_empty() => format!("/vault/{}", d),
        _ => "/vault".to_string(),
    };
    let is_relative =
        |l: &str| !(l.starts_with('/') || l.starts_with("http") || l.starts_with("data:"));

    let md = IMG_MD
        .replace_all(md, |c: &regex::Captures| {
            let link = c[2].trim().to_string();
            if is_relative(&link) {
                let resolved = if link.starts_with("./") {
                    format!("{}/{}", dir.trim_end_matches('/'), &link[2..])
                } else {
                    format!("{}/{}", dir.trim_end_matches('/'), link)
                };
                format!("{}{}{}", &c[1], enc(&resolved), &c[3])
            } else {
                c[0].to_string()
            }
        })
        .to_string();

    let md = LINK_MD
        .replace_all(&md, |c: &regex::Captures| {
            let link = c[2].trim().to_string();
            if is_relative(&link) {
                let resolved = if link.starts_with("./") {
                    format!("/{}", &link[2..])
                } else {
                    format!("/{}", link)
                };
                format!("{}{}{}", &c[1], enc(&resolved), &c[3])
            } else {
                c[0].to_string()
            }
        })
        .to_string();

    WIKI.replace_all(&md, |c: &regex::Captures| {
        let target = c[1].trim();
        format!("[{}](/#/doc/{})", &c[0].to_string(), enc(target))
    })
    .to_string()
}

async fn md2text(bytes: Bytes) -> ApiResp {
    let req: RenderReq = parse(&bytes);
    let text = crate::parse::plain_text(&req.md);
    ApiResp::ok(json!({"content": text}))
}
