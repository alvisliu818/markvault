//! Embedded-browser multi-window tab management (M6).
//!
//! One tab = one `WebviewWindow` (label `browser-<tabId>`) loading an external
//! URL — Tauri v2 has no nested webviews, so each page tab is its own window.
//! The SPA browser panel is the controller (open/navigate/act/close/focus via
//! IPC commands); this module tracks tab state, reports url/title changes to
//! the main window (`browser:opened` / `browser:url` / `browser:closed`
//! events) and injects a small in-page action bar whose buttons talk to the
//! kernel directly over HTTP (bookmark / clipHtml — the page context has the
//! logged-in DOM the kernel-side clipUrl cannot fetch).

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};

#[derive(Clone, Debug, serde::Serialize)]
pub struct TabInfo {
    #[serde(rename = "tabId")]
    pub tab_id: String,
    pub url: String,
    pub title: String,
}

#[derive(Default)]
pub struct BrowserState {
    /// ordered registry of open browser tabs
    pub tabs: Mutex<Vec<TabInfo>>,
    /// kernel origin for the injected action bar (http://127.0.0.1:<port>)
    pub kernel_origin: Mutex<String>,
}

const POLL_MS: u64 = 500;

fn label_of(tab_id: &str) -> String {
    format!("browser-{}", tab_id)
}

/// In-page action bar (收藏/剪藏). Kernel origin is substituted in.
/// Re-injected on every poll cycle; `window.__mvBar` guards duplicates.
const BAR_JS: &str = r#"(function(){
if (window.__mvBar) return;
window.__mvBar = true;
var K = "__MV_KERNEL__";
function st(msg){
  var s=document.createElement('div');
  s.textContent=msg;
  s.style.cssText='position:fixed;top:36px;right:8px;z-index:2147483647;background:#1e1e28;color:#cdd6f4;padding:6px 12px;border-radius:6px;font:12px system-ui;box-shadow:0 2px 8px rgba(0,0,0,.4)';
  document.body.appendChild(s);
  setTimeout(function(){s.remove();},2600);
}
function post(path, body, ok){
  fetch(K+path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)})
    .then(function(r){return r.json()})
    .then(function(j){ if(j.code===0){st(ok(j.data));} else {st('失败: '+j.msg);} })
    .catch(function(e){st('网络错误（页面 CSP 可能拦截）: '+e);});
}
function bar(){
  if (document.getElementById('__mv-bar')) return;
  var d=document.createElement('div');
  d.id='__mv-bar';
  d.style.cssText='position:fixed;top:8px;right:8px;z-index:2147483647;display:flex;gap:4px;font:12px system-ui;background:rgba(30,30,40,.88);padding:4px 6px;border-radius:6px;box-shadow:0 2px 8px rgba(0,0,0,.4)';
  var mk=function(t,fn){
    var b=document.createElement('button');
    b.textContent=t;
    b.style.cssText='border:0;background:#3a3a4a;color:#eee;padding:3px 9px;border-radius:4px;cursor:pointer';
    b.onclick=fn;
    return b;
  };
  d.appendChild(mk('收藏',function(){
    post('/api/bookmark/add',{url:location.href,title:document.title},function(){return '已收藏: '+document.title;});
  }));
  d.appendChild(mk('剪藏',function(){
    post('/api/clip/clipHtml',{html:document.documentElement.outerHTML,url:location.href,title:document.title},function(d){return '已剪藏: '+(d&&d.title||'');});
  }));
  document.body.appendChild(d);
}
if (document.body) { bar(); } else { document.addEventListener('DOMContentLoaded', bar); }
})()"#;

fn inject_bar(w: &tauri::WebviewWindow, kernel_origin: &str) {
    let js = BAR_JS.replace("__MV_KERNEL__", kernel_origin);
    let _ = w.eval(&js);
}

fn register_tab(st: &BrowserState, tab_id: &str, url: &str) {
    st.tabs.lock().unwrap().push(TabInfo {
        tab_id: tab_id.to_string(),
        url: url.to_string(),
        title: String::new(),
    });
}

