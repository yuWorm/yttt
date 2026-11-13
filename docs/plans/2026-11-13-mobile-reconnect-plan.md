# 手机端断线重连与刷新：计划

- 日期：2026-11-13
- 分支：`feat/mobile-reconnect`（worktree `.worktrees/mobile-reconnect`）
- 依据：用户反馈（iPhone 真机）：网络出错、断开重连后看不到终端，要退出再进来才好；iOS tab 条“全部 Tab”按钮去掉背景后，tab 滚到按钮下面很难看。

## 现状（经评审核对代码）

- `ClientCore` 断线后自己重连（退避最长 5 s）；每次新连接重新订阅已附着的终端、重新打开已打开过的 workspace（`OpenExisting`）、重新上报可见终端。Host 重启（新 epoch）时清掉旧终端并发 `TerminalUnavailable`。
- **附着失败不再重试**：终端页只在创建时 `attach` 一次；链路不通时失败，只写进调试统计。`ClientCore` 只重新订阅附着成功的终端，所以断线期间打开的终端页一直空白，直到退出再进——就是用户遇到的情况。
- **终端列表只按数量刷新**：两端都在概要的终端数变了才重新取终端列表。Host 重启后终端 id 全换了、数量多半不变，应用拿着旧 id，项目页显示“这个 pane 没有在运行”。
- **控制权不同步**：Host 断线后为手机保留接管 60 s，重连回来照旧；核心却在每次 `Ready` 把接管清零，手机以为交还了，电脑上的终端还停在手机的尺寸。
- **workspace 只在第一次打开**：第一次失败、重连后被告知不存在、桌面换了当前 workspace，三种情况都不再打开。
- 应用不知道链路状态，没有提示和重试入口；回到前台不检查网络（iOS）。
- 整个重建连接时，终端页按终端 id 建视图，新会话来了仍用旧会话。

## 设计

### 核心（`yttt-mobile-core`）

1. **想要附着的终端**：`attach` 记下意图，链路不通也不报错；每次 `Ready` 补附着还没附着上的；终端在 Host 上没了（`TerminalUnavailable`）就从意图里去掉。
2. **接管跟着 Host**：Host epoch 没变的重连保留接管；变了才清零。
3. **workspace**：`Ready` 时若还没有、或 Host epoch 变了、或上次打开失败，重新打开桌面的当前 workspace。
4. **概要带终端 id**：`Overview` 加上终端 id 列表，应用按 id 变化刷新终端列表。
5. **链路状态导出**：`HostSession.link_state()`（`Connecting`、`Ready`、`Reconnecting { attempt, reason }`、`Lost { reason }`）、`watch_link(listener)`、`retry_now()`。
6. **测试**（`tests/session.rs`，用可切断的连接器，同 `yttt-client-core/tests/transport_contract.rs` 的 `Cuttable`）：切断期间附着的终端在恢复后出画面；概要的项目与终端回来；接管在短暂切断后还在；`LinkState` 经过 `Reconnecting` → `Ready`。

### 两端

1. **状态横幅**：总控、项目页、终端页顶部：重连中“连接断开，正在重连…”与“立即重试”；超过 15 s 还没好，或 `Lost`，加“重新连接”（整个重建，等于退出再进）。`Lost` 时自动重建一次。
2. **重连后刷新**：从非 `Ready` 回到 `Ready`、或概要的终端 id 变了，重新取终端列表。
3. **回到前台**：立即 `networkChanged()`。
4. **下拉刷新**：总控下拉：链路好时重新取终端与概要；不好时立即重试。
5. **视图跟随会话**：终端视图按（会话、终端 id）建，新会话来了重建；单独打开的终端在 Host 上没了，显示“终端已结束”并可返回。
6. **iOS tab 条**（已改，`2ae8363`）：tab 与按钮并排，同一背景。

### 暂不做（记录）

- `IrohConnector` 缓存的 QUIC 连接在握手应答后不因超时丢弃，重连可能一直用死连接直到空闲超时；`network_changed` 的探测会换掉它，先靠回到前台与网络变化触发。
- 重连时没有“只经 relay”的 8 s 兜底；手动“重新连接”会走首连的兜底。

## 验证

- 核心集成测试（见上）；两端单测（状态横幅文案、20 s 兜底计时的纯逻辑）。
- 真机（iPhone）：飞行模式开关、切 Wi-Fi / 蜂窝、桌面 yttt 重启，终端自动回来；横幅与重试。
- Android 模拟器：断网恢复、Host 重启。
