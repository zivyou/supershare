## Why

当前 GUI 仅是静态配置界面，与运行时完全分离。用户无法在 GUI 中启停 Server/Client，也无法看到实时连接状态。需要将 GUI 改造为一体化控制面板，使其成为应用的默认入口。

## What Changes

- **BREAKING**: `supershare` 无参数时默认打开 GUI（而非显示帮助）
- `--server` / `--client` 作为无头模式 CLI 参数保留
- 移除 `gui` 子命令（GUI 成为默认行为）
- Server tab 增加"启动/停止 Server"按钮和运行状态显示
- Server tab 显示实时已连接客户端列表（名称、IP、分辨率）
- Client tab 增加"连接/断开"按钮和连接状态显示
- Client tab 显示当前连接的 Server 地址和 Server 屏幕分辨率
- 窗口关闭时彻底退出应用（不最小化到托盘）
- 新增 `SharedAppState` 在 UI 和后台 tokio runtime 之间共享状态
- 新增 `AppCommand` 通道从 UI 发送控制命令到后台

## Capabilities

### New Capabilities

- `gui-runtime-controls`: GUI 实时状态显示和启停控制——SharedAppState 共享状态、AppCommand 命令通道、Server/Client 生命周期管理、实时客户端列表

### Modified Capabilities

- `config-ui`: CLI 入口行为变更——默认打开 GUI，移除 gui 子命令，新增 --server/--client 无头模式参数

## Impact

- **src/main.rs**: CLI 入口重构，命令行参数变更
- **crates/ss-ui/src/app.rs**: UI 重构，增加状态显示和控制按钮
- **crates/ss-ui/src/lib.rs**: 新增 SharedAppState、AppCommand 类型
- **crates/ss-network/src/server.rs**: ServerState 增加连接/断开通知
- **crates/ss-network/src/client.rs**: 连接状态通知
- **用户体验**: GUI 成为一体化控制面板，无需手动管理 CLI 进程
