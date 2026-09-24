//! rdev-based input capture with delta-based cursor tracking.
//!
//! How it works:
//! 1. Grab input devices via rdev::grab (evdev grab) so events can be suppressed
//!    before they reach the X Server.
//! 2. rdev reports positions from its OWN internal accumulator, driven by raw
//!    evdev relative movements and clamped to the screen. XTest warps
//!    (rdev::simulate) do NOT go through evdev, so the accumulator never
//!    reflects warps. Therefore deltas MUST be computed from consecutive
//!    reported positions only, and no cursor warping is needed (or valid).
//! 3. Local mode: events pass through. When the logical cursor crosses the
//!    right edge AND at least one client is connected (has_client), enter
//!    remote mode and notify the client (BoundaryEnter). With no client,
//!    events pass through and the cursor just stops at the edge.
//! 4. Remote mode: ALL events are suppressed, so the real cursor stays frozen
//!    at the crossing point; deltas are forwarded to the client.
//! 5. The client decides when to return (its own left edge) and sends
//!    BoundaryLeave; the server then sets exit_remote to switch back. If the
//!    client disconnects while remote (has_client goes false), remote mode
//!    exits automatically — otherwise the frozen cursor would be stuck forever
//!    with no one left to send BoundaryLeave.

use rdev::EventType;
use ss_core::protocol::Button;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// A captured input event, before conversion to protocol message.
#[derive(Debug, Clone)]
pub enum WarpInputEvent {
    /// Mouse delta calculated from consecutive reported positions
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
    /// Receiver for captured input events (with deltas).
    pub event_rx: mpsc::Receiver<WarpInputEvent>,
    /// Screen width for boundary detection.
    pub screen_width: u32,
    /// Screen height for boundary detection.
    pub screen_height: u32,
    /// Whether we're in remote mode (cursor on client screen).
    pub is_remote: Arc<AtomicBool>,
    /// External signal to exit remote mode (set to true to exit).
    pub exit_remote: Arc<AtomicBool>,
    /// Whether at least one client is connected. Remote mode is only entered
    /// while this is true; if it goes false mid-session, remote mode exits
    /// automatically on the next event.
    pub has_client: Arc<AtomicBool>,
}

/// Shared state for the rdev callback.
struct CallbackState {
    tx: mpsc::Sender<WarpInputEvent>,
    screen_width: f64,
    screen_height: f64,
    /// Previous position reported by rdev (for delta extraction).
    prev_x: f64,
    prev_y: f64,
    /// Whether prev_x/prev_y have been initialized from a real event.
    prev_initialized: bool,
    /// Logical cursor position on the server screen, maintained by applying
    /// deltas ourselves. Stays in sync with the real cursor in local mode
    /// (same deltas, same clamping); in remote mode the real cursor is frozen
    /// at `frozen_*` while the logical position keeps tracking movement.
    pos_x: f64,
    pos_y: f64,
    /// Logical position where the cursor was frozen when entering remote mode.
    /// The real cursor does not move in remote mode (all events suppressed),
    /// so on exit we restore pos to this point and everything is back in sync.
    frozen_x: f64,
    frozen_y: f64,
    /// Whether we're in remote mode (cursor on client screen)
    is_remote: Arc<AtomicBool>,
    /// External signal to exit remote mode.
    exit_remote: Arc<AtomicBool>,
    /// Whether at least one client is connected.
    has_client: Arc<AtomicBool>,
}

