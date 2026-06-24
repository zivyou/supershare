## 1. 协议扩展 (ss-core)

- [x] 1.1 在 `protocol.rs` 中新增 `Message::MouseDelta { dx: f32, dy: f32 }` 消息类型
- [x] 1.2 在 `MessageType` 枚举中新增 `MouseDelta = 0x0E` 变体
- [x] 1.3 更新 `Message::msg_type()` 和 `TryFrom<u8>` 实现
- [x] 1.4 修改 `Handshake` 消息，新增 `screen_width: u32, screen_height: u32` 字段（Client 上报屏幕尺寸）

## 2. 光标回弹输入捕获 (ss-input)

- [x] 2.1 新建 `crates/ss-input/src/warp_capture.rs` 模块（替代 evdev_capture）
- [x] 2.2 实现 rdev::listen 输入捕获（XRecord，不需要 root）
- [x] 2.3 实现光标回弹：鼠标到达右边缘时用 rdev::simulate 弹回中央
- [x] 2.4 实现 delta 计算：从中央位置计算鼠标移动量
- [x] 2.5 实现边界检测：鼠标到达左边缘时触发返回本地模式

## 3. 虚拟光标管理 (ss-input)

- [x] 3.1 新建 `crates/ss-input/src/virtual_cursor.rs` 模块
- [x] 3.2 实现 `VirtualCursor` 结构体：维护 `global_x: f64, global_y: f64`
- [x] 3.3 实现 `apply_delta(dx, dy)` 方法：累加 delta，clamp 到全局坐标系边界
- [x] 3.4 实现 `current_screen(&CoordinateSystem)` 方法：返回光标所在的 ScreenInfo
- [x] 3.5 实现 `check_boundary()` 方法：检测光标是否跨越屏幕边界，返回 BoundaryEvent

## 4. Server 端输入循环重构 (main.rs)

- [x] 4.1 替换 `start_capture()` 调用为 `evdev_capture::start()`，传入虚拟光标和坐标系统
- [x] 4.2 实现 Server 输入主循环：读取 evdev 事件 → 更新虚拟光标 → 判断所在屏幕 → 本地放行或网络转发
- [ ] 4.3 实现本地模式（LOCAL）：事件写入 UInput 设备，光标在 Server 屏幕内
- [x] 4.4 实现远程模式（REMOTE）：MouseDelta/KeyPress/MouseButton 发送到 Client，事件不写入 UInput
- [x] 4.5 实现模式切换：LOCAL→REMOTE 时发送 BoundaryEnter，REMOTE→LOCAL 时发送 BoundaryLeave
- [x] 4.6 处理 BoundaryLeave 接收：Client 端不再发送 BoundaryLeave（由 Server 统一判断），删除相关处理逻辑

## 5. Client 端简化 (main.rs)

- [x] 5.1 删除 Client 端的 `start_capture()` 调用和 input_rx 转发逻辑
- [x] 5.2 删除 Client 端的 suppressed 管理逻辑
- [x] 5.3 删除 Client 端的边界检测逻辑（左边缘 x<=5 判断）
- [x] 5.4 修改 Client Handshake，上报 screen_width/screen_height
- [x] 5.5 实现 MouseDelta 处理：接收 delta → 累加虚拟光标 → clamp → rdev::simulate(MouseMove)
- [x] 5.6 保留 KeyPress/MouseButton/MouseScroll 的注入逻辑
- [x] 5.7 保留 BoundaryEnter 处理：设置虚拟光标初始位置

## 6. 边界检测简化 (ss-input)

- [x] 6.1 简化 `boundary.rs`：保留 `CoordinateSystem` 结构体和 `add_screen/remove_screen` 方法
- [x] 6.2 删除 `check_boundary()` 方法（边界检测移至 VirtualCursor）
- [x] 6.3 更新 `capture.rs`：保留 `InputEvent` 枚举和 `to_message()` 函数（仍用于其他场景）

## 7. 清理与测试

- [ ] 7.1 删除或废弃 `capture.rs` 中的 `start_capture()` 函数（被 evdev_capture 替代）
- [x] 7.2 更新 `lib.rs` 导出：新增 `evdev_capture` 和 `virtual_cursor` 模块
- [x] 7.3 为 VirtualCursor 编写单元测试
- [ ] 7.4 为 evdev 事件解析编写单元测试
- [ ] 7.5 端到端测试：Server 鼠标移动到边缘 → Client 光标移动 → Server 鼠标移回
