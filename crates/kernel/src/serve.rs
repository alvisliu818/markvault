use crate::api;
use crate::auth;
use crate::conf::{markvault_dir, Conf};
use crate::state::AppState;
use axum::extract::State;
use axum::middleware;
use axum::response::IntoResponse;
use axum::Router;
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

pub struct ServeOpts {
    pub workspace: PathBuf,
    pub port: Option<u16>,
    pub network_serve: bool,
    pub readonly: bool,
}

pub async fn serve(opts: ServeOpts) {
    let workspace = opts.workspace.canonicalize().unwrap_or(opts.workspace.clone());
    let mv_dir = markvault_dir(&workspace);
    std::fs::create_dir_all(&mv_dir).expect("create .markvault dir");
    std::fs::create_dir_all(mv_dir.join("storage").join("local")).expect("create storage dir");

    let mut conf = Conf::load(&mv_dir);
    if let Some(p) = opts.port {
        conf.port = p;
    }
    if opts.network_serve {
        conf.network_serve = true;
    }
    if opts.readonly {
        conf.readonly = true;
    }
    if conf.save(&mv_dir).is_ok() {
        tracing::info!("workspace: {}", workspace.display());
    }

    let db_path = mv_dir.join("cache").join("index.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).expect("create cache dir");
    let db = crate::index::open(&db_path).expect("open index db");

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let state = AppState::new(workspace.clone(), conf.clone(), db, shutdown_tx);

    // Boot: incremental index (mtime+size stamp skips unchanged docs)
    {
        let st = state.clone();
        tokio::task::spawn_blocking(move || {
            let db = st.db.lock().unwrap();
            let n = crate::index::builder::rebuild_all_ext(&db, &st.vault, &|rel| st.is_ignored(rel), false, &|done, total| {
                if done % 20 == 0 || done == total {
                    crate::ws::push_progress(done as u64, total as u64);
                }
            });
            tracing::info!("indexed {} docs", n);
        })
        .await
        .ok();
    }

    start_watcher(state.clone());

    let app = build_router(state.clone());

    let host = if conf.network_serve { "0.0.0.0" } else { "127.0.0.1" };
    let addr = format!("{}:{}", host, conf.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    tracing::info!(
        "markvault kernel listening on http://{} (workspace: {})",
        addr,
        workspace.display()
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
            tracing::info!("shutting down");
        })
        .await
        .expect("server error");
}

/// Frontend static dir candidates (env > cwd variants > exe variants)
fn frontend_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MARKVAULT_FRONTEND_DIR") {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    let cwd = std::env::current_dir().ok()?;
    let exe = std::env::current_exe().ok()?;
    let mut candidates = Vec::new();
    for base in [
        cwd.clone(),
        cwd.join(".."),
        cwd.join("../.."),
        exe.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
        exe.parent().map(|p| p.join("../..")).unwrap_or_default(),
    ] {
        candidates.push(base.join("frontend").join("dist"));
        candidates.push(base.join("dist"));
    }
    candidates.into_iter().find(|p| p.exists())
}

fn build_router(state: Arc<AppState>) -> Router {
    let mut app = Router::new()
        .route("/check-auth", axum::routing::get(auth::check_auth_page))
        .route("/vault/{*path}", axum::routing::get(vault_static).with_state(state.clone()))
        .merge(api::routes().with_state(state.clone()))
        .merge(api::ws_routes().with_state(state.clone()));

    // petal plugin static hosting (read-only, /petal/<name>/index.js entry)
    let petal_dir = crate::conf::markvault_dir(&state.workspace).join("petal");
    app = app.nest_service("/petal", ServeDir::new(&petal_dir));

    app = match frontend_dir() {
        Some(dir) => {
            let index_file = dir.join("index.html");
            app.fallback_service(
                ServeDir::new(&dir)
                    .append_index_html_on_directories(true)
                    .fallback(ServeFile::new(index_file)),
            )
        }
        None => app.fallback_service(axum::routing::get(welcome)),
    };

    app.layer(middleware::from_fn_with_state(state.clone(), auth::auth_middleware))
        .layer(CompressionLayer::new())
        .layer(CorsLayer::very_permissive())
}

