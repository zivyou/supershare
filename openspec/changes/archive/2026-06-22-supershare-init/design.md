## Context

这是一个全新项目，目标是用 Rust 构建一个轻量级跨平台 KVM 共享工具。现有方案（Synergy/Barrier/Input Leap）基于 C/C++，内存占用较高且维护困难。项目需要支持 Ubuntu 和 Windows 11，核心功能为鼠标键盘共享和剪切板同步（含图片）。

## Goals / Non-Goals

**Goals:**
- 在多台异构机器之间共享鼠标键盘，支持水平屏幕布局
- 跨机器同步剪切板内容（文本 + 图片）
- 运行时内存占用低于 30 MB
- 使用 TLS + mTLS 保障通信安全
- 提供 egui 配置界面和系统托盘
- 单二进制部署，子命令切换角色

**Non-Goals:**
- 不支持 macOS（后续可扩展）
- 不支持 Wayland 下的输入注入（提示用户使用 X11）
- 不支持文件拖拽传输
- 不支持多显示器（单机多屏）
- 不支持垂直/网格屏幕布局（仅水平）

## Decisions

### D1: 拓扑模型 — Server-Client

**选择**: Server-Client 模式，一台机器作为 Server（主控），其他作为 Client。

**理由**: Server 维护全局逻辑坐标系，负责边界检测和事件路由，逻辑集中、实现简单。Peer-to-Peer 需要每台机器都知道全局状态，复杂度高且对初期场景无必要。

**替代方案**: Peer-to-Peer — 更灵活但复杂度高，适合后续版本考虑。

### D2: 双通道网络架构

**选择**: 控制通道（端口 9876）+ 数据通道（端口 9877）分离。

**理由**: 鼠标事件高频低延迟（每秒数百次），剪切板数据低频大体积（图片可达数 MB）。单通道会导致大图片传输阻塞鼠标事件，用户可感知 200ms+ 的卡顿。双通道确保输入事件始终低延迟。

**替代方案**: 单通道 + 优先级队列 — 实现简单但无法保证物理隔离，仍有延迟风险。

### D3: TLS + mTLS 双向认证

**选择**: 使用 rustls 实现 TLS，Server 和 Client 各持有证书，双向验证。

**理由**: 剪切板可能包含密码、token 等敏感数据，必须加密传输。mTLS 确保只有持有合法证书的设备才能连接，同时证书指纹可作为设备标识。

**证书管理**: 提供 `supershare gen-cert` 子命令一键生成自签名 CA 和设备证书，降低用户配置门槛。

**替代方案**: 共享密钥 — 实现简单但安全性弱，无法标识设备。

### D4: 输入捕获与注入 — rdev + uinput 备选

**选择**: 使用 `rdev` crate 进行跨平台输入捕获和注入。Linux 下额外支持 `uinput` 作为注入备选方案。

**理由**: `rdev` 是 Rust 生态中最成熟的跨平台输入库，封装了 Windows (Win32)、Linux (X11)、macOS (Quartz) 的原生 API。但其 Linux 注入能力在 Wayland 下受限，因此需要 uinput 作为备选。

**平台差异处理**:
- Windows: rdev 捕获 + 注入，需管理员权限（UAC）
- Linux X11: rdev 捕获 + 注入
- Linux Wayland: rdev 捕获，uinput 注入（需 root 或 input 组权限）

**替代方案**: `enigo` — 注入能力强但捕获弱；原生 API 直调 — 最可控但工作量大。

### D5: 剪切板监控 — arboard + 轮询

**选择**: 使用 `arboard` crate 进行剪切板读写，通过轮询检测变化（200ms 间隔）。

**理由**: Linux 上剪切板没有原生变更通知 API，只能轮询。Windows 有 `AddClipboardFormatListener` 但 arboard 未暴露，统一用轮询简化跨平台逻辑。200ms 间隔在体感和 CPU 占用之间取得平衡。

**图片处理流程**: arboard 读取 RGBA 像素 → blake3 hash → zstd 压缩 → 分片传输 → 接收端解压 → arboard 写入。

**替代方案**: 各平台原生 API 直调 — 工作量大且 arboard 已足够。

### D6: 协议序列化 — bincode

**选择**: 使用 `bincode` 进行事件序列化。

**理由**: 鼠标移动事件每秒可能数百次，bincode 比 JSON 快 10x+，体积也更小。协议格式为 `[Magic: 2B][Type: 1B][Length: 4B][Payload: variable]`，紧凑且易于解析。

**替代方案**: JSON — 人类可读但性能差；protobuf — 需要额外 schema 定义，对内部协议过度设计。

### D7: 图片压缩 — zstd

**选择**: 使用 zstd 压缩剪切板图片数据，压缩级别 3。

**理由**: zstd 在压缩速度和压缩比之间取得极佳平衡。级别 3 可在毫秒级完成数 MB 图片的压缩，压缩比约 3-5x。比 gzip 快 10x+，压缩比相当。

**替代方案**: lz4 — 更快但压缩比低；brotli — 压缩比更好但慢。

### D8: UI 框架 — egui (eframe)

**选择**: 使用 `egui` + `eframe` 作为配置界面框架，`tray-icon` + `muda` 实现系统托盘。

**理由**: egui 是即时模式 UI，编译后为单个二进制，无额外运行时依赖。内存占用约 5-10 MB，远低于 Tauri（50-80 MB）。配置界面逻辑简单（IP/端口/证书/布局设置），egui 完全胜任。

**系统托盘**: tray-icon 和 eframe 底层都基于 winit，集成良好。窗口关闭时最小化到托盘，托盘双击重新显示。

**替代方案**: Tauri — 生态好但重（WebView2）；Slint — 声明式但学习成本高。

### D9: 项目结构 — Cargo Workspace

**选择**: Cargo workspace，按职责拆分为 6 个 crate。

```
supershare/
├── crates/
│   ├── ss-core/        # 协议定义、事件类型、配置结构
│   ├── ss-input/       # 输入捕获/注入（rdev + uinput）
│   ├── ss-clipboard/   # 剪切板监控/同步
│   ├── ss-network/     # 网络通信（tokio + rustls）
│   └── ss-ui/          # egui 配置界面 + 系统托盘
├── src/main.rs         # 单二进制入口（clap 子命令）
└── Cargo.toml          # workspace 根
```

**理由**: 模块化便于独立测试和编译缓存。单二进制入口通过 clap 子命令切换 `server` / `client` / `gui` / `gen-cert` 模式。

## Risks / Trade-offs

| 风险 | 影响 | 缓解策略 |
|------|------|---------|
| rdev Wayland 注入失败 | Linux Wayland 用户无法使用 | uinput 作为备选注入方案；启动时检测 Wayland 并提示切换 X11 |
| Linux 输入捕获需 root | 用户体验差 | 提供 udev rules 配置脚本，允许 input 组用户无需 root |
| 大图片传输阻塞 | 已通过双通道解决 | 控制通道和数据通道物理隔离，输入事件不受影响 |
| 剪切板回环 | 无限同步循环 | suppression flag（写入后 1 秒忽略变化）+ blake3 hash 去重双重保护 |
| TLS 证书配置复杂 | 用户配置困难 | `gen-cert` 子命令一键生成，配置文件存储证书路径 |
| Windows UAC 弹窗 | 用户体验差 | 清单文件声明 `requireAdministrator`，或提供安装脚本自动提权 |
| arboard Linux 依赖 | 需要 X11/Wayland 开发库 | README 中列出依赖安装命令，构建时给出清晰错误提示 |
