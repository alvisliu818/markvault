//! Shell-side settings (M10): vault picker dialog, workspace persistence
//! (%APPDATA%/markvault/config.json) and Windows autostart (HKCU Run key).
//! The kernel conf (port/auth/readonly/...) stays in the SPA settings panel;
//! these are shell-owned concerns that only exist inside the Tauri app.

use std::path::PathBuf;
use serde_json::json;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "markvault";

/// Workspace + port resolved at boot, for the settings panel to display.
#[derive(Default)]
pub struct AppMeta {
    pub workspace: std::sync::Mutex<String>,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(base).join("markvault").join("config.json"))
}

fn autostart_enabled() -> bool {
    use winreg::enums::*;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

/// Open a native folder picker (must run off the UI thread; rfd handles COM
/// per-thread on Windows).
pub fn pick_folder_dialog(title: &str) -> Option<String> {
    rfd::FileDialog::new()
        .set_title(title)
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn pick_vault_dir() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| pick_folder_dialog("选择 vault 目录"))
        .await
        .unwrap_or(None)
}

/// Persist workspace to %APPDATA%/markvault/config.json (read back on next boot).
#[tauri::command]
pub fn set_workspace(path: String) -> Result<(), String> {
    let p = PathBuf::from(path.trim());
    if !p.is_dir() {
        return Err(format!("目录不存在: {}", p.display()));
    }
    let cfg = config_path().ok_or("APPDATA unavailable")?;
    if let Some(parent) = cfg.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut v: serde_json::Value = std::fs::read_to_string(&cfg)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    v["workspace"] = json!(p.to_string_lossy().to_string());
    std::fs::write(&cfg, serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| format!("write config: {}", e))
}

#[tauri::command]
pub fn set_autostart(enable: bool) -> Result<bool, String> {
    use winreg::enums::*;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE | KEY_QUERY_VALUE)
        .map_err(|e| format!("open run key: {}", e))?;
    if enable {
        let exe = std::env::current_exe()
            .map_err(|e| format!("current_exe: {}", e))?
            .to_string_lossy()
            .to_string();
        run.set_value(RUN_VALUE, &exe)
            .map_err(|e| format!("set value: {}", e))?;
    } else {
        run.delete_value(RUN_VALUE)
            .map_err(|e| format!("delete value: {}", e))?;
    }
    Ok(autostart_enabled())
}

#[tauri::command]
pub fn get_shell_conf(meta: tauri::State<'_, AppMeta>) -> serde_json::Value {
    let ws = meta.workspace.lock().unwrap().clone();
    json!({
        "workspace": ws,
        "autostart": autostart_enabled(),
    })
}
