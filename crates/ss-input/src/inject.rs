use ss_core::protocol::{Button, Message};
use blake3;

/// Inject input events into the local system.
/// Uses rdev::simulate on Windows and Linux X11.
/// On Linux Wayland, falls back to uinput.
pub fn inject_event(msg: &Message) {
    match msg {
        Message::MouseMove { x, y } => {
            let event_type = rdev::EventType::MouseMove {
                x: *x as f64,
                y: *y as f64,
            };
            inject_rdev(&event_type);
        }
        Message::MouseButton { button, pressed } => {
            let btn = map_button(*button);
            let event_type = if *pressed {
                rdev::EventType::ButtonPress(btn)
            } else {
                rdev::EventType::ButtonRelease(btn)
            };
            inject_rdev(&event_type);
        }
        Message::MouseScroll { dx, dy } => {
            let event_type = rdev::EventType::Wheel {
                delta_x: *dx as i64,
                delta_y: *dy as i64,
            };
            inject_rdev(&event_type);
        }
        Message::KeyPress { keycode, pressed } => {
            // Reconstruct rdev Key from keycode
            // Since we can't directly convert u32 back to Key,
            // we'll use a lookup approach based on common keys
            if let Some(key) = u32_to_key(*keycode) {
                let event_type = if *pressed {
                    rdev::EventType::KeyPress(key)
                } else {
                    rdev::EventType::KeyRelease(key)
                };
                inject_rdev(&event_type);
            } else {
                tracing::warn!("Unknown keycode: {keycode}");
            }
        }
        _ => {} // Not an input event
    }
}

/// Use rdev::simulate to inject an event
fn inject_rdev(event_type: &rdev::EventType) {
    if let Err(e) = rdev::simulate(event_type) {
        tracing::warn!("rdev::simulate failed: {:?}", e);
        // On Linux Wayland, try uinput fallback
        #[cfg(target_os = "linux")]
        {
            inject_uinput(event_type);
        }
    }
}

/// Map our protocol Button to rdev Button
fn map_button(btn: Button) -> rdev::Button {
    match btn {
        Button::Left => rdev::Button::Left,
        Button::Right => rdev::Button::Right,
        Button::Middle => rdev::Button::Middle,
    }
}

/// Convert a u32 keycode back to rdev::Key
/// This uses the same hashing approach as capture::key_to_u32
fn u32_to_key(keycode: u32) -> Option<rdev::Key> {
    // Build a lookup table of common keys
    // In a production system, this would be a proper bidirectional mapping
    let common_keys = [
        rdev::Key::Alt,
        rdev::Key::AltGr,
        rdev::Key::Backspace,
        rdev::Key::CapsLock,
        rdev::Key::ControlLeft,
        rdev::Key::ControlRight,
        rdev::Key::Delete,
        rdev::Key::DownArrow,
        rdev::Key::End,
        rdev::Key::Escape,
        rdev::Key::F1,
        rdev::Key::F2,
        rdev::Key::F3,
        rdev::Key::F4,
        rdev::Key::F5,
        rdev::Key::F6,
        rdev::Key::F7,
        rdev::Key::F8,
        rdev::Key::F9,
        rdev::Key::F10,
        rdev::Key::F11,
        rdev::Key::F12,
        rdev::Key::Home,
        rdev::Key::LeftArrow,
        rdev::Key::MetaLeft,
        rdev::Key::MetaRight,
        rdev::Key::PageDown,
        rdev::Key::PageUp,
        rdev::Key::Return,
        rdev::Key::RightArrow,
        rdev::Key::ShiftLeft,
        rdev::Key::ShiftRight,
        rdev::Key::Space,
        rdev::Key::Tab,
        rdev::Key::UpArrow,
    ];

    // Find the key by computing the same hash
    for key in common_keys.iter() {
        let name = format!("{:?}", key);
        let hash = blake3::hash(name.as_bytes());
        let bytes: [u8; 4] = hash.as_bytes()[..4].try_into().unwrap_or([0; 4]);
        let computed = u32::from_le_bytes(bytes);
        if computed == keycode {
            return Some(*key);
        }
    }

    None
}

/// Linux-specific: uinput-based injection fallback for Wayland
#[cfg(target_os = "linux")]
fn inject_uinput(event_type: &rdev::EventType) {
    // uinput injection is a more complex implementation
    // that creates a virtual input device via /dev/uinput
    // For now, log a warning
    tracing::warn!(
        "uinput injection not yet implemented for event: {:?}. \
         Consider using X11 for full input injection support.",
        event_type
    );
}
