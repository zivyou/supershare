# SuperShare Bug 记录

开发过程中遇到的典型 bug、根因分析和修复方式。

## Bug 1: GUI 连接无反馈（connect_with_retry 无限重试）

**现象**：GUI 点击 Connect 后永远显示 Disconnected，无错误信息。

**根因**：GUI 模式使用 `connect_with_retry()` 连接，该函数失败时无限重试，永远不会返回错误给 GUI。

**修复**：GUI 模式改用 `connect()` + `tokio::time::timeout(10s)`，超时或失败时设置 `state.last_error` 显示到 GUI。

---

## Bug 2: 连接后无共享功能

**现象**：CLI/GUI 连接成功，但鼠标键盘剪切板均无共享表现。

**根因**：连接建立后只维持心跳，没有启动任何输入捕获、事件注入或剪切板监听。所有 building blocks（capture、inject、clipboard monitor）都已实现但从未被调用。

**修复**：连接成功后启动完整的共享循环：
- Client：start_capture + start_monitor + 转发输入 + 注入事件 + 剪切板同步
- Server：start_capture + boundary 检测 + 转发到 client + 注入 client 输入 + 剪切板同步

---

## Bug 3: "invalid socket address"（GUI 连接失败）

**现象**：GUI 报错 `Connection failed: invalid socket address`。

**根因**：config.toml 中 `server_address` 存的是 `10.2.154.163`（无端口），`connect()` 函数直接传给 `TcpStream::connect()`，缺少端口导致 OS 报错。

**修复**：`connect()` 函数检测地址是否含 `:`，无则自动补 `:9876`。

---

## Bug 4: 屏幕分辨率硬编码 1920x1080

**现象**：鼠标到 x=1679 就到物理边缘，但边界检测要 x>=1915 才触发，永远检测不到。

**根因**：`CoordinateSystem::new(1920, 1080)` 硬编码，实际屏幕是 1680x1050。

**修复**：添加 `detect_screen_size()` 函数，Linux 用 `xrandr --query`，Windows 用 `wmic`，fallback 1920x1080。

---

## Bug 5: 边界弹回（bounce-back）

**现象**：鼠标切换到 client 后立刻弹回 server。

**根因**：`check_boundary` 返回 `enter_x=0.0`，而 client 的边界检测是 `x<=5.0`。x=0.0 在边界区内，client 立刻触发 BoundaryLeave。

**修复**：`enter_x` 从 `0.0` 改为 `BOUNDARY_ZONE_PX + 1 = 6.0`，落在边界区外。

---

## Bug 6: BoundaryEnter 被 main loop 消费丢弃

**现象**：Server 发送 BoundaryEnter 成功，但 Client 永远收不到。

**根因**：Client 的 `control_rx` 有两个订阅者——main loop 和 boundary listener。Main loop 的 `Ok(_) => {}` 把 BoundaryEnter 消费后丢弃了，boundary listener 永远收不到。

**修复**：合并为单一循环，同时处理 BoundaryEnter/BoundaryLeave 和 disconnect 检测。

---

## Bug 7: TLS writer 被 heartbeat task 独占（根因最深）

**现象**：Server 报 `Failed to send BoundaryEnter: channel closed`。

**根因**：Server 的 `handle_control_connection` 中，`writer` 被 `tokio::spawn(async move { ... })` 移入 heartbeat task，`ctrl_rx` 无人消费。Heartbeat task 独占了 TLS writer，BoundaryEnter 消息堆在 mpsc channel 里发不出去。

修复尝试 1：合并 heartbeat + ctrl_rx 到同一 writer task。但 `shutdown_writer` 订阅了 `state.broadcast_rx`（和客户端消息同一 channel），客户端一发 MouseMove 就触发 shutdown，writer task 退出。

**最终修复**：移除 `shutdown_writer`，writer task 只在 `ctrl_rx` 关闭时退出（`ctrl_tx` 被 drop = 客户端已移除）。

---

## Bug 8: server_width 变量未使用

**现象**：编译警告 `unused variable: server_width`。

**根因**：Client 端捕获了 `server_width` 但从未使用，是死代码。

**修复**：cargo fix 自动移除。后续如需 client 端边界检测可重新引入。

---

## Bug 9: Client 未切换屏幕也响应 Server 的点击/按键

**现象**：Client 连上 Server 后，无论鼠标有没有越过边界切到 Client 屏幕，Server 本机的鼠标点击、键盘、滚轮事件 Client 都会响应（本机同时也响应，等于一次操作触发两台机器）。

