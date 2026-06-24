//! evdev-based input capture for Linux.
//!
//! Directly reads from `/dev/input/event*` devices to get raw mouse deltas
//! (EV_REL::REL_X/REL_Y) that are NOT clamped by screen boundaries.
//!
//! Uses grab to get exclusive access to devices, and creates UInput virtual
//! device copies for local event pass-through.
//!
//! IMPORTANT: Events are ALWAYS passed through to UInput by default.
//! The server must explicitly suppress pass-through when switching to REMOTE mode.

use evdev::{EventType, InputEvent, Key, RelativeAxisType};
use ss_core::protocol::Button;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// A captured input event from evdev, before conversion to protocol message.
#[derive(Debug, Clone)]
pub enum EvdevInputEvent {
    /// Raw mouse delta from REL_X/REL_Y (in device units, typically pixels)
    MouseDelta { dx: f32, dy: f32 },
    /// Mouse button press/release
    MouseButton { button: Button, pressed: bool },
    /// Keyboard key press/release
    KeyPress { keycode: u32, pressed: bool },
    /// Scroll wheel
    Scroll { dx: f32, dy: f32 },
}

/// Handle to the evdev capture system.
pub struct EvdevCaptureHandle {
    /// Receiver for captured input events.
    pub event_rx: mpsc::Receiver<EvdevInputEvent>,
    /// Flag to control whether events are passed through to UInput.
    /// When true: events are passed through (LOCAL mode).
    /// When false: events are suppressed (REMOTE mode).
    pub pass_through: Arc<AtomicBool>,
}

