//! rdev-based input capture with cursor warping for delta calculation.
//!
//! This approach uses rdev::listen (XRecord) for input capture and rdev::simulate (XTest)
//! for cursor warping. It doesn't require root permissions or udev rules.
//!
//! How it works:
//! 1. Capture mouse events via rdev::listen (absolute coordinates)
//! 2. When cursor hits right edge, warp it back to screen center
//! 3. Calculate delta from center position
//! 4. Forward delta to client
//!
//! This is the same approach used by Deskflow/Synergy/Barrier.

use rdev::EventType;
use ss_core::protocol::Button;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// A captured input event, before conversion to protocol message.
#[derive(Debug, Clone)]
pub enum WarpInputEvent {
    /// Mouse delta calculated from cursor warping
    MouseDelta { dx: f32, dy: f32 },
    /// Mouse button press/release
    MouseButton { button: Button, pressed: bool },
    /// Keyboard key press/release
    KeyPress { keycode: u32, pressed: bool },
    /// Scroll wheel
    Scroll { dx: f32, dy: f32 },
}

/// Handle to the warp capture system.
pub struct WarpCaptureHandle {
    /// Receiver for captured input events.
    pub event_rx: mpsc::Receiver<WarpInputEvent>,
    /// Flag to control whether events are processed.
    /// When true: events are processed (LOCAL mode).
    /// When false: events are suppressed (REMOTE mode).
    pub enabled: Arc<AtomicBool>,
    /// Screen width for boundary detection.
    pub screen_width: u32,
    /// Screen height for boundary detection.
    pub screen_height: u32,
}

/// Shared state for the rdev callback.
struct CallbackState {
    tx: mpsc::Sender<WarpInputEvent>,
    enabled: Arc<AtomicBool>,
    screen_width: f64,
    screen_height: f64,
    /// Whether we're currently in a warp (to suppress the warp event itself)
    is_warping: Arc<AtomicBool>,
    /// Last known position (for delta calculation after warp)
    last_x: Arc<Mutex<f64>>,
    last_y: Arc<Mutex<f64>>,
    /// Whether we're in remote mode (cursor on client screen)
    is_remote: Arc<AtomicBool>,
}

