use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::mpsc;

/// Information about a connected client
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub name: String,
    pub connected_at: Instant,
}

/// Shared state between UI and backend runtime
#[derive(Debug)]
pub struct SharedAppState {
    /// Whether the server is currently running
    pub server_running: bool,
    /// The port the server is listening on
    pub server_port: Option<u16>,
    /// Currently connected clients
    pub connected_clients: Vec<ClientInfo>,
    /// Whether the client is connected to a server
    pub client_connected: bool,
    /// The address of the connected server
    pub client_server_addr: Option<String>,
    /// The server's screen resolution (width, height)
    pub server_screen_size: Option<(u32, u32)>,
    /// Last error message to display
    pub last_error: Option<String>,
    /// Current pairing PIN to display (server side, when pairing enabled)
    pub pairing_pin: Option<String>,
    /// Whether the client needs the user to enter a pairing PIN
    pub pairing_required: bool,
    /// Status message for an in-progress or failed pairing (client side)
    pub pairing_status: Option<String>,
}

impl Default for SharedAppState {
    fn default() -> Self {
        Self {
            server_running: false,
            server_port: None,
            connected_clients: Vec::new(),
            client_connected: false,
            client_server_addr: None,
            server_screen_size: None,
            last_error: None,
            pairing_pin: None,
            pairing_required: false,
            pairing_status: None,
        }
    }
}

/// Thread-safe shared state
pub type SharedState = Arc<RwLock<SharedAppState>>;

/// Commands sent from UI to backend
#[derive(Debug)]
pub enum AppCommand {
    /// Start the server with the given configuration.
    /// Cert paths are optional; when omitted the server auto-generates a CA.
    StartServer {
        control_port: u16,
        data_port: u16,
        cert_path: Option<std::path::PathBuf>,
        key_path: Option<std::path::PathBuf>,
        ca_path: Option<std::path::PathBuf>,
        /// Whether to enable PIN-based pairing.
        pairing_enabled: bool,
    },
    /// Stop the running server
    StopServer,
    /// Connect as a client to the given server.
    /// Cert paths are optional; when omitted, persisted trust (or pairing) is used.
    ConnectClient {
        server_address: String,
        cert_path: Option<std::path::PathBuf>,
        key_path: Option<std::path::PathBuf>,
        ca_path: Option<std::path::PathBuf>,
        device_name: String,
    },
    /// Pair with a server using a PIN, then connect.
    PairAndConnect {
        server_address: String,
        pin: String,
        device_name: String,
    },
    /// Disconnect the client
    DisconnectClient,
}

/// Command sender type
pub type CommandSender = mpsc::Sender<AppCommand>;
/// Command receiver type
pub type CommandReceiver = mpsc::Receiver<AppCommand>;
