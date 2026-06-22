use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

/// Tray menu item IDs
pub const MENU_OPEN: &str = "open";
pub const MENU_STATUS: &str = "status";
pub const MENU_QUIT: &str = "quit";

/// Create a system tray icon with a context menu.
/// Returns the tray icon handle and a channel for menu events.
pub fn create_tray() -> anyhow::Result<(TrayIcon, tokio::sync::mpsc::Receiver<String>)> {
    // Build the context menu
    let menu = Menu::new();
    let open_item = MenuItem::with_id(MENU_OPEN, "Open Settings", true, None);
    let status_item = MenuItem::with_id(MENU_STATUS, "Connection Status", true, None);
    let quit_item = MenuItem::with_id(MENU_QUIT, "Quit", true, None);

    menu.append(&open_item)?;
    menu.append(&status_item)?;
    menu.append(&quit_item)?;

    // Create a simple icon (1x1 pixel placeholder)
    // In a real app, you'd embed a proper icon
    let icon_data = create_placeholder_icon();
    let icon = Icon::from_rgba(icon_data, 1, 1)
        .map_err(|e| anyhow::anyhow!("Failed to create icon: {e}"))?;

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("SuperShare")
        .with_icon(icon)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create tray icon: {e}"))?;

    // Channel for menu events
    let (tx, rx) = tokio::sync::mpsc::channel(16);

    // Listen for menu events
    let menu_channel = MenuEvent::receiver();
    tokio::spawn(async move {
        loop {
            // Poll for menu events (tray-icon uses a global event receiver)
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if let Ok(event) = menu_channel.try_recv() {
                let id = event.id.0.clone();
                let _ = tx.send(id).await;
            }
        }
    });

    Ok((tray_icon, rx))
}

/// Create a minimal 1x1 RGBA icon as a placeholder
fn create_placeholder_icon() -> Vec<u8> {
    // Simple blue pixel
    vec![0x40, 0x80, 0xFF, 0xFF]
}
