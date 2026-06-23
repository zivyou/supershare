mod certgen;

use clap::Parser;
use ss_core::config::AppConfig;
use ss_core::protocol::Message;
use ss_network::ServerEvent;
use ss_ui::state::{AppCommand, ClientInfo, SharedAppState, SharedState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{broadcast, mpsc};

#[derive(Parser)]
#[command(name = "supershare", version, about = "Cross-machine keyboard, mouse and clipboard sharing")]
struct Cli {
    /// Run in headless server mode
    #[arg(long)]
    server: bool,

    /// Run in headless client mode
    #[arg(long)]
    client: bool,

    /// Listen port (server mode)
    #[arg(long, default_value = "9876")]
    port: u16,

    /// Server address to connect to (client mode)
    #[arg(long)]
    connect: Option<String>,

    /// TLS certificate file path
    #[arg(long)]
    cert: Option<PathBuf>,

    /// TLS private key file path
    #[arg(long)]
    key: Option<PathBuf>,

    /// CA certificate file path
    #[arg(long)]
    ca: Option<PathBuf>,

    /// Device name (client mode)
    #[arg(long, default_value = "")]
    name: String,

    /// Generate certificates
    #[arg(long)]
    gen_cert: bool,

    /// Output directory for generated certificates
    #[arg(long, default_value = "./certs")]
    output: PathBuf,

    /// Generate device certificate with this name
    #[arg(long)]
    device: Option<String>,

    /// CA certificate path (for signing device certs)
    #[arg(long)]
    ca_cert: Option<PathBuf>,

    /// CA key path (for signing device certs)
    #[arg(long)]
    ca_key: Option<PathBuf>,

    /// Additional IP addresses for certificate SAN (can specify multiple)
    #[arg(long)]
    ip: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Certificate generation mode (no async needed)
    if cli.gen_cert {
        return run_gen_cert(cli.output, cli.device, cli.ca_cert, cli.ca_key, cli.ip);
    }

    // Headless server mode
    if cli.server {
        let cert = cli.cert.ok_or_else(|| anyhow::anyhow!("--cert is required"))?;
        let key = cli.key.ok_or_else(|| anyhow::anyhow!("--key is required"))?;
        let ca = cli.ca.ok_or_else(|| anyhow::anyhow!("--ca is required"))?;
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(run_headless_server(cli.port, cert, key, ca));
    }

    // Headless client mode
    if cli.client {
        let server = cli.connect.ok_or_else(|| anyhow::anyhow!("--connect is required"))?;
        let cert = cli.cert.ok_or_else(|| anyhow::anyhow!("--cert is required"))?;
        let key = cli.key.ok_or_else(|| anyhow::anyhow!("--key is required"))?;
        let ca = cli.ca.ok_or_else(|| anyhow::anyhow!("--ca is required"))?;
        let name = if cli.name.is_empty() {
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        } else {
            cli.name
        };
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(run_headless_client(server, cert, key, ca, name));
    }

    // Default: GUI mode — tokio runtime on background thread, GUI on main thread
    run_gui_mode()
}

/// Run the GUI with integrated runtime.
/// Tokio runtime runs on a background thread; GUI runs on the main thread.
fn run_gui_mode() -> anyhow::Result<()> {
    let config = AppConfig::load();

    let shared_state: SharedState = Arc::new(RwLock::new(SharedAppState::default()));
    let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>(16);

    // Spawn a dedicated thread for the tokio runtime
    let state_for_rt = shared_state.clone();
    let _rt_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async move {
            command_handler(cmd_rx, state_for_rt).await;
        });
    });

    // Run the GUI on the main thread (blocks until window is closed)
    let result = ss_ui::app::run_gui(config, shared_state, cmd_tx);

    // GUI exited — the runtime thread will also exit when cmd_tx is dropped
    // (cmd_rx will get a RecvError and the handler loop will end)
    result
}

