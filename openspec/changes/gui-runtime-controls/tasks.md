## 1. Shared State & Command Types

- [x] 1.1 Create `SharedAppState` struct in ss-ui (server_running, server_port, connected_clients, client_connected, client_server_addr, server_screen_size)
- [x] 1.2 Create `ClientInfo` struct (name, connected_at)
- [x] 1.3 Create `AppCommand` enum (StartServer, StopServer, ConnectClient, DisconnectClient)
- [x] 1.4 Create `ServerEvent` enum in ss-network (ClientConnected, ClientDisconnected)

## 2. Server Runtime Integration

- [x] 2.1 Add `notify_tx: broadcast::Sender<ServerEvent>` to ServerState
- [x] 2.2 Emit ClientConnected event when a client completes handshake in server.rs
- [x] 2.3 Emit ClientDisconnected event when a client disconnects in server.rs
- [x] 2.4 Expose server start/stop as async functions that accept SharedAppState

## 3. Client Runtime Integration

- [x] 3.1 Expose client connect/disconnect as async functions that accept SharedAppState
- [x] 3.2 Update SharedAppState on successful connection (client_connected, client_server_addr, server_screen_size)
- [x] 3.3 Update SharedAppState on disconnection

## 4. Background Runtime & Command Handler

- [x] 4.1 Create tokio runtime on a dedicated background thread in run_gui()
- [x] 4.2 Implement command handler loop: receive AppCommand, spawn/abort server or client tasks
- [x] 4.3 Wire server task to listen for ServerEvent and update SharedAppState.connected_clients
- [x] 4.4 Implement graceful shutdown: on GUI exit, stop all tasks and drop runtime

## 5. GUI Server Tab

- [x] 5.1 Add "Start Server" / "Stop Server" toggle button based on server_running state
- [x] 5.2 Add server status display (Running on port X / Stopped)
- [x] 5.3 Replace static client config list with real-time connected_clients list
- [x] 5.4 Display "No clients connected" when list is empty

## 6. GUI Client Tab

- [x] 6.1 Add "Connect" / "Disconnect" toggle button based on client_connected state
- [x] 6.2 Add connection status display (Connected to X / Disconnected / Connection failed)
- [x] 6.3 Display Server screen resolution when connected

## 7. CLI Entry Point Refactor

- [x] 7.1 Change default behavior (no args) to open GUI
- [x] 7.2 Convert server/client subcommands to --server/--client flags
- [x] 7.3 Remove `gui` subcommand
- [x] 7.4 Update clap help text and descriptions

## 8. Cleanup

- [x] 8.1 Remove window close-to-tray behavior (exit on close)
- [x] 8.2 Remove tray-icon dependency and tray module (no longer needed)
- [x] 8.3 Update README to reflect new CLI behavior