/// Start capturing input events with delta-based boundary detection.
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
    let is_remote = Arc::new(AtomicBool::new(false));
    let exit_remote = Arc::new(AtomicBool::new(false));
    let has_client = Arc::new(AtomicBool::new(false));

    let state = CallbackState {
        tx: tx.clone(),
        screen_width: screen_width as f64,
        screen_height: screen_height as f64,
        prev_x: 0.0,
        prev_y: 0.0,
        prev_initialized: false,
        pos_x: (screen_width / 2) as f64,
        pos_y: (screen_height / 2) as f64,
        frozen_x: 0.0,
        frozen_y: 0.0,
        is_remote: is_remote.clone(),
        exit_remote: exit_remote.clone(),
        has_client: has_client.clone(),
    };

    let state = Arc::new(Mutex::new(state));

    // Spawn rdev grab listener in a separate thread
    std::thread::spawn(move || {
        let callback_state = state.clone();

        let callback = move |event: rdev::Event| -> Option<rdev::Event> {
            let mut state = callback_state.lock().unwrap();

            // Exit remote mode when requested externally, OR when the client
            // went away while remote (disconnect, or never connected). Without
            // the latter, the frozen cursor would be stuck forever: no client
            // means no BoundaryLeave will ever arrive.
            let exit_requested = state.exit_remote.swap(false, Ordering::Relaxed);
            let client_gone = state.is_remote.load(Ordering::Relaxed)
                && !state.has_client.load(Ordering::Relaxed);
            if (exit_requested || client_gone) && state.is_remote.swap(false, Ordering::Relaxed) {
                // The real cursor was frozen at the crossing point the whole
                // time we were in remote mode (all events suppressed), so
                // restoring the logical position to the frozen point keeps it
                // in sync with the real cursor.
                state.pos_x = state.frozen_x;
                state.pos_y = state.frozen_y;
                tracing::info!(
                    "Exiting remote mode, cursor restored to ({:.0}, {:.0})",
                    state.frozen_x,
                    state.frozen_y
                );
            }

            let is_remote = state.is_remote.load(Ordering::Relaxed);

            match event.event_type {
                EventType::MouseMove { x, y } => {
                    // Delta = difference between consecutive reported positions.
                    // This is exact even when rdev's accumulator clamps at a
                    // screen edge: clamping only drops overshoot, it never
                    // invents movement, and reversal responds immediately.
                    let (dx, dy) = if state.prev_initialized {
                        (x - state.prev_x, y - state.prev_y)
                    } else {
                        state.prev_initialized = true;
                        // Sync logical position with rdev's accumulator (which
                        // rdev initialized from the real cursor position).
                        state.pos_x = x;
                        state.pos_y = y;
                        (0.0, 0.0)
                    };
                    state.prev_x = x;
                    state.prev_y = y;

                    state.pos_x = (state.pos_x + dx).clamp(0.0, state.screen_width);
                    state.pos_y = (state.pos_y + dy).clamp(0.0, state.screen_height);

                    if is_remote {
                        // Forward the delta to the client (skip pure-noise zeros).
                        if dx != 0.0 || dy != 0.0 {
                            let _ = state.tx.try_send(WarpInputEvent::MouseDelta {
                                dx: dx as f32,
                                dy: dy as f32,
                            });
                        }
                        // Suppress: the real cursor stays frozen in remote mode.
                        return None;
                    }

                    // Local mode: cross into remote mode at the right edge,
                    // but only when a client is connected. With no client,
                    // entering remote mode would freeze the real cursor with
                    // no one able to bring it back — pass through instead and
                    // let the cursor stop at the edge like a normal screen.
                    if state.pos_x >= state.screen_width - 1.0
                        && state.has_client.load(Ordering::Relaxed)
                    {
                        state.is_remote.store(true, Ordering::Relaxed);
                        state.frozen_x = state.pos_x;
                        state.frozen_y = state.pos_y;

                        // Tell the client to position its cursor just past its
                        // left boundary zone (BOUNDARY_ZONE_PX + 1).
                        let _ = state.tx.try_send(WarpInputEvent::BoundaryEnter {
                            enter_x: 6.0,
                            enter_y: state.pos_y as f32,
                        });

                        tracing::info!(
                            "Entering remote mode at ({:.0}, {:.0})",
                            state.pos_x,
                            state.pos_y
                        );

                        // Suppress this event from X Server
                        return None;
                    }

                    // Normal local movement - pass through to X Server
                    Some(event)
                }
                EventType::ButtonPress(btn) => {
                    if is_remote {
                        if let Some(button) = map_button(btn) {
                            let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                                button,
                                pressed: true,
                            });
                        }
                        // Suppress event from X Server
                        return None;
                    }
                    // Local mode: button events belong to the local machine.
                    // Do NOT emit them to the forwarding channel — the only
                    // consumer forwards everything it receives to clients.
                    Some(event)
                }
                EventType::ButtonRelease(btn) => {
                    if is_remote {
                        if let Some(button) = map_button(btn) {
                            let _ = state.tx.try_send(WarpInputEvent::MouseButton {
                                button,
                                pressed: false,
                            });
                        }
                        return None;
                    }
                    Some(event)
                }
                EventType::Wheel { delta_x, delta_y } => {
                    if is_remote {
                        let _ = state.tx.try_send(WarpInputEvent::Scroll {
                            dx: delta_x as f32,
                            dy: delta_y as f32,
                        });
                        return None;
                    }
                    Some(event)
                }
                EventType::KeyPress(key) => {
                    if is_remote {
                        let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                            keycode: key_to_u32(key),
                            pressed: true,
                        });
                        return None;
                    }
                    Some(event)
                }
                EventType::KeyRelease(key) => {
                    if is_remote {
                        let _ = state.tx.try_send(WarpInputEvent::KeyPress {
                            keycode: key_to_u32(key),
                            pressed: false,
                        });
                        return None;
                    }
                    Some(event)
                }
            }
        };

        if let Err(e) = rdev::grab(callback) {
            tracing::error!("rdev::grab failed: {:?}", e);
        }
    });

    tracing::info!(
        "Started input capture (screen: {screen_width}x{screen_height})"
    );

    Ok(WarpCaptureHandle {
        event_rx: rx,
        screen_width,
        screen_height,
        is_remote,
        exit_remote,
        has_client,
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
