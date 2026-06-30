# SuperShare

<p align="center">
  <strong>[English](#english) | [中文](#中文)</strong>
</p>

---

<a name="english"></a>

## English

A lightweight, cross-platform keyboard, mouse, and clipboard sharing tool written in Rust.

Share a single keyboard and mouse across multiple machines on the same network, with seamless clipboard synchronization (including images).

### Features

- **Mouse & Keyboard Sharing** — Move your mouse cursor across screens seamlessly, as if using a single machine
- **Clipboard Sync** — Copy text or images on one machine, paste on another
- **Low Memory** — ~10-15 MB idle, ~25-30 MB during image transfer
- **Secure** — TLS 1.3 with mutual authentication (mTLS)
- **Cross-Platform** — Supports Ubuntu (X11) and Windows 11

### Requirements

#### Linux (Ubuntu)

```bash
# Install build dependencies
sudo apt update
sudo apt install -y build-essential pkg-config libx11-dev libxcb1-dev libxdo-dev libxtst-dev

# For input device access without root:
sudo cp assets/99-superShare.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
sudo usermod -aG input $USER
# Log out and back in for group changes to take effect
```

**Note:** Input injection on Wayland is limited. For full functionality, use X11.

#### Windows 11

- No additional dependencies required
- Requires administrator privileges for global input capture (UAC prompt)

### Installation

#### From source

```bash
# Clone the repository
git clone <repo-url>
cd supershare

# Build
cargo build --release

# Binary will be at target/release/supershare
```

### Quick Start

#### 1. Launch SuperShare

Simply run `supershare` to open the GUI:

```bash
supershare
```

The GUI provides:
- **Server tab**: Start/stop the server and enable pairing; the current pairing **PIN** is shown while running. See connected clients in real-time.
- **Client tab**: Enter just the server's IP and connect. The first time, you'll be prompted for the PIN shown on the server.
- **Clipboard tab**: Configure clipboard sync settings.

No certificate files to copy — the server is its own CA and provisions the client automatically during pairing. Once paired, reconnecting needs no PIN.

#### 2. Pairing (how trust is established)

1. On the **server**, enable pairing — it displays a 6-digit PIN.
2. On the **client**, enter the server IP and the PIN, then connect.
3. The two negotiate trust over a PIN-authenticated (SPAKE2) channel; the server signs a certificate for the client. A man-in-the-middle without the PIN cannot intercept this.
4. Trust is saved on both sides; future connections are silent.

#### 3. Headless Mode (Optional)

For servers or scripted environments, use headless mode. Certificates are optional — omit them to auto-generate/pair:

```bash
# Headless server (auto-generates CA, prints the pairing PIN)
supershare --server --port 9876

# Headless client — first time: pair with a PIN (read from stdin)
supershare --client --connect 192.168.1.100 --pair --name my-pc

# Headless client — afterwards: reconnect silently using saved trust
supershare --client --connect 192.168.1.100 --name my-pc
```

#### Advanced: manual certificates

For manual/advanced deployments you can still generate and supply certificates explicitly (this bypasses pairing):

```bash
# Generate CA certificate
supershare --gen-cert --output ./certs

# Generate device certificates (run for each machine)
supershare --gen-cert --device machine1 --ca-cert certs/ca.pem --ca-key certs/ca-key.pem --output ./certs

# Use them (all three paths required; --no-pairing disables the PIN listener)
supershare --server --port 9876 --no-pairing --cert certs/server.pem --key certs/server-key.pem --ca certs/ca.pem
supershare --client --connect 192.168.1.100:9876 --cert certs/machine1.pem --key certs/machine1-key.pem --ca certs/ca.pem --name my-pc
```

### Usage

Once connected:
- Move your mouse to the right edge of the server's screen to switch to the client machine
- Move your mouse to the left edge of the client's screen to return to the server
- Copy text or images on either machine — the clipboard is synced automatically

### Architecture

```
┌─────────────────────────────────────────────────────┐
│  Machine A (Server)         Machine B (Client)      │
│  ┌───────────────┐         ┌───────────────┐        │
│  │ Input Capture  │         │ Input Inject   │        │
│  │ (rdev)         │         │ (rdev/uinput)  │        │
│  ├───────────────┤         ├───────────────┤        │
│  │ Boundary       │         │ Boundary       │        │
│  │ Detection      │         │ Detection      │        │
│  ├───────────────┤         ├───────────────┤        │
│  │ Clipboard      │         │ Clipboard      │        │
│  │ Monitor        │         │ Monitor        │        │
│  └───────┬───────┘         └───────┬───────┘        │
│          │                         │                 │
│  ┌───────┴───────┐         ┌───────┴───────┐        │
│  │ Control Ch.    │◄─TLS───│ Control Ch.    │        │
│  │ (port 9876)    │        │ (port 9876)    │        │
│  │ Data Ch.       │◄─TLS───│ Data Ch.       │        │
│  │ (port 9877)    │        │ (port 9877)    │        │
│  └───────────────┘         └───────────────┘        │
└─────────────────────────────────────────────────────┘
```

### Project Structure

```
supershare/
├── Cargo.toml              # Workspace root
├── src/
│   ├── main.rs             # CLI entry point (server/client/gui/gen-cert)
│   └── certgen.rs          # Certificate generation
├── crates/
│   ├── ss-core/            # Protocol types, config, serialization
│   ├── ss-input/           # Input capture & injection (rdev)
│   ├── ss-clipboard/       # Clipboard monitoring & sync
│   ├── ss-network/         # TLS networking (tokio + rustls)
│   └── ss-ui/              # egui configuration UI
└── assets/
    ├── supershare.exe.manifest  # Windows UAC manifest
    └── 99-superShare.rules      # Linux udev rules
```

### Configuration

Configuration file is stored at:
- **Linux:** `~/.config/supershare/config.toml`
- **Windows:** `%APPDATA%\supershare\config.toml`

Example configuration:

```toml
[server]
control_port = 9876
data_port = 9877
cert_path = "/path/to/cert.pem"
key_path = "/path/to/key.pem"

[[server.clients]]
name = "laptop"
ip = "192.168.1.101"
screen_width = 1920
screen_height = 1080
position = "right"

[client]
device_name = "desktop"
server_address = "192.168.1.100:9876"
cert_path = "/path/to/cert.pem"
key_path = "/path/to/key.pem"
ca_path = "/path/to/ca.pem"

[clipboard]
text_enabled = true
image_enabled = true
max_image_size = 10485760  # 10 MB
```

### License

MIT

---

<a name="中文"></a>

## 中文

一个用 Rust 编写的轻量级、跨平台键盘鼠标和剪切板共享工具。

在同一网络的多台机器之间共享键盘和鼠标，并实现无缝的剪切板同步（包括图片）。

### 功能特性

- **键鼠共享** — 鼠标光标在多台机器的屏幕之间无缝切换，如同使用单台机器
- **剪切板同步** — 在一台机器上复制文本或图片，在另一台机器上粘贴
- **低内存占用** — 空闲时约 10-15 MB，传输图片时约 25-30 MB
- **安全可靠** — TLS 1.3 加密，支持双向认证（mTLS）
- **跨平台** — 支持 Ubuntu（X11）和 Windows 11

### 系统要求

#### Linux (Ubuntu)

```bash
# 安装构建依赖
sudo apt update
sudo apt install -y build-essential pkg-config libx11-dev libxcb1-dev libxdo-dev libxtst-dev

# 无需 root 权限即可访问输入设备：
sudo cp assets/99-superShare.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
sudo usermod -aG input $USER
# 注销并重新登录以使组更改生效
```

**注意：** 在 Wayland 下输入注入功能受限。如需完整功能，请使用 X11。

#### Windows 11

- 无需额外依赖
- 全局输入捕获需要管理员权限（UAC 弹窗）

### 安装

#### 从源码构建

```bash
# 克隆仓库
git clone <repo-url>
cd supershare

# 构建
cargo build --release

# 二进制文件位于 target/release/supershare
```

### 快速开始

#### 1. 生成证书

在任意一台机器上生成 CA 和设备证书：

```bash
# 生成 CA 证书
supershare --gen-cert --output ./certs

# 为每台机器生成设备证书
supershare --gen-cert --device machine1 --output ./certs
supershare --gen-cert --device machine2 --output ./certs
```

将证书分发到各台机器：
- 每台机器需要：`ca.pem`、自己的 `<name>.pem` 和 `<name>-key.pem`

#### 2. 启动 SuperShare

直接运行 `supershare` 打开图形界面：

```bash
supershare
```

图形界面提供：
- **Server 标签页**: 配置并启动/停止服务端，实时查看已连接客户端
- **Client 标签页**: 配置并连接/断开服务端
- **Clipboard 标签页**: 配置剪切板同步设置

#### 3. 无头模式（可选）

在服务器或脚本环境中使用无头模式：

```bash
# 无头服务端
supershare --server --port 9876 --cert certs/server.pem --key certs/server-key.pem --ca certs/ca.pem

# 无头客户端
supershare --client --connect 192.168.1.100:9876 --cert certs/client.pem --key certs/client-key.pem --ca certs/ca.pem --name my-pc
```

### 使用方法

连接成功后：
- 将鼠标移动到服务端屏幕的右边缘，切换到客户端机器
- 将鼠标移动到客户端屏幕的左边缘，返回服务端机器
- 在任意一台机器上复制文本或图片，剪切板会自动同步

### 架构设计

```
┌─────────────────────────────────────────────────────┐
│  机器 A (服务端)              机器 B (客户端)         │
│  ┌───────────────┐         ┌───────────────┐        │
│  │ 输入捕获       │         │ 输入注入       │        │
│  │ (rdev)         │         │ (rdev/uinput)  │        │
│  ├───────────────┤         ├───────────────┤        │
│  │ 边界检测       │         │ 边界检测       │        │
│  ├───────────────┤         ├───────────────┤        │
│  │ 剪切板监控     │         │ 剪切板监控     │        │
│  └───────┬───────┘         └───────┬───────┘        │
│          │                         │                 │
│  ┌───────┴───────┐         ┌───────┴───────┐        │
│  │ 控制通道       │◄─TLS───│ 控制通道       │        │
│  │ (端口 9876)    │        │ (端口 9876)    │        │
│  │ 数据通道       │◄─TLS───│ 数据通道       │        │
│  │ (端口 9877)    │        │ (端口 9877)    │        │
│  └───────────────┘         └───────────────┘        │
└─────────────────────────────────────────────────────┘
```

### 项目结构

```
supershare/
├── Cargo.toml              # 工作区根目录
├── src/
│   ├── main.rs             # CLI 入口 (server/client/gui/gen-cert)
│   └── certgen.rs          # 证书生成
├── crates/
│   ├── ss-core/            # 协议类型、配置、序列化
│   ├── ss-input/           # 输入捕获与注入 (rdev)
│   ├── ss-clipboard/       # 剪切板监控与同步
│   ├── ss-network/         # TLS 网络通信 (tokio + rustls)
│   └── ss-ui/              # egui 配置界面
└── assets/
    ├── supershare.exe.manifest  # Windows UAC 清单
    └── 99-superShare.rules      # Linux udev 规则
```

### 配置文件

配置文件存储位置：
- **Linux:** `~/.config/supershare/config.toml`
- **Windows:** `%APPDATA%\supershare\config.toml`

配置示例：

```toml
[server]
control_port = 9876
data_port = 9877
cert_path = "/path/to/cert.pem"
key_path = "/path/to/key.pem"

[[server.clients]]
name = "laptop"
ip = "192.168.1.101"
screen_width = 1920
screen_height = 1080
position = "right"

[client]
device_name = "desktop"
server_address = "192.168.1.100:9876"
cert_path = "/path/to/cert.pem"
key_path = "/path/to/key.pem"
ca_path = "/path/to/ca.pem"

[clipboard]
text_enabled = true
image_enabled = true
max_image_size = 10485760  # 10 MB
```

### 许可证

MIT