fn spawn_poll(app: AppHandle, tab_id: String, init_url: String) {
    std::thread::spawn(move || {
        let label = label_of(&tab_id);
        let mut last_url = init_url;
        let mut last_title = String::new();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
            let Some(w) = app.get_webview_window(&label) else {
                break; // window destroyed (SPA close or OS close button)
            };
            let cur = w.url().map(|u| u.to_string()).unwrap_or_default();
            let title = w.title().unwrap_or_default();
            let origin = app
                .state::<BrowserState>()
                .kernel_origin
                .lock()
                .unwrap()
                .clone();
            if !origin.is_empty() {
                inject_bar(&w, &origin);
            }
            if (cur != last_url || title != last_title) && (!cur.is_empty() || !title.is_empty()) {
                last_url = if cur.is_empty() { last_url.clone() } else { cur.clone() };
                last_title = title.clone();
                let st = app.state::<BrowserState>();
                {
                    let mut tabs = st.tabs.lock().unwrap();
                    if let Some(t) = tabs.iter_mut().find(|t| t.tab_id == tab_id) {
                        if !cur.is_empty() {
                            t.url = cur.clone();
                        }
                        t.title = title.clone();
                    }
                }
                let _ = app.emit(
                    "browser:url",
                    serde_json::json!({"tabId": tab_id, "url": last_url, "title": last_title}),
                );
            }
        }
        // cleanup after the window is gone
        {
            let st = app.state::<BrowserState>();
            st.tabs.lock().unwrap().retain(|t| t.tab_id != tab_id);
        }
        let _ = app.emit("browser:closed", serde_json::json!({"tabId": tab_id}));
    });
}

fn open_tab(app: &AppHandle, st: &BrowserState, tab_id: &str, url: &str) -> Result<(), String> {
    let label = label_of(tab_id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    let parsed: Url = url
        .parse()
        .map_err(|e| format!("invalid url {}: {}", url, e))?;
    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(parsed))
        .title(url)
        .inner_size(1080.0, 780.0)
        .on_document_title_changed(|webview, title| {
            let _ = webview.set_title(&title);
        })
        .build()
        .map_err(|e| e.to_string())?;
    // keep keyboard focus on the SPA main window after spawning a page window
    if let Some(m) = app.get_webview_window("main") {
        let _ = m.set_focus();
    }
    register_tab(st, tab_id, url);
    let _ = app.emit("browser:opened", serde_json::json!({"tabId": tab_id, "url": url}));
    spawn_poll(app.clone(), tab_id.to_string(), url.to_string());
    Ok(())
}

#[tauri::command]
pub fn browser_open(
    app: AppHandle,
    st: State<'_, BrowserState>,
    tab_id: String,
    url: String,
) -> Result<(), String> {
    open_tab(&app, &st, &tab_id, &url)
}

#[tauri::command]
pub fn browser_navigate(
    app: AppHandle,
    st: State<'_, BrowserState>,
    tab_id: String,
    url: String,
) -> Result<(), String> {
    let label = label_of(&tab_id);
    match app.get_webview_window(&label) {
        Some(w) => {
            let parsed: Url = url.parse().map_err(|e| format!("invalid url {}: {}", url, e))?;
            w.navigate(parsed).map_err(|e| e.to_string())
        }
        None => open_tab(&app, &st, &tab_id, &url),
    }
}

/// back/forward/reload/stop — page-context history ops on the webview
#[tauri::command]
pub fn browser_act(app: AppHandle, tab_id: String, action: String) -> Result<(), String> {
    let label = label_of(&tab_id);
    let w = app.get_webview_window(&label).ok_or("tab not found")?;
    let js = match action.as_str() {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "reload" => "location.reload()",
        "stop" => "window.stop()",
        other => return Err(format!("unknown action {}", other)),
    };
    w.eval(js).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_close(app: AppHandle, tab_id: String) -> Result<(), String> {
    let label = label_of(&tab_id);
    let w = app.get_webview_window(&label).ok_or("tab not found")?;
    w.close().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_focus(app: AppHandle, tab_id: String) -> Result<(), String> {
    let label = label_of(&tab_id);
    let w = app.get_webview_window(&label).ok_or("tab not found")?;
    let _ = w.unminimize();
    w.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_list(st: State<'_, BrowserState>) -> Vec<TabInfo> {
    st.tabs.lock().unwrap().clone()
}
