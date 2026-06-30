use eframe::egui;
use ss_core::config::AppConfig;
use crate::state::{AppCommand, CommandSender, SharedState};

/// Tab selection for the configuration UI
#[derive(PartialEq)]
enum Tab {
    Server,
    Client,
    Clipboard,
}

/// The main configuration application
pub struct SuperShareApp {
    config: AppConfig,
    selected_tab: Tab,
    /// Shared state with backend
    shared_state: SharedState,
    /// Command sender to backend
    cmd_tx: CommandSender,
    /// Validation error message
    validation_error: Option<String>,
    /// PIN entered by the user when pairing is required
    pin_input: String,
}

impl SuperShareApp {
    pub fn new(config: AppConfig, shared_state: SharedState, cmd_tx: CommandSender) -> Self {
        Self {
            config,
            selected_tab: Tab::Server,
            shared_state,
            cmd_tx,
            validation_error: None,
            pin_input: String::new(),
        }
    }
}

impl eframe::App for SuperShareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request repaint every frame to keep UI responsive
        ctx.request_repaint();

        // Top tab bar
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_tab, Tab::Server, "🖥 Server");
                ui.selectable_value(&mut self.selected_tab, Tab::Client, "💻 Client");
                ui.selectable_value(&mut self.selected_tab, Tab::Clipboard, "📋 Clipboard");
            });
        });

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.selected_tab {
                Tab::Server => self.show_server_tab(ui),
                Tab::Client => self.show_client_tab(ui),
                Tab::Clipboard => self.show_clipboard_tab(ui),
            }
        });
    }
}

impl SuperShareApp {
    fn show_server_tab(&mut self, ui: &mut egui::Ui) {
        // Read shared state (synchronous read lock)
        let state = self.shared_state.read().unwrap();

        ui.heading("Server Settings");
        ui.add_space(8.0);

        // Configuration fields (only editable when server is stopped)
        let enabled = !state.server_running;

        egui::Grid::new("server_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Control Port:");
                ui.add_enabled(
                    enabled,
                    egui::DragValue::new(&mut self.config.server.control_port).range(1024..=65535),
                );
                ui.end_row();

                ui.label("Data Port:");
                ui.add_enabled(
                    enabled,
                    egui::DragValue::new(&mut self.config.server.data_port).range(1024..=65535),
                );
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.add_enabled(
            enabled,
            egui::Checkbox::new(
                &mut self.config.server.pairing_enabled,
                "Enable pairing (let clients connect with just a PIN)",
            ),
        );

        // Show the current pairing PIN when the server is running with pairing.
        if state.server_running && self.config.server.pairing_enabled {
            if let Some(pin) = &state.pairing_pin {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Pairing PIN:");
                    ui.label(
                        egui::RichText::new(pin)
                            .heading()
                            .color(egui::Color32::LIGHT_BLUE)
                            .monospace(),
                    );
                });
                ui.label(
                    egui::RichText::new("Enter this PIN on the client to pair.")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            }
        }

