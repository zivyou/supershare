//! End-to-end pairing tests over a real loopback TCP connection.
//!
//! These exercise the full pairing handshake: the server listener signs a
//! client CSR after a successful PIN exchange, and the client persists the
//! provisioned certificate. Negative cases cover a wrong PIN and lockout.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

use ss_network::cert;
use ss_network::pairing::client::{pair_with_server, PairError};
use ss_network::pairing::server::{run_pairing_listener, PairingServerConfig, PinManager};
use ss_network::ServerEvent;

/// Spin up a pairing listener on an ephemeral port, returning the port, the
/// PIN manager (so tests can read the current PIN), and a shutdown sender.
async fn start_server(
    pin_manager: Arc<PinManager>,
) -> (
    u16,
    mpsc::Receiver<ss_core::config::PairedClient>,
    broadcast::Sender<()>,
) {
    // Generate a CA + server cert in a temp dir.
    let tmp = std::env::temp_dir().join(format!("ss-pair-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ca = cert::ensure_server_ca(&tmp, &[]).unwrap();
    let ca_cert_pem = std::fs::read_to_string(&ca.ca_cert_path).unwrap();
    let ca_key_pem = std::fs::read_to_string(&ca.ca_key_path).unwrap();

    // Bind an ephemeral port first to learn the number, then hand it to the
    // listener (which rebinds — fine on loopback for tests).
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let (paired_tx, paired_rx) = mpsc::channel(8);
    let (notify_tx, _notify_rx) = broadcast::channel(8);
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    let config = PairingServerConfig {
        pairing_port: port,
        ca_cert_pem,
        ca_key_pem,
    };
    let pm = pin_manager.clone();
    let notify = notify_tx.clone();
    tokio::spawn(async move {
        let _ = run_pairing_listener(config, pm, paired_tx, notify, shutdown_rx).await;
    });

    // Give the listener a moment to bind.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = notify_tx; // keep alive
    (port, paired_rx, shutdown_tx)
}

#[tokio::test]
async fn pairing_succeeds_with_correct_pin() {
    let pin_manager = Arc::new(PinManager::new());
    let pin = pin_manager.current_pin();
    let (port, mut paired_rx, shutdown) = start_server(pin_manager.clone()).await;

    let material = pair_with_server("127.0.0.1", port, &pin, "test-laptop")
        .await
        .expect("pairing should succeed");

    // Client received a signed cert + CA + its own key.
    assert!(material.client_cert_pem.contains("BEGIN CERTIFICATE"));
    assert!(material.ca_cert_pem.contains("BEGIN CERTIFICATE"));
    assert!(material.client_key_pem.contains("PRIVATE KEY"));

    // Server recorded the paired client.
    let paired = tokio::time::timeout(Duration::from_secs(2), paired_rx.recv())
        .await
        .expect("should receive paired record")
        .expect("channel open");
    assert_eq!(paired.name, "test-laptop");
    assert_eq!(paired.cert_fingerprint.len(), 64);

    let _ = shutdown.send(());
}

#[tokio::test]
async fn pairing_fails_with_wrong_pin() {
    let pin_manager = Arc::new(PinManager::new());
    let correct = pin_manager.current_pin();
    let wrong = if correct == "000000" { "111111" } else { "000000" };
    let (port, _paired_rx, shutdown) = start_server(pin_manager.clone()).await;

    let result = pair_with_server("127.0.0.1", port, wrong, "bad-client").await;
    match result {
        Err(PairError::BadPin) | Err(PairError::Rejected(_)) | Err(PairError::Protocol(_)) => {}
        other => panic!("expected pairing failure, got {other:?}"),
    }

    let _ = shutdown.send(());
}

#[tokio::test]
async fn server_locks_out_after_repeated_failures() {
    // Small failure threshold for a fast test.
    let pin_manager = Arc::new(PinManager::with_params(
        Duration::from_secs(180),
        2,
        Duration::from_secs(60),
    ));
    let correct = pin_manager.current_pin();
    let wrong = if correct == "000000" { "111111" } else { "000000" };
    let (port, _paired_rx, shutdown) = start_server(pin_manager.clone()).await;

    // Two failed attempts trip the lockout.
    let _ = pair_with_server("127.0.0.1", port, wrong, "c1").await;
    let _ = pair_with_server("127.0.0.1", port, wrong, "c2").await;

    // Give the server a moment to process the second failure.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(pin_manager.is_locked(), "server should be locked out");

    // Even the correct PIN is now rejected while locked.
    let result = pair_with_server("127.0.0.1", port, &correct, "c3").await;
    assert!(result.is_err(), "locked server must reject pairing");

    let _ = shutdown.send(());
}
