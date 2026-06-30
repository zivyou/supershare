pub mod tls;
pub mod framing;
pub mod server;
pub mod client;
pub mod pairing;
pub mod cert;

/// Events emitted by the server for state synchronization
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// A client has connected
    ClientConnected { name: String },
    /// A client has disconnected
    ClientDisconnected { name: String },
    /// A client has completed pairing (cert provisioned)
    ClientPaired { name: String },
}
