use eframe::egui;
use ss_core::config::AppConfig;

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
    /// Status message to display
    status_msg: Option<String>,
}

impl SuperShareApp {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            selected_tab: Tab::Server,
            status_msg: None,
        }
    }
}

impl eframe::App for SuperShareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top tab bar
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_tab, Tab::Server, "🖥 Server");
                ui.selectable_value(&mut self.selected_tab, Tab::Client, "💻 Client");
                ui.selectable_value(&mut self.selected_tab, Tab::Clipboard, "📋 Clipboard");
            });
        });

        // Status bar at bottom
        let mut clear_status = false;
        if let Some(msg) = &self.status_msg {
            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(msg);
                    if ui.button("✕").clicked() {
                        clear_status = true;
                    }
                });
            });
        }
        if clear_status {
            self.status_msg = None;
        }

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
        ui.heading("Server Settings");
        ui.add_space(8.0);

        egui::Grid::new("server_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Control Port:");
                ui.add(egui::DragValue::new(&mut self.config.server.control_port).range(1024..=65535));
                ui.end_row();

                ui.label("Data Port:");
                ui.add(egui::DragValue::new(&mut self.config.server.data_port).range(1024..=65535));
                ui.end_row();

                ui.label("TLS Certificate:");
                let cert_text = self
                    .config
                    .server
                    .cert_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                ui.label(&cert_text);
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PEM", &["pem", "crt"])
                        .pick_file()
                    {
                        self.config.server.cert_path = Some(path);
                    }
                }
                ui.end_row();

                ui.label("TLS Key:");
                let key_text = self
                    .config
                    .server
                    .key_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                ui.label(&key_text);
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PEM", &["pem", "key"])
                        .pick_file()
                    {
                        self.config.server.key_path = Some(path);
                    }
                }
                ui.end_row();
            });

        ui.add_space(16.0);
        ui.heading("Connected Clients");
        ui.add_space(4.0);

        egui::Grid::new("clients_grid")
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("IP");
                ui.strong("Resolution");
                ui.strong("Position");
                ui.end_row();

                // Show existing clients
                let mut remove_idx = None;
                for (i, client) in self.config.server.clients.iter().enumerate() {
                    ui.label(&client.name);
                    ui.label(&client.ip);
                    ui.label(format!("{}×{}", client.screen_width, client.screen_height));
                    ui.label(&client.position);
                    if ui.button("🗑").clicked() {
                        remove_idx = Some(i);
                    }
                    ui.end_row();
                }
                if let Some(idx) = remove_idx {
                    self.config.server.clients.remove(idx);
                }
            });

        ui.add_space(4.0);
        if ui.button("+ Add Client").clicked() {
            self.config.server.clients.push(ss_core::config::ClientEntry {
                name: "New Client".to_string(),
                ip: "192.168.1.x".to_string(),
                screen_width: 1920,
                screen_height: 1080,
                position: "right".to_string(),
            });
        }

        ui.add_space(16.0);
        if ui.button("💾 Save Configuration").clicked() {
            match self.config.save() {
                Ok(()) => self.status_msg = Some("Configuration saved.".to_string()),
                Err(e) => self.status_msg = Some(format!("Save failed: {e}")),
            }
        }
    }

    fn show_client_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Client Settings");
        ui.add_space(8.0);

        egui::Grid::new("client_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Device Name:");
                ui.text_edit_singleline(&mut self.config.client.device_name);
                ui.end_row();

                ui.label("Server Address:");
                let mut addr = self
                    .config
                    .client
                    .server_address
                    .clone()
                    .unwrap_or_default();
                ui.text_edit_singleline(&mut addr);
                self.config.client.server_address = if addr.is_empty() { None } else { Some(addr) };
                ui.end_row();

                ui.label("TLS Certificate:");
                let cert_text = self
                    .config
                    .client
                    .cert_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                ui.label(&cert_text);
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PEM", &["pem", "crt"])
                        .pick_file()
                    {
                        self.config.client.cert_path = Some(path);
                    }
                }
                ui.end_row();

                ui.label("TLS Key:");
                let key_text = self
                    .config
                    .client
                    .key_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                ui.label(&key_text);
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PEM", &["pem", "key"])
                        .pick_file()
                    {
                        self.config.client.key_path = Some(path);
                    }
                }
                ui.end_row();

                ui.label("CA Certificate:");
                let ca_text = self
                    .config
                    .client
                    .ca_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                ui.label(&ca_text);
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PEM", &["pem", "crt"])
                        .pick_file()
                    {
                        self.config.client.ca_path = Some(path);
                    }
                }
                ui.end_row();
            });

        ui.add_space(16.0);
        if ui.button("💾 Save Configuration").clicked() {
            match self.config.save() {
                Ok(()) => self.status_msg = Some("Configuration saved.".to_string()),
                Err(e) => self.status_msg = Some(format!("Save failed: {e}")),
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
            match self.config.save() {
                Ok(()) => self.status_msg = Some("Configuration saved.".to_string()),
                Err(e) => self.status_msg = Some(format!("Save failed: {e}")),
            }
        }
    }
}

/// Launch the configuration GUI
pub fn run_gui(config: AppConfig) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 400.0])
            .with_min_inner_size([400.0, 300.0]),
        ..Default::default()
    };

    eframe::run_native(
        "SuperShare",
        options,
        Box::new(|_cc| Ok(Box::new(SuperShareApp::new(config)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))
}
