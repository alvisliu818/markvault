use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Kernel config persisted at `<workspace>/.markvault/conf.json`.
/// Modeled after SiYuan's conf.json (kernel/conf/model) but minimal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conf {
    pub kernel_version: String,
    /// HTTP port (default 7806; SiYuan uses 6806)
    #[serde(default = "default_port")]
    pub port: u16,
    /// 伺服: bind 0.0.0.0 so LAN/browser clients can reach the kernel
    #[serde(default)]
    pub network_serve: bool,
    /// Access auth code protecting the served instance (lock screen)
    #[serde(default)]
    pub access_auth_code: String,
    /// API token for programmatic access (Authorization: Token <t>)
    #[serde(default)]
    pub api_token: String,
    #[serde(default)]
    pub readonly: bool,
    /// Frontmatter property holding virtual-tree parent wikilinks
    #[serde(default = "default_parent_attr")]
    pub parent_attr: String,
    /// Frontmatter numeric weight field for virtual-tree sorting
    #[serde(default = "default_weight_attr")]
    pub weight_attr: String,
    /// Extra ignore patterns (glob-ish prefix patterns) for indexing
    #[serde(default = "default_ignore")]
    pub ignore_patterns: Vec<String>,
    /// UI theme: 0 light, 1 dark
    #[serde(default = "default_theme")]
    pub theme: u8,
}

fn default_port() -> u16 {
    7806
}
fn default_parent_attr() -> String {
    "parent".into()
}
fn default_weight_attr() -> String {
    "weight".into()
}
fn default_theme() -> u8 {
    1
}
fn default_ignore() -> Vec<String> {
    vec![
        ".markvault/**".into(),
        ".obsidian/**".into(),
        ".git/**".into(),
        "node_modules/**".into(),
        "trash/**".into(),
    ]
}

impl Default for Conf {
    fn default() -> Self {
        Conf {
            kernel_version: env!("CARGO_PKG_VERSION").into(),
            port: default_port(),
            network_serve: false,
            access_auth_code: String::new(),
            api_token: String::new(),
            readonly: false,
            parent_attr: default_parent_attr(),
            weight_attr: default_weight_attr(),
            ignore_patterns: default_ignore(),
            theme: default_theme(),
        }
    }
}

impl Conf {
    pub fn load(dir: &Path) -> Conf {
        let path = dir.join("conf.json");
        match fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Conf>(&s) {
                Ok(mut c) => {
                    c.kernel_version = env!("CARGO_PKG_VERSION").into();
                    c
                }
                Err(e) => {
                    tracing::warn!("conf.json parse failed ({}), using defaults", e);
                    Conf::default()
                }
            },
            Err(_) => Conf::default(),
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = dir.join("conf.json");
        let tmp = dir.join("conf.json.tmp");
        fs::create_dir_all(dir)?;
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }
}

pub fn markvault_dir(workspace: &Path) -> PathBuf {
    workspace.join(".markvault")
}
