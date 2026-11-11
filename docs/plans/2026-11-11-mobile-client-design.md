# 移动端原生客户端设计

- 日期：2026-11-11
- 状态：设计已确认，spike 进行中（见 [`2026-11-11-mobile-spike-plan.md`](./2026-11-11-mobile-spike-plan.md)）
- 依据：[`host-client-architecture.md`](../host-client-architecture.md)、[`p2p-relay-architecture.md`](../p2p-relay-architecture.md) §10、§13、§15.4

## 1. 定位

手机是 Host 的又一个 Client，不运行 Host。它不复刻桌面工作台，主要用途是：看哪些 Agent 在跑、谁在等人，点进去看终端，必要时接管终端回复。

不做：分屏布局、文件 tab、Vim、在手机上运行 Host、`yttt ctl`/MCP、托盘与登录启动、iOS 推送（暂缓）。

## 2. 界面

两端各用系统 UI 技术与系统样式（iOS：SwiftUI；Android：Jetpack Compose + Material 3），两端外观不必一致，不做自定义设计系统。可点击的交互原型只在本地保存，用来定流程，不作为视觉稿。

| 页面 | 内容 |
|---|---|
| 首页（总控） | 当前 Host 与连接状态；项目、tab、终端、Agent 数量；“需要处理”（等批准 → 等回答 → 失败）卡片；运行中列表；已结束折叠 |
| 项目页 | 顶栏：左抽屉按钮、项目名、右抽屉按钮；终端 tab 横向列表（不含文件）；当前 tab 有多个 pane 时才显示 ◀ n/m ▶ 切换条；终端 |
| 左抽屉 | “总控”入口；项目列表，带等待、失败、运行数量角标；设置、切换 Host |
| 右抽屉 | “文件 / 会话”分段；文件树；Agent 会话历史 |
| 文件 | 点文件全屏打开，可编辑保存，关闭回到终端 |

- 首页与项目页是平级根页面，由左抽屉切换，不使用推入式导航：iOS 左边缘返回手势会与左抽屉冲突。
- 抽屉两端都从侧边滑出。iOS 没有系统抽屉，容器自写，内容用系统组件；Android 左侧 `ModalNavigationDrawer`，右侧同一组件外层改为 RTL、内容恢复 LTR。终端可横向滚动时，右抽屉只响应屏幕边缘约 20pt 内开始的拖动。
- 未接管的终端在底部悬浮“接管终端”提示；Agent 等待时提示改为“等批准，接管后回复”。接管后提示换成按键栏（Esc、Tab、Ctrl、方向键、回车、键盘）和“交还电脑”。

## 3. 分层

Rust 只做核心链路，其余交给原生。

| Rust（`yttt-mobile-core`，UniFFI） | 原生 |
|---|---|
| iroh / TLS 连接、Relay ticket、配对协议与 Ed25519 签名 | 全部界面状态与导航；首页计数、排序、分组 |
| 帧编解码、压缩、分块、重连、按 sequence 恢复、epoch 校验、请求关联（复用 `yttt-client-core`） | 已配对 Host 列表存储；Keychain / Keystore 保存设备私钥 |
| 终端镜像与 VT 重绘编码 | 扫码、通知、网络变化与前后台监听（只通知 Rust 事件） |
| | 文件树状态、文件编辑与语法高亮（编辑器库）、账号登录界面与设备码流程 |

FFI 约束：

1. 终端字节按屏幕刷新拉取：原生在 `CADisplayLink` / `Choreographer` 每帧调用一次 `drain_terminal(id)`，拿到合并后的一块字节；没有新内容返回空。
2. 只有正在显示的终端设为可见（`set_visible_terminals`），其余不收渲染帧。
3. 输入单向：`send_input(id, bytes)` 立即返回。
4. 概要与布局变化在 Rust 侧合并 50–100 ms 后推一份快照，不逐事件回调。
5. Rust 自带 tokio 运行时；FFI 调用不阻塞调用线程，回调到原生后切主线程。
6. 大块数据（终端帧、文件内容）按字节数组传。

## 4. 终端

组件：iOS 用 SwiftTerm（MIT）；Android 首选 Termux `terminal-emulator` + `terminal-view`（许可证选型时核实；备选 ConnectBot termlib）。

Host 下发的是语义增量（`SemanticDelta.changed_rows`，每行一组带样式的 span，另有光标、模式位、调色板），组件只吃 VT 字节。Rust 核心把镜像状态重新编码为重绘序列：

- 变化行：`CUP` 到该行 → 每个 span 的 SGR 与文本 → `EL`；
- 光标：位置、`DECSCUSR` 形状、`DECTCEM` 显隐；
- `TerminalModes` 模式位 → `DECSET`（光标键模式、bracketed paste、鼠标模式），组件据此自行编码按键；
- 超链接 → OSC 8；图片 iOS 可转 iTerm2 内联图片，Android 先占位。

终端仍只在 Host 解析一次；组件解析的是我们生成的重绘序列。按键编码由组件完成，不必从 `yttt-terminal/src/input.rs` 抽取。

- 滚动历史在 Host：组件本地 scrollback 设为 0，滑动转 `ScrollTerminal`，搜索用 `SearchTerminal`。
- 只观察时组件列数固定为 Host 尺寸，放进可横向滚动的容器，双指缩放字号。接管后按手机宽度计算行列并 resize。
- 断线重连或 epoch 变化：先清屏，再发完整快照的重绘。

## 5. 与 Host 的约定

- 导航只在本地：手机不发 `SelectProject`、`FocusPane`、`ActivateItem`、`OpenFile`，电脑上的视图不受影响。进入 tab 时默认显示 Host 共享视图里聚焦的 pane。
- 文件：`Project::ReadFile`（大文件 `ReadFileChunk`）读；`Project::SaveFile { mode: Check(fingerprint) }` 存，`Conflict` 时让用户选覆盖（`Force`）或重新加载。不使用文档 lease：`acquire_document` 要求文件已经 `OpenFile` 进工作区布局。工作区若显示该文件在电脑上有未保存草稿，打开时提示。
- 首页数据：`AgentSnapshot` 概要（`view_state()`、`waiting_reason`、`waiting_message`、`task`、`current_action`、`state_started_at`），不订阅 board；`TerminalSessionId` 的 `{project}:{tab}:{pane}` 与工作区布局对应。

## 6. Android 通知

前台服务（类型 `specialUse`；`dataSync` 在 Android 15 每天限 6 小时）保持连接，只订阅 Agent 概要、所有终端不可见。Agent 进入 Waiting 时发本地通知，按 Agent generation 与状态去重，正在查看该 Agent 时不发，点通知打开对应项目、tab、pane。不接 FCM：无需推送服务器，自建 Relay 可用，没有 Google 服务的手机也能用。需引导用户关闭电池优化。

## 7. 未决问题

1. 终端控制权粒度：`TakeTerminalController` 让一个 Client 持有全部 Interactive lease。手机接管时希望只影响当前 pane，进后台自动交还。spike 先观察现有行为，再决定是否改协议。
2. Termux 组件接远程数据源的方式（自写 session 还是 fork）。
3. iOS 推送：APNs 只能由 App 发布者发送，需要官方推送网关，暂缓。
4. Relay 账号登录：现有 OAuth 回调只到 loopback，手机先用设备码流程。

## 8. Spike 结论

（spike 完成后填写。）
