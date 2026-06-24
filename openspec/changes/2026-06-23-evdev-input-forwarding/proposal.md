## Why

当前的输入共享架构存在根本性缺陷：当鼠标从 Server 切换到 Client 后，Server 端的物理鼠标停在屏幕边缘，`rdev::listen` 捕获到的绝对坐标被 X11 裁剪，delta 信息丢失。用户无法用 Server 的物理鼠标控制 Client 机器——这违背了 KVM 工具的核心目的。

根本原因：`rdev::listen` 基于 X11 XRecord，只提供被屏幕边界裁剪过的绝对坐标，无法获取原始鼠标移动量。而 `rdev::grab` 基于 evdev，虽然能拦截原始事件，但其回调仍然输出裁剪后的绝对坐标。

解决方案：直接使用 evdev 读取 `/dev/input/event*` 设备，获取原始 `EV_REL::REL_X/REL_Y` delta，绕过 X11 裁剪。同时用 evdev 的 grab 机制实现本地事件抑制，用 UInput 实现本地事件放行。

## What Changes

- **重写 ss-input 捕获层**：用 evdev 直接读取替换 `rdev::listen`，获取原始鼠标 delta
- **新增虚拟光标管理**：Server 端维护全局虚拟光标位置，基于 delta 累加，检测屏幕边界
- **协议扩展**：新增 `MouseDelta { dx, dy }` 消息类型，用于传输相对鼠标移动
- **简化 Client 端**：Client 变为纯被动模式，只接收事件并注入，不再捕获本地输入
- **简化边界检测**：由 Server 端基于虚拟光标位置统一判断，不再依赖 Client 端上报

## Capabilities

### Modified Capabilities

- `input-sharing`: 从 `rdev::listen` + 绝对坐标模式改为 evdev + delta 模式，Server 端完全控制输入路由

## Impact

- **改动范围**: ss-input（重写 capture 层）、ss-core（协议扩展）、main.rs（重构 server/client 逻辑）
- **新增依赖**: `evdev` (已在 Cargo.toml 中声明)、`epoll`、`inotify`（evdev 设备热插拔）
- **权限要求**: Server 端需要 `input` 组权限（已有 udev 规则），grab 模式需要对 `/dev/input/*` 有读写权限
- **平台兼容**: 当前仅 Linux。evdev 在 X11 和 Wayland 下都能工作
- **Breaking Change**: 协议新增 MouseDelta 消息类型，与旧版本不兼容
