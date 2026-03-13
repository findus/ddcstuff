use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub fade: FadeConfig,
    pub backlight: BacklightConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Override the Unix socket path. Empty = use XDG_RUNTIME_DIR/ddcd.sock.
    pub socket_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FadeConfig {
    /// Enable fade by default (can be overridden per-request).
    pub enabled: bool,
    pub duration_ms: u64,
    pub steps: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BacklightConfig {
    /// Override backlight sysfs path. Empty = auto-detect.
    pub path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            fade: FadeConfig::default(),
            backlight: BacklightConfig::default(),
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: String::new(),
        }
    }
}

impl Default for FadeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            duration_ms: 10,
            steps: 5,
        }
    }
}

impl Default for BacklightConfig {
    fn default() -> Self {
        Self { path: String::new() }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let path = config_path();
        if path.exists() {
            let text = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&text)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }
}

fn config_path() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("ddcd").join("config.toml")
    } else {
        PathBuf::from("/etc/ddcd/config.toml")
    }
}