**根因**：两层叠加——

1. `warp_capture.rs` 本地模式下，`ButtonPress/ButtonRelease/Wheel/KeyPress/KeyRelease` 的处理是「既透传给本机 X Server，又 `try_send` 到转发通道」。只有 `MouseMove` 正确地不在本地模式产生事件，所以鼠标移动没这个问题。
2. `main.rs` 转发任务收到 `MouseButton/KeyPress/Scroll` 后不检查 `is_remote`，无条件转发给所有 Client。

**修复**：

1. `warp_capture.rs` 本地模式的按键/滚轮/键盘事件只透传本机，不再进入转发通道（源头修复）。
2. `main.rs` 转发任务增加 `is_remote` 检查，非远程模式直接 `continue`（防御层）。

**教训**：捕获层「本地透传」和「远程转发」两条路径必须互斥。本地事件进入转发通道没有任何消费者需要它，只会被误转发。

---

## Bug 10: Client 屏幕下边界（及任意边缘）误触发切换

**现象**：光标在 Client 屏幕上时，移动到下边界会意外切回 Server。预期只有 Client 左边界才触发切换。

**根因**：`warp_capture` 的 delta 计算建立在一个错误假设上——它以为 rdev 事件里的 `(x, y)` 是真实光标位置（会被 XTest warp 重置），所以每次 warp 后把 `last_x/last_y` 重置为 warp 目标点。但 `rdev::grab`（Linux）直接读 evdev 相对位移并维护**自己的内部坐标累加器**（钳制在屏幕范围内），XTest warp 不经过 evdev，累加器对 warp 毫无感知。于是：

- 越界进入 remote 模式后，累加器 x 仍钳制在右边缘 1920，而 `last_x` 被重置为 960 → 之后**每一个**移动事件（包括纯向下移动）都产生 `dx=+960` 的虚假 delta 并触发幻影 `hit_right`；
- 累加器钳制在左边缘（x=0）时，任意方向移动产生 `dx=-1918` 虚假 delta → Client 虚拟光标 `vc.0` 瞬间 ≤0 → 发送返回信号 → 误切换；
- 顶/底边缘同理产生 ±1079 虚假 delta（Client 光标瞬移）。

**修复**：

1. `warp_capture.rs` 彻底移除 warp 机制：delta 改用「相邻两次上报坐标之差」（永远精确，边缘钳制只丢弃越界过冲、绝不虚构位移）；由捕获层自维护逻辑光标位置做边界检测。remote 模式下真实光标冻结在越界点（所有事件被 suppress），退出时逻辑位置恢复冻结点，与真实光标天然重新同步。
2. 清理 magic value：`-1.0` 特殊信号会吃掉大量真实 1px 左移 delta（`main.rs` 转发循环中的该分支已删除）；Client 返回请求由 `MouseDelta{dx:-3.0}` 改为复用协议消息 `BoundaryLeave`。

**教训**：`rdev::grab` 上报的坐标是 evdev 相对位移的累加值，不是真实光标位置；XTest 注入不会回流到 evdev。任何「warp + 绝对坐标」的混合方案都会产生幻影 delta。相对位移场景只用相邻事件差值。

---

## 经验总结

1. **tokio::spawn 的 move 语义**：变量被 move 进 async block 后，原作用域不可再用。多个 task 共享资源时必须 clone。
2. **broadcast channel 的多订阅者**：每个订阅者独立消费消息。如果两个 task 订阅同一 channel，消息会被其中一个抢走。
3. **mpsc channel 用于控制消息**：避免 broadcast 的竞争问题，一对多场景用 broadcast，一对一用 mpsc。
4. **TLS stream 不能并发写**：split 后的 writer 只能由一个 task 持有，多 task 写需要通过 mpsc 汇总到单一 writer。
5. **不要用 broadcast channel 做 shutdown 信号**：如果同一 channel 还承载业务消息，业务消息会误触发 shutdown。
6. **rdev::grab 的坐标不是真实光标位置**：Linux 下它是 evdev 相对位移的内部累加值，XTest warp 不会回流。相对位移只用相邻事件差值计算，不要做 warp。
7. **不要用 magic value 当协议信号**：`-1.0`/`-3.0` 这类特殊值会和真实数据碰撞（真实 1px 左移就是 `(-1, 0)`）。协议里已有专用消息类型（如 `BoundaryLeave`）就直接用。
