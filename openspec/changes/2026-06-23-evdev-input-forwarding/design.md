## Context

当前 SuperShare 的输入共享使用 `rdev::listen`（X11 XRecord）捕获鼠标事件，输出绝对坐标。当鼠标到达屏幕边缘时，X11 将坐标裁剪到屏幕边界，delta 信息丢失。这导致用户无法用 Server 的物理鼠标控制 Client 机器。

rdev 提供了 `grab` API（基于 evdev），可以拦截原始输入事件。但其回调仍然输出裁剪后的绝对坐标，无法获取原始 REL_X/REL_Y delta。

因此需要直接使用 evdev 读取输入设备，获取原始鼠标 delta。

## Goals / Non-Goals

**Goals:**
- Server 端物理鼠标可以无缝控制 Client 机器（像 Synergy/Barrier 一样）
- 精确的鼠标 delta 传递，不受屏幕边界裁剪
- Server 端完全控制输入路由，Client 端纯被动
- 兼容 X11 和 Wayland（evdev 层面）
- 架构支持未来扩展到多 Client

**Non-Goals:**
- Windows/macOS 支持（本期仅 Linux）
- 多 Client 同时连接（架构预留，首期只实现单 Client）
- 鼠标加速/灵敏度同步
- 剪切板功能变更

## Decisions

### 1. 使用 evdev 直接读取输入设备

**选择**: 直接操作 `/dev/input/event*` 设备，使用 `evdev` crate 读取原始事件

**替代方案**:
- `rdev::grab` + 光标回弹：hacky，有竞态风险
- fork rdev 修改 grab 回调：维护成本高

**理由**: evdev 提供原始 REL_X/REL_Y delta，不受 X11 裁剪。evdev 在 X11 和 Wayland 下都能工作。

### 2. grab + UInput 本地放行机制

**选择**: grab 物理设备，创建 UInput 副本设备。本地模式下将事件写入 UInput（放行），远程模式下抑制（不写入）。

**参考**: rdev 的 `grab.rs` 已实现此模式（`setup_devices()` 创建 UInput 副本，`filter_map_events()` 决定放行或抑制）。

### 3. 协议新增 MouseDelta 消息

**选择**: 新增 `MouseDelta { dx: f32, dy: f32 }` 消息类型，用于传输相对鼠标移动。

**保留**: `MouseMove { x, y }` 仅用于边界切换时的初始定位（BoundaryEnter 携带进入坐标）。

### 4. Server 端虚拟光标管理

**选择**: Server 维护全局虚拟光标 `(global_x, global_y)`，基于 evdev delta 累加。通过 `CoordinateSystem` 判断光标所在屏幕，决定本地放行或网络转发。

**边界检测**: 由 Server 统一判断，不再依赖 Client 上报。

### 5. Client 端纯被动模式

**选择**: Client 不捕获本地输入，只接收 Server 转发的事件并注入。

**理由**: 简化 Client 逻辑，避免 server/client 状态不同步问题。

## Risks / Trade-offs

**风险**: evdev grab 需要对 `/dev/input/*` 有读写权限，通常需要 `input` 组或 root。
**缓解**: 项目已有 udev 规则（`assets/99-superShare.rules`），确保用户在 `input` 组中。

**风险**: evdev 设备热插拔处理。
**缓解**: 使用 inotify 监听 `/dev/input` 目录的 CREATE 事件，动态添加新设备。rdev 的 grab.rs 已有此实现。

**风险**: 鼠标加速在 evdev 层面不生效（evdev 给出原始 delta，不经过 X11 的加速处理）。
**缓解**: 首期接受此行为。未来可在 Server 端应用简单的加速算法。

**Trade-off**: 仅 Linux 支持。Windows/macOS 需要不同的实现（Raw Input / CGEvent）。
**接受**: 首期目标平台是 Linux（Ubuntu），Windows 支持作为后续迭代。