        // Advanced: optional explicit TLS certificate paths.
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Advanced: TLS certificates (optional)")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Leave blank to auto-generate a CA on first run. Set these only for manual/advanced deployments.",
                    )
                    .small()
                    .color(egui::Color32::GRAY),
                );
                egui::Grid::new("server_cert_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        cert_path_row(ui, enabled, "TLS Certificate:", &mut self.config.server.cert_path, &["pem", "crt"]);
                        cert_path_row(ui, enabled, "TLS Key:", &mut self.config.server.key_path, &["pem", "key"]);
                        cert_path_row(ui, enabled, "CA Certificate:", &mut self.config.server.ca_path, &["pem", "crt"]);
                    });
            });

        ui.add_space(12.0);

        // Start/Stop button and status
        let mut cmd_to_send: Option<AppCommand> = None;
        ui.horizontal(|ui| {
            if state.server_running {
                if ui.button("■ Stop Server").clicked() {
                    cmd_to_send = Some(AppCommand::StopServer);
                }
            } else {
                if ui.button("▶ Start Server").clicked() {
                    // Cert paths are optional now — the server auto-generates a
                    // CA when they are not provided. But if any are set, all must be.
                    let some = self.config.server.cert_path.is_some()
                        || self.config.server.key_path.is_some()
                        || self.config.server.ca_path.is_some();
                    let all = self.config.server.cert_path.is_some()
                        && self.config.server.key_path.is_some()
                        && self.config.server.ca_path.is_some();
                    if some && !all {
                        self.validation_error = Some(
                            "Provide all three (cert, key, CA) or leave all blank to auto-generate.".to_string(),
                        );
                    } else {
                        cmd_to_send = Some(AppCommand::StartServer {
                            control_port: self.config.server.control_port,
                            data_port: self.config.server.data_port,
                            cert_path: self.config.server.cert_path.clone(),
                            key_path: self.config.server.key_path.clone(),
                            ca_path: self.config.server.ca_path.clone(),
                            pairing_enabled: self.config.server.pairing_enabled,
                        });
                        self.validation_error = None;
                    }
                }
            }

            // Status indicator
            if state.server_running {
                ui.colored_label(egui::Color32::GREEN, "●");
                ui.label(format!("Running (port {})", state.server_port.unwrap_or(0)));
            } else {
                ui.colored_label(egui::Color32::GRAY, "●");
                ui.label("Stopped");
            }
        });

        // Error display (backend errors)
        if let Some(err) = &state.last_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, format!("⚠ {err}"));
        }

        // Validation errors (from button click)
        if let Some(err) = &self.validation_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::YELLOW, format!("⚠ {err}"));
        }

        ui.add_space(12.0);

        // Connected clients list
        ui.heading("Connected Clients");
        ui.add_space(4.0);

        if state.connected_clients.is_empty() {
            ui.label(egui::RichText::new("No clients connected").italics().color(egui::Color32::GRAY));
        } else {
            egui::Grid::new("clients_grid")
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Connected");
                    ui.end_row();

                    for client in &state.connected_clients {
                        ui.colored_label(egui::Color32::GREEN, "●");
                        ui.label(&client.name);
                        ui.end_row();
                    }
                });
        }

        // Drop the read lock
        drop(state);

        // Send command after lock is released
        if let Some(cmd) = cmd_to_send {
            tracing::info!("Sending command to backend...");
            if let Err(e) = self.cmd_tx.try_send(cmd) {
                tracing::error!("Failed to send command: {e}");
            }
        }

        ui.add_space(16.0);
        if ui.button("💾 Save Configuration").clicked() {
            if let Err(e) = self.config.save() {
                tracing::error!("Failed to save config: {e}");
            }
        }
    }

    fn show_client_tab(&mut self, ui: &mut egui::Ui) {
        let state = self.shared_state.read().unwrap();

        ui.heading("Client Settings");
        ui.add_space(8.0);

        let enabled = !state.client_connected;

        egui::Grid::new("client_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Device Name:");
                ui.add_enabled(enabled, egui::TextEdit::singleline(&mut self.config.client.device_name));
                ui.end_row();

                ui.label("Server Address:");
                let mut addr = self
                    .config
                    .client
                    .server_address
                    .clone()
                    .unwrap_or_default();
                ui.add_enabled(enabled, egui::TextEdit::singleline(&mut addr));
                self.config.client.server_address = if addr.is_empty() { None } else { Some(addr) };
                ui.end_row();
            });

        // Advanced: optional explicit TLS certificate paths (override pairing).
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Advanced: TLS certificates (optional)")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Leave blank to pair automatically with a PIN. Set these only to use manually-issued certificates.",
                    )
                    .small()
                    .color(egui::Color32::GRAY),
                );
                egui::Grid::new("client_cert_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        cert_path_row(ui, enabled, "TLS Certificate:", &mut self.config.client.cert_path, &["pem", "crt"]);
                        cert_path_row(ui, enabled, "TLS Key:", &mut self.config.client.key_path, &["pem", "key"]);
                        cert_path_row(ui, enabled, "CA Certificate:", &mut self.config.client.ca_path, &["pem", "crt"]);
                    });
            });

        ui.add_space(12.0);

        // PIN prompt when the backend signals pairing is required.
        if state.pairing_required && !state.client_connected {
            ui.separator();
            ui.label(
                egui::RichText::new("This server is not paired yet. Enter the PIN shown on the server:")
                    .color(egui::Color32::LIGHT_YELLOW),
            );
            ui.horizontal(|ui| {
                ui.label("PIN:");
                ui.add(egui::TextEdit::singleline(&mut self.pin_input).desired_width(120.0));
            });
            if let Some(status) = &state.pairing_status {
                ui.colored_label(egui::Color32::LIGHT_BLUE, status);
            }
        }

        // Connect/Disconnect button and status
        let mut cmd_to_send: Option<AppCommand> = None;
        ui.horizontal(|ui| {
            if state.client_connected {
                if ui.button("■ Disconnect").clicked() {
                    cmd_to_send = Some(AppCommand::DisconnectClient);
                }
            } else if state.pairing_required {
                if ui.button("🔑 Pair & Connect").clicked() {
                    if self.config.client.server_address.is_none() {
                        self.validation_error = Some("Please enter the server address.".to_string());
                    } else if self.pin_input.trim().is_empty() {
                        self.validation_error = Some("Please enter the pairing PIN.".to_string());
                    } else {
                        cmd_to_send = Some(AppCommand::PairAndConnect {
                            server_address: self.config.client.server_address.clone().unwrap(),
                            pin: self.pin_input.trim().to_string(),
                            device_name: self.config.client.device_name.clone(),
                        });
                        self.validation_error = None;
                    }
                }
            } else {
                if ui.button("▶ Connect").clicked() {
                    if self.config.client.server_address.is_none() {
                        self.validation_error = Some("Please enter the server address.".to_string());
                    } else {
                        // Cert paths optional: pass them through; backend resolves
                        // persisted trust / pairing when they are absent.
                        cmd_to_send = Some(AppCommand::ConnectClient {
                            server_address: self.config.client.server_address.clone().unwrap(),
                            cert_path: self.config.client.cert_path.clone(),
                            key_path: self.config.client.key_path.clone(),
                            ca_path: self.config.client.ca_path.clone(),
                            device_name: self.config.client.device_name.clone(),
                        });
                        self.validation_error = None;
                    }
                }
            }

            // Status
            if state.client_connected {
                ui.colored_label(egui::Color32::GREEN, "●");
                ui.label(format!(
                    "Connected to {}",
                    state.client_server_addr.as_deref().unwrap_or("unknown")
                ));
            } else {
                ui.colored_label(egui::Color32::GRAY, "●");
                ui.label("Disconnected");
            }
        });

        // Error display (backend errors)
        if let Some(err) = &state.last_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, format!("⚠ {err}"));
        }

        // Validation errors
        if let Some(err) = &self.validation_error {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::YELLOW, format!("⚠ {err}"));
        }

        // Server screen info
        if let Some((w, h)) = state.server_screen_size {
            ui.add_space(8.0);
            ui.label(format!("Server screen: {w}×{h}"));
        }

        // Drop lock before sending command
        drop(state);

        // Send command
        if let Some(cmd) = cmd_to_send {
            if let Err(e) = self.cmd_tx.try_send(cmd) {
                tracing::error!("Failed to send command: {e}");
            }
        }

        ui.add_space(16.0);
        if ui.button("💾 Save Configuration").clicked() {
            if let Err(e) = self.config.save() {
                tracing::error!("Failed to save config: {e}");
            }
        }
    }

    fn show_clipboard_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Clipboard Settings");
        ui.add_space(8.0);

        ui.checkbox(&mut self.config.clipboard.text_enabled, "Enable text clipboard sync");
        ui.checkbox(&mut self.config.clipboard.image_enabled, "Enable image clipboard sync");

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Max image size (MB):");
            let mut max_mb = (self.config.clipboard.max_image_size / (1024 * 1024)) as u32;
            ui.add(egui::DragValue::new(&mut max_mb).range(1..=100));
            self.config.clipboard.max_image_size = (max_mb as usize) * 1024 * 1024;
        });

        ui.add_space(16.0);
        if ui.button("💾 Save Configuration").clicked() {
            if let Err(e) = self.config.save() {
                tracing::error!("Failed to save config: {e}");
            }
        }
    }
}

/// Render a labelled certificate-path row with a Browse button into a Grid.
fn cert_path_row(
    ui: &mut egui::Ui,
    enabled: bool,
    label: &str,
    path: &mut Option<std::path::PathBuf>,
    filters: &[&str],
) {
    ui.label(label);
    let text = path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(&text);
        if ui.add_enabled(enabled, egui::Button::new("Browse...")).clicked() {
            if let Some(picked) = rfd::FileDialog::new()
                .add_filter("PEM", filters)
                .pick_file()
            {
                *path = Some(picked);
            }
        }
        if path.is_some() && ui.add_enabled(enabled, egui::Button::new("✖")).clicked() {
            *path = None;
        }
    });
    ui.end_row();
}

/// Launch the configuration GUI with runtime integration
pub fn run_gui(
    config: AppConfig,
    shared_state: SharedState,
    cmd_tx: CommandSender,
) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 450.0])
            .with_min_inner_size([400.0, 350.0]),
        ..Default::default()
    };

    eframe::run_native(
        "SuperShare",
        options,
        Box::new(move |_cc| Ok(Box::new(SuperShareApp::new(config, shared_state, cmd_tx)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))
}
