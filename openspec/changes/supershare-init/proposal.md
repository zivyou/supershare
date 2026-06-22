## Why

跨平台 KVM 共享工具（如 Synergy、Barrier）大多基于 C/C++，存在内存占用高、维护困难等问题。需要用 Rust 构建一个轻量、安全的替代方案，支持在多台异构机器（Ubuntu / Windows 11）之间共享鼠标键盘和剪切板（含图片），以提升多机协作效率。

## What Changes

- 新建 Rust workspace 项目，包含核心协议、输入捕获/注入、剪切板同步、网络通信、UI 配置界面等模块
- 实现 Server-Client 架构，支持水平屏幕布局
- 实现 TLS + mTLS 双向证书认证的双通道网络通信（控制通道 + 数据通道）
- 实现鼠标键盘事件的跨机器捕获与注入
- 实现剪切板文本与图片的跨机器同步（zstd 压缩 + blake3 hash 去重）
- 实现 egui 配置界面 + 系统托盘（tray-icon）
- 单二进制部署，通过子命令切换 Server / Client / GUI 模式

## Capabilities

### New Capabilities

- `input-sharing`: 跨机器鼠标键盘共享——输入事件捕获、网络传输、远程注入、屏幕边界检测与切换
- `clipboard-sync`: 跨机器剪切板同步——文本与图片内容的变更检测、压缩传输、防回环写入
- `network-transport`: TLS 双通道网络传输——控制通道（高频低延迟事件）+ 数据通道（大容量剪切板数据），mTLS 认证，心跳与重连
- `config-ui`: 配置界面与系统托盘——egui 配置窗口（Server/Client 模式切换、客户端管理、证书配置、剪切板选项）+ 系统托盘图标与菜单

### Modified Capabilities

（无，这是一个全新项目）

## Impact

- **新增代码**: 整个项目为新建，预计 6 个 crate（ss-core, ss-input, ss-clipboard, ss-network, ss-ui, ss-server/ss-client 入口）
- **外部依赖**: tokio, rdev, arboard, rustls, egui/eframe, tray-icon, zstd, blake3, bincode, clap
- **系统要求**: Linux（X11 优先，Wayland 降级支持）需 root 或 uinput 权限；Windows 需管理员权限（UAC）
- **网络**: 使用 TLS 端口（默认 9876 控制 + 9877 数据），需防火墙放行
