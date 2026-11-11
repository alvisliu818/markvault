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

async fn render_md(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    let _ = st;
    let req: RenderReq = parse(&bytes);
    let mut md = req.md.clone();
    if req.rewrite_links {
        let (_, offset) = crate::vault::extract_frontmatter(&md);
        let body = md[offset.min(md.len())..].to_string();
        md = rewrite_relative_links(&body, &req.path);
    }
    let html = comrak::markdown_to_html(&md, &comrak_options());
    ApiResp::ok(json!({"content": html}))
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