/// Start capturing input events from all evdev devices.
///
/// Returns a handle with:
/// - `event_rx`: receives captured input events
/// - `pass_through`: set to false to suppress local injection (REMOTE mode)
///
/// IMPORTANT: Events are ALWAYS passed through to UInput by default.
/// This ensures the local keyboard/mouse always work.
pub fn start_capture() -> anyhow::Result<EvdevCaptureHandle> {
    let (event_tx, event_rx) = mpsc::channel::<EvdevInputEvent>(256);
    let pass_through = Arc::new(AtomicBool::new(true)); // Default: pass through (LOCAL mode)

    // Discover and open all input devices
    let mut device_count = 0usize;

    for (idx, (path, mut device)) in evdev::enumerate().enumerate() {
        let path_str = path.to_string_lossy().to_string();

        // Skip virtual devices (uinput) to avoid feedback loops
        if path_str.contains("uinput") {
            tracing::debug!("Skipping virtual device: {path_str}");
            continue;
        }

        // Classify device capabilities
        let supported = device.supported_events();
        let has_relative = supported.contains(EventType::RELATIVE);
        let has_keys = supported.contains(EventType::KEY);

        // Skip devices that aren't input devices (no keys or relative axes)
        if !has_relative && !has_keys {
            tracing::debug!("Skipping non-input device: {path_str}");
            continue;
        }

        let is_mouse = has_relative;
        let is_keyboard = has_keys;

        tracing::info!(
            "Opening device: {path_str} (mouse={is_mouse}, keyboard={is_keyboard})"
        );

        // Grab the device for exclusive access
        device.grab().map_err(|e| {
            anyhow::anyhow!("Failed to grab device {path_str}: {e}")
        })?;

        // Create UInput virtual device copy for pass-through
        let uinput = create_uinput_copy(&device)?;

        // Convert to async event stream
        let stream = device.into_event_stream().map_err(|e| {
            anyhow::anyhow!("Failed to create event stream for {path_str}: {e}")
        })?;

        // Spawn a task for this device
        let event_tx = event_tx.clone();
        let pass_through = pass_through.clone();
        let device_name = path_str.clone();

        tokio::spawn(async move {
            let mut stream = stream;
            let mut uinput = uinput;
            let mut event_count: u64 = 0;

            loop {
                match stream.next_event().await {
                    Ok(raw_event) => {
                        event_count += 1;

                        // Log first few events for debugging
                        if event_count <= 5 {
                            tracing::info!(
                                "Device {device_name}: event type={:?} code={} value={} pass_through={}",
                                raw_event.event_type(),
                                raw_event.code(),
                                raw_event.value(),
                                pass_through.load(Ordering::Relaxed)
                            );
                        }

                        // ALWAYS pass through to UInput first (so local input works)
                        if pass_through.load(Ordering::Relaxed) {
                            if let Err(e) = uinput.emit(&[raw_event.clone()]) {
                                tracing::warn!("Failed to emit to UInput for {device_name}: {e}");
                            }
                        }

                        // Parse and send to channel for forwarding
                        if let Some(parsed) = parse_event(&raw_event, is_mouse, is_keyboard) {
                            if event_tx.send(parsed).await.is_err() {
                                tracing::debug!("Event receiver dropped for {device_name}");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Event stream error for {device_name}: {e}");
                        break;
                    }
                }
            }
        });

        device_count += 1;
        tracing::info!("Grabbed device: {path_str}");
    }

    if device_count == 0 {
        // Check if /dev/input exists and has event devices
        let input_dir = std::path::Path::new("/dev/input");
        if !input_dir.exists() {
            anyhow::bail!("/dev/input directory not found. Is this a Linux system?");
        }

        let event_devices: Vec<_> = std::fs::read_dir(input_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
            .collect();

        if event_devices.is_empty() {
            anyhow::bail!("No input event devices found in /dev/input/. Is evdev supported?");
        }

        anyhow::bail!(
            "Found {} event devices in /dev/input/ but could not access any of them.\n\
             This is likely a permission issue. To fix:\n\
             1. Install udev rules: sudo cp assets/99-superShare.rules /etc/udev/rules.d/\n\
             2. Reload rules: sudo udevadm control --reload-rules && sudo udevadm trigger\n\
             3. Add user to input group: sudo usermod -aG input $USER\n\
             4. Log out and back in for group changes to take effect",
            event_devices.len()
        );
    }

    tracing::info!("Capturing {device_count} input devices (pass-through enabled by default)");

    Ok(EvdevCaptureHandle {
        event_rx,
        pass_through,
    })
}

/// Parse a raw evdev InputEvent into our EvdevInputEvent.
fn parse_event(
    event: &InputEvent,
    is_mouse: bool,
    is_keyboard: bool,
) -> Option<EvdevInputEvent> {
    match event.event_type() {
        EventType::RELATIVE => {
            if !is_mouse {
                return None;
            }
            let code = RelativeAxisType(event.code());
            let value = event.value() as f32;
            match code {
                RelativeAxisType::REL_X => Some(EvdevInputEvent::MouseDelta {
                    dx: value,
                    dy: 0.0,
                }),
                RelativeAxisType::REL_Y => Some(EvdevInputEvent::MouseDelta {
                    dx: 0.0,
                    dy: value,
                }),
                RelativeAxisType::REL_WHEEL => Some(EvdevInputEvent::Scroll {
                    dx: 0.0,
                    dy: value,
                }),
                RelativeAxisType::REL_HWHEEL => Some(EvdevInputEvent::Scroll {
                    dx: value,
                    dy: 0.0,
                }),
                _ => None,
            }
        }
        EventType::KEY => {
            if !is_keyboard {
                return None;
            }
            let key = Key(event.code());
            let pressed = event.value() != 0;

            // Check if it's a mouse button
            if let Some(button) = key_to_button(key) {
                Some(EvdevInputEvent::MouseButton { button, pressed })
            } else {
                // It's a keyboard key
                Some(EvdevInputEvent::KeyPress {
                    keycode: event.code() as u32,
                    pressed,
                })
            }
        }
        _ => None,
    }
}

/// Convert an evdev Key to our protocol Button.
fn key_to_button(key: Key) -> Option<Button> {
    match key {
        Key::BTN_LEFT => Some(Button::Left),
        Key::BTN_RIGHT => Some(Button::Right),
        Key::BTN_MIDDLE => Some(Button::Middle),
        _ => None,
    }
}

/// Create a UInput virtual device copy of a physical device.
/// This is used to pass through events when in LOCAL mode.
fn create_uinput_copy(device: &evdev::Device) -> anyhow::Result<evdev::uinput::VirtualDevice> {
    let builder = evdev::uinput::VirtualDeviceBuilder::new()?;

    // Copy device name
    let name = device.name().unwrap_or("SuperShare Virtual Device");
    let builder = builder.name(name.as_bytes());

    // Copy relative axes if supported
    let builder = if let Some(rel_axes) = device.supported_relative_axes() {
        builder.with_relative_axes(rel_axes)?
    } else {
        builder
    };

    // Copy keys if supported
    let builder = if let Some(keys) = device.supported_keys() {
        builder.with_keys(keys)?
    } else {
        builder
    };

    builder.build().map_err(|e| anyhow::anyhow!("Failed to create UInput device: {e}"))
}

/// Convert an EvdevInputEvent to a protocol Message.
pub fn to_message(event: &EvdevInputEvent) -> ss_core::protocol::Message {
    match event {
        EvdevInputEvent::MouseDelta { dx, dy } => {
            ss_core::protocol::Message::MouseDelta {
                dx: *dx,
                dy: *dy,
            }
        }
        EvdevInputEvent::MouseButton { button, pressed } => {
            ss_core::protocol::Message::MouseButton {
                button: *button,
                pressed: *pressed,
            }
        }
        EvdevInputEvent::KeyPress { keycode, pressed } => {
            ss_core::protocol::Message::KeyPress {
                keycode: *keycode,
                pressed: *pressed,
            }
        }
        EvdevInputEvent::Scroll { dx, dy } => {
            ss_core::protocol::Message::MouseScroll {
                dx: *dx,
                dy: *dy,
            }
        }
    }
}
