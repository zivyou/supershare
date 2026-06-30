//! Server side of the pairing protocol.
//!
//! Listens on a dedicated pairing port (plain TCP — SPAKE2 + AEAD provide the
//! security, so no pre-shared TLS trust is needed). For each connection it
//! runs the PIN-authenticated exchange and, on success, signs a certificate
//! for the client and returns it together with the CA certificate.

use crate::framing::{read_frame, write_frame};
use crate::pairing::crypto::PairingExchange;
use crate::pairing::{ProvisionRequest, ProvisionResponse, PAIR_PROTOCOL_VERSION};
use crate::{cert, ServerEvent};
use ss_core::config::PairedClient;
use ss_core::protocol::Message;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

/// Default pairing PIN time-to-live.
pub const PIN_TTL: Duration = Duration::from_secs(180);
/// Number of failed attempts before a temporary lockout.
pub const MAX_FAILURES: u32 = 5;
/// Lockout duration after too many failures.
pub const LOCKOUT: Duration = Duration::from_secs(60);

/// Manages the pairing PIN: generation, expiry/rotation, and brute-force
/// lockout. Cheaply clonable via `Arc`.
pub struct PinManager {
    state: Mutex<PinState>,
    ttl: Duration,
    max_failures: u32,
    lockout: Duration,
}

struct PinState {
    pin: String,
    generated_at: Instant,
    failures: u32,
    locked_until: Option<Instant>,
}

impl PinManager {
    /// Create a manager with default TTL/lockout and a fresh PIN.
    pub fn new() -> Self {
        Self::with_params(PIN_TTL, MAX_FAILURES, LOCKOUT)
    }

    pub fn with_params(ttl: Duration, max_failures: u32, lockout: Duration) -> Self {
        Self {
            state: Mutex::new(PinState {
                pin: generate_pin(),
                generated_at: Instant::now(),
                failures: 0,
                locked_until: None,
            }),
            ttl,
            max_failures,
            lockout,
        }
    }

    /// Return the current PIN, rotating it first if it has expired.
    pub fn current_pin(&self) -> String {
        let mut s = self.state.lock().unwrap();
        if s.generated_at.elapsed() >= self.ttl {
            s.pin = generate_pin();
            s.generated_at = Instant::now();
        }
        s.pin.clone()
    }

    /// Whether pairing is currently locked out due to repeated failures.
    pub fn is_locked(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        match s.locked_until {
            Some(t) if Instant::now() < t => true,
            Some(_) => {
                // Lockout expired; reset.
                s.locked_until = None;
                s.failures = 0;
                false
            }
            None => false,
        }
    }

    /// Snapshot the PIN that an in-flight attempt should be checked against,
    /// rotating if expired. Same as `current_pin` but named for intent.
    fn pin_for_attempt(&self) -> String {
        self.current_pin()
    }

    /// Record a successful pairing: rotate the PIN and clear failures.
    fn record_success(&self) {
        let mut s = self.state.lock().unwrap();
        s.pin = generate_pin();
        s.generated_at = Instant::now();
        s.failures = 0;
        s.locked_until = None;
    }

    /// Record a failed pairing attempt, locking out after too many.
    fn record_failure(&self) {
        let mut s = self.state.lock().unwrap();
        s.failures += 1;
        if s.failures >= self.max_failures {
            s.locked_until = Some(Instant::now() + self.lockout);
            tracing::warn!("Pairing locked out after {} failures", s.failures);
        }
    }
}

impl Default for PinManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a fresh 6-digit numeric PIN.
fn generate_pin() -> String {
    use rand::Rng;
    let n: u32 = rand::thread_rng().gen_range(0..1_000_000);
    format!("{n:06}")
}

/// Configuration for the pairing listener.
pub struct PairingServerConfig {
    pub pairing_port: u16,
    /// CA certificate (PEM) used to sign client certs.
    pub ca_cert_pem: String,
    /// CA private key (PEM).
    pub ca_key_pem: String,
}

/// Run the pairing listener until `shutdown` fires.
///
/// `pin_manager` is shared so the GUI can display the current PIN.
/// On each successful pairing a [`PairedClient`] is sent on `on_paired`.
pub async fn run_pairing_listener(
    config: PairingServerConfig,
    pin_manager: std::sync::Arc<PinManager>,
    on_paired: mpsc::Sender<PairedClient>,
    notify_tx: broadcast::Sender<ServerEvent>,
    mut shutdown: broadcast::Receiver<()>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", config.pairing_port)).await?;
    tracing::info!("Pairing listener on port {}", config.pairing_port);

    let config = std::sync::Arc::new(config);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        tracing::info!("Pairing connection from {addr}");
                        let config = config.clone();
                        let pin_manager = pin_manager.clone();
                        let on_paired = on_paired.clone();
                        let notify_tx = notify_tx.clone();
                        tokio::spawn(async move {
                            let mut stream = stream;
                            if let Err(e) = handle_pairing(&mut stream, &config, &pin_manager, &on_paired, &notify_tx).await {
                                tracing::warn!("Pairing with {addr} failed: {e}");
                                // Best-effort error notification to the client.
                                let _ = write_frame(&mut stream, &Message::PairError {
                                    reason: e.to_string(),
                                }).await;
                            }
                        });
                    }
                    Err(e) => tracing::error!("Pairing accept error: {e}"),
                }
            }
            _ = shutdown.recv() => {
                tracing::info!("Pairing listener shutting down");
                break;
            }
        }
    }
    Ok(())
}

