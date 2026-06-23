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

## 经验总结

1. **tokio::spawn 的 move 语义**：变量被 move 进 async block 后，原作用域不可再用。多个 task 共享资源时必须 clone。
2. **broadcast channel 的多订阅者**：每个订阅者独立消费消息。如果两个 task 订阅同一 channel，消息会被其中一个抢走。
3. **mpsc channel 用于控制消息**：避免 broadcast 的竞争问题，一对多场景用 broadcast，一对一用 mpsc。
4. **TLS stream 不能并发写**：split 后的 writer 只能由一个 task 持有，多 task 写需要通过 mpsc 汇总到单一 writer。
5. **不要用 broadcast channel 做 shutdown 信号**：如果同一 channel 还承载业务消息，业务消息会误触发 shutdown。
