use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub client: ClientConfig,

    #[serde(default)]
    pub clipboard: ClipboardConfig,
}

/// Server-mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Control channel listen port
    #[serde(default = "default_control_port")]
    pub control_port: u16,

    /// Data channel listen port
    #[serde(default = "default_data_port")]
    pub data_port: u16,

    /// TLS certificate file path
    pub cert_path: Option<PathBuf>,

    /// TLS private key file path
    pub key_path: Option<PathBuf>,

    /// CA certificate file path
    pub ca_path: Option<PathBuf>,

    /// Connected clients (name, IP, screen position)
    #[serde(default)]
    pub clients: Vec<ClientEntry>,
}

/// A registered client entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEntry {
    pub name: String,
    pub ip: String,
    pub screen_width: u32,
    pub screen_height: u32,
    /// Position relative to server: "right" (default)
    #[serde(default = "default_position")]
    pub position: String,
}

/// Client-mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Server address (host:port)
    pub server_address: Option<String>,

    /// TLS certificate file path
    pub cert_path: Option<PathBuf>,

    /// TLS private key file path
    pub key_path: Option<PathBuf>,

    /// CA certificate file path
    pub ca_path: Option<PathBuf>,

    /// Device name shown to other machines
    #[serde(default = "default_device_name")]
    pub device_name: String,
}

/// Clipboard sync configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardConfig {
    /// Enable text clipboard sync
    #[serde(default = "default_true")]
    pub text_enabled: bool,

    /// Enable image clipboard sync
    #[serde(default = "default_true")]
    pub image_enabled: bool,

    /// Maximum image transfer size in bytes (default: 10 MB)
    #[serde(default = "default_max_image_size")]
    pub max_image_size: usize,
}

// Default value functions
fn default_control_port() -> u16 {
    9876
}

fn default_data_port() -> u16 {
    9877
}

fn default_position() -> String {
    "right".to_string()
}

fn default_device_name() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn default_true() -> bool {
    true
}

fn default_max_image_size() -> usize {
    10 * 1024 * 1024
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            control_port: default_control_port(),
            data_port: default_data_port(),
            cert_path: None,
            key_path: None,
            ca_path: None,
            clients: Vec::new(),
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_address: None,
            cert_path: None,
            key_path: None,
            ca_path: None,
            device_name: default_device_name(),
        }
    }
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            text_enabled: true,
            image_enabled: true,
            max_image_size: default_max_image_size(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            client: ClientConfig::default(),
            clipboard: ClipboardConfig::default(),
        }
    }
}

/// Get the platform-appropriate config directory path
pub fn config_dir() -> anyhow::Result<PathBuf> {
    dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))
        .map(|d| d.join("supershare"))
}

/// Get the full config file path
pub fn config_file_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

impl AppConfig {
    /// Load config from the default platform path, or return default if not found
    pub fn load() -> Self {
        match config_file_path() {
            Ok(path) => {
                if path.exists() {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => match toml::from_str::<AppConfig>(&content) {
                            Ok(config) => {
                                tracing::info!("Loaded config from {}", path.display());
                                config
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse config at {}: {e}", path.display());
                                Self::default()
                            }
                        },
                        Err(e) => {
                            tracing::warn!("Failed to read config at {}: {e}", path.display());
                            Self::default()
                        }
                    }
                } else {
                    tracing::info!("No config file found, using defaults");
                    Self::default()
                }
            }
            Err(e) => {
                tracing::warn!("Could not determine config path: {e}");
                Self::default()
            }
        }
    }

    /// Save config to the default platform path
    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        tracing::info!("Saved config to {}", path.display());
        Ok(())
    }
}
