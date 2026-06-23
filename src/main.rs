mod certgen;

use clap::Parser;
use ss_core::config::AppConfig;
use ss_network::ServerEvent;
use ss_ui::state::{AppCommand, ClientInfo, SharedAppState, SharedState};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
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

                let server_state = Arc::new(ss_network::server::ServerState::new(1920, 1080));

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
                    match ss_network::client::connect_with_retry(config).await {
                        Ok(mut conn) => {
                            // Update state on successful connection
                            {
                                let mut s = state_clone.write().unwrap();
                                s.client_connected = true;
                                s.client_server_addr = Some(server_address.clone());
                                s.server_screen_size = conn.server_screen;
                                s.last_error = None;
                            }

                            // Keep connection alive, listen for disconnect
                            loop {
                                tokio::select! {
                                    msg = conn.control_rx.recv() => {
                                        match msg {
                                            Ok(ss_core::protocol::Message::Heartbeat) => {}
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
                        Err(e) => {
                            tracing::error!("Client connection error: {e}");
                            let mut s = state_clone.write().unwrap();
                            s.client_connected = false;
                            s.last_error = Some(format!("Connection failed: {e}"));
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

    let server_state = Arc::new(ss_network::server::ServerState::new(1920, 1080));
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

    let _conn = ss_network::client::connect_with_retry(config).await?;
    tracing::info!("Connected. Press Ctrl+C to disconnect.");

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Disconnecting");
    Ok(())
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
