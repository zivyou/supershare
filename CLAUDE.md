# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                          # debug build
cargo build --release                # release build (preferred for testing — rdev perf matters)
cargo test                           # run all tests
cargo test -p ss-input               # run tests for a specific crate
cargo fix --bin supershare --allow-dirty  # auto-fix warnings in main binary
```

No linter config; use `cargo clippy` if available. No CI config found.

## Architecture

**Single binary** (`supershare`) with 4 modes: GUI (default), `--server`, `--client`, `--gen-cert`.

### Workspace Crates

- **ss-core** — Protocol types (`Message` enum), frame format (`[Magic:2B][Type:1B][Length:4B][Payload]`), config structs (`AppConfig`), constants (ports, boundary zone, timeouts). No async, no I/O.
- **ss-input** — `capture::start_capture(suppressed)` spawns an rdev listener thread, returns `mpsc::Receiver<InputEvent>`. `inject::inject_event(msg)` calls `rdev::simulate`. `boundary::CoordinateSystem` manages horizontal screen layout and boundary detection.
- **ss-clipboard** — `monitor::ClipboardMonitor` polls clipboard via arboard (200ms). `sync::prepare_transfer` compresses images with zstd, chunks them. `sync::handle_clipboard_message` reassembles chunks.
- **ss-network** — `server::start()` listens on two ports (control + data). `client::connect()` establishes dual TLS channels. All networking is tokio async with rustls TLS.
- **ss-ui** — egui immediate-mode GUI. `SharedAppState` (std::sync::RwLock) for UI↔backend state. `AppCommand` (mpsc) for UI→backend commands.

### Key Data Flow

**Server input → Client injection:**
1. Server `start_capture(suppressed=false)` → rdev events
2. Boundary check: if mouse at right edge → send `BoundaryEnter` to client via control_tx, set suppressed=true
3. While on client screen: forward input events via client's data_tx
4. Client receives on data_rx → `inject_event()` via rdev::simulate
5. Client detects left edge → sends `BoundaryLeave` → server unsuppresses

**Clipboard sync (bidirectional):**
1. `ClipboardMonitor::poll()` detects change → `prepare_transfer()` → send via data channel
2. Peer receives → `handle_clipboard_message()` → write to local clipboard
3. `suppress(hash)` prevents echo loop for 1 second

### GUI↔Backend Communication

- GUI runs on **main thread** (required by winit/eframe)
- Tokio runtime runs on a **background std::thread**
- `SharedState = Arc<std::sync::RwLock<SharedAppState>>` — GUI reads every frame (~16ms)
- `AppCommand` via `mpsc::channel` — GUI sends commands, runtime executes
- **Never hold RwLock across .await** — use block scopes to drop guards before async ops

### Trust establishment: pairing (default) and manual certs (advanced)

**Pairing (default, recommended).** The Server is its own CA: on first run it auto-generates `ca.pem`/`server.pem` under the config dir's `trust/` folder — no manual `gen-cert` needed. A Client connects with only the Server IP; it then pairs:

1. Server displays a 6-digit PIN (GUI) or prints it (`--server`, unless `--no-pairing`).
2. Client enters the PIN (GUI prompt, or `--client --connect <ip> --pair` reads it from stdin).
3. SPAKE2 (PAKE) over a dedicated pairing port (default `control_port − 1`) authenticates the channel with the PIN — a LAN MITM without the PIN cannot intercept. The Server signs the Client's CSR and returns the cert + CA, AEAD-encrypted.
4. Both persist trust (`ServerConfig.paired_clients`, `ClientConfig.known_servers`); reconnects need no PIN and reuse the existing mTLS path unchanged.

Pairing code: `ss-network/src/pairing/{crypto,server,client}.rs`; CA ops in `ss-network/src/cert.rs`; PIN lifecycle (6-digit, TTL rotation, rotate-after-success, lockout) in `pairing::server::PinManager`.

**Manual certs (advanced).** Still supported and take precedence when explicit cert paths are set:

1. Generate CA: `supershare --gen-cert --output ./certs`
2. Generate device certs signed by CA: `supershare --gen-cert --device name --ca-cert ca.pem --ca-key ca-key.pem --ip <server-ip>`
3. Pass `--cert/--key/--ca` (all three) to skip pairing entirely.

## Critical Design Constraints

- **Screen resolution is detected at runtime** via `xrandr` (Linux) or `wmic` (Windows). Never hardcode resolution values — the boundary detection depends on accurate screen dimensions.
- **`std::sync::RwLock`** (not tokio::sync::RwLock) for SharedAppState — GUI thread has no tokio runtime. The "no reactor running" panic occurs if tokio sync primitives are used on the GUI thread.
- **Single TLS writer per connection** — control channel messages (heartbeats + BoundaryEnter/etc) are funneled through one mpsc channel to one writer task. Multiple writers corrupt the TLS stream.
- **broadcast channel pitfalls** — each subscriber independently consumes messages. Two tasks subscribing to the same broadcast channel will steal messages from each other. Use mpsc for point-to-point control messages.
- **Boundary enter_x must be BOUNDARY_ZONE_PX + 1** — entering at x=0.0 falls inside the boundary zone (x<=5), causing immediate bounce-back.

## Platform Notes

- Linux input capture requires either root or `input` group membership (udev rules in `assets/99-superShare.rules`)
- Wayland: input injection via rdev is limited; uinput fallback is a stub
- Windows: requires admin for global input capture

## Known Bug History

开发过程中踩过的坑详见 [`docs/bugs-and-fixes.md`](docs/bugs-and-fixes.md)。以下是最关键的几个：

| Bug | 一句话根因 |
|-----|-----------|
| GUI 连接无反馈 | `connect_with_retry` 无限重试不报错 |
| 边界检测不到 | 屏幕分辨率硬编码 1920x1080 |
| 鼠标切换后弹回 | `enter_x=0.0` 在边界区内 |
| BoundaryEnter 丢失 | broadcast 两订阅者竞争 + TLS writer 被 heartbeat 独占 |
| channel closed | broadcast 业务消息误触发 shutdown |
