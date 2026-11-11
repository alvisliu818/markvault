#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

mod browser;
mod settings;

struct KernelHandle(Mutex<Option<Child>>);

/// Resolve the kernel binary: target-triple sidecar next to this exe > plain exe > dev workspace target dir.
fn resolve_kernel() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MARKVAULT_KERNEL") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidates = Vec::new();
    // tauri externalBin sidecar: <name>-<target-triple>.exe
    if let Ok(triple) = std::env::var("TAURI_TARGET_TRIPLE") {
        candidates.push(dir.join(format!("markvault-kernel-{}.exe", triple)));
    }
    // common Windows triple fallback
    candidates.push(dir.join("markvault-kernel-x86_64-pc-windows-msvc.exe"));
    // plain name (manual copy next to exe)
    candidates.push(dir.join("markvault-kernel.exe"));
    // dev: cargo workspace target dir baked in at compile time
    // (src-tauri is <root>/app/src-tauri)
    let manifest = env!("CARGO_MANIFEST_DIR");
    candidates.push(PathBuf::from(manifest).join("../../target/debug/markvault-kernel.exe"));
    candidates.push(PathBuf::from(manifest).join("../../target/release/markvault-kernel.exe"));
    candidates.into_iter().find(|p| p.exists())
}

/// Workspace: env > %APPDATA%/markvault/config.json > <home>/markvault-vault
fn resolve_workspace() -> PathBuf {
    if let Ok(p) = std::env::var("MARKVAULT_WORKSPACE") {
        let p = PathBuf::from(p);
        if p.exists() {
            return p;
        }
    }
    if let Some(cfg) = dirs_config() {
        return cfg;
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("markvault-vault")
}

fn dirs_config() -> Option<PathBuf> {
    let base = std::env::var("APPDATA").ok()?;
    let p = PathBuf::from(base).join("markvault").join("config.json");
    let s = std::fs::read_to_string(p).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    let ws = v.get("workspace")?.as_str()?.to_string();
    let p = PathBuf::from(ws);
    if p.exists() { Some(p) } else { None }
}

fn pick_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(7806)
}

/// Wait until the kernel accepts TCP connections on the port.
fn wait_port_open(port: u16, timeout: Duration) -> bool {
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

fn spawn_kernel(port: u16, workspace: &PathBuf) -> Result<Child, String> {
    let kernel = resolve_kernel().ok_or_else(|| {
        "markvault-kernel.exe not found (sidecar missing; set MARKVAULT_KERNEL)".to_string()
    })?;
    Command::new(&kernel)
        .arg("serve")
        .arg("--workspace")
        .arg(workspace)
        .arg("--port")
        .arg(port.to_string())
        .arg("--log-file")
        .spawn()
        .map_err(|e| format!("spawn kernel {}: {}", kernel.display(), e))
}

fn main() {
    let port = pick_port();
    let workspace = resolve_workspace();

    tauri::Builder::default()
        .manage(KernelHandle(Mutex::new(None)))
        .manage(browser::BrowserState::default())
        .manage(settings::AppMeta::default())
        .invoke_handler(tauri::generate_handler![
            browser::browser_open,
            browser::browser_navigate,
            browser::browser_act,
            browser::browser_close,
            browser::browser_focus,
            browser::browser_list,
            settings::pick_vault_dir,
            settings::set_workspace,
            settings::get_shell_conf,
            settings::set_autostart,
        ])
        .setup(move |app| {
            let child = spawn_kernel(port, &workspace).map_err(|e| e.to_string())?;
            *app.state::<KernelHandle>().0.lock().unwrap() = Some(child);
            *app.state::<browser::BrowserState>()
                .kernel_origin
                .lock()
                .unwrap() = format!("http://127.0.0.1:{}", port);
            *app.state::<settings::AppMeta>()
                .workspace
                .lock()
                .unwrap() = workspace.to_string_lossy().to_string();

            if !wait_port_open(port, Duration::from_secs(10)) {
                return Err("kernel did not start in time".into());
            }

            let url: tauri::Url = format!("http://127.0.0.1:{}", port)
                .parse()
                .map_err(|e| format!("url parse: {}", e))?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("markvault")
                .inner_size(1280.0, 860.0)
                .build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(child) = window
                    .app_handle()
                    .state::<KernelHandle>()
                    .0
                    .lock()
                    .unwrap()
                    .as_mut()
                {
                    let _ = child.kill();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
