//! Multi-workspace pool (M11, route A): one kernel instance per vault.
//! The main window navigates between instances (`ws_switch`); switching
//! keeps other vaults warm (max 3, LRU kills the oldest inactive one).
//! Every instance runs its own random port; the SPA needs no changes —
//! navigation loads it from the target kernel directly.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use serde::Serialize;
use tauri::{AppHandle, Manager, Url};

const MAX_INSTANCES: usize = 3;

pub struct Instance {
    pub workspace: String,
    pub port: u16,
    pub child: Child,
}

pub struct WorkspacePool {
    pub instances: Mutex<Vec<Instance>>,
    pub active: Mutex<String>,
}

impl WorkspacePool {
    pub fn kill_all(&self) {
        for mut i in self.instances.lock().unwrap().drain(..) {
            let _ = i.child.kill();
        }
    }
}

#[derive(Clone, Serialize)]
pub struct WsInfo {
    pub workspace: String,
    pub port: u16,
    pub active: bool,
}

// ---- kernel spawn helpers (moved from main.rs so all callers share them) ----

/// Resolve the kernel binary: target-triple sidecar next to this exe > plain exe > dev workspace target dir.
pub fn resolve_kernel() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MARKVAULT_KERNEL") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidates = Vec::new();
    if let Ok(triple) = std::env::var("TAURI_TARGET_TRIPLE") {
        candidates.push(dir.join(format!("markvault-kernel-{}.exe", triple)));
    }
    candidates.push(dir.join("markvault-kernel-x86_64-pc-windows-msvc.exe"));
    candidates.push(dir.join("markvault-kernel.exe"));
    let manifest = env!("CARGO_MANIFEST_DIR");
    candidates.push(PathBuf::from(manifest).join("../../target/debug/markvault-kernel.exe"));
    candidates.push(PathBuf::from(manifest).join("../../target/release/markvault-kernel.exe"));
    candidates.into_iter().find(|p| p.exists())
}

pub fn pick_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(7806)
}

pub fn wait_port_open(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            std::thread::sleep(Duration::from_millis(200));
            return true;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    false
}

fn spawn_kernel(workspace: &Path) -> Result<Instance, String> {
    let kernel = resolve_kernel().ok_or_else(|| {
        "markvault-kernel.exe not found (sidecar missing; set MARKVAULT_KERNEL)".to_string()
    })?;
    let port = pick_port();
    let child = Command::new(&kernel)
        .arg("serve")
        .arg("--workspace")
        .arg(workspace)
        .arg("--port")
        .arg(port.to_string())
        .arg("--log-file")
        .spawn()
        .map_err(|e| format!("spawn kernel {}: {}", kernel.display(), e))?;
    if !wait_port_open(port, Duration::from_secs(10)) {
        let mut c = child;
        let _ = c.kill();
        return Err("kernel did not start in time".to_string());
    }
    Ok(Instance { workspace: workspace.to_string_lossy().to_string(), port, child })
}

fn norm(path: &str) -> String {
    PathBuf::from(path.trim())
        .canonicalize()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.trim().to_string())
}

/// Make sure an instance for `path` exists (LRU-capped), return its port.
fn ensure_instance(app: &AppHandle, path: &str) -> Result<u16, String> {
    let key = norm(path);
    if !Path::new(&key).is_dir() {
        return Err(format!("目录不存在: {}", key));
    }
    let pool = app.state::<WorkspacePool>();
    let mut instances = pool.instances.lock().unwrap();
    if let Some(i) = instances.iter().find(|i| i.workspace == key) {
        return Ok(i.port);
    }
    // LRU: drop the oldest inactive instance beyond the cap
    let active = pool.active.lock().unwrap().clone();
    while instances.len() >= MAX_INSTANCES {
        let drop_at = instances
            .iter()
            .position(|i| i.workspace != active)
            .unwrap_or(0);
        let mut victim = instances.remove(drop_at);
        let _ = victim.child.kill();
    }
    let inst = spawn_kernel(Path::new(&key))?;
    let port = inst.port;
    instances.push(inst);
    Ok(port)
}

#[tauri::command]
pub fn ws_list(pool: tauri::State<'_, WorkspacePool>) -> Vec<WsInfo> {
    let active = pool.active.lock().unwrap().clone();
    pool.instances
        .lock()
        .unwrap()
        .iter()
        .map(|i| WsInfo {
            workspace: i.workspace.clone(),
            port: i.port,
            active: i.workspace == active,
        })
        .collect()
}

/// Point the main window at the kernel serving `path` (spawning one if needed).
#[tauri::command]
pub async fn ws_switch(app: AppHandle, path: String) -> Result<WsInfo, String> {
    let app2 = app.clone();
    let (port, key) = tauri::async_runtime::spawn_blocking(move || -> Result<(u16, String), String> {
        let port = ensure_instance(&app2, &path)?;
        let pool = app2.state::<WorkspacePool>();
        let key = pool
            .instances
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.port == port)
            .map(|i| i.workspace.clone())
            .ok_or_else(|| "instance vanished".to_string())?;
        Ok((port, key))
    })
    .await
    .map_err(|e| format!("join: {}", e))??;

    let url: Url = format!("http://127.0.0.1:{}", port)
        .parse()
        .map_err(|e| format!("url parse: {}", e))?;
    let win = app
        .get_webview_window("main")
        .ok_or("main window not found")?;
    win.navigate(url).map_err(|e| format!("navigate: {}", e))?;

    let pool = app.state::<WorkspacePool>();
    *pool.active.lock().unwrap() = key.clone();
    // injected action bars in browser tabs follow the active vault's kernel
    *app.state::<crate::browser::BrowserState>()
        .kernel_origin
        .lock()
        .unwrap() = format!("http://127.0.0.1:{}", port);
    *app.state::<crate::settings::AppMeta>()
        .workspace
        .lock()
        .unwrap() = key.clone();

    Ok(WsInfo { workspace: key, port, active: true })
}

/// Kill a specific instance (never the active one).
#[tauri::command]
pub fn ws_close(pool: tauri::State<'_, WorkspacePool>, path: String) -> Result<(), String> {
    let key = norm(&path);
    let active = pool.active.lock().unwrap().clone();
    if key == active {
        return Err("cannot close the active workspace".into());
    }
    let mut instances = pool.instances.lock().unwrap();
    if let Some(pos) = instances.iter().position(|i| i.workspace == key) {
        let mut victim = instances.remove(pos);
        let _ = victim.child.kill();
        Ok(())
    } else {
        Err("workspace not open".into())
    }
}
