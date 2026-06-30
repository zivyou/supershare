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

    /// Pair with the server using a PIN (client mode, prompts on stdin)
    #[arg(long)]
    pair: bool,

    /// Disable PIN-based pairing (server mode)
    #[arg(long)]
    no_pairing: bool,

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
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(run_headless_server(
            cli.port,
            cli.cert,
            cli.key,
            cli.ca,
            !cli.no_pairing,
        ));
    }

    // Headless client mode
    if cli.client {
        let server = cli.connect.ok_or_else(|| anyhow::anyhow!("--connect is required"))?;
        let name = if cli.name.is_empty() {
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        } else {
            cli.name
        };
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(run_headless_client(
            server, cli.cert, cli.key, cli.ca, name, cli.pair,
        ));
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
    let config_for_rt = config.clone();
    let _rt_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async move {
            command_handler(cmd_rx, state_for_rt, config_for_rt).await;
        });
    });

    // Run the GUI on the main thread (blocks until window is closed)
    let result = ss_ui::app::run_gui(config, shared_state, cmd_tx);

    // GUI exited — the runtime thread will also exit when cmd_tx is dropped
    // (cmd_rx will get a RecvError and the handler loop will end)
    result
}

/// Command handler: receives AppCommand and manages server/client lifecycle
async fn command_handler(
    mut cmd_rx: mpsc::Receiver<AppCommand>,
    state: SharedState,
    mut config: AppConfig,
) {
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
                pairing_enabled,
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
                            ServerEvent::ClientPaired { name } => {
                                tracing::info!("Client paired: {name}");
                            }
                        }
                    }
                });

                // --- Server-side sharing: rdev input capture with cursor warping + clipboard ---

                // Start warp-based input capture (no root required)
                let warp_capture = ss_input::warp_capture::start_capture(screen_w, screen_h)
                    .expect("Failed to start warp capture.");
                let mut input_rx = warp_capture.event_rx;

                // Start clipboard monitor
                let monitor = ss_clipboard::monitor::ClipboardMonitor::new();
                let (mut clip_change_rx, clip_suppress_tx) = ss_clipboard::monitor::start_monitor(monitor);

                // Task: process warp input events (boundary-aware, delta-based)
                let server_state_input = server_state.clone();
                let mut move_count: u64 = 0;
                tokio::spawn(async move {
                    while let Some(event) = input_rx.recv().await {
                        match &event {
                            ss_input::warp_capture::WarpInputEvent::BoundaryEnter { enter_x, enter_y } => {
                                tracing::info!("*** BOUNDARY ENTER: sending to client pos=({enter_x:.0}, {enter_y:.0}) ***");
                                let msg = Message::BoundaryEnter {
                                    enter_x: *enter_x,
                                    enter_y: *enter_y,
                                    target_screen: 1,
                                };
                                let clients = server_state_input.clients.read().await;
                                for (name, client) in clients.iter() {
                                    if let Err(e) = client.control_tx.send(msg.clone()).await {
                                        tracing::warn!("Failed to send BoundaryEnter to {name}: {e}");
                                    }
                                }
                            }
                            ss_input::warp_capture::WarpInputEvent::MouseDelta { dx, dy } => {
                                move_count += 1;

                                // Check for special return signal
                                if *dx == -1.0 && *dy == 0.0 {
                                    tracing::info!("*** BOUNDARY RETURN: cursor returned to local screen ***");

                                    // Send BoundaryLeave to client
                                    let clients = server_state_input.clients.read().await;
                                    for (name, client) in clients.iter() {
                                        if let Err(e) = client.control_tx.send(Message::BoundaryLeave {
                                            source_screen: 1,
                                        }).await {
                                            tracing::error!("Failed to send BoundaryLeave to {name}: {e}");
                                        }
                                    }
                                    continue;
                                }

                                if move_count % 200 == 0 {
                                    tracing::debug!("Mouse delta: ({dx:.0}, {dy:.0})");
                                }

                                // Forward delta to client
                                let msg = ss_input::warp_capture::to_message(&event);
                                let clients = server_state_input.clients.read().await;
                                for (name, client) in clients.iter() {
                                    if let Err(e) = client.data_tx.send(msg.clone()).await {
                                        tracing::warn!("Failed to send input to {name}: {e}");
                                    }
                                }
                            }
                            ss_input::warp_capture::WarpInputEvent::MouseButton { .. }
                            | ss_input::warp_capture::WarpInputEvent::KeyPress { .. }
                            | ss_input::warp_capture::WarpInputEvent::Scroll { .. } => {
                                // Forward other events to client
                                let msg = ss_input::warp_capture::to_message(&event);
                                let clients = server_state_input.clients.read().await;
                                for (name, client) in clients.iter() {
                                    if let Err(e) = client.data_tx.send(msg.clone()).await {
                                        tracing::warn!("Failed to send input to {name}: {e}");
                                    }
                                }
                            }
                        }
                    }
                });

                // Task: receive messages from clients (via broadcast channel)
                let mut broadcast_rx = server_state.broadcast_rx.subscribe();
                let clip_suppress_server = clip_suppress_tx.clone();
                tokio::spawn(async move {
                    let mut reassembler: Option<ss_clipboard::sync::ClipboardReassembler> = None;
                    loop {
                        match broadcast_rx.recv().await {
                            Ok((client_name, msg)) => {
                                match &msg {
                                    // Input events from client: no longer expected in new architecture
                                    // (server handles all input via evdev, client is passive)
                                    Message::MouseMove { .. }
                                    | Message::MouseDelta { .. }
                                    | Message::MouseButton { .. }
                                    | Message::MouseScroll { .. }
                                    | Message::KeyPress { .. } => {
                                        tracing::debug!("Received input event from {client_name} (ignored in new architecture)");
                                    }
                                    // Boundary events from client: no longer expected
                                    Message::BoundaryLeave { .. } => {
                                        tracing::debug!("Received BoundaryLeave from {client_name} (ignored, server handles boundaries)");
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

                // Task: log client connections
                let mut notify_rx_coord = server_state.notify_tx.subscribe();
                tokio::spawn(async move {
                    while let Ok(event) = notify_rx_coord.recv().await {
                        match event {
                            ServerEvent::ClientConnected { name } => {
                                tracing::info!("Client {name} connected");
                            }
                            ServerEvent::ClientDisconnected { name } => {
                                tracing::info!("Client {name} disconnected");
                            }
                            ServerEvent::ClientPaired { name } => {
                                tracing::info!("Client {name} paired");
                            }
                        }
                    }
                });

                let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
                server_shutdown = Some(shutdown_tx);

                // Resolve TLS material: use explicit paths if all provided,
                // otherwise auto-generate a CA + server cert in the trust dir.
                let resolved = match resolve_server_certs(&cert_path, &key_path, &ca_path) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("Failed to resolve server certificates: {e}");
                        let mut s = state.write().unwrap();
                        s.last_error = Some(format!("Certificate error: {e}"));
                        continue;
                    }
                };

                // Set up pairing support if enabled.
                let pairing = if pairing_enabled {
                    match build_pairing_support(&resolved, control_port).await {
                        Ok((support, pin_manager, mut paired_rx)) => {
                            // Reflect the current PIN into shared state for the GUI,
                            // and persist newly paired clients into the config.
                            let pin_state = state.clone();
                            let pin_mgr = pin_manager.clone();
                            tokio::spawn(async move {
                                let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
                                loop {
                                    tick.tick().await;
                                    let pin = pin_mgr.current_pin();
                                    let mut s = pin_state.write().unwrap();
                                    s.pairing_pin = Some(pin);
                                }
                            });
                            tokio::spawn(async move {
                                while let Some(paired) = paired_rx.recv().await {
                                    tracing::info!("Persisting paired client: {}", paired.name);
                                    let mut cfg = AppConfig::load();
                                    cfg.server.upsert_paired_client(paired);
                                    let _ = cfg.save();
                                }
                            });
                            Some(support)
                        }
                        Err(e) => {
                            tracing::error!("Failed to enable pairing: {e}");
                            None
                        }
                    }
                } else {
                    None
                };

                let config = ss_network::server::ServerConfig {
                    control_port,
                    data_port,
                    cert_path: resolved.cert_path.clone(),
                    key_path: resolved.key_path.clone(),
                    ca_path: resolved.ca_path.clone(),
                    pairing,
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

                // Resolve client trust material: explicit paths > known server.
                // If neither is available, signal that pairing is required.
                let resolved = resolve_client_certs(&config, &server_address, &cert_path, &key_path, &ca_path);
                let (cert_path, key_path, ca_path) = match resolved {
                    Some(paths) => paths,
                    None => {
                        tracing::info!("No trust for {server_address}; pairing required");
                        let mut s = state.write().unwrap();
                        s.pairing_required = true;
                        s.pairing_status = None;
                        s.last_error = Some("This server is not paired. Enter the PIN to pair.".to_string());
                        continue;
                    }
                };

                {
                    let mut s = state.write().unwrap();
                    s.pairing_required = false;
                }

                let (screen_w, screen_h) = detect_screen_size();
                let config_net = ss_network::client::ClientConfig {
                    server_address: server_address.clone(),
                    cert_path,
                    key_path,
                    ca_path,
                    device_name,
                    screen_width: screen_w,
                    screen_height: screen_h,
                };

                let state_clone = state.clone();
                let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);
                client_shutdown = Some(shutdown_tx);

                spawn_client_connection(config_net, server_address, state_clone);
            }
            AppCommand::PairAndConnect {
                server_address,
                pin,
                device_name,
            } => {
                tracing::info!("Pairing with {server_address}");
                {
                    let mut s = state.write().unwrap();
                    s.pairing_status = Some("Pairing…".to_string());
                    s.last_error = None;
                }

                let host = server_address.split(':').next().unwrap_or(&server_address).to_string();
                let control_port: u16 = server_address
                    .split(':')
                    .nth(1)
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(9876);
                let pairing_port = ss_network::pairing::client::default_pairing_port(control_port);

                let dev = if device_name.is_empty() {
                    hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown".to_string())
                } else {
                    device_name.clone()
                };

                match ss_network::pairing::client::pair_with_server(&host, pairing_port, &pin, &dev).await {
                    Ok(material) => {
                        // Persist trust material and record the known server.
                        match persist_client_trust(&server_address, &material) {
                            Ok((cert_path, key_path, ca_path)) => {
                                let fingerprint = ss_network::cert::cert_fingerprint(&material.ca_cert_pem).unwrap_or_default();
                                config.client.upsert_known_server(ss_core::config::KnownServer {
                                    address: server_address.clone(),
                                    cert_path: cert_path.clone(),
                                    key_path: key_path.clone(),
                                    ca_path: ca_path.clone(),
                                    server_fingerprint: fingerprint,
                                });
                                let _ = config.save();

                                {
                                    let mut s = state.write().unwrap();
                                    s.pairing_required = false;
                                    s.pairing_status = Some("Paired! Connecting…".to_string());
                                }

                                let (screen_w, screen_h) = detect_screen_size();
                                let config_net = ss_network::client::ClientConfig {
                                    server_address: server_address.clone(),
                                    cert_path,
                                    key_path,
                                    ca_path,
                                    device_name: dev,
                                    screen_width: screen_w,
                                    screen_height: screen_h,
                                };
                                let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);
                                client_shutdown = Some(shutdown_tx);
                                spawn_client_connection(config_net, server_address, state.clone());
                            }
                            Err(e) => {
                                tracing::error!("Failed to persist trust material: {e}");
                                let mut s = state.write().unwrap();
                                s.pairing_status = Some(format!("Pairing failed: {e}"));
                                s.last_error = Some(format!("Pairing failed: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Pairing failed: {e}");
                        let mut s = state.write().unwrap();
                        s.pairing_status = Some(format!("Pairing failed: {e}"));
                        s.last_error = Some(format!("Pairing failed: {e}"));
                    }
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

/// Spawn the background task that maintains a client connection (with
/// reconnect/backoff) and wires up input injection + clipboard sync.
fn spawn_client_connection(
    config: ss_network::client::ClientConfig,
    server_address: String,
    state_clone: SharedState,
) {
    tokio::spawn(async move {
                    let mut backoff = std::time::Duration::from_secs(1);
                    let max_backoff = std::time::Duration::from_secs(30);
                    let mut first_attempt = true;

                    loop {
                        // Use connect() with timeout so errors are reported to GUI
                        let connect_result = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            ss_network::client::connect(config.clone()),
                        ).await;

                        match connect_result {
                            Ok(Ok(mut conn)) => {
                                // Reset backoff on successful connection
                                backoff = std::time::Duration::from_secs(1);
                                first_attempt = false;

                                // Update state on successful connection
                                {
                                    let mut s = state_clone.write().unwrap();
                                    s.client_connected = true;
                                    s.client_server_addr = Some(server_address.clone());
                                    s.server_screen_size = conn.server_screen;
                                    s.last_error = None;
                                }

                                // --- Client in pure passive mode (no local input capture) ---

                                // Server screen dimensions (received during handshake)
                                let server_screen = conn.server_screen.unwrap_or((1920, 1080));
                                tracing::info!("Server screen: {}x{}", server_screen.0, server_screen.1);

                                // Client virtual cursor (for receiving MouseDelta from server)
                                let (screen_w, screen_h) = detect_screen_size();
                                let virtual_cursor = Arc::new(Mutex::new(((screen_w / 2) as f32, (screen_h / 2) as f32)));

                                // Start clipboard monitor
                                let monitor = ss_clipboard::monitor::ClipboardMonitor::new();
                                let (mut clip_change_rx, clip_suppress_tx) = ss_clipboard::monitor::start_monitor(monitor);

                                // Task: inject input events received from server (passive mode)
                                let mut data_rx_inject = conn.data_rx.resubscribe();
                                let virtual_cursor_inject = virtual_cursor.clone();
                                tokio::spawn(async move {
                                let mut inject_count: u64 = 0;
                                loop {
                                    match data_rx_inject.recv().await {
                                        Ok(msg) => {
                                            match &msg {
                                                Message::MouseDelta { dx, dy } => {
                                                    inject_count += 1;
                                                    // Apply delta to virtual cursor
                                                    let (vx, vy) = {
                                                        let mut vc = virtual_cursor_inject.lock().unwrap();
                                                        vc.0 += dx;
                                                        vc.1 += dy;
                                                        // Clamp to screen bounds
                                                        vc.0 = vc.0.clamp(0.0, (screen_w - 1) as f32);
                                                        vc.1 = vc.1.clamp(0.0, (screen_h - 1) as f32);
                                                        (vc.0, vc.1)
                                                    };
                                                    if inject_count % 50 == 1 {
                                                        tracing::info!("Injecting MouseMove ({:.0}, {:.0}) [delta={dx:.0},{dy:.0}]", vx, vy);
                                                    }
                                                    // Inject absolute position
                                                    let move_msg = Message::MouseMove { x: vx, y: vy };
                                                    ss_input::inject::inject_event(&move_msg);
                                                }
                                                Message::MouseMove { x, y } => {
                                                    inject_count += 1;
                                                    // Absolute position (e.g., from BoundaryEnter)
                                                    {
                                                        let mut vc = virtual_cursor_inject.lock().unwrap();
                                                        vc.0 = *x;
                                                        vc.1 = *y;
                                                    }
                                                    if inject_count % 50 == 1 {
                                                        tracing::info!("Injecting MouseMove ({x:.0}, {y:.0})");
                                                    }
                                                    ss_input::inject::inject_event(&msg);
                                                }
                                                Message::MouseButton { .. }
                                                | Message::MouseScroll { .. }
                                                | Message::KeyPress { .. } => {
                                                    tracing::debug!("Injecting input event from server");
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

                            // Single control channel handler: boundary events + disconnect detection
                            let virtual_cursor_ctrl = virtual_cursor.clone();
                            loop {
                                match conn.control_rx.recv().await {
                                    Ok(Message::BoundaryEnter { enter_x, enter_y, target_screen }) => {
                                        tracing::info!("*** CLIENT BoundaryEnter: screen={target_screen} pos=({enter_x:.0}, {enter_y:.0}) ***");
                                        // Move cursor to the enter position using inject
                                        {
                                            let mut vc = virtual_cursor_ctrl.lock().unwrap();
                                            vc.0 = enter_x;
                                            vc.1 = enter_y;
                                        }
                                        let move_msg = Message::MouseMove { x: enter_x, y: enter_y };
                                        ss_input::inject::inject_event(&move_msg);
                                        tracing::info!("Injected MouseMove to ({enter_x:.0}, {enter_y:.0})");
                                    }
                                    Ok(Message::BoundaryLeave { .. }) => {
                                        tracing::info!("Boundary leave received (no action needed in passive mode)");
                                    }
                                    Ok(Message::Heartbeat) => {
                                        // Keep-alive, no action needed
                                    }
                                    Ok(_) => {
                                        // Other control messages, ignore
                                    }
                                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                    Err(broadcast::error::RecvError::Closed) => {
                                        tracing::info!("Control channel closed");
                                        break;
                                    }
                                }
                            }

                            // Connection closed - attempt reconnect
                            tracing::warn!("Connection lost, reconnecting in {} seconds...", backoff.as_secs());
                            {
                                let mut s = state_clone.write().unwrap();
                                s.client_connected = false;
                                s.last_error = Some(format!("Connection lost, reconnecting in {}s...", backoff.as_secs()));
                            }
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(max_backoff);
                            continue; // Retry connection
                        }
                        Ok(Err(e)) => {
                            tracing::error!("Client connection error: {e}");
                            if first_attempt {
                                // On first attempt, report error and don't retry
                                let mut s = state_clone.write().unwrap();
                                s.client_connected = false;
                                s.last_error = Some(format!("Connection failed: {e}"));
                                break;
                            }
                            // On subsequent attempts, retry with backoff
                            tracing::warn!("Reconnecting in {} seconds...", backoff.as_secs());
                            {
                                let mut s = state_clone.write().unwrap();
                                s.last_error = Some(format!("Reconnecting in {}s...", backoff.as_secs()));
                            }
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(max_backoff);
                            continue;
                        }
                        Err(_) => {
                            tracing::error!("Client connection timed out");
                            if first_attempt {
                                let mut s = state_clone.write().unwrap();
                                s.client_connected = false;
                                s.last_error = Some("Connection timed out (10s)".to_string());
                                break;
                            }
                            // On subsequent attempts, retry with backoff
                            tracing::warn!("Reconnecting in {} seconds...", backoff.as_secs());
                            {
                                let mut s = state_clone.write().unwrap();
                                s.last_error = Some(format!("Reconnecting in {}s...", backoff.as_secs()));
                            }
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(max_backoff);
                            continue;
                        }
                    }
                    } // end loop
                });
}

/// Resolved server TLS material (all three PEM paths).
struct ResolvedServerCerts {
    cert_path: PathBuf,
    key_path: PathBuf,
    ca_path: PathBuf,
    ca_cert_pem: String,
    ca_key_pem: String,
}

/// Resolve server TLS material. If all explicit paths are given, use them;
/// otherwise auto-generate (or reuse) a CA + server cert in the trust dir.
fn resolve_server_certs(
    cert_path: &Option<PathBuf>,
    key_path: &Option<PathBuf>,
    ca_path: &Option<PathBuf>,
) -> anyhow::Result<ResolvedServerCerts> {
    if let (Some(cert), Some(key), Some(ca)) = (cert_path, key_path, ca_path) {
        let ca_cert_pem = std::fs::read_to_string(ca)?;
        // CA key sits next to the CA cert by convention (ca.pem -> ca-key.pem).
        let ca_key_pem = std::fs::read_to_string(ca.with_file_name(
            ca.file_stem().map(|s| format!("{}-key.pem", s.to_string_lossy())).unwrap_or_else(|| "ca-key.pem".to_string()),
        ))
        .unwrap_or_default();
        return Ok(ResolvedServerCerts {
            cert_path: cert.clone(),
            key_path: key.clone(),
            ca_path: ca.clone(),
            ca_cert_pem,
            ca_key_pem,
        });
    }

    // Auto-generate / reuse in the trust dir.
    let dir = ss_core::config::ensure_trust_dir()?;
    let server_ips = local_ips();
    let ca = ss_network::cert::ensure_server_ca(&dir, &server_ips)?;
    let ca_cert_pem = std::fs::read_to_string(&ca.ca_cert_path)?;
    let ca_key_pem = std::fs::read_to_string(&ca.ca_key_path)?;
    Ok(ResolvedServerCerts {
        cert_path: ca.server_cert_path,
        key_path: ca.server_key_path,
        ca_path: ca.ca_cert_path,
        ca_cert_pem,
        ca_key_pem,
    })
}

/// Build pairing support for the server: the listener config, a shared PIN
/// manager, and a receiver for newly-paired clients.
async fn build_pairing_support(
    resolved: &ResolvedServerCerts,
    control_port: u16,
) -> anyhow::Result<(
    ss_network::server::PairingSupport,
    Arc<ss_network::pairing::server::PinManager>,
    mpsc::Receiver<ss_core::config::PairedClient>,
)> {
    let pin_manager = Arc::new(ss_network::pairing::server::PinManager::new());
    let (paired_tx, paired_rx) = mpsc::channel(8);
    let pairing_port = ss_network::pairing::client::default_pairing_port(control_port);
    tracing::info!("Pairing PIN: {}", pin_manager.current_pin());
    let support = ss_network::server::PairingSupport {
        pairing_port,
        ca_cert_pem: resolved.ca_cert_pem.clone(),
        ca_key_pem: resolved.ca_key_pem.clone(),
        pin_manager: pin_manager.clone(),
        on_paired: paired_tx,
    };
    Ok((support, pin_manager, paired_rx))
}

/// Resolve client TLS material for a server address. Explicit paths win;
/// otherwise use a known (paired) server. Returns None if pairing is needed.
fn resolve_client_certs(
    config: &AppConfig,
    server_address: &str,
    cert_path: &Option<PathBuf>,
    key_path: &Option<PathBuf>,
    ca_path: &Option<PathBuf>,
) -> Option<(PathBuf, PathBuf, PathBuf)> {
    if let (Some(c), Some(k), Some(a)) = (cert_path, key_path, ca_path) {
        return Some((c.clone(), k.clone(), a.clone()));
    }
    config
        .client
        .find_known_server(server_address)
        .map(|s| (s.cert_path.clone(), s.key_path.clone(), s.ca_path.clone()))
}

/// Persist provisioned client trust material to the trust dir.
fn persist_client_trust(
    server_address: &str,
    material: &ss_network::pairing::PairedMaterial,
) -> anyhow::Result<(PathBuf, PathBuf, PathBuf)> {
    ss_core::config::write_trust_material(
        server_address,
        &material.client_cert_pem,
        &material.client_key_pem,
        &material.ca_cert_pem,
    )
}

/// Best-effort enumeration of local non-loopback IPv4 addresses for cert SANs.
fn local_ips() -> Vec<std::net::IpAddr> {
    // Use a UDP socket trick to discover the primary outbound IP.
    let mut ips = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                ips.push(addr.ip());
            }
        }
    }
    ips
}

/// Run headless server (CLI mode)
async fn run_headless_server(
    port: u16,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    ca: Option<PathBuf>,
    pairing_enabled: bool,
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
                ServerEvent::ClientPaired { name } => tracing::info!("Client paired: {name}"),
            }
        }
    });

    // Resolve TLS material (auto-generate CA if not provided).
    let resolved = resolve_server_certs(&cert, &key, &ca)?;

    // Set up pairing if enabled.
    let pairing = if pairing_enabled {
        let (support, pin_manager, mut paired_rx) = build_pairing_support(&resolved, port).await?;
        println!("Pairing enabled. PIN: {}", pin_manager.current_pin());
        // Persist newly paired clients.
        tokio::spawn(async move {
            while let Some(paired) = paired_rx.recv().await {
                let mut cfg = AppConfig::load();
                cfg.server.upsert_paired_client(paired);
                let _ = cfg.save();
            }
        });
        Some(support)
    } else {
        None
    };

    let config = ss_network::server::ServerConfig {
        control_port: port,
        data_port: port + 1,
        cert_path: resolved.cert_path,
        key_path: resolved.key_path,
        ca_path: resolved.ca_path,
        pairing,
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
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    ca: Option<PathBuf>,
    name: String,
    pair: bool,
) -> anyhow::Result<()> {
    tracing::info!("Connecting to {server} as {name}");

    let mut config = AppConfig::load();

    // Resolve trust: explicit paths > known server > pairing.
    let (cert_path, key_path, ca_path) =
        match resolve_client_certs(&config, &server, &cert, &key, &ca) {
            Some(paths) => paths,
            None => {
                if !pair {
                    anyhow::bail!(
                        "No trust material for {server}. Re-run with --pair to pair using a PIN."
                    );
                }
                // Prompt for the PIN on stdin and pair.
                let host = server.split(':').next().unwrap_or(&server).to_string();
                let control_port: u16 = server
                    .split(':')
                    .nth(1)
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(9876);
                let pairing_port = ss_network::pairing::client::default_pairing_port(control_port);

                eprint!("Enter pairing PIN shown on the server: ");
                use std::io::Write;
                std::io::stderr().flush().ok();
                let mut pin = String::new();
                std::io::stdin().read_line(&mut pin)?;
                let pin = pin.trim().to_string();

                let material = ss_network::pairing::client::pair_with_server(
                    &host,
                    pairing_port,
                    &pin,
                    &name,
                )
                .await
                .map_err(|e| anyhow::anyhow!("pairing failed: {e}"))?;

                let (cert_path, key_path, ca_path) = persist_client_trust(&server, &material)?;
                let fingerprint =
                    ss_network::cert::cert_fingerprint(&material.ca_cert_pem).unwrap_or_default();
                config.client.upsert_known_server(ss_core::config::KnownServer {
                    address: server.clone(),
                    cert_path: cert_path.clone(),
                    key_path: key_path.clone(),
                    ca_path: ca_path.clone(),
                    server_fingerprint: fingerprint,
                });
                let _ = config.save();
                tracing::info!("Paired successfully; trust persisted.");
                (cert_path, key_path, ca_path)
            }
        };

    let (screen_w, screen_h) = detect_screen_size();
    let config = ss_network::client::ClientConfig {
        server_address: server,
        cert_path,
        key_path,
        ca_path,
        device_name: name,
        screen_width: screen_w,
        screen_height: screen_h,
    };

    let conn = ss_network::client::connect_with_retry(config).await?;
    tracing::info!("Connected. Press Ctrl+C to disconnect.");

    // Client virtual cursor (for receiving MouseDelta from server)
    let virtual_cursor = Arc::new(Mutex::new(((screen_w / 2) as f32, (screen_h / 2) as f32)));

    // Start clipboard monitor
    let monitor = ss_clipboard::monitor::ClipboardMonitor::new();
    let (mut clip_change_rx, clip_suppress_tx) = ss_clipboard::monitor::start_monitor(monitor);

    // Task: inject input events received from server (passive mode)
    let mut data_rx_inject = conn.data_rx.resubscribe();
    let virtual_cursor_inject = virtual_cursor.clone();
    tokio::spawn(async move {
        loop {
            match data_rx_inject.recv().await {
                Ok(msg) => {
                    match &msg {
                        Message::MouseDelta { dx, dy } => {
                            // Apply delta to virtual cursor
                            let (vx, vy) = {
                                let mut vc = virtual_cursor_inject.lock().unwrap();
                                vc.0 += dx;
                                vc.1 += dy;
                                vc.0 = vc.0.clamp(0.0, (screen_w - 1) as f32);
                                vc.1 = vc.1.clamp(0.0, (screen_h - 1) as f32);
                                (vc.0, vc.1)
                            };
                            let move_msg = Message::MouseMove { x: vx, y: vy };
                            ss_input::inject::inject_event(&move_msg);
                        }
                        Message::MouseMove { x, y } => {
                            {
                                let mut vc = virtual_cursor_inject.lock().unwrap();
                                vc.0 = *x;
                                vc.1 = *y;
                            }
                            ss_input::inject::inject_event(&msg);
                        }
                        Message::MouseButton { .. }
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

    // Task: listen for boundary events
    let mut control_rx_boundary = conn.control_rx.resubscribe();
    let virtual_cursor_ctrl = virtual_cursor.clone();
    tokio::spawn(async move {
        loop {
            match control_rx_boundary.recv().await {
                Ok(Message::BoundaryEnter { enter_x, enter_y, .. }) => {
                    tracing::info!("Boundary enter at ({enter_x:.0}, {enter_y:.0})");
                    // Update virtual cursor position
                    {
                        let mut vc = virtual_cursor_ctrl.lock().unwrap();
                        vc.0 = enter_x;
                        vc.1 = enter_y;
                    }
                    // Move cursor to enter position
                    ss_input::inject::inject_event(&Message::MouseMove { x: enter_x, y: enter_y });
                }
                Ok(Message::BoundaryLeave { .. }) => {
                    tracing::info!("Boundary leave received");
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
    tracing::info!("Detecting screen size...");

    // Try xrandr first (Linux X11)
    #[cfg(target_os = "linux")]
    {
        tracing::debug!("Trying xrandr...");
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
                                    tracing::info!("Detected screen via xrandr: {w}x{h}");
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
                                tracing::info!("Detected screen via xrandr fallback: {w}x{h}");
                                return (w, h);
                            }
                        }
                    }
                }
            }
            tracing::warn!("xrandr output did not contain resolution info");
        } else {
            tracing::warn!("xrandr command failed");
        }
    }

    // Windows: Use Win32 API (GetSystemMetrics)
    #[cfg(target_os = "windows")]
    {
        tracing::debug!("Trying Win32 GetSystemMetrics...");
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

            let w = GetSystemMetrics(SM_CXSCREEN) as u32;
            let h = GetSystemMetrics(SM_CYSCREEN) as u32;
            if w > 0 && h > 0 {
                tracing::info!("Detected screen via Win32 API: {w}x{h}");
                return (w, h);
            }
            tracing::warn!("GetSystemMetrics returned invalid size: {w}x{h}");
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