/// Start capturing input events with cursor warping.
///
/// Returns a handle with:
/// - `event_rx`: receives captured input events (with deltas)
/// - `enabled`: set to false to suppress event processing
/// - `screen_width/height`: screen dimensions for boundary detection
pub fn start_capture(
    screen_width: u32,
    screen_height: u32,
) -> anyhow::Result<WarpCaptureHandle> {
    let (tx, rx) = mpsc::channel::<WarpInputEvent>(256);
    let enabled = Arc::new(AtomicBool::new(true));
    let is_warping = Arc::new(AtomicBool::new(false));
    let is_remote = Arc::new(AtomicBool::new(false));
    let last_x = Arc::new(Mutex::new((screen_width / 2) as f64));
    let last_y = Arc::new(Mutex::new((screen_height / 2) as f64));

    let state = CallbackState {
        tx: tx.clone(),
        enabled: enabled.clone(),
        screen_width: screen_width as f64,
        screen_height: screen_height as f64,
        is_warping: is_warping.clone(),
        last_x: last_x.clone(),
        last_y: last_y.clone(),
        is_remote: is_remote.clone(),
    };

    let state = Arc::new(Mutex::new(state));

    // Spawn rdev listener in a separate thread
    std::thread::spawn(move || {
        let callback_state = state.clone();

        let callback = move |event: rdev::Event| {
            let state = callback_state.lock().unwrap();

            // Check if we're warping (suppress the warp event itself)
            if state.is_warping.load(Ordering::Relaxed) {
                return;
            }

            // Check if processing is enabled
            if !state.enabled.load(Ordering::Relaxed) {
                return;
            }

            match event.event_type {
                EventType::MouseMove { x, y } => {
                    let mut last_x = state.last_x.lock().unwrap();
                    let mut last_y = state.last_y.lock().unwrap();

                    // Calculate delta from last position
                    let dx = x - *last_x;
                    let dy = y - *last_y;

                    // Check if cursor hit right edge
                    if x >= state.screen_width - 1.0 {
                        // Warp cursor to center
                        let center_x = state.screen_width / 2.0;
                        let center_y = state.screen_height / 2.0;

                        // Set warping flag to suppress the warp event
                        state.is_warping.store(true, Ordering::Relaxed);

                        // Warp cursor to center
                        if let Err(e) = rdev::simulate(&EventType::MouseMove {
                            x: center_x,
                            y: center_y,
                        }) {
                            tracing::warn!("Failed to warp cursor: {:?}", e);
                        }

                        // Update last position to center
                        *last_x = center_x;
                        *last_y = center_y;

                        // Clear warping flag after a small delay
                        let is_warping = state.is_warping.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                            is_warping.store(false, Ordering::Relaxed);
                        });

                        // Send delta (from last position to edge)
                        // The delta should be the movement that caused the cursor to hit the edge
                        let edge_dx = state.screen_width - 1.0 - *last_x;
                        let edge_dy = y - *last_y;

                        let _ = state.tx.try_send(WarpInputEvent::MouseDelta {
                            dx: edge_dx as f32,
                            dy: edge_dy as f32,
                        });

                        // Mark as remote
                        state.is_remote.store(true, Ordering::Relaxed);

                        tracing::debug!(
                            "Cursor warped: edge=({x:.0}, {y:.0}) center=({center_x:.0}, {center_y:.0}) delta=({edge_dx:.0}, {edge_dy:.0})"
                        );
                    } else if x <= 0.0 && state.is_remote.load(Ordering::Relaxed) {
                        // Cursor hit left edge while in remote mode - return to local
                        state.is_remote.store(false, Ordering::Relaxed);

                        // Send a special delta to indicate return to local
                        let _ = state.tx.try_send(WarpInputEvent::MouseDelta {
                            dx: -1.0, // Special value to indicate return
                            dy: 0.0,
                        });

                        tracing::debug!("Cursor returned to local screen");
                    } else {
                        // Normal movement - send delta
                        *last_x = x;
                        *last_y = y;

                        // Only send delta if we're in remote mode
                        if state.is_remote.load(Ordering::Relaxed) {
                            let _ = state.tx.try_send(WarpInputEvent::MouseDelta {
                                dx: dx as f32,
                                dy: dy as f32,
                            });
                        }
                    }
                }
                EventType::ButtonPress(btn) => {
                    if let Some(button) = map_button(btn) {
                        let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                            button,
                            pressed: true,
                        });
                    }
                }
                EventType::ButtonRelease(btn) => {
                    if let Some(button) = map_button(btn) {
                        let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                            button,
                            pressed: false,
                        });
                    }
                }
                EventType::Wheel { delta_x, delta_y } => {
                    let _ = state.tx.try_send(WarpInputEvent::Scroll {
                        dx: delta_x as f32,
                        dy: delta_y as f32,
                    });
                }
                EventType::KeyPress(key) => {
                    let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                        keycode: key_to_u32(key),
                        pressed: true,
                    });
                }
                EventType::KeyRelease(key) => {
                    let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                        keycode: key_to_u32(key),
                        pressed: false,
                    });
                }
            }
        };

        if let Err(e) = rdev::listen(callback) {
            tracing::error!("rdev::listen failed: {:?}", e);
        }
    });

    tracing::info!(
        "Started warp capture (screen: {screen_width}x{screen_height})"
    );

    Ok(WarpCaptureHandle {
        event_rx: rx,
        enabled,
        screen_width,
        screen_height,
    })
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

/// Convert rdev::Key to a u32 keycode
fn key_to_u32(key: rdev::Key) -> u32 {
    let name = format!("{:?}", key);
    let hash = blake3::hash(name.as_bytes());
    let bytes: [u8; 4] = hash.as_bytes()[..4].try_into().unwrap_or([0; 4]);
    u32::from_le_bytes(bytes)
}

/// Convert a WarpInputEvent to a protocol Message
pub fn to_message(event: &WarpInputEvent) -> ss_core::protocol::Message {
    match event {
        WarpInputEvent::MouseDelta { dx, dy } => {
            ss_core::protocol::Message::MouseDelta {
                dx: *dx,
                dy: *dy,
            }
        }
        WarpInputEvent::MouseButton { button, pressed } => {
            ss_core::protocol::Message::MouseButton {
                button: *button,
                pressed: *pressed,
            }
        }
        WarpInputEvent::KeyPress { keycode, pressed } => {
            ss_core::protocol::Message::KeyPress {
                keycode: *keycode,
                pressed: *pressed,
            }
        }
        WarpInputEvent::Scroll { dx, dy } => {
            ss_core::protocol::Message::MouseScroll {
                dx: *dx,
                dy: *dy,
            }
        }
    }
}
