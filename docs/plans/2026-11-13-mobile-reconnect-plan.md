# 手机端断线重连与刷新：计划

- 日期：2026-11-13
- 分支：`feat/mobile-reconnect`（worktree `.worktrees/mobile-reconnect`）
- 依据：用户反馈（iPhone 真机）：网络出错、断开重连后看不到终端，要退出再进来才好；iOS tab 条“全部 Tab”按钮去掉背景后，tab 滚到按钮下面很难看。

## 现状

- `ClientCore` 断线后自己重连（退避最长 5 s），重连后重新订阅已附着的终端；`retry_now` / `probe_now` 可立即重试。
- 手机核心只在第一次连接时打开桌面的当前 workspace（项目、tab、pane 的来源）。重连后不再打开；Host 重启过（桌面更新、重启电脑）则 host epoch 变了，旧 workspace 的布局、终端 id 都不再有效，总控与项目页就没有终端。
- 应用不知道链路状态：断开时界面照旧，没有提示，也没有重试入口。
- 应用整个重建连接（换 Host、登录变化）时，已打开的终端页仍拿着旧的 `HostSession`，画面停住。

## 设计

### 核心（`yttt-mobile-core`）

1. **链路状态导出**：`HostSession.link_state() -> LinkState`（`Connecting`、`Ready`、`Reconnecting { attempt, reason }`、`Lost { reason }`），`watch_link(listener)` 每次变化回调；`retry_now()`。
2. **重连后恢复**：会话自己在每次重新 `Ready` 后重新打开桌面的当前 workspace，再通知概要（overview）更新；终端由 `ClientCore` 重新订阅、编码器整屏重画（已有）。
3. **测试**：集成测试里让 Host 的连接断掉（连接器可以切断已建的连接），断言 `LinkState` 依次经过 `Reconnecting` → `Ready`、workspace 重新打开、概要里的项目与终端回来、附着的终端继续出画面。

### 两端

1. **状态提示**：总控、项目页、终端页顶部一条细横幅：重连中显示“连接断开，正在重连…”与“立即重试”；`Lost` 显示原因与“重新连接”。`Ready` 时不显示。
2. **重连后刷新**：收到从非 `Ready` 回到 `Ready`，重新取终端列表（项目页的 tab/pane 跟着概要更新）。
3. **兜底重建**：前台时链路连续 20 s 不是 `Ready`，或进入 `Lost`，丢掉旧会话、新建连接（等于用户“退出再进来”）。
4. **回到前台**：立即 `networkChanged()`（重新检查网络并探测，死连接马上换）。
5. **下拉刷新**：总控下拉：链路好时重新打开 workspace 并取终端；不好时立即重试。
6. **终端页跟随会话**：终端、项目页按会话身份建视图，新会话来了重建，不再停在旧会话上。
7. **iOS tab 条**：tab 的滚动区域与按钮并排（`HStack`），tab 不会滚到按钮下面；按钮与整条用同一个背景，没有色差。

## 验证

- 核心集成测试（见上）；两端单测（状态横幅文案、20 s 兜底计时的纯逻辑）。
- 真机（iPhone）：飞行模式开关、切 Wi-Fi / 蜂窝、桌面 yttt 重启，终端自动回来；横幅与重试。
- Android 模拟器：断网恢复、Host 重启。
