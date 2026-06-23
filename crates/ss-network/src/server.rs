use crate::framing::{read_frame, write_frame};
use crate::tls;
use crate::ServerEvent;
use ss_core::protocol::{
    Message, HEARTBEAT_INTERVAL_SECS, HEARTBEAT_TIMEOUT_SECS,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::{interval, timeout, Duration};
use tokio_rustls::TlsAcceptor;

/// Represents a connected client with its control and data channels
pub struct ConnectedClient {
    pub name: String,
    pub control_tx: mpsc::Sender<Message>,
    pub data_tx: mpsc::Sender<Message>,
}

/// Shared server state
pub struct ServerState {
    /// Connected clients by name
    pub clients: RwLock<HashMap<String, ConnectedClient>>,
    /// Broadcast channel for messages received from any client
    pub broadcast_rx: broadcast::Sender<(String, Message)>,
    /// Notification channel for client connect/disconnect events
    pub notify_tx: broadcast::Sender<ServerEvent>,
    /// Screen width of the server
    pub screen_width: u32,
    /// Screen height of the server
    pub screen_height: u32,
}

impl ServerState {
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        let (broadcast_tx, _) = broadcast::channel(256);
        let (notify_tx, _) = broadcast::channel(64);
        Self {
            clients: RwLock::new(HashMap::new()),
            broadcast_rx: broadcast_tx,
            notify_tx,
            screen_width,
            screen_height,
        }
    }
}

/// Server configuration
pub struct ServerConfig {
    pub control_port: u16,
    pub data_port: u16,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_path: PathBuf,
}

/// Start the server with control and data channel listeners
pub async fn start(
    config: ServerConfig,
    state: Arc<ServerState>,
    mut shutdown: broadcast::Receiver<()>,
) -> anyhow::Result<()> {
    let tls_config = tls::build_server_config(&config.cert_path, &config.key_path, &config.ca_path)?;
    let acceptor = TlsAcceptor::from(tls_config);

    let control_listener = TcpListener::bind(("0.0.0.0", config.control_port)).await?;
    let data_listener = TcpListener::bind(("0.0.0.0", config.data_port)).await?;

    tracing::info!(
        "Server listening on ports {} (control) and {} (data)",
        config.control_port,
        config.data_port
    );

    // Pending data connections: device_name -> sender
    let pending_data: Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>> =
        Arc::new(RwLock::new(HashMap::new()));

    // Spawn control channel listener
    let state_ctrl = state.clone();
    let acceptor_ctrl = acceptor.clone();
    let pending_data_ctrl = pending_data.clone();
    let mut shutdown_ctrl = shutdown.resubscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = control_listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            tracing::info!("Control connection from {addr}");
                            let acceptor = acceptor_ctrl.clone();
                            let state = state_ctrl.clone();
                            let pending = pending_data_ctrl.clone();
                            tokio::spawn(async move {
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => {
                                        if let Err(e) = handle_control_connection(tls_stream, state, pending).await {
                                            tracing::error!("Control connection error: {e}");
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("TLS handshake failed from {addr}: {e}");
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {e}");
                        }
                    }
                }
                _ = shutdown_ctrl.recv() => {
                    tracing::info!("Control listener shutting down");
                    break;
                }
            }
        }
    });

    // Spawn data channel listener
    let state_data = state.clone();
    let acceptor_data = acceptor.clone();
    let pending_data_d = pending_data.clone();
    let mut shutdown_data = shutdown.resubscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = data_listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            tracing::info!("Data connection from {addr}");
                            let acceptor = acceptor_data.clone();
                            let state = state_data.clone();
                            let pending = pending_data_d.clone();
                            tokio::spawn(async move {
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => {
                                        if let Err(e) = handle_data_connection(tls_stream, state, pending).await {
                                            tracing::error!("Data connection error: {e}");
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("TLS handshake failed from {addr}: {e}");
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {e}");
                        }
                    }
                }
                _ = shutdown_data.recv() => {
                    tracing::info!("Data listener shutting down");
                    break;
                }
            }
        }
    });

    // Wait for shutdown signal
    let _ = shutdown.recv().await;
    tracing::info!("Server shutting down");
    Ok(())
}

