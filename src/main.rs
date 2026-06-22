mod certgen;

use clap::{Parser, Subcommand};
use ss_core::config::AppConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Parser)]
#[command(name = "supershare", version, about = "Cross-machine keyboard, mouse and clipboard sharing")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start in server mode
    Server {
        /// Listen port for control channel
        #[arg(long, default_value = "9876")]
        port: u16,

        /// TLS certificate file path
        #[arg(long)]
        cert: PathBuf,

        /// TLS private key file path
        #[arg(long)]
        key: PathBuf,

        /// CA certificate file path for mTLS
        #[arg(long)]
        ca: PathBuf,
    },
    /// Start in client mode
    Client {
        /// Server address (host:port)
        #[arg(long)]
        server: String,

        /// TLS certificate file path
        #[arg(long)]
        cert: PathBuf,

        /// TLS private key file path
        #[arg(long)]
        key: PathBuf,

        /// CA certificate file path
        #[arg(long)]
        ca: PathBuf,

        /// Device name
        #[arg(long, default_value = "")]
        name: String,
    },
    /// Open the configuration GUI
    Gui,
    /// Generate TLS certificates
    GenCert {
        /// Output directory for generated certificates
        #[arg(long, default_value = "./certs")]
        output: PathBuf,

        /// Generate device certificate (requires --ca-cert and --ca-key)
        #[arg(long)]
        device: Option<String>,

        /// CA certificate path (for signing device certs)
        #[arg(long)]
        ca_cert: Option<PathBuf>,

        /// CA key path (for signing device certs)
        #[arg(long)]
        ca_key: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Server {
            port,
            cert,
            key,
            ca,
        } => {
            run_server(port, cert, key, ca).await?;
        }
        Commands::Client {
            server,
            cert,
            key,
            ca,
            name,
        } => {
            run_client(server, cert, key, ca, name).await?;
        }
        Commands::Gui => {
            run_gui()?;
        }
        Commands::GenCert {
            output,
            device,
            ca_cert,
            ca_key,
        } => {
            run_gen_cert(output, device, ca_cert, ca_key)?;
        }
    }

    Ok(())
}

/// Run the server
async fn run_server(
    port: u16,
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
) -> anyhow::Result<()> {
    let config = AppConfig::load();
    let data_port = port + 1;

    tracing::info!("Starting SuperShare server");
    tracing::info!("  Control port: {port}");
    tracing::info!("  Data port: {data_port}");

    let state = Arc::new(ss_network::server::ServerState::new(1920, 1080)); // TODO: detect actual screen size

    let server_config = ss_network::server::ServerConfig {
        control_port: port,
        data_port,
        cert_path: cert,
        key_path: key,
        ca_path: ca,
    };

    let (shutdown_tx, _) = broadcast::channel(1);

    // Handle Ctrl+C
    let shutdown_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received");
        let _ = shutdown_signal.send(());
    });

    // Start input capture
    let suppressed = Arc::new(std::sync::Mutex::new(false));
    let mut input_rx = ss_input::capture::start_capture(suppressed.clone());

    // Start clipboard monitor
    let clipboard_monitor = ss_clipboard::monitor::ClipboardMonitor::new();
    let (mut clipboard_rx, _suppress_tx) = ss_clipboard::monitor::start_monitor(clipboard_monitor);

    let state_input = state.clone();
    let state_clipboard = state.clone();

    // Spawn input event forwarder
    tokio::spawn(async move {
        while let Some(event) = input_rx.recv().await {
            let msg = ss_input::capture::to_message(&event);
            // TODO: boundary detection and routing
            ss_network::server::broadcast_control(&state_input, &msg).await;
        }
    });

    // Spawn clipboard event forwarder
    tokio::spawn(async move {
        while let Some(change) = clipboard_rx.recv().await {
            tracing::info!("Clipboard changed, syncing...");
            match ss_clipboard::sync::prepare_transfer(&change.content) {
                Ok(messages) => {
                    for msg in messages {
                        ss_network::server::broadcast_data(&state_clipboard, &msg).await;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to prepare clipboard transfer: {e}");
                }
            }
        }
    });

    // Run the server
    ss_network::server::start(server_config, state, shutdown_tx.subscribe()).await?;

    Ok(())
}

