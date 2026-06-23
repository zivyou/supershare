use crate::framing::{read_frame, write_frame};
use crate::tls;
use ss_core::protocol::{Message, HEARTBEAT_INTERVAL_SECS, HEARTBEAT_TIMEOUT_SECS};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, timeout, Duration};

/// Client configuration
pub struct ClientConfig {
    pub server_address: String,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_path: PathBuf,
    pub device_name: String,
}

/// Connection state for a client
pub struct ClientConnection {
    /// Send messages to server control channel
    pub control_tx: mpsc::Sender<Message>,
    /// Send messages to server data channel
    pub data_tx: mpsc::Sender<Message>,
    /// Receive messages from server control channel
    pub control_rx: broadcast::Receiver<Message>,
    /// Receive messages from server data channel
    pub data_rx: broadcast::Receiver<Message>,
    /// Server screen dimensions (received during handshake)
    pub server_screen: Option<(u32, u32)>,
}

/// Connect to the server with both control and data channels.
/// Returns a ClientConnection on success.
pub async fn connect(config: ClientConfig) -> anyhow::Result<ClientConnection> {
    let tls_config = tls::build_client_config(&config.cert_path, &config.key_path, &config.ca_path)?;

    // Ensure address has a port (default to 9876)
    let server_address = if config.server_address.contains(':') {
        config.server_address.clone()
    } else {
        format!("{}:9876", config.server_address)
    };

    let server_name = tls::parse_server_name(&server_address)?;

    // Connect control channel
    tracing::info!("Connecting control channel to {}", server_address);
    let control_stream = TcpStream::connect(&server_address).await?;
    let connector = tokio_rustls::TlsConnector::from(tls_config.clone());
    let mut control_tls = connector.connect(server_name.clone(), control_stream).await?;

    // Send handshake on control channel
    write_frame(
        &mut control_tls,
        &Message::Handshake {
            version: 1,
            name: config.device_name.clone(),
        },
    )
    .await?;

    // Read screen config from server
    let msg = timeout(Duration::from_secs(10), read_frame(&mut control_tls)).await??;
    let server_screen = match msg {
        Some(Message::ScreenConfig { width, height }) => {
            tracing::info!("Server screen: {width}x{height}");
            Some((width, height))
        }
        other => {
            anyhow::bail!("Expected ScreenConfig, got {:?}", other);
        }
    };

    // Split control channel
    let (mut control_reader, mut control_writer) = tokio::io::split(control_tls);

    // Create channels
    let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<Message>(128);
    let (ctrl_broadcast_tx, ctrl_broadcast_rx) = broadcast::channel::<Message>(256);

    // Spawn control writer
    tokio::spawn(async move {
        while let Some(msg) = ctrl_rx.recv().await {
            if write_frame(&mut control_writer, &msg).await.is_err() {
                break;
            }
        }
    });

    // Spawn control reader + heartbeat
    let ctrl_broadcast = ctrl_broadcast_tx.clone();
    let ctrl_tx_hb = ctrl_tx.clone();
    tokio::spawn(async move {
        let mut hb_interval = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        loop {
            tokio::select! {
                result = read_frame(&mut control_reader) => {
                    match result {
                        Ok(Some(Message::Heartbeat)) => {
                            // Heartbeat received
                        }
                        Ok(Some(msg)) => {
                            let _ = ctrl_broadcast.send(msg);
                        }
                        Ok(None) => {
                            tracing::info!("Control channel closed by server");
                            break;
                        }
                        Err(e) => {
                            tracing::error!("Control channel read error: {e}");
                            break;
                        }
                    }
                }
                _ = hb_interval.tick() => {
                    // Send heartbeat to server via the writer channel
                    let _ = ctrl_tx_hb.send(Message::Heartbeat).await;
                }
            }
        }
    });

    // Connect data channel (port = control port + 1)
    let data_address = {
        let parts: Vec<&str> = server_address.split(':').collect();
        if parts.len() == 2 {
            let port: u16 = parts[1].parse().unwrap_or(9876);
            format!("{}:{}", parts[0], port + 1)
        } else {
            format!("{}:9877", config.server_address)
        }
    };
    tracing::info!("Connecting data channel to {}", data_address);
    let data_stream = TcpStream::connect(&data_address).await?;
    let data_name = tls::parse_server_name(&data_address)?;
    let mut data_tls = connector.connect(data_name, data_stream).await?;

    // Send handshake on data channel
    write_frame(
        &mut data_tls,
        &Message::Handshake {
            version: 1,
            name: config.device_name.clone(),
        },
    )
    .await?;

    // Split data channel
    let (mut data_reader, mut data_writer) = tokio::io::split(data_tls);

    let (data_tx, mut data_rx) = mpsc::channel::<Message>(64);
    let (data_broadcast_tx, data_broadcast_rx) = broadcast::channel::<Message>(256);

    // Spawn data writer
    tokio::spawn(async move {
        while let Some(msg) = data_rx.recv().await {
            if write_frame(&mut data_writer, &msg).await.is_err() {
                break;
            }
        }
    });

    // Spawn data reader
    tokio::spawn(async move {
        loop {
            match read_frame(&mut data_reader).await {
                Ok(Some(msg)) => {
                    let _ = data_broadcast_tx.send(msg);
                }
                Ok(None) => {
                    tracing::info!("Data channel closed by server");
                    break;
                }
                Err(e) => {
                    tracing::error!("Data channel read error: {e}");
                    break;
                }
            }
        }
    });

    tracing::info!("Client fully connected");

    Ok(ClientConnection {
        control_tx: ctrl_tx,
        data_tx,
        control_rx: ctrl_broadcast_rx,
        data_rx: data_broadcast_rx,
        server_screen,
    })
}

/// Connect with automatic reconnection and exponential backoff
pub async fn connect_with_retry(config: ClientConfig) -> anyhow::Result<ClientConnection> {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        match connect(ClientConfig {
            server_address: config.server_address.clone(),
            cert_path: config.cert_path.clone(),
            key_path: config.key_path.clone(),
            ca_path: config.ca_path.clone(),
            device_name: config.device_name.clone(),
        })
        .await
        {
            Ok(conn) => return Ok(conn),
            Err(e) => {
                tracing::warn!(
                    "Connection failed: {e}. Retrying in {} seconds...",
                    backoff.as_secs()
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        }
    }
}