/// Handle an incoming control channel connection: perform handshake, then process messages
async fn handle_control_connection(
    mut tls_stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    state: Arc<ServerState>,
    pending_data: Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>,
) -> anyhow::Result<()> {
    // Read handshake from client
    let msg = timeout(Duration::from_secs(10), read_frame(&mut tls_stream)).await??;
    let client_name = match msg {
        Some(Message::Handshake { version, name }) => {
            if version != 1 {
                write_frame(&mut tls_stream, &Message::Heartbeat).await?; // TODO: error message
                anyhow::bail!("Unsupported protocol version: {version}");
            }
            tracing::info!("Client handshake: {name}");
            name
        }
        _ => anyhow::bail!("Expected Handshake message, got {:?}", msg),
    };

    // Send screen config
    write_frame(
        &mut tls_stream,
        &Message::ScreenConfig {
            width: state.screen_width,
            height: state.screen_height,
        },
    )
    .await?;

    // Create channels for this client
    let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<Message>(128);

    // Wait for the data channel to arrive
    let data_rx = {
        let mut attempts = 0;
        loop {
            {
                let mut pending = pending_data.write().await;
                if let Some(data_tx) = pending.remove(&client_name) {
                    let (_data_tx_for_write, data_rx) = mpsc::channel::<Message>(128);
                    // Store the client
                    let client = ConnectedClient {
                        name: client_name.clone(),
                        control_tx: ctrl_tx.clone(),
                        data_tx,
                    };
                    state.clients.write().await.insert(client_name.clone(), client);
                    break data_rx;
                }
            }
            attempts += 1;
            if attempts > 50 {
                anyhow::bail!("Timeout waiting for data channel from {client_name}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    tracing::info!("Client {client_name} fully connected");

    // Notify that a client has connected
    let _ = state.notify_tx.send(ServerEvent::ClientConnected {
        name: client_name.clone(),
    });

    // Split the TLS stream for concurrent read/write
    let (mut reader, writer) = tokio::io::split(tls_stream);

    // Single writer task: handles both heartbeats and control messages
    // Exits when ctrl_rx closes (ctrl_tx dropped = client removed from clients map)
    let mut hb_interval = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    tokio::spawn(async move {
        let mut writer = writer;
        loop {
            tokio::select! {
                _ = hb_interval.tick() => {
                    if write_frame(&mut writer, &Message::Heartbeat).await.is_err() {
                        break;
                    }
                }
                msg = ctrl_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            if write_frame(&mut writer, &msg).await.is_err() {
                                break;
                            }
                        }
                        None => {
                            // ctrl_tx dropped, client disconnected
                            break;
                        }
                    }
                }
            }
        }
    });

    // Main message loop: read from client, forward to broadcast
    loop {
        let hb_timeout = Duration::from_secs(HEARTBEAT_TIMEOUT_SECS);
        match timeout(hb_timeout, read_frame(&mut reader)).await {
            Ok(Ok(Some(msg))) => {
                match &msg {
                    Message::Heartbeat => {
                        // Heartbeat received, connection is alive
                    }
                    _ => {
                        tracing::trace!("Received from {client_name}: {:?}", msg.msg_type());
                        // Broadcast to other components
                        let _ = state.broadcast_rx.send((client_name.clone(), msg));
                    }
                }
            }
            Ok(Ok(None)) => {
                tracing::info!("Client {client_name} disconnected (EOF)");
                break;
            }
            Ok(Err(e)) => {
                tracing::error!("Error reading from {client_name}: {e}");
                break;
            }
            Err(_) => {
                tracing::warn!("Client {client_name} timed out (no heartbeat)");
                break;
            }
        }
    }

    // Cleanup
    state.clients.write().await.remove(&client_name);
    tracing::info!("Client {client_name} removed");

    // Notify that a client has disconnected
    let _ = state.notify_tx.send(ServerEvent::ClientDisconnected {
        name: client_name,
    });

    Ok(())
}

/// Handle an incoming data channel connection: match to pending client, process clipboard data
async fn handle_data_connection(
    mut tls_stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    state: Arc<ServerState>,
    pending_data: Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>,
) -> anyhow::Result<()> {
    // Read handshake from client
    let msg = timeout(Duration::from_secs(10), read_frame(&mut tls_stream)).await??;
    let client_name = match msg {
        Some(Message::Handshake { version: _, name }) => {
            tracing::info!("Data channel handshake: {name}");
            name
        }
        _ => anyhow::bail!("Expected Handshake on data channel"),
    };

    // Register this data channel as pending
    let (data_tx, mut data_rx) = mpsc::channel::<Message>(64);
    {
        let mut pending = pending_data.write().await;
        pending.insert(client_name.clone(), data_tx);
    }

    // Split for concurrent read/write
    let (mut reader, mut writer) = tokio::io::split(tls_stream);

    // Spawn writer task: send queued messages to client
    let client_name_w = client_name.clone();
    tokio::spawn(async move {
        while let Some(msg) = data_rx.recv().await {
            if write_frame(&mut writer, &msg).await.is_err() {
                tracing::warn!("Data channel write error for {client_name_w}");
                break;
            }
        }
    });

    // Read loop: receive clipboard data from client
    loop {
        match read_frame(&mut reader).await {
            Ok(Some(msg)) => {
                tracing::trace!("Data channel received from {client_name}: {:?}", msg.msg_type());
                let _ = state.broadcast_rx.send((client_name.clone(), msg));
            }
            Ok(None) => {
                tracing::info!("Data channel closed for {client_name}");
                break;
            }
            Err(e) => {
                tracing::error!("Data channel error for {client_name}: {e}");
                break;
            }
        }
    }

    Ok(())
}

/// Send a message to a specific client's control channel
pub async fn send_to_client(state: &ServerState, client_name: &str, msg: Message) -> anyhow::Result<()> {
    let clients = state.clients.read().await;
    if let Some(client) = clients.get(client_name) {
        client.control_tx.send(msg).await?;
    }
    Ok(())
}

/// Send a message to a specific client's data channel
pub async fn send_data_to_client(state: &ServerState, client_name: &str, msg: Message) -> anyhow::Result<()> {
    let clients = state.clients.read().await;
    if let Some(client) = clients.get(client_name) {
        client.data_tx.send(msg).await?;
    }
    Ok(())
}

/// Broadcast a message to all clients' control channels
pub async fn broadcast_control(state: &ServerState, msg: &Message) {
    let clients = state.clients.read().await;
    for (name, client) in clients.iter() {
        if let Err(e) = client.control_tx.send(msg.clone()).await {
            tracing::warn!("Failed to send to {name}: {e}");
        }
    }
}

/// Broadcast a message to all clients' data channels
pub async fn broadcast_data(state: &ServerState, msg: &Message) {
    let clients = state.clients.read().await;
    for (name, client) in clients.iter() {
        if let Err(e) = client.data_tx.send(msg.clone()).await {
            tracing::warn!("Failed to send data to {name}: {e}");
        }
    }
}
