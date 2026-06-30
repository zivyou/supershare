//! PIN-authenticated device pairing: bootstrap mTLS trust without pre-shared
//! certificates. See [`crypto`] for the SPAKE2 + AEAD primitives, [`server`]
//! for the listener that signs client certs, and [`client`] for the side that
//! requests provisioning.

use serde::{Deserialize, Serialize};

pub mod client;
pub mod crypto;
pub mod server;

/// Pairing protocol version. Both sides must agree.
pub const PAIR_PROTOCOL_VERSION: u8 = 1;

/// Encrypted provisioning request sent by the client (inside `PairConfirm`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionRequest {
    /// Client certificate signing request (PEM).
    pub csr_pem: String,
    /// Optional device name override (falls back to the PairRequest name).
    pub name: Option<String>,
}

/// Encrypted provisioning response sent by the server (inside `PairResult`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionResponse {
    /// The signed client certificate (PEM).
    pub client_cert_pem: String,
    /// The CA certificate (PEM) the client should trust.
    pub ca_cert_pem: String,
}

/// Outcome of a successful client-side pairing: the provisioned material the
/// client must persist to its trust store.
#[derive(Debug, Clone)]
pub struct PairedMaterial {
    pub client_cert_pem: String,
    pub client_key_pem: String,
    pub ca_cert_pem: String,
}
