use ss_core::protocol::{Button, Message};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Captured input event before conversion to protocol message
#[derive(Debug, Clone)]
pub enum InputEvent {
    MouseMove { x: f64, y: f64 },
    MouseButton { button: Button, pressed: bool },
    MouseScroll { dx: f64, dy: f64 },
    KeyPress { keycode: u32, pressed: bool },
}

/// State shared with the rdev callback
struct CaptureState {
    tx: mpsc::Sender<InputEvent>,
    suppressed: Arc<Mutex<bool>>,
}

/// Start capturing global input events.
/// Returns a receiver that yields InputEvents.
/// The `suppressed` flag, when set to true, causes all events to be ignored
/// (used when the mouse is on a remote screen).
pub fn start_capture(
    suppressed: Arc<Mutex<bool>>,
) -> mpsc::Receiver<InputEvent> {
    let (tx, rx) = mpsc::channel(256);

    std::thread::spawn(move || {
        let state = CaptureState {
            tx: tx.clone(),
            suppressed,
        };
        let state = Arc::new(Mutex::new(state));

        let callback_state = state.clone();
        let callback = move |event: rdev::Event| {
            let state = callback_state.lock().unwrap();
            // Check if input is suppressed
            if *state.suppressed.lock().unwrap() {
                return;
            }

            let input_event = match event.event_type {
                rdev::EventType::MouseMove { x, y } => {
                    Some(InputEvent::MouseMove { x, y })
                }
                rdev::EventType::ButtonPress(btn) => {
                    map_button(btn).map(|b| InputEvent::MouseButton {
                        button: b,
                        pressed: true,
                    })
                }
                rdev::EventType::ButtonRelease(btn) => {
                    map_button(btn).map(|b| InputEvent::MouseButton {
                        button: b,
                        pressed: false,
                    })
                }
                rdev::EventType::Wheel { delta_x, delta_y } => {
                    Some(InputEvent::MouseScroll {
                        dx: delta_x as f64,
                        dy: delta_y as f64,
                    })
                }
                rdev::EventType::KeyPress(key) => {
                    Some(InputEvent::KeyPress {
                        keycode: key_to_u32(key),
                        pressed: true,
                    })
                }
                rdev::EventType::KeyRelease(key) => {
                    Some(InputEvent::KeyPress {
                        keycode: key_to_u32(key),
                        pressed: false,
                    })
                }
            };

            if let Some(event) = input_event {
                let _ = state.tx.try_send(event);
            }
        };

        if let Err(e) = rdev::listen(callback) {
            tracing::error!("rdev::listen failed: {:?}", e);
        }
    });

    rx
}

/// Map rdev button to our protocol Button
fn map_button(btn: rdev::Button) -> Option<Button> {
    match btn {
        rdev::Button::Left => Some(Button::Left),
        rdev::Button::Right => Some(Button::Right),
        rdev::Button::Middle => Some(Button::Middle),
        _ => None,
    }
}

/// Convert rdev::Key to a u32 keycode for transmission
fn key_to_u32(key: rdev::Key) -> u32 {
    // Use the Debug format hash as a simple numeric representation
    // This is a simplified approach; a production system would use a proper mapping table
    let name = format!("{:?}", key);
    let hash = blake3::hash(name.as_bytes());
    let bytes: [u8; 4] = hash.as_bytes()[..4].try_into().unwrap_or([0; 4]);
    u32::from_le_bytes(bytes)
}

/// Convert an InputEvent to a protocol Message
pub fn to_message(event: &InputEvent) -> Message {
    match event {
        InputEvent::MouseMove { x, y } => Message::MouseMove {
            x: *x as f32,
            y: *y as f32,
        },
        InputEvent::MouseButton { button, pressed } => Message::MouseButton {
            button: *button,
            pressed: *pressed,
        },
        InputEvent::MouseScroll { dx, dy } => Message::MouseScroll {
            dx: *dx as f32,
            dy: *dy as f32,
        },
        InputEvent::KeyPress { keycode, pressed } => Message::KeyPress {
            keycode: *keycode,
            pressed: *pressed,
        },
    }
}