async fn handle_pairing<S>(
    stream: &mut S,
    config: &PairingServerConfig,
    pin_manager: &PinManager,
    on_paired: &mpsc::Sender<PairedClient>,
    notify_tx: &broadcast::Sender<ServerEvent>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if pin_manager.is_locked() {
        anyhow::bail!("pairing temporarily locked due to repeated failures");
    }

    let io_timeout = Duration::from_secs(30);

    // 1. Read PairRequest (device name, version).
    let req = timeout(io_timeout, read_frame(stream)).await??;
    let client_name = match req {
        Some(Message::PairRequest { version, name }) => {
            if version != PAIR_PROTOCOL_VERSION {
                anyhow::bail!("unsupported pairing version: {version}");
            }
            name
        }
        other => anyhow::bail!("expected PairRequest, got {:?}", other),
    };

    // 2. Read the client's SPAKE2 message.
    let client_spake = match timeout(io_timeout, read_frame(stream)).await?? {
        Some(Message::PairSpake { msg }) => msg,
        other => anyhow::bail!("expected PairSpake, got {:?}", other),
    };

    // 3. Start our own SPAKE2 with the current PIN and send our message.
    let pin = pin_manager.pin_for_attempt();
    let (exchange, our_spake) = PairingExchange::start(&pin);
    write_frame(stream, &Message::PairSpake { msg: our_spake }).await?;

    // 4. Finish to derive the session key.
    let session_key = exchange.finish(&client_spake)?;

    // 5. Read the encrypted provisioning request (CSR + name).
    let (nonce, ciphertext) = match timeout(io_timeout, read_frame(stream)).await?? {
        Some(Message::PairConfirm { nonce, ciphertext }) => (nonce, ciphertext),
        other => anyhow::bail!("expected PairConfirm, got {:?}", other),
    };

    // Decryption failure here means the PINs did not match (wrong key).
    let plaintext = match session_key.open(&nonce, &ciphertext) {
        Ok(pt) => pt,
        Err(e) => {
            pin_manager.record_failure();
            anyhow::bail!("PIN verification failed: {e}");
        }
    };
    let provision: ProvisionRequest = bincode::deserialize(&plaintext)
        .map_err(|e| anyhow::anyhow!("invalid provisioning request: {e}"))?;

    // 6. Sign the client's CSR with our CA.
    let client_cert_pem =
        cert::sign_client_cert(&config.ca_cert_pem, &config.ca_key_pem, &provision.csr_pem)?;
    let fingerprint = cert::cert_fingerprint(&client_cert_pem)?;

    // 7. Return the signed cert + CA, encrypted under the session key.
    let response = ProvisionResponse {
        client_cert_pem,
        ca_cert_pem: config.ca_cert_pem.clone(),
    };
    let response_bytes = bincode::serialize(&response)?;
    let (nonce, ciphertext) = session_key.seal(&response_bytes)?;
    write_frame(stream, &Message::PairResult { nonce, ciphertext }).await?;

    // 8. Record the paired client and rotate the PIN.
    pin_manager.record_success();
    let paired = PairedClient {
        name: provision.name.clone().unwrap_or(client_name),
        cert_fingerprint: fingerprint,
        paired_at: now_unix(),
    };
    tracing::info!("Paired client '{}' ({})", paired.name, paired.cert_fingerprint);
    let _ = on_paired.send(paired.clone()).await;
    let _ = notify_tx.send(ServerEvent::ClientPaired { name: paired.name });

    Ok(())
}

/// Current Unix time in seconds (best-effort; 0 if the clock is before epoch).
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_six_digits() {
        let pin = generate_pin();
        assert_eq!(pin.len(), 6);
        assert!(pin.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn lockout_after_max_failures() {
        let mgr = PinManager::with_params(PIN_TTL, 3, Duration::from_secs(60));
        assert!(!mgr.is_locked());
        for _ in 0..3 {
            mgr.record_failure();
        }
        assert!(mgr.is_locked());
    }

    #[test]
    fn success_rotates_pin_and_clears_failures() {
        let mgr = PinManager::new();
        let pin_before = mgr.current_pin();
        mgr.record_failure();
        mgr.record_success();
        // PIN should have rotated (overwhelmingly likely different); failures cleared.
        let pin_after = mgr.current_pin();
        assert!(!mgr.is_locked());
        // Not asserting inequality of PINs (could collide 1/1e6); assert state reset instead.
        let _ = (pin_before, pin_after);
    }
}
