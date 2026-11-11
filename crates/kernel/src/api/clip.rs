use super::{check_writable, parse, ApiResp};
use crate::state::AppState;
use crate::vault;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Web clip pipeline (ported from the SiYuan browser plugin's data plane):
/// fetch URL kernel-side -> Readability extraction -> HTML-to-Markdown ->
/// create a markdown doc with source frontmatter.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/clipUrl", post(clip_url))
        .route("/clipHtml", post(clip_html))
}

#[derive(Deserialize, Default)]
struct ClipReq {
    #[serde(default)]
    url: String,
    #[serde(default)]
    notebook: String,
    /// hpath like "/剪藏/标题"; default "/剪藏/<page title>"
    #[serde(default)]
    path: String,
    #[serde(default)]
    tags: Vec<String>,
}

async fn fetch_page(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) markvault-clipper")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .map_err(|e| format!("fetch failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

/// Readability extract + convert to markdown. Returns (title, markdown).
fn html_to_markdown(html: &str, url: &str) -> Result<(String, String), String> {
    let mut readability = dom_smoothie::Readability::new(html, Some(url), None)
        .map_err(|e| format!("readability init: {:?}", e))?;
    let article = readability
        .parse()
        .map_err(|e| format!("readability parse: {:?}", e))?;
    let title = article.title.clone();
    let md = htmd::convert(&article.content).map_err(|e| format!("html2md: {}", e))?;
    Ok((title, md))
}

fn build_clip_doc(title: &str, url: &str, md: &str, tags: &[String]) -> String {
    use std::fmt::Write as _;
    let mut fm = String::from("---\n");
    let _ = writeln!(fm, "title: {}", title);
    let _ = writeln!(fm, "source: \"{}\"", url.replace('"', "'"));
    let _ = writeln!(
        fm,
        "created: {}",
        chrono::Local::now().format("%Y%m%d%H%M%S")
    );
    if !tags.is_empty() {
        let _ = writeln!(fm, "tags:");
        for t in tags {
            let _ = writeln!(fm, "  - {}", t);
        }
    }
    fm.push_str("---\n\n");
    fm.push_str(&format!("# {}\n\n", title));
    fm.push_str(&format!("> 剪藏自 [{}]({})\n\n", title, url));
    fm.push_str(md.trim());
    fm.push('\n');
    fm
}

async fn clip_url(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: ClipReq = parse(&bytes);
    if req.url.trim().is_empty() {
        return ApiResp::err(-1, "url required");
    }
    let notebook = if req.notebook.is_empty() {
        match st.vault.ls_notebooks().first() {
            Some(n) => n.clone(),
            None => return ApiResp::err(-1, "no notebook available"),
        }
    } else {
        req.notebook.clone()
    };

    let url = req.url.trim().to_string();
    let html = match fetch_page(&url).await {
        Ok(h) => h,
        Err(e) => return ApiResp::err(-1, &e),
    };
    let (title, md) = match html_to_markdown(&html, &url) {
        Ok(t) => t,
        Err(e) => return ApiResp::err(-1, &e),
    };
    let doc_md = build_clip_doc(&title, &url, &md, &req.tags);
    let hpath = if req.path.trim().is_empty() {
        format!("/剪藏/{}", vault::sanitize_filename(&title))
    } else {
        req.path.clone()
    };

    match super::filetree::create_doc_internal(&st, &notebook, &hpath, &doc_md) {
        Ok(path) => {
            super::ops::notify_tree();
            ApiResp::ok(json!({"path": path, "title": title}))
        }
        Err(e) => ApiResp::err(-1, &e),
    }
}

#[derive(Deserialize, Default)]
struct ClipHtmlReq {
    #[serde(default)]
    html: String,
    /// page URL for readability context + source frontmatter (may be empty)
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    notebook: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// Clip from client-supplied HTML (used by the future embedded-browser clipper
/// which has page context the kernel cannot fetch, e.g. logged-in pages).
async fn clip_html(State(st): State<Arc<AppState>>, bytes: Bytes) -> ApiResp {
    if let Some(e) = check_writable(&st) {
        return e;
    }
    let req: ClipHtmlReq = parse(&bytes);
    if req.html.trim().is_empty() {
        return ApiResp::err(-1, "html required");
    }
    let notebook = if req.notebook.is_empty() {
        match st.vault.ls_notebooks().first() {
            Some(n) => n.clone(),
            None => return ApiResp::err(-1, "no notebook available"),
        }
    } else {
        req.notebook.clone()
    };
    let (title, md) = html_to_markdown(&req.html, if req.url.is_empty() { "https://localhost/" } else { &req.url })
        .map_err(|e| e)
        .unwrap_or((req.title.clone(), req.html.clone()));
    let title = if title.is_empty() { req.title.clone() } else { title };
    let doc_md = build_clip_doc(&title, &req.url, &md, &req.tags);
    let hpath = if req.path.trim().is_empty() {
        format!("/剪藏/{}", vault::sanitize_filename(&title))
    } else {
        req.path.clone()
    };
    match super::filetree::create_doc_internal(&st, &notebook, &hpath, &doc_md) {
        Ok(path) => {
            super::ops::notify_tree();
            ApiResp::ok(json!({"path": path, "title": title}))
        }
        Err(e) => ApiResp::err(-1, &e),
    }
}
