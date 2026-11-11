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

- 自动换行始终关闭（`CSI ?7l`）：组件从不直接收 Host 的原始输出，关掉后组件与 Host 字宽表不一致时也不会折行或滚屏。
- 变化行：先 `CUP` 到行首、`CSI 2K` 清行（此时光标在第 1 列，没有待换行状态），再让每个 span 按 `start_column` 单独 `CUP`，输出 SGR 与文本。写完文本后不输出 `EL`：写满一行后光标处于待换行状态，xterm 语义的组件会让 `EL` 擦掉最后一列。
- 光标：位置、`DECSCUSR` 形状、`DECTCEM` 显隐，放在每次输出末尾。
- 调色板：`TerminalPalette` 的覆盖项转 OSC 4 / 10 / 11。
- 下划线颜色：只在有下划线样式且颜色不等于前景时输出 SGR 58，默认色用 59。
- 模式位（alacritty `TermMode`）：`APP_CURSOR` → `?1`，`BRACKETED_PASTE` → `?2004`，`APP_KEYPAD` → `DECKPAM`/`DECKPNM`；接管后才同步鼠标（`CLICK` → 1000、`DRAG` → 1002、`MOTION` → 1003、`SGR` → 1006）与焦点（`FOCUS` → 1004）模式，避免观察时的点按变成鼠标上报。`LINE_WRAP`、`INSERT`、`ORIGIN` 不同步。
- 超链接 → OSC 8；图片 iOS 可转 iTerm2 内联图片，Android 先占位。
- `drain` 先比较 `terminal_metadata().sequence`，没变化就不克隆 viewport。

终端仍只在 Host 解析一次；组件解析的是我们生成的重绘序列。按键编码由组件完成，不必从 `yttt-terminal/src/input.rs` 抽取。

- 滚动历史在 Host：组件本地 scrollback 设为 0，滑动转 `ScrollTerminal`（`display_offset` 按每个附加单独记录，不影响电脑），搜索用 `SearchTerminal`。备用屏（全屏 TUI）里没有可滚的历史。
- 始终以 Observer 附加。以 Interactive 附加会 resize PTY 并改写 query palette，还会让手机成为 controller。用户点“接管”之前，不发任何 Interactive 的附加或 lease 请求。
- 只观察时组件列数固定为 Host 尺寸，放进可横向滚动的容器，双指缩放字号。接管后按手机宽度计算行列并 resize。
- 断线重连或 epoch 变化：先清屏，再发完整快照的重绘。

## 5. 与 Host 的约定

- 导航只在本地：手机不发 `SelectProject`、`FocusPane`、`ActivateItem`、`OpenFile`，电脑上的视图不受影响。进入 tab 时默认显示 Host 共享视图里聚焦的 pane。
- 文件：`Project::ReadFile`（大文件 `ReadFileChunk`）读；`Project::SaveFile { mode: Check(fingerprint) }` 存，`Conflict` 时让用户选覆盖（`Force`）或重新加载。不使用文档 lease：`acquire_document` 要求文件已经 `OpenFile` 进工作区布局。工作区若显示该文件在电脑上有未保存草稿，打开时提示。
- SSH 项目的文件走 `RemoteFile`：`Project::ReadFile` 在 SSH 项目上返回 NotFound。
- 工作区：先 `WorkspaceRequest::List`，再 `Open` 其中标为 active 的那个；`Open` 一个未知 id 会得到空布局。不发 `SetActive`。
- `client_instance_id` 每次连接新生成（与桌面相同），不持久化。Host 把一个 ID 绑定到第一次使用它的认证会话（入口、凭据代数、session nonce、设备公钥），断开后保留 5 分钟（`CLIENT_SESSION_RETENTION`），期间拒绝其他会话使用这个 ID：持久化 ID 的 App 被杀后 5 分钟内重开会一直连不上。连 nonce 一起持久化也不行：Host 的请求日志按 `request_id` 回放，新进程从 1 重新编号会拿到旧响应。App 被杀后 lease 本来也不保留（断线后 60 s 转给别的 Client），重启只需重新附加。
- 手机上的输入会把对应 Agent 标为由人驱动，并清空编排器待投递的消息（`agent_control.rs`），与桌面输入相同。
- 首页数据：`AgentSnapshot` 概要（`view_state()`、`waiting_reason`、`waiting_message`、`task`、`current_action`、`state_started_at`），不订阅 board；`TerminalSessionId` 的 `{project}:{tab}:{pane}` 与工作区布局对应。

## 6. Android 通知

前台服务（类型 `specialUse`；`dataSync` 在 Android 15 每天限 6 小时）保持连接，只订阅 Agent 概要、所有终端不可见。Agent 进入 Waiting 时发本地通知，按 Agent generation 与状态去重，正在查看该 Agent 时不发，点通知打开对应项目、tab、pane。不接 FCM：无需推送服务器，自建 Relay 可用，没有 Google 服务的手机也能用。需引导用户关闭电池优化。

## 7. 未决问题

1. **终端控制权需要改协议。** 现状：`take_terminal_control` 把所有 lease 移给接管方（`yttt-host/src/runtime.rs`）；第一次申请写入的 Client 不需要 Take 就成为 controller（`claim_locked`）；没有“交还”请求，`ReleaseTerminalControl` 只放一个 lease，controller 不变；断线或进后台后控制权保留 60 s，再交给最近连接的 Client。这与 `p2p-relay-architecture.md` §10.4 不采用全局抢占的原则相矛盾。“只接管当前 pane”“交还电脑”“进后台自动交还”需要：按 session 的 controller、显式交还请求、后台时立即交还。spike 期间“交还”只能由电脑点接管完成。
2. **协议版本锁步。** 两端都用 `ProtocolRange::exact`。商店审核会让手机版落后于桌面版，桌面一升协议手机就连不上。需要决定：Host 支持一个版本范围，或两端同步发布。
3. **iOS 上架限制。** `yttt-transport-iroh` 无条件启用 iroh 的 `fast-apple-datapath`，用到私有 API `sendmsg_x`/`recvmsg_x`，有被 App Store 拒审的风险，需要改为按 target 启用。portmapper 的 UPnP 在 iOS 上需要 multicast entitlement，移动端关掉。局域网直连需要 `NSLocalNetworkUsageDescription`。
4. 网络变化：`IrohEndpoint` 没有封装 iroh 的 `network_change`，`link_watch` 在桌面 bin 里，需要新接口。
5. Termux 组件接远程数据源的方式（自写 session 还是 fork）。
6. iOS 推送：APNs 只能由 App 发布者发送，需要官方推送网关，暂缓。
7. Relay 账号登录：现有 OAuth 回调只到 loopback，手机先用设备码流程。不登录账号时，配对得到的 relay 提示只对自建 relay 有效。

## 8. Spike 结论

（spike 完成后填写。）