/// Read-only raw vault file serving (images/attachments for preview).
/// Denies any dot-segment to keep `.markvault` private.
async fn vault_static(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    let rel = percent_dec(&path);
    if rel.split('/').any(|s| s.starts_with('.')) || state.is_ignored(&rel) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    match state.vault.read_file(&rel) {
        Some(data) => {
            let ct = mime_of(&rel);
            (
                [(axum::http::header::CONTENT_TYPE, ct)],
                axum::body::Body::from(data),
            )
                .into_response()
        }
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

fn percent_dec(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.to_string())
        .unwrap_or_else(|_| s.to_string())
}

fn mime_of(rel: &str) -> &'static str {
    let ext = rel.rsplit('.').next().unwrap_or("");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "md" => "text/markdown; charset=utf-8",
        "css" => "text/css",
        "js" => "text/javascript",
        _ => "application/octet-stream",
    }
}

async fn welcome() -> impl IntoResponse {
    axum::response::Html(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>markvault kernel</title></head>
<body style="font-family:system-ui;background:#1e1e28;color:#dcdfe4;display:flex;align-items:center;justify-content:center;height:100vh;margin:0">
<div><h1>markvault kernel</h1><p>API ready. Build the frontend (frontend/ → dist) to serve the UI here.</p></div>
</body></html>"#,
    )
}

/// Watch the workspace and incrementally reindex changed markdown files.
/// `.markvault/` and dot-dirs are filtered to avoid feedback loops.
fn start_watcher(state: Arc<AppState>) {
    let tx = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
    let (path_tx, mut path_rx) = tx;

    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        None,
        move |res: DebounceEventResult| {
            if let Ok(events) = res {
                let mut paths: Vec<String> = Vec::new();
                for ev in events {
                    for p in &ev.paths {
                        paths.push(p.to_string_lossy().replace('\\', "/"));
                    }
                }
                let _ = path_tx.send(paths);
            }
        },
    )
    .expect("create watcher");

    debouncer
        .watch(&state.workspace, notify::RecursiveMode::Recursive)
        .expect("watch workspace");

    std::thread::spawn(move || {
        // The debouncer MUST live as long as this thread: dropping it stops
        // its internal watcher thread and kills the event stream (pre-v0.3
        // bug: it sat on start_watcher's stack and was dropped immediately,
        // so external file changes were never reindexed).
        let _debouncer = debouncer;
        while let Some(paths) = path_rx.blocking_recv() {
            let st = state.clone();
            // Bulk apply (LiveSync import) writes thousands of files and runs
            // its own force rebuild right after — per-file reindex here would
            // duplicate all that work (pre-v0.3.0: 300s timeouts + WS spam).
            if st.bulk_apply.load(std::sync::atomic::Ordering::SeqCst) {
                continue;
            }
            let mut changed_md = false;
            let mut structural = false;
            for p in &paths {
                let rel = match st.vault.rel(Path::new(p)) {
                    Some(r) => r,
                    None => continue,
                };
                if rel.split('/').any(|s| s.starts_with('.')) || st.is_ignored(&rel) {
                    continue;
                }
                if rel.ends_with(".md") {
                    changed_md = true;
                    if st.vault.exists(&rel) {
                        super::api::ops::reindex_one(&st, &rel);
                    } else {
                        super::api::ops::unindex(&st, &rel);
                        let mut vt = crate::vt::VtStore::load(&st);
                        crate::vt::sync_delete(&mut vt, &rel);
                        vt.save(&st);
                    }
                } else {
                    structural = true;
                    // Directory-level event (create/remove/rename). Precisely
                    // reindex the affected prefix instead of leaving stale rows
                    // until next boot; the tree broadcast below still notifies
                    // the frontend. Passing a trailing "/" keeps unindex's
                    // LIKE-prefix from matching sibling names (dirA%).
                    if st.vault.is_dir(&rel) {
                        // created/renamed into existence: reindex its .md tree
                        // (defensive unindex keeps the "/" so LIKE won't hit siblings)
                        super::api::ops::reindex_prefix(&st, &format!("{}/", rel), &rel);
                    } else if !st.vault.exists(&rel) {
                        // removed/renamed away: drop index rows under it
                        super::api::ops::unindex(&st, &format!("{}/", rel));
                    }
                }
            }
            if structural {
                api::ops::notify_tree();
            } else if changed_md {
                crate::ws::push_filetree_changed();
                crate::ws::push_vt_changed();
            }
        }
    });
}
