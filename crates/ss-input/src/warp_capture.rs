//! rdev-based input capture with cursor warping for delta calculation.
//!
//! This approach uses rdev::grab (evdev grab) for input capture and rdev::simulate (XTest)
//! for cursor warping. It doesn't require root permissions or udev rules.
//!
//! How it works:
//! 1. Grab input devices via rdev::grab (intercept events before X Server)
//! 2. When cursor hits right edge, warp it back to screen center
//! 3. Calculate delta from the real position before warp
//! 4. Forward delta to client
//! 5. In remote mode, suppress events from reaching X Server (return None)
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
    /// Cursor crossed into a client screen
    BoundaryEnter { enter_x: f32, enter_y: f32 },
}

/// Handle to the warp capture system.
pub struct WarpCaptureHandle {
    /// Receiver for captured input events.
    pub event_rx: mpsc::Receiver<WarpInputEvent>,
    /// Screen width for boundary detection.
    pub screen_width: u32,
    /// Screen height for boundary detection.
    pub screen_height: u32,
    /// Whether we're in remote mode (cursor on client screen).
    pub is_remote: Arc<AtomicBool>,
}

/// Shared state for the rdev callback.
struct CallbackState {
    tx: mpsc::Sender<WarpInputEvent>,
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
/// - `screen_width/height`: screen dimensions for boundary detection
/// - `is_remote`: set to true when cursor is on client screen
pub fn start_capture(
    screen_width: u32,
    screen_height: u32,
) -> anyhow::Result<WarpCaptureHandle> {
    let (tx, rx) = mpsc::channel::<WarpInputEvent>(256);
    let is_warping = Arc::new(AtomicBool::new(false));
    let is_remote = Arc::new(AtomicBool::new(false));
    let last_x = Arc::new(Mutex::new((screen_width / 2) as f64));
    let last_y = Arc::new(Mutex::new((screen_height / 2) as f64));

    let state = CallbackState {
        tx: tx.clone(),
        screen_width: screen_width as f64,
        screen_height: screen_height as f64,
        is_warping: is_warping.clone(),
        last_x: last_x.clone(),
        last_y: last_y.clone(),
        is_remote: is_remote.clone(),
    };

    let state = Arc::new(Mutex::new(state));

    // Spawn rdev grab listener in a separate thread
    std::thread::spawn(move || {
        let callback_state = state.clone();

        let callback = move |event: rdev::Event| -> Option<rdev::Event> {
            let state = callback_state.lock().unwrap();

            // Check if we're warping (suppress the warp event itself)
            if state.is_warping.load(Ordering::Relaxed) {
                return Some(event); // Pass through warp events
            }

            // In remote mode, suppress ALL events from reaching X Server
            if state.is_remote.load(Ordering::Relaxed) {
                match event.event_type {
                    EventType::MouseMove { x, y } => {
                        let mut last_x = state.last_x.lock().unwrap();
                        let mut last_y = state.last_y.lock().unwrap();

                        // Calculate delta from last position
                        let dx = x - *last_x;
                        let dy = y - *last_y;

                        // Check if cursor hit left edge (return to local)
                        if x <= 0.0 {
                            // Return to local mode
                            state.is_remote.store(false, Ordering::Relaxed);

                            // Send a special delta to indicate return to local
                            let _ = state.tx.try_send(WarpInputEvent::MouseDelta {
                                dx: -1.0, // Special value to indicate return
                                dy: 0.0,
                            });

                            tracing::debug!("Cursor returned to local screen");

                            // Update last position
                            *last_x = x;
                            *last_y = y;

                            // Suppress this event (don't let X Server see it)
                            return None;
                        }

                        // ==============================================
                        // Fix: Detect top/bottom/right edges in remote mode
                        // Warp silently and only send delta - no boundary events!
                        // ==============================================
                        let hit_top = y <= 0.0;
                        let hit_bottom = y >= state.screen_height - 1.0;
                        let hit_right = x >= state.screen_width - 1.0;

                        if hit_top || hit_bottom || hit_right {
                            // Calculate wrap-around position
                            let new_y = if hit_top {
                                state.screen_height - 2.0  // warp to just above bottom
                            } else if hit_bottom {
                                1.0  // warp to just below top
                            } else {
                                y  // Y direction unchanged
                            };

                            let new_x = if hit_right {
                                state.screen_width / 2.0  // warp to horizontal center
                            } else {
                                x  // X direction unchanged
                            };

                            // Set warping flag to suppress the warp event itself
                            state.is_warping.store(true, Ordering::Relaxed);

                            // Warp cursor to the new position
                            if let Err(e) = rdev::simulate(&EventType::MouseMove {
                                x: new_x,
                                y: new_y,
                            }) {
                                tracing::warn!("Failed to warp cursor in remote mode: {:?}", e);
                            }

                            // Update last position to the warped position
                            *last_x = new_x;
                            *last_y = new_y;

                            // Clear warping flag after a small delay
                            let is_warping = state.is_warping.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                                is_warping.store(false, Ordering::Relaxed);
                            });

                            // Send delta (use the original delta, NOT the warp delta!)
                            let _ = state.tx.try_send(WarpInputEvent::MouseDelta {
                                dx: dx as f32,
                                dy: dy as f32,
                            });

                            tracing::debug!(
                                "Remote mode warp: hit={}, pos=({:.0}, {:.0}) warp_to=({:.0}, {:.0})",
                                if hit_top { "top" } else if hit_bottom { "bottom" } else { "right" },
                                x, y, new_x, new_y
                            );

                            // Suppress this event from X Server
                            return None;
                        }

                        // Normal movement in remote mode
                        *last_x = x;
                        *last_y = y;

                        let _ = state.tx.try_send(WarpInputEvent::MouseDelta {
                            dx: dx as f32,
                            dy: dy as f32,
                        });

                        // Suppress event from X Server
                        return None;
                    }
                    EventType::ButtonPress(btn) => {
                        if let Some(button) = map_button(btn) {
                            let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                                button,
                                pressed: true,
                            });
                        }
                        // Suppress event from X Server
                        return None;
                    }
                    EventType::ButtonRelease(btn) => {
                        if let Some(button) = map_button(btn) {
                            let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                                button,
                                pressed: false,
                            });
                        }
                        // Suppress event from X Server
                        return None;
                    }
                    EventType::Wheel { delta_x, delta_y } => {
                        let _ = state.tx.try_send(WarpInputEvent::Scroll {
                            dx: delta_x as f32,
                            dy: delta_y as f32,
                        });
                        // Suppress event from X Server
                        return None;
                    }
                    EventType::KeyPress(key) => {
                        let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                            keycode: key_to_u32(key),
                            pressed: true,
                        });
                        // Suppress event from X Server
                        return None;
                    }
                    EventType::KeyRelease(key) => {
                        let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                            keycode: key_to_u32(key),
                            pressed: false,
                        });
                        // Suppress event from X Server
                        return None;
                    }
                }
            }

            // Local mode - process events normally
            match event.event_type {
                EventType::MouseMove { x, y } => {
                    let mut last_x = state.last_x.lock().unwrap();
                    let mut last_y = state.last_y.lock().unwrap();

                    // Calculate delta from last position
                    let _dx = x - *last_x;
                    let _dy = y - *last_y;

                    // Check if cursor hit right edge
                    if x >= state.screen_width - 1.0 {
                        // Warp cursor to center
                        let center_x = state.screen_width / 2.0;
                        let center_y = state.screen_height / 2.0;

                        // ==============================================
                        // Fix: Set is_remote FIRST, send BoundaryEnter FIRST
                        // Do NOT send edge_dx delta - BoundaryEnter sets the correct position
                        // ==============================================

                        // Mark as remote FIRST (before any event sending)
                        state.is_remote.store(true, Ordering::Relaxed);

                        // Send BoundaryEnter FIRST - this tells the client to position
                        // the cursor at the left edge
                        let _ = state.tx.try_send(WarpInputEvent::BoundaryEnter {
                            enter_x: 6.0,  // BOUNDARY_ZONE_PX + 1
                            enter_y: y as f32,
                        });

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

                        // Note: Do NOT send edge_dx MouseDelta!
                        // BoundaryEnter already sets the correct initial position.
                        // Sending delta would cause double-movement.

                        tracing::debug!(
                            "Cursor warped into remote mode: edge=({x:.0}, {y:.0}) center=({center_x:.0}, {center_y:.0})"
                        );

                        // Suppress this event from X Server
                        return None;
                    }

                    // Normal movement - update last position
                    *last_x = x;
                    *last_y = y;

                    // Pass through to X Server
                    Some(event)
                }
                EventType::ButtonPress(btn) => {
                    if let Some(button) = map_button(btn) {
                        let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                            button,
                            pressed: true,
                        });
                    }
                    // Pass through to X Server
                    Some(event)
                }
                EventType::ButtonRelease(btn) => {
                    if let Some(button) = map_button(btn) {
                        let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                            button,
                            pressed: false,
                        });
                    }
                    // Pass through to X Server
                    Some(event)
                }
                EventType::Wheel { delta_x, delta_y } => {
                    let _ = state.tx.try_send(WarpInputEvent::Scroll {
                        dx: delta_x as f32,
                        dy: delta_y as f32,
                    });
                    // Pass through to X Server
                    Some(event)
                }
                EventType::KeyPress(key) => {
                    let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                        keycode: key_to_u32(key),
                        pressed: true,
                    });
                    // Pass through to X Server
                    Some(event)
                }
                EventType::KeyRelease(key) => {
                    let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                        keycode: key_to_u32(key),
                        pressed: false,
                    });
                    // Pass through to X Server
                    Some(event)
                }
            }
        };

        if let Err(e) = rdev::grab(callback) {
            tracing::error!("rdev::grab failed: {:?}", e);
        }
    });

    tracing::info!(
        "Started warp capture (screen: {screen_width}x{screen_height})"
    );

    Ok(WarpCaptureHandle {
        event_rx: rx,
        screen_width,
        screen_height,
        is_remote,
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
        WarpInputEvent::BoundaryEnter { enter_x, enter_y } => {
            ss_core::protocol::Message::BoundaryEnter {
                enter_x: *enter_x,
                enter_y: *enter_y,
                target_screen: 1,
            }
        }
    }
}
