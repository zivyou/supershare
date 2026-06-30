//! Client side of the pairing protocol.
//!
//! Connects to the server's pairing port, runs the PIN-authenticated SPAKE2
//! exchange, sends a CSR, and receives a signed certificate + CA certificate.
//! The caller persists the returned [`PairedMaterial`] to the trust store.

use crate::cert;
use crate::framing::{read_frame, write_frame};
use crate::pairing::crypto::PairingExchange;
use crate::pairing::{PairedMaterial, ProvisionRequest, ProvisionResponse, PAIR_PROTOCOL_VERSION};
use ss_core::protocol::Message;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Errors that pairing can produce, typed so the UI/CLI can react.
#[derive(Debug, thiserror::Error)]
pub enum PairError {
    #[error("could not reach pairing service: {0}")]
    Connect(String),
    #[error("pairing was rejected: {0}")]
    Rejected(String),
    #[error("wrong or expired PIN")]
    BadPin,
    #[error("pairing protocol error: {0}")]
    Protocol(String),
}

/// Default pairing port relative to the control port.
pub fn default_pairing_port(control_port: u16) -> u16 {
    control_port.saturating_sub(1)
}

/// Pair with a server: returns provisioned cert material on success.
///
/// `host` is the server host (no port). `pairing_port` is where the server's
/// pairing listener is bound. `pin` is the PIN displayed by the server.
pub async fn pair_with_server(
    host: &str,
    pairing_port: u16,
    pin: &str,
    device_name: &str,
) -> Result<PairedMaterial, PairError> {
    let addr = format!("{host}:{pairing_port}");
    let io_timeout = Duration::from_secs(30);

    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| PairError::Connect(format!("{addr}: {e}")))?;

    // 1. Send PairRequest.
    write_frame(
        &mut stream,
        &Message::PairRequest {
            version: PAIR_PROTOCOL_VERSION,
            name: device_name.to_string(),
        },
    )
    .await
    .map_err(|e| PairError::Protocol(e.to_string()))?;

    // 2. Start SPAKE2 and send our message.
    let (exchange, our_spake) = PairingExchange::start(pin);
    write_frame(&mut stream, &Message::PairSpake { msg: our_spake })
        .await
        .map_err(|e| PairError::Protocol(e.to_string()))?;

    // 3. Read the server's SPAKE2 message (or a PairError).
    let server_spake = match timeout(io_timeout, read_frame(&mut stream)).await {
        Ok(Ok(Some(Message::PairSpake { msg }))) => msg,
        Ok(Ok(Some(Message::PairError { reason }))) => return Err(PairError::Rejected(reason)),
        Ok(Ok(other)) => return Err(PairError::Protocol(format!("expected PairSpake, got {other:?}"))),
        Ok(Err(e)) => return Err(PairError::Protocol(e.to_string())),
        Err(_) => return Err(PairError::Protocol("timed out waiting for server".into())),
    };

    // 4. Derive the session key.
    let session_key = exchange
        .finish(&server_spake)
        .map_err(|e| PairError::Protocol(e.to_string()))?;

    // 5. Generate a keypair + CSR, encrypt the provisioning request, send it.
    let (csr_pem, key_pem) =
        cert::generate_client_csr(device_name).map_err(|e| PairError::Protocol(e.to_string()))?;
    let request = ProvisionRequest {
        csr_pem,
        name: Some(device_name.to_string()),
    };
    let request_bytes =
        bincode::serialize(&request).map_err(|e| PairError::Protocol(e.to_string()))?;
    let (nonce, ciphertext) = session_key
        .seal(&request_bytes)
        .map_err(|e| PairError::Protocol(e.to_string()))?;
    write_frame(&mut stream, &Message::PairConfirm { nonce, ciphertext })
        .await
        .map_err(|e| PairError::Protocol(e.to_string()))?;

    // 6. Read the encrypted result (or a PairError = wrong PIN / rejection).
    let (nonce, ciphertext) = match timeout(io_timeout, read_frame(&mut stream)).await {
        Ok(Ok(Some(Message::PairResult { nonce, ciphertext }))) => (nonce, ciphertext),
        Ok(Ok(Some(Message::PairError { reason }))) => {
            // The server rejects with PairError when our PIN was wrong.
            if reason.contains("PIN") {
                return Err(PairError::BadPin);
            }
            return Err(PairError::Rejected(reason));
        }
        Ok(Ok(other)) => return Err(PairError::Protocol(format!("expected PairResult, got {other:?}"))),
        Ok(Err(e)) => return Err(PairError::Protocol(e.to_string())),
        Err(_) => return Err(PairError::Protocol("timed out waiting for result".into())),
    };

    // 7. Decrypt. Failure here means our PIN did not match the server's.
    let plaintext = session_key
        .open(&nonce, &ciphertext)
        .map_err(|_| PairError::BadPin)?;
    let response: ProvisionResponse =
        bincode::deserialize(&plaintext).map_err(|e| PairError::Protocol(e.to_string()))?;

    Ok(PairedMaterial {
        client_cert_pem: response.client_cert_pem,
        client_key_pem: key_pem,
        ca_cert_pem: response.ca_cert_pem,
    })
}
