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

    /// Whether pairing (PIN-based provisioning) is enabled
    #[serde(default = "default_true")]
    pub pairing_enabled: bool,

    /// Clients that have completed pairing (trust store)
    #[serde(default)]
    pub paired_clients: Vec<PairedClient>,
}

/// A client that has paired with this server. Identified by certificate
/// fingerprint so it can be recognised / revoked independently of its name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedClient {
    /// Device name presented during pairing
    pub name: String,
    /// Hex-encoded fingerprint of the client's provisioned certificate
    pub cert_fingerprint: String,
    /// Unix timestamp (seconds) when pairing completed
    #[serde(default)]
    pub paired_at: u64,
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

    /// Servers this client has paired with (trust store), keyed by address
    #[serde(default)]
    pub known_servers: Vec<KnownServer>,
}

/// A server this client has paired with. The provisioned certificate, key and
/// CA are persisted so reconnecting needs no PIN.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownServer {
    /// Server address (host or host:port) used as the lookup key
    pub address: String,
    /// Path to the provisioned client certificate (PEM)
    pub cert_path: PathBuf,
    /// Path to the client private key (PEM)
    pub key_path: PathBuf,
    /// Path to the CA certificate (PEM) returned during pairing
    pub ca_path: PathBuf,
    /// Hex-encoded fingerprint of the server's certificate observed at pairing
    #[serde(default)]
    pub server_fingerprint: String,
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
            pairing_enabled: true,
            paired_clients: Vec::new(),
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
            known_servers: Vec::new(),
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

/// Get the directory holding provisioned trust material (certs/keys/CA).
/// Layout: `<config_dir>/trust/`.
pub fn trust_dir() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("trust"))
}

/// Ensure the trust directory exists and return its path.
pub fn ensure_trust_dir() -> anyhow::Result<PathBuf> {
    let dir = trust_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Persist provisioned PEM material for a server into the trust directory and
/// return the paths written. Files are named per `key` (e.g. the server
/// address with unsafe characters replaced) to keep multiple servers separate.
pub fn write_trust_material(
    key: &str,
    client_cert_pem: &str,
    client_key_pem: &str,
    ca_cert_pem: &str,
) -> anyhow::Result<(PathBuf, PathBuf, PathBuf)> {
    let dir = ensure_trust_dir()?;
    let safe: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let cert_path = dir.join(format!("{safe}.pem"));
    let key_path = dir.join(format!("{safe}-key.pem"));
    let ca_path = dir.join(format!("{safe}-ca.pem"));
    std::fs::write(&cert_path, client_cert_pem)?;
    std::fs::write(&key_path, client_key_pem)?;
    std::fs::write(&ca_path, ca_cert_pem)?;
    Ok((cert_path, key_path, ca_path))
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

impl ClientConfig {
    /// Find a known (paired) server by address.
    pub fn find_known_server(&self, address: &str) -> Option<&KnownServer> {
        self.known_servers.iter().find(|s| s.address == address)
    }

    /// Insert or replace a known server entry, keyed by address.
    pub fn upsert_known_server(&mut self, server: KnownServer) {
        if let Some(existing) = self
            .known_servers
            .iter_mut()
            .find(|s| s.address == server.address)
        {
            *existing = server;
        } else {
            self.known_servers.push(server);
        }
    }

    /// Forget a known server by address. Returns true if an entry was removed.
    pub fn forget_server(&mut self, address: &str) -> bool {
        let before = self.known_servers.len();
        self.known_servers.retain(|s| s.address != address);
        self.known_servers.len() != before
    }
}

impl ServerConfig {
    /// Whether a client with the given certificate fingerprint is paired.
    pub fn is_client_paired(&self, fingerprint: &str) -> bool {
        self.paired_clients
            .iter()
            .any(|c| c.cert_fingerprint == fingerprint)
    }

    /// Insert or replace a paired-client record, keyed by certificate fingerprint.
    pub fn upsert_paired_client(&mut self, client: PairedClient) {
        if let Some(existing) = self
            .paired_clients
            .iter_mut()
            .find(|c| c.cert_fingerprint == client.cert_fingerprint)
        {
            *existing = client;
        } else {
            self.paired_clients.push(client);
        }
    }

    /// Revoke a paired client by certificate fingerprint. Returns true if removed.
    pub fn revoke_client(&mut self, fingerprint: &str) -> bool {
        let before = self.paired_clients.len();
        self.paired_clients
            .retain(|c| c.cert_fingerprint != fingerprint);
        self.paired_clients.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_preserves_trust_store() {
        let mut config = AppConfig::default();
        config.client.upsert_known_server(KnownServer {
            address: "192.168.1.10".to_string(),
            cert_path: PathBuf::from("/trust/client.pem"),
            key_path: PathBuf::from("/trust/client-key.pem"),
            ca_path: PathBuf::from("/trust/ca.pem"),
            server_fingerprint: "abcd".to_string(),
        });
        config.server.upsert_paired_client(PairedClient {
            name: "laptop".to_string(),
            cert_fingerprint: "deadbeef".to_string(),
            paired_at: 42,
        });

        let serialized = toml::to_string_pretty(&config).unwrap();
        let loaded: AppConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(loaded.client.known_servers.len(), 1);
        assert_eq!(loaded.server.paired_clients.len(), 1);
        assert_eq!(
            loaded.client.find_known_server("192.168.1.10").unwrap().server_fingerprint,
            "abcd"
        );
        assert!(loaded.server.is_client_paired("deadbeef"));
    }

    #[test]
    fn known_server_upsert_and_forget() {
        let mut client = ClientConfig::default();
        let make = |addr: &str, fp: &str| KnownServer {
            address: addr.to_string(),
            cert_path: PathBuf::from("c"),
            key_path: PathBuf::from("k"),
            ca_path: PathBuf::from("ca"),
            server_fingerprint: fp.to_string(),
        };
        client.upsert_known_server(make("host-a", "fp1"));
        client.upsert_known_server(make("host-a", "fp2")); // replace, not duplicate
        assert_eq!(client.known_servers.len(), 1);
        assert_eq!(client.find_known_server("host-a").unwrap().server_fingerprint, "fp2");

        assert!(client.forget_server("host-a"));
        assert!(!client.forget_server("host-a"));
        assert!(client.find_known_server("host-a").is_none());
    }

    #[test]
    fn paired_client_upsert_and_revoke() {
        let mut server = ServerConfig::default();
        let make = |name: &str, fp: &str| PairedClient {
            name: name.to_string(),
            cert_fingerprint: fp.to_string(),
            paired_at: 0,
        };
        server.upsert_paired_client(make("a", "fp1"));
        server.upsert_paired_client(make("a-renamed", "fp1")); // replace by fingerprint
        assert_eq!(server.paired_clients.len(), 1);
        assert!(server.is_client_paired("fp1"));

        assert!(server.revoke_client("fp1"));
        assert!(!server.revoke_client("fp1"));
        assert!(!server.is_client_paired("fp1"));
    }
}
