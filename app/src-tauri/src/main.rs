#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

mod browser;
mod settings;
mod workspace;

use workspace::{pick_port, wait_port_open, WorkspacePool};

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

fn main() {
    let port = pick_port();
    let workspace = resolve_workspace();

    tauri::Builder::default()
        .manage(WorkspacePool {
            instances: Mutex::new(Vec::new()),
            active: Mutex::new(String::new()),
        })
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
            workspace::ws_list,
            workspace::ws_switch,
            workspace::ws_close,
        ])
        .setup(move |app| {
            // boot kernel for the startup workspace and register it in the pool
            let kernel = workspace::resolve_kernel()
                .ok_or("markvault-kernel.exe not found (sidecar missing; set MARKVAULT_KERNEL)")?;
            let mut child = std::process::Command::new(&kernel)
                .arg("serve")
                .arg("--workspace")
                .arg(&workspace)
                .arg("--port")
                .arg(port.to_string())
                .arg("--log-file")
                .spawn()
                .map_err(|e| format!("spawn kernel {}: {}", kernel.display(), e))?;
            if !wait_port_open(port, Duration::from_secs(10)) {
                let _ = child.kill();
                return Err("kernel did not start in time".into());
            }
            let ws = workspace.to_string_lossy().to_string();
            app.state::<WorkspacePool>().instances.lock().unwrap().push(
                workspace::Instance {
                    workspace: ws.clone(),
                    port,
                    child,
                },
            );
            *app.state::<WorkspacePool>().active.lock().unwrap() = ws.clone();
            *app.state::<browser::BrowserState>()
                .kernel_origin
                .lock()
                .unwrap() = format!("http://127.0.0.1:{}", port);
            *app.state::<settings::AppMeta>()
                .workspace
                .lock()
                .unwrap() = ws.clone();

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
                window
                    .app_handle()
                    .state::<WorkspacePool>()
                    .kill_all();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