/// Command handler: receives AppCommand and manages server/client lifecycle
async fn command_handler(mut cmd_rx: mpsc::Receiver<AppCommand>, state: SharedState) {
    let mut server_shutdown: Option<broadcast::Sender<()>> = None;
    let mut client_shutdown: Option<broadcast::Sender<()>> = None;

    while let Some(cmd) = cmd_rx.recv().await {
        tracing::info!("Command handler received command: {:?}", cmd);
        match cmd {
            AppCommand::StartServer {
                control_port,
                data_port,
                cert_path,
                key_path,
                ca_path,
            } => {
                tracing::info!("Starting server on port {control_port}");

                // Detect actual screen resolution
                let (screen_w, screen_h) = detect_screen_size();
                tracing::info!("Detected screen: {screen_w}x{screen_h}");
                let server_state = Arc::new(ss_network::server::ServerState::new(screen_w, screen_h));

                // Listen for server events
                let mut notify_rx = server_state.notify_tx.subscribe();
                let state_for_events = state.clone();
                tokio::spawn(async move {
                    while let Ok(event) = notify_rx.recv().await {
                        let mut s = state_for_events.write().unwrap();
                        match event {
                            ServerEvent::ClientConnected { name } => {
                                tracing::info!("Client connected: {name}");
                                s.connected_clients.push(ClientInfo {
                                    name,
                                    connected_at: std::time::Instant::now(),
                                });
                            }
                            ServerEvent::ClientDisconnected { name } => {
                                tracing::info!("Client disconnected: {name}");
                                s.connected_clients.retain(|c| c.name != name);
                            }
                        }
                    }
                });

                // --- Server-side sharing: input capture + boundary + clipboard ---

                // Start input capture (not suppressed — server has control initially)
                let suppressed = Arc::new(Mutex::new(false));
                let suppressed_clone = suppressed.clone();
                let mut input_rx = ss_input::capture::start_capture(suppressed_clone);

                // Coordinate system for boundary detection
                let coord = Arc::new(Mutex::new(ss_input::boundary::CoordinateSystem::new(screen_w, screen_h)));

                // Track which screen the cursor is on (0 = server, 1+ = client)
                let active_screen: Arc<Mutex<u8>> = Arc::new(Mutex::new(0));

                // Start clipboard monitor
                let monitor = ss_clipboard::monitor::ClipboardMonitor::new();
                let (mut clip_change_rx, clip_suppress_tx) = ss_clipboard::monitor::start_monitor(monitor);

                // Task: forward input events to connected clients (boundary-aware)
                let server_state_input = server_state.clone();
                let active_screen_input = active_screen.clone();
                let coord_input = coord.clone();
                let suppressed_input = suppressed.clone();
                tokio::spawn(async move {
                    let mut move_count: u64 = 0;
                    while let Some(event) = input_rx.recv().await {
                        let screen = *active_screen_input.lock().unwrap();

                        // Only forward if cursor is on a client screen (screen > 0)
                        if screen > 0 {
                            let msg = ss_input::capture::to_message(&event);
                            let clients = server_state_input.clients.read().await;
                            for (name, client) in clients.iter() {
                                if let Err(e) = client.data_tx.send(msg.clone()).await {
                                    tracing::warn!("Failed to send input to {name}: {e}");
                                }
                            }
                        }

                        // Check boundary on mouse moves
                        if let ss_input::capture::InputEvent::MouseMove { x, y } = event {
                            move_count += 1;
                            let current_screen = *active_screen_input.lock().unwrap();

                            // Log near edges for debugging
                            let screen_width = {
                                let c = coord_input.lock().unwrap();
                                c.screens.get(current_screen as usize).map(|s| s.width).unwrap_or(1920)
                            };
                            let near_edge = x as f32 >= (screen_width as f32 - 20.0) || x as f32 <= 20.0;
                            if move_count % 200 == 0 || near_edge {
                                tracing::debug!("Mouse: x={x:.0} y={y:.0} screen={current_screen} width={screen_width}");
                            }

                            // Check boundary without holding lock across await
                            let boundary_result = {
                                let coord = coord_input.lock().unwrap();
                                coord.check_boundary(current_screen, x as f32, y as f32)
                            };
                            if let Some((target, enter_x, enter_y)) = boundary_result {
                                if target != current_screen {
                                    tracing::info!("*** BOUNDARY CROSSED: screen {current_screen} -> {target} at ({enter_x:.0}, {enter_y:.0}) ***");

                                    if target > 0 {
                                        // Moving to client: forward last position, then suppress
                                        let msg = Message::MouseMove { x: enter_x, y: enter_y };
                                        let clients = server_state_input.clients.read().await;
                                        for (name, client) in clients.iter() {
                                            let _ = client.data_tx.send(msg.clone()).await;
                                        }
                                        // Send BoundaryEnter to client
                                        for (_, client) in clients.iter() {
                                            let _ = client.control_tx.send(Message::BoundaryEnter {
                                                target_screen: target,
                                                enter_x,
                                                enter_y,
                                            }).await;
                                        }
                                        // Suppress server input capture AFTER sending
                                        *suppressed_input.lock().unwrap() = true;
                                    } else {
                                        // Returning to server: unsuppress, send BoundaryLeave
                                        *suppressed_input.lock().unwrap() = false;
                                        let clients = server_state_input.clients.read().await;
                                        for (_, client) in clients.iter() {
                                            let _ = client.control_tx.send(Message::BoundaryLeave { source_screen: current_screen }).await;
                                        }
                                    }
                                    *active_screen_input.lock().unwrap() = target;
                                }
                            }
                        }
                    }
                });

                // Task: receive messages from clients (via broadcast channel)
                let mut broadcast_rx = server_state.broadcast_rx.subscribe();
                let clip_suppress_server = clip_suppress_tx.clone();
                let active_screen_recv = active_screen.clone();
                let suppressed_recv = suppressed.clone();
                tokio::spawn(async move {
                    let mut reassembler: Option<ss_clipboard::sync::ClipboardReassembler> = None;
                    loop {
                        match broadcast_rx.recv().await {
                            Ok((client_name, msg)) => {
                                match &msg {
                                    // Input events from client: inject locally (when cursor is on server)
                                    Message::MouseMove { .. }
                                    | Message::MouseButton { .. }
                                    | Message::MouseScroll { .. }
                                    | Message::KeyPress { .. } => {
                                        let screen = *active_screen_recv.lock().unwrap();
                                        if screen == 0 {
                                            // Cursor on server, inject client input locally
                                            ss_input::inject::inject_event(&msg);
                                        }
                                    }
                                    // Boundary events from client
                                    Message::BoundaryLeave { source_screen: _ } => {
                                        tracing::info!("Client {client_name} returned control to server");
                                        *active_screen_recv.lock().unwrap() = 0;
                                        *suppressed_recv.lock().unwrap() = false;
                                    }
                                    // Clipboard messages from client
                                    Message::ClipboardData { .. }
                                    | Message::ClipboardBegin { .. }
                                    | Message::ClipboardChunk { .. }
                                    | Message::ClipboardEnd { .. } => {
                                        if let Some(content) = ss_clipboard::sync::handle_clipboard_message(&msg, &mut reassembler) {
                                            let hash = content.hash();
                                            let write_result = match &content {
                                                ss_core::protocol::ClipboardContent::Text(text) => {
                                                    ss_clipboard::monitor::write_clipboard_text(text)
                                                }
                                                ss_core::protocol::ClipboardContent::Image { width, height, rgba } => {
                                                    ss_clipboard::monitor::write_clipboard_image(*width, *height, rgba)
                                                }
                                            };
                                            if let Err(e) = write_result {
                                                tracing::error!("Failed to write clipboard from {client_name}: {e}");
                                            } else {
                                                let _ = clip_suppress_server.send(hash).await;
                                                tracing::info!("Clipboard received from {client_name}");
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });

                // Task: forward clipboard changes to all clients
                let server_state_clip = server_state.clone();
                tokio::spawn(async move {
                    while let Some(change) = clip_change_rx.recv().await {
                        match ss_clipboard::sync::prepare_transfer(&change.content) {
                            Ok(messages) => {
                                let clients = server_state_clip.clients.read().await;
                                for (name, client) in clients.iter() {
                                    for msg in &messages {
                                        if let Err(e) = client.data_tx.send(msg.clone()).await {
                                            tracing::warn!("Failed to send clipboard to {name}: {e}");
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to prepare clipboard transfer: {e}");
                            }
                        }
                    }
                });

                // Task: when a new client connects, add its screen to coordinate system
                let mut notify_rx_coord = server_state.notify_tx.subscribe();
                let coord_for_clients = coord.clone();
                tokio::spawn(async move {
                    let mut next_screen_id = 1u8;
                    while let Ok(event) = notify_rx_coord.recv().await {
                        match event {
                            ServerEvent::ClientConnected { name } => {
                                let mut c = coord_for_clients.lock().unwrap();
                                // Add client screen to the right (default 1920x1080 for now)
                                c.add_screen(next_screen_id, name.clone(), 1920, 1080);
                                tracing::info!("Added screen {next_screen_id} for {name}, total width: {}", c.total_width());
                                next_screen_id += 1;
                            }
                            ServerEvent::ClientDisconnected { name } => {
                                // Note: we don't remove screens on disconnect to keep IDs stable
                                tracing::info!("Client {name} disconnected (screen kept in layout)");
                            }
                        }
                    }
                });

                let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
                server_shutdown = Some(shutdown_tx);

                let config = ss_network::server::ServerConfig {
                    control_port,
                    data_port,
                    cert_path,
                    key_path,
                    ca_path,
                };

                let state_clone = state.clone();
                let server_state_clone = server_state.clone();
                tokio::spawn(async move {
                    if let Err(e) = ss_network::server::start(config, server_state_clone, shutdown_rx).await {
                        tracing::error!("Server error: {e}");
                        let mut s = state_clone.write().unwrap();
                        s.last_error = Some(format!("Server error: {e}"));
                        s.server_running = false;
                        s.server_port = None;
                    }
                });

                // Update state
                {
                    let mut s = state.write().unwrap();
                    s.server_running = true;
                    s.server_port = Some(control_port);
                    s.last_error = None;
                }
            }
            AppCommand::StopServer => {
                tracing::info!("Stopping server");
                if let Some(tx) = server_shutdown.take() {
                    let _ = tx.send(());
                }
                let mut s = state.write().unwrap();
                s.server_running = false;
                s.server_port = None;
                s.connected_clients.clear();
            }
            AppCommand::ConnectClient {
                server_address,
                cert_path,
                key_path,
                ca_path,
                device_name,
            } => {
                tracing::info!("Connecting to {server_address}");

                let config = ss_network::client::ClientConfig {
                    server_address: server_address.clone(),
                    cert_path,
                    key_path,
                    ca_path,
                    device_name,
                };

                let state_clone = state.clone();
                let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);
                client_shutdown = Some(shutdown_tx);

                tokio::spawn(async move {
                    // Use connect() with timeout so errors are reported to GUI
                    // (connect_with_retry would loop forever without feedback)
                    let connect_result = tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        ss_network::client::connect(config),
                    ).await;

                    match connect_result {
                        Ok(Ok(mut conn)) => {
                            // Update state on successful connection
                            {
                                let mut s = state_clone.write().unwrap();
                                s.client_connected = true;
                                s.client_server_addr = Some(server_address.clone());
                                s.server_screen_size = conn.server_screen;
                                s.last_error = None;
                            }

                            // --- Start sharing after connection ---

                            // Suppressed flag: when true, local input capture is disabled
                            let suppressed = Arc::new(Mutex::new(true)); // starts suppressed (cursor on server)
                            let suppressed_clone = suppressed.clone();

                            // Server screen dimensions (received during handshake)
                            let server_screen = conn.server_screen.unwrap_or((1920, 1080));
                            tracing::info!("Server screen: {}x{}", server_screen.0, server_screen.1);

                            // Start local input capture
                            let mut input_rx = ss_input::capture::start_capture(suppressed_clone);

                            // Start clipboard monitor
                            let monitor = ss_clipboard::monitor::ClipboardMonitor::new();
                            let (mut clip_change_rx, clip_suppress_tx) = ss_clipboard::monitor::start_monitor(monitor);

                            // Task: forward captured input events to server via data channel
                            // Also detect boundary: if cursor hits left edge, return control to server
                            let data_tx_input = conn.data_tx.clone();
                            let suppressed_input = suppressed.clone();
                            let server_width = server_screen.0 as f32;
                            tokio::spawn(async move {
                                while let Some(event) = input_rx.recv().await {
                                    // Check boundary on mouse moves: left edge = return to server
                                    if let ss_input::capture::InputEvent::MouseMove { x, y: _ } = &event {
                                        if *x as f32 <= 5.0 {
                                            tracing::info!("Client boundary: cursor at left edge (x={x:.0}), returning control to server");
                                            // Suppress local capture
                                            *suppressed_input.lock().unwrap() = true;
                                            // Send BoundaryLeave to server
                                            let _ = data_tx_input.send(Message::BoundaryLeave { source_screen: 1 }).await;
                                            continue; // Don't forward this event
                                        }
                                    }
                                    let msg = ss_input::capture::to_message(&event);
                                    if data_tx_input.send(msg).await.is_err() {
                                        tracing::info!("Data channel closed, stopping input forward");
                                        break;
                                    }
                                }
                            });

                            // Task: inject input events received from server
                            let mut data_rx_inject = conn.data_rx.resubscribe();
                            tokio::spawn(async move {
                                loop {
                                    match data_rx_inject.recv().await {
                                        Ok(msg) => {
                                            // Inject input events from server locally
                                            match &msg {
                                                Message::MouseMove { .. }
                                                | Message::MouseButton { .. }
                                                | Message::MouseScroll { .. }
                                                | Message::KeyPress { .. } => {
                                                    ss_input::inject::inject_event(&msg);
                                                }
                                                _ => {}
                                            }
                                        }
                                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                        Err(broadcast::error::RecvError::Closed) => break,
                                    }
                                }
                            });

                            // Task: forward clipboard changes to server
                            let data_tx_clip = conn.data_tx.clone();
                            tokio::spawn(async move {
                                while let Some(change) = clip_change_rx.recv().await {
                                    match ss_clipboard::sync::prepare_transfer(&change.content) {
                                        Ok(messages) => {
                                            for msg in messages {
                                                if data_tx_clip.send(msg).await.is_err() {
                                                    tracing::warn!("Failed to send clipboard data");
                                                    break;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to prepare clipboard transfer: {e}");
                                        }
                                    }
                                }
                            });

                            // Task: receive clipboard data from server and write locally
                            let mut data_rx_clip = conn.data_rx.resubscribe();
                            let clip_suppress = clip_suppress_tx.clone();
                            tokio::spawn(async move {
                                let mut reassembler: Option<ss_clipboard::sync::ClipboardReassembler> = None;
                                loop {
                                    match data_rx_clip.recv().await {
                                        Ok(msg) => {
                                            if let Some(content) = ss_clipboard::sync::handle_clipboard_message(&msg, &mut reassembler) {
                                                let hash = content.hash();
                                                // Write to local clipboard
                                                let write_result = match &content {
                                                    ss_core::protocol::ClipboardContent::Text(text) => {
                                                        ss_clipboard::monitor::write_clipboard_text(text)
                                                    }
                                                    ss_core::protocol::ClipboardContent::Image { width, height, rgba } => {
                                                        ss_clipboard::monitor::write_clipboard_image(*width, *height, rgba)
                                                    }
                                                };
                                                if let Err(e) = write_result {
                                                    tracing::error!("Failed to write clipboard: {e}");
                                                } else {
                                                    // Suppress local clipboard monitor to prevent echo
                                                    let _ = clip_suppress.send(hash).await;
                                                    tracing::info!("Clipboard received from server");
                                                }
                                            }
                                        }
                                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                        Err(broadcast::error::RecvError::Closed) => break,
                                    }
                                }
                            });

                            // Task: listen for boundary events from server to toggle suppression
                            let mut control_rx_boundary = conn.control_rx.resubscribe();
                            let suppressed_boundary = suppressed.clone();
                            tokio::spawn(async move {
                                loop {
                                    match control_rx_boundary.recv().await {
                                        Ok(Message::BoundaryEnter { enter_x, enter_y, .. }) => {
                                            tracing::info!("Boundary enter at ({enter_x:.0}, {enter_y:.0}): enabling local input capture");
                                            // Move cursor to the enter position using inject
                                            ss_input::inject::inject_event(&Message::MouseMove {
                                                x: enter_x,
                                                y: enter_y,
                                            });
                                            // Unsuppress to start capturing
                                            *suppressed_boundary.lock().unwrap() = false;
                                        }
                                        Ok(Message::BoundaryLeave { .. }) => {
                                            tracing::info!("Boundary leave: disabling local input capture");
                                            *suppressed_boundary.lock().unwrap() = true;
                                        }
                                        Ok(_) => {}
                                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                        Err(broadcast::error::RecvError::Closed) => break,
                                    }
                                }
                            });

                            // Main loop: listen for disconnect
                            loop {
                                tokio::select! {
                                    msg = conn.control_rx.recv() => {
                                        match msg {
                                            Ok(Message::Heartbeat) => {}
                                            Ok(_) => {}
                                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                                            Err(broadcast::error::RecvError::Closed) => {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }

                            // Connection closed
                            let mut s = state_clone.write().unwrap();
                            s.client_connected = false;
                            s.client_server_addr = None;
                            s.server_screen_size = None;
                        }
                        Ok(Err(e)) => {
                            tracing::error!("Client connection error: {e}");
                            let mut s = state_clone.write().unwrap();
                            s.client_connected = false;
                            s.last_error = Some(format!("Connection failed: {e}"));
                        }
                        Err(_) => {
                            tracing::error!("Client connection timed out");
                            let mut s = state_clone.write().unwrap();
                            s.client_connected = false;
                            s.last_error = Some("Connection timed out (10s)".to_string());
                        }
                    }
                });

                // Update state optimistically
                {
                    let mut s = state.write().unwrap();
                    s.last_error = None;
                }
            }
            AppCommand::DisconnectClient => {
                tracing::info!("Disconnecting client");
                if let Some(tx) = client_shutdown.take() {
                    let _ = tx.send(());
                }
                let mut s = state.write().unwrap();
                s.client_connected = false;
                s.client_server_addr = None;
                s.server_screen_size = None;
            }
        }
    }

    tracing::info!("Command handler exiting");
}

/// Run headless server (CLI mode)
async fn run_headless_server(
    port: u16,
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
) -> anyhow::Result<()> {
    tracing::info!("Starting headless server on port {port}");

    let (screen_w, screen_h) = detect_screen_size();
    tracing::info!("Detected screen: {screen_w}x{screen_h}");
    let server_state = Arc::new(ss_network::server::ServerState::new(screen_w, screen_h));
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    // Log client events
    let mut notify_rx = server_state.notify_tx.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = notify_rx.recv().await {
            match event {
                ServerEvent::ClientConnected { name } => tracing::info!("Client connected: {name}"),
                ServerEvent::ClientDisconnected { name } => tracing::info!("Client disconnected: {name}"),
            }
        }
    });

    let config = ss_network::server::ServerConfig {
        control_port: port,
        data_port: port + 1,
        cert_path: cert,
        key_path: key,
        ca_path: ca,
    };

    // Handle Ctrl+C
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received");
        let _ = shutdown_tx.send(());
    });

    ss_network::server::start(config, server_state, shutdown_rx).await
}

/// Run headless client (CLI mode)
async fn run_headless_client(
    server: String,
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
    name: String,
) -> anyhow::Result<()> {
    tracing::info!("Connecting to {server} as {name}");

    let config = ss_network::client::ClientConfig {
        server_address: server,
        cert_path: cert,
        key_path: key,
        ca_path: ca,
        device_name: name,
    };

    let conn = ss_network::client::connect_with_retry(config).await?;
    tracing::info!("Connected. Press Ctrl+C to disconnect.");

    // Suppressed flag: starts suppressed (cursor on server side)
    let suppressed = Arc::new(Mutex::new(true));

    // Start local input capture
    let suppressed_clone = suppressed.clone();
    let mut input_rx = ss_input::capture::start_capture(suppressed_clone);

    // Start clipboard monitor
    let monitor = ss_clipboard::monitor::ClipboardMonitor::new();
    let (mut clip_change_rx, clip_suppress_tx) = ss_clipboard::monitor::start_monitor(monitor);

    // Task: forward captured input events to server, with boundary detection
    let data_tx_input = conn.data_tx.clone();
    let suppressed_input = suppressed.clone();
    tokio::spawn(async move {
        while let Some(event) = input_rx.recv().await {
            // Check boundary: left edge = return to server
            if let ss_input::capture::InputEvent::MouseMove { x, y: _ } = &event {
                if *x as f32 <= 5.0 {
                    tracing::info!("Client boundary: cursor at left edge (x={x:.0}), returning control");
                    *suppressed_input.lock().unwrap() = true;
                    let _ = data_tx_input.send(Message::BoundaryLeave { source_screen: 1 }).await;
                    continue;
                }
            }
            let msg = ss_input::capture::to_message(&event);
            if data_tx_input.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task: inject input events received from server
    let mut data_rx_inject = conn.data_rx.resubscribe();
    tokio::spawn(async move {
        loop {
            match data_rx_inject.recv().await {
                Ok(msg) => {
                    match &msg {
                        Message::MouseMove { .. }
                        | Message::MouseButton { .. }
                        | Message::MouseScroll { .. }
                        | Message::KeyPress { .. } => {
                            ss_input::inject::inject_event(&msg);
                        }
                        _ => {}
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Task: forward clipboard changes to server
    let data_tx_clip = conn.data_tx.clone();
    tokio::spawn(async move {
        while let Some(change) = clip_change_rx.recv().await {
            if let Ok(messages) = ss_clipboard::sync::prepare_transfer(&change.content) {
                for msg in messages {
                    if data_tx_clip.send(msg).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Task: receive clipboard data from server
    let mut data_rx_clip = conn.data_rx.resubscribe();
    let clip_suppress = clip_suppress_tx.clone();
    tokio::spawn(async move {
        let mut reassembler: Option<ss_clipboard::sync::ClipboardReassembler> = None;
        loop {
            match data_rx_clip.recv().await {
                Ok(msg) => {
                    if let Some(content) = ss_clipboard::sync::handle_clipboard_message(&msg, &mut reassembler) {
                        let hash = content.hash();
                        let write_result = match &content {
                            ss_core::protocol::ClipboardContent::Text(text) => {
                                ss_clipboard::monitor::write_clipboard_text(text)
                            }
                            ss_core::protocol::ClipboardContent::Image { width, height, rgba } => {
                                ss_clipboard::monitor::write_clipboard_image(*width, *height, rgba)
                            }
                        };
                        if let Err(e) = write_result {
                            tracing::error!("Failed to write clipboard: {e}");
                        } else {
                            let _ = clip_suppress.send(hash).await;
                            tracing::info!("Clipboard received from server");
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Task: listen for boundary events to toggle suppression
    let mut control_rx_boundary = conn.control_rx.resubscribe();
    let suppressed_boundary = suppressed.clone();
    tokio::spawn(async move {
        loop {
            match control_rx_boundary.recv().await {
                Ok(Message::BoundaryEnter { enter_x, enter_y, .. }) => {
                    tracing::info!("Boundary enter at ({enter_x:.0}, {enter_y:.0}): enabling local input capture");
                    // Move cursor to enter position
                    ss_input::inject::inject_event(&Message::MouseMove { x: enter_x, y: enter_y });
                    *suppressed_boundary.lock().unwrap() = false;
                }
                Ok(Message::BoundaryLeave { .. }) => {
                    tracing::info!("Boundary leave: disabling local input capture");
                    *suppressed_boundary.lock().unwrap() = true;
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Disconnecting");
    Ok(())
}

/// Detect the primary screen resolution
fn detect_screen_size() -> (u32, u32) {
    // Try xrandr first (Linux X11)
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("xrandr")
            .arg("--query")
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // Look for "connected primary" line with resolution
                // e.g., "eDP-1 connected primary 1680x1050+0+0"
                if line.contains("connected primary") || (line.contains("connected") && line.contains("+0+0")) {
                    for part in line.split_whitespace() {
                        if let Some(res) = part.strip_suffix("+0+0") {
                            let dims: Vec<&str> = res.split('x').collect();
                            if dims.len() == 2 {
                                if let (Ok(w), Ok(h)) = (dims[0].parse::<u32>(), dims[1].parse::<u32>()) {
                                    return (w, h);
                                }
                            }
                        }
                    }
                }
            }
            // Fallback: look for first resolution line with "*"
            for line in stdout.lines() {
                if line.contains('*') {
                    for part in line.split_whitespace() {
                        let dims: Vec<&str> = part.split('x').collect();
                        if dims.len() == 2 {
                            if dims[0].parse::<u32>().is_ok() && dims[1].parse::<u32>().is_ok() {
                                let w: u32 = dims[0].parse().unwrap();
                                let h: u32 = dims[1].parse().unwrap();
                                return (w, h);
                            }
                        }
                    }
                }
            }
        }
    }

    // Try WMIC on Windows
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("wmic")
            .args(["desktopmonitor", "get", "screenwidth,screenheight", "/format:value"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut w = 0u32;
            let mut h = 0u32;
            for line in stdout.lines() {
                if let Some(val) = line.strip_prefix("ScreenWidth=") {
                    w = val.trim().parse().unwrap_or(0);
                }
                if let Some(val) = line.strip_prefix("ScreenHeight=") {
                    h = val.trim().parse().unwrap_or(0);
                }
            }
            if w > 0 && h > 0 {
                return (w, h);
            }
        }
    }

    tracing::warn!("Could not detect screen size, using default 1920x1080");
    (1920, 1080)
}

/// Generate certificates
fn run_gen_cert(
    output: PathBuf,
    device: Option<String>,
    ca_cert: Option<PathBuf>,
    ca_key: Option<PathBuf>,
    ips: Vec<String>,
) -> anyhow::Result<()> {
    match device {
        Some(device_name) => {
            let ca_cert_path = ca_cert.ok_or_else(|| {
                anyhow::anyhow!("--ca-cert is required when generating device certificate")
            })?;
            let ca_key_path = ca_key.ok_or_else(|| {
                anyhow::anyhow!("--ca-key is required when generating device certificate")
            })?;
            let extra_ips: Vec<std::net::IpAddr> = ips
                .iter()
                .map(|s| s.parse())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow::anyhow!("Invalid IP address: {e}"))?;
            certgen::generate_device_cert(&output, &ca_cert_path, &ca_key_path, &device_name, &extra_ips)?;
            println!("Device certificate for '{}' generated in {}", device_name, output.display());
        }
        None => {
            certgen::generate_ca(&output)?;
            println!("CA certificate and key generated in {}", output.display());
            println!("Next steps:");
            println!("  1. Generate device certs: supershare --gen-cert --device <name> --ca-cert certs/ca.pem --ca-key certs/ca-key.pem --ip <server-ip>");
        }
    }
    Ok(())
}
