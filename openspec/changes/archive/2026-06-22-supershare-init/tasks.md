## 1. Project Setup

- [x] 1.1 Create Cargo workspace with root Cargo.toml
- [x] 1.2 Create crate scaffolding: ss-core, ss-input, ss-clipboard, ss-network, ss-ui, and main binary
- [x] 1.3 Add all dependencies to respective Cargo.toml files (tokio, rdev, arboard, rustls, egui, tray-icon, zstd, blake3, bincode, clap, toml, tracing)
- [x] 1.4 Create CLI entry point with clap subcommands: server, client, gui, gen-cert

## 2. Core Protocol (ss-core)

- [x] 2.1 Define message frame format: Magic (0x5353), Type, Length, Payload
- [x] 2.2 Define all message types as enums: MouseMove, MouseButton, MouseScroll, KeyPress, ClipboardData, ClipboardBegin/Chunk/End, Handshake, Heartbeat, ScreenConfig, BoundaryEnter, BoundaryLeave
- [x] 2.3 Implement bincode serialization/deserialization for all message types
- [x] 2.4 Define configuration structs (ServerConfig, ClientConfig) with TOML serde support
- [x] 2.5 Implement config file load/save to platform-appropriate paths (~/.config/supershare/config.toml or %APPDATA%\supershare\config.toml)

## 3. Network Transport (ss-network)

- [x] 3.1 Implement TLS certificate loading and rustls server/client config builder
- [x] 3.2 Implement control channel server (TLS listener on configurable port, default 9876)
- [x] 3.3 Implement data channel server (TLS listener on configurable port, default 9877)
- [x] 3.4 Implement client connection logic (connect to server IP:port, TLS handshake)
- [x] 3.5 Implement mTLS: server verifies client cert, client verifies server cert, both against shared CA
- [x] 3.6 Implement application-level handshake: Client sends Handshake, Server responds with ScreenConfig
- [x] 3.7 Implement message framing: read/write framed messages over TLS streams
- [x] 3.8 Implement heartbeat: send every 5s, detect timeout at 15s
- [x] 3.9 Implement reconnection with exponential backoff (1s, 2s, 4s, max 30s)
- [x] 3.10 Implement dual-channel manager: pair control + data channels for each client

## 4. Input Sharing (ss-input)

- [x] 4.1 Implement input capture wrapper using rdev (listen for mouse move, button, scroll, key events)
- [x] 4.2 Implement input injection wrapper using rdev::simulate for Windows and Linux X11
- [x] 4.3 Implement uinput-based injection fallback for Linux Wayland
- [x] 4.4 Implement boundary detection: detect when mouse enters 5px edge zone
- [x] 4.5 Implement global coordinate system: map multiple screens horizontally (Screen A: [0, W_A), Screen B: [W_A, W_A+W_B), etc.)
- [x] 4.6 Implement BoundaryEnter/Leave logic: suppress local capture, forward events to target client
- [x] 4.7 Implement client-side mouse position reporting back to server
- [x] 4.8 Implement keyboard event routing: forward to active client when mouse is on client screen

## 5. Clipboard Sync (ss-clipboard)

- [x] 5.1 Implement clipboard polling monitor using arboard (200ms interval)
- [x] 5.2 Implement blake3 hashing of clipboard content for change detection
- [x] 5.3 Implement text clipboard read/write via arboard
- [x] 5.4 Implement image clipboard read as RGBA pixels via arboard
- [x] 5.4 Implement image clipboard write as RGBA pixels via arboard
- [x] 5.5 Implement zstd compression/decompression for image data (level 3)
- [x] 5.6 Implement chunked transfer: ClipboardBegin + ClipboardChunk (64KB) + ClipboardEnd
- [x] 5.7 Implement suppression flag: ignore local clipboard changes for 1s after remote write
- [x] 5.8 Implement hash-based deduplication: skip if local hash matches last received remote hash
- [x] 5.9 Implement image size limit enforcement (default 10MB)

## 6. Configuration UI (ss-ui)

- [x] 6.1 Create egui main window with tab layout: Server mode / Client mode
- [x] 6.2 Implement Server mode settings panel: port, cert/key paths, connected client list
- [x] 6.3 Implement Client mode settings panel: server address, cert/CA paths, device name
- [x] 6.4 Implement clipboard settings panel: enable/disable text sync, enable/disable image sync, max image size
- [x] 6.5 Implement file selection dialogs for certificate and key paths
- [x] 6.6 Implement system tray icon with tray-icon crate
- [x] 6.7 Implement tray right-click menu: Open Settings, Connection Status, Quit
- [x] 6.8 Implement window close-to-tray behavior (close hides window, tray double-click restores)
- [x] 6.9 Wire UI to config persistence: save/load TOML on settings change

## 7. Certificate Generation (CLI)

- [x] 7.1 Implement `gen-cert` subcommand: generate self-signed CA cert + key
- [x] 7.2 Implement device cert generation signed by the CA
- [x] 7.3 Save generated certs to specified output directory with clear naming

## 8. Integration & Wiring

- [x] 8.1 Wire `server` subcommand: start control+data listeners, accept clients, route input events
- [x] 8.2 Wire `client` subcommand: connect to server, start input capture/inject, start clipboard monitor
- [x] 8.3 Wire `gui` subcommand: launch egui window with tray icon, start background service on config apply
- [x] 8.4 Implement graceful shutdown: signal handling, cleanup on exit
- [x] 8.5 Add structured logging with tracing (configurable log level)

## 9. Platform-Specific Polish

- [x] 9.1 Windows: add application manifest for UAC elevation request
- [x] 9.2 Linux: add udev rules file for input group permissions
- [x] 9.3 Linux: detect Wayland vs X11 at runtime, warn user if Wayland injection may be limited
- [x] 9.4 Add README with build instructions, dependencies, and usage examples