/// Run the client
async fn run_client(
    server: String,
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
    name: String,
) -> anyhow::Result<()> {
    let device_name = if name.is_empty() {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    } else {
        name
    };

    tracing::info!("Starting SuperShare client");
    tracing::info!("  Server: {server}");
    tracing::info!("  Device: {device_name}");

    let client_config = ss_network::client::ClientConfig {
        server_address: server,
        cert_path: cert,
        key_path: key,
        ca_path: ca,
        device_name,
    };

    let mut conn = ss_network::client::connect_with_retry(client_config).await?;

    tracing::info!("Connected to server");
    if let Some((w, h)) = conn.server_screen {
        tracing::info!("  Server screen: {w}x{h}");
    }

    // Start clipboard monitor
    let clipboard_monitor = ss_clipboard::monitor::ClipboardMonitor::new();
    let (mut clipboard_rx, _suppress_tx) = ss_clipboard::monitor::start_monitor(clipboard_monitor);

    let data_tx = conn.data_tx.clone();

    // Spawn clipboard sync task
    tokio::spawn(async move {
        while let Some(change) = clipboard_rx.recv().await {
            tracing::info!("Clipboard changed, sending to server...");
            match ss_clipboard::sync::prepare_transfer(&change.content) {
                Ok(messages) => {
                    for msg in messages {
                        let _ = data_tx.send(msg).await;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to prepare clipboard transfer: {e}");
                }
            }
        }
    });

    // Process incoming control messages (input events to inject)
    let suppressed = Arc::new(std::sync::Mutex::new(false));
    let suppressed_clone = suppressed.clone();

    tokio::spawn(async move {
        loop {
            match conn.control_rx.recv().await {
                Ok(msg) => {
                    match &msg {
                        ss_core::protocol::Message::BoundaryEnter { .. } => {
                            *suppressed_clone.lock().unwrap() = true;
                        }
                        ss_core::protocol::Message::BoundaryLeave { .. } => {
                            *suppressed_clone.lock().unwrap() = false;
                        }
                        _ => {
                            // Inject input event
                            ss_input::inject::inject_event(&msg);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Control channel error: {e}");
                    break;
                }
            }
        }
    });

    // Process incoming data messages (clipboard data)
    tokio::spawn(async move {
        let mut reassembler = None;
        loop {
            match conn.data_rx.recv().await {
                Ok(msg) => {
                    if let Some(content) =
                        ss_clipboard::sync::handle_clipboard_message(&msg, &mut reassembler)
                    {
                        tracing::info!("Received clipboard content from server");
                        match content {
                            ss_core::protocol::ClipboardContent::Text(text) => {
                                if let Err(e) = ss_clipboard::monitor::write_clipboard_text(&text) {
                                    tracing::error!("Failed to write clipboard text: {e}");
                                }
                            }
                            ss_core::protocol::ClipboardContent::Image {
                                width,
                                height,
                                rgba,
                            } => {
                                if let Err(e) =
                                    ss_clipboard::monitor::write_clipboard_image(width, height, &rgba)
                                {
                                    tracing::error!("Failed to write clipboard image: {e}");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Data channel error: {e}");
                    break;
                }
            }
        }
    });

    // Wait for shutdown
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Client shutting down");

    Ok(())
}

/// Run the GUI
fn run_gui() -> anyhow::Result<()> {
    let _config = AppConfig::load();
    ss_ui::app::run_gui(_config)
}

/// Run certificate generation
fn run_gen_cert(
    output: PathBuf,
    device: Option<String>,
    _ca_cert: Option<PathBuf>,
    _ca_key: Option<PathBuf>,
) -> anyhow::Result<()> {
    match device {
        Some(device_name) => {
            // Generate self-signed device cert
            certgen::generate_device_cert(&output, &device_name)?;
            println!("Device certificate for '{}' generated in {}", device_name, output.display());
        }
        None => {
            // Generate CA cert
            certgen::generate_ca(&output)?;
            println!("CA certificate and key generated in {}", output.display());
            println!("Next steps:");
            println!("  1. Generate device certs: supershare gen-cert --device <name>");
        }
    }
    Ok(())
}
