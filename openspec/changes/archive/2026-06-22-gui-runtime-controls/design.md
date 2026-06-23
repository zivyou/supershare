## Context

当前 SuperShare 的 GUI（egui）和后端（tokio runtime、Server/Client 逻辑）完全分离。GUI 只持有 `AppConfig`，无法感知运行时状态，也无法控制 Server/Client 的生命周期。用户需要通过手动在终端启动 `supershare server` 或 `supershare client` 来运行服务。

## Goals / Non-Goals

**Goals:**
- GUI 成为一体化控制面板，可直接启停 Server/Client
- GUI 实时显示连接状态和客户端列表
- `supershare` 无参数时默认打开 GUI
- 窗口关闭时彻底退出应用

**Non-Goals:**
- 不改变 CLI 无头模式的功能（`--server` / `--client` 保留）
- 不实现窗口最小化到托盘（按用户要求，关闭即退出）
- 不改变网络协议或核心功能逻辑

## Decisions

### D1: 共享状态模型 — Arc<RwLock<SharedAppState>>

**选择**: 使用 `Arc<RwLock<SharedAppState>>` 在 UI 线程和 tokio runtime 之间共享状态。

**理由**: egui 的 `update()` 是同步调用，每帧执行。需要一种线程安全的方式来在 UI 和后台 task 之间共享状态。`Arc<RwLock>` 是 Rust 中最标准的共享状态方案，读多写少场景下性能良好。

**SharedAppState 结构**:
```rust
pub struct SharedAppState {
    pub server_running: bool,
    pub server_port: Option<u16>,
    pub connected_clients: Vec<ClientInfo>,
    pub client_connected: bool,
    pub client_server_addr: Option<String>,
    pub server_screen_size: Option<(u32, u32)>,
}

pub struct ClientInfo {
    pub name: String,
    pub connected_at: Instant,
}
```

**替代方案**: `watch` channel — 单生产者多消费者，但每次更新需要替换整个状态，不如 RwLock 灵活。

### D2: 命令通道 — mpsc::Sender<AppCommand>

**选择**: 使用 `mpsc` channel 从 UI 发送控制命令到后台 command handler。

**理由**: egui 是同步的，不能直接 await 异步操作。通过 channel 发送命令，后台 task 接收并执行，是最干净的解耦方式。

**AppCommand 枚举**:
```rust
pub enum AppCommand {
    StartServer { config: ServerConfig },
    StopServer,
    ConnectClient { config: ClientConfig },
    DisconnectClient,
}
```

**替代方案**: 直接在 UI 线程 spawn tokio task — 可行但会导致 UI 线程与 runtime 耦合过紧。

### D3: CLI 入口重构

**选择**: `supershare` 无参数默认打开 GUI，`--server` / `--client` 作为无头模式。

**理由**: GUI 是主要使用场景，应作为默认行为。无头模式保留用于服务器环境和脚本。

**命令行设计**:
```
supershare                          # 打开 GUI（默认）
supershare --server --port 9876 ... # 无头 server 模式
supershare --client --server ...    # 无头 client 模式
supershare gen-cert --output ...    # 证书生成
```

移除 `gui`、`server`、`client` 子命令，改为顶层参数。

### D4: 后台 Runtime 生命周期

**选择**: 在 `run_gui()` 中创建 tokio runtime，命令 handler 作为第一个 spawn 的 task。

**理由**: egui 的 `run_native` 会阻塞主线程。需要在进入 egui 之前启动 runtime，或者在单独线程中运行 runtime。

**方案**: 在单独线程中运行 tokio runtime：
```
主线程: egui event loop
后台线程: tokio runtime
         ├── command handler loop
         ├── server task (on demand)
         └── client task (on demand)
```

### D5: Server 连接通知

**选择**: ServerState 增加 `notify_tx: broadcast::Sender<ServerEvent>`，客户端连接/断开时发送事件。

**理由**: UI 需要知道客户端何时连接/断开，以便更新显示。通过 broadcast channel，多个监听者可以独立接收事件。

**ServerEvent 枚举**:
```rust
pub enum ServerEvent {
    ClientConnected { name: String },
    ClientDisconnected { name: String },
}
```

## Risks / Trade-offs

| 风险 | 影响 | 缓解策略 |
|------|------|---------|
| egui 每帧读 RwLock | 理论上可能阻塞 | 实际上读锁竞争极低，UI 每帧 ~16ms，写入仅在连接事件时发生 |
| tokio runtime 在单独线程 | 调试复杂度增加 | 使用 tracing 结构化日志，runtime panic 时 UI 显示错误状态 |
| 命令 channel 满 | UI 发送阻塞 | 使用 `try_send` 非阻塞发送，失败时显示错误 |
| Server task panic | 状态不一致 | 使用 `tokio::spawn` + 错误回调更新 SharedAppState |
