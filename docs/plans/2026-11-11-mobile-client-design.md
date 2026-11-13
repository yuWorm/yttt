# 移动端原生客户端设计

- 日期：2026-11-11
- 状态：设计已确认，spike 已完成（见 [`2026-11-11-mobile-spike-plan.md`](./2026-11-11-mobile-spike-plan.md) 与 §8）
- 依据：[`host-client-architecture.md`](../host-client-architecture.md)、[`p2p-relay-architecture.md`](../p2p-relay-architecture.md) §10、§13、§15.4

## 1. 定位

手机是 Host 的又一个 Client，不运行 Host。它不复刻桌面工作台，主要用途是：看哪些 Agent 在跑、谁在等人，点进去看终端，必要时接管终端回复。

不做：分屏布局、文件 tab、Vim、在手机上运行 Host、`yttt ctl`/MCP、托盘与登录启动、iOS 推送（暂缓）。

## 2. 界面

两端各用系统 UI 技术与系统样式（iOS：SwiftUI；Android：Jetpack Compose + Material 3），两端外观不必一致，不做自定义设计系统。可点击的交互原型只在本地保存，用来定流程，不作为视觉稿。

**2026-11-13 起按平台原生方式（`docs/plans/2026-11-13-mobile-native-ui-plan.md`）：** 取消侧滑抽屉，导航用各平台自己的结构。

| 页面 | iOS | Android |
|---|---|---|
| 根 | `TabView`：总控 / 项目 / 设置，各自一个导航栈；“总控”的角标是需要处理的数量 | `NavigationBar` 同样三个目的地，Navigation Compose 返回栈，预测性返回；边到边，Android 12 起动态取色 |
| 总控 | Host 行（菜单：切换、添加、管理）、四个数量、需要处理、运行中、已结束折叠；下拉刷新；空状态 | 同左，Host 是带下拉菜单的 chip |
| 项目列表 | 项目，带正在跑的 Agent 的 logo、等待数、运行数 | 同左 |
| 项目页（推入，隐藏标签栏 / 导航栏） | 标题菜单就地换项目；胶囊 tab 一排（Agent logo、等待点），最后是 tab 总览 sheet；底部细的 pane 条和接管栏 | 标题下拉换项目；带图标的 `PrimaryScrollableTabRow`；底部细的 pane 条；接管 FAB |
| 文件与会话 | 半屏 / 全屏 sheet，分段切换 | `ModalBottomSheet`，`PrimaryTabRow` 切换 |
| 编辑器 | `fullScreenCover` | 独立页面，Snackbar 提示 |
| 设置 | Host 列表（点切换，左滑忘记）、添加、版本、开源许可 | Host 列表（菜单忘记）、添加、后台运行、通知、版本、开源许可 |

- Agent 用桌面版的 logo（`scripts/build-mobile-agent-logos.py` 从 `assets/icons/agent-*.svg` 生成）。Codex、Claude 彩色，其余单色，跟随文字颜色。状态、数量、文件类型、设置项都带图标。
- pane 条只在当前 tab 有多个 pane 时出现。接管前在接管栏（Android 是导航栏）上方，接管后在按键栏上方，可以点也可以左右滑。
- 点 Agent、点通知：推入它所在的项目页，定位到那个 pane；不在 pane 里的终端单独推入。
- 终端的调试统计只在调试版显示。
- iOS 的界面测试 `YtttUITests` 对着 spike Host 真点一遍，每步截图。

- 按键栏（2026-11-12，`docs/plans/2026-11-12-mobile-key-bar-plan.md`）：按键编码在核心（`send_key`，方向键随程序的光标模式），Android 自做按键栏与一次性 Ctrl，iOS 用 SwiftTerm 自带的 `TerminalAccessory`。
- 未接管的终端在底部悬浮“接管终端”提示；Agent 等待时提示改为“等批准，接管后回复”。接管后提示换成按键栏（Esc、Tab、Ctrl、方向键、回车、键盘）和“交还电脑”。

**2026-11-12 的 spike 版（已被上面的结构取代）：** 首页与项目页为平级根页面，左抽屉切换（iOS 自写侧滑容器，Android `ModalNavigationDrawer`）；项目页有终端 tab、多 pane 时的 ◀ n/m ▶、终端与随 Agent 等待变化的接管提示；进入项目打开电脑选中的 tab 与其聚焦 pane；首页点 Agent、点通知都定位到它所在的 pane。右抽屉（文件 / 会话）与全屏文件编辑已实现（`docs/plans/2026-11-12-mobile-files-plan.md`；编辑器 iOS Runestone、Android sora-editor）。Android 的抽屉手势只在打开后生效（观察终端时横向拖动留给终端），由顶栏按钮打开。

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

**存储与配对（2026-11-12，`docs/plans/2026-11-12-mobile-secure-pairing-plan.md`）：**

- 设备密钥：
  - iOS 存 Keychain，`AfterFirstUnlockThisDeviceOnly`；
  - Android 用 Keystore 里不可导出的 AES-256-GCM 密钥加密后存文件。
  - 两端都会把 spike 留下的明文文件迁移进去，然后删除。
- 配对记录不进备份。
- 配对页可以扫电脑上的二维码，也可以从图片识别：
  - iOS 用 VisionKit 和 CoreImage；
  - Android 用 CameraX 和 ZXing，不依赖 Google 服务。

**多台 Host（2026-11-12，`docs/plans/2026-11-12-mobile-multi-host-plan.md`）：**

- 手机记住多台 Host（`hosts.json`，按添加顺序），同一时间只连当前那台。设备密钥一份，所有 Host 共用。
- 首页顶部显示当前 Host 的名字，点开可以切换、添加、忘记。
  - 重名时在名字后加 id 的末 4 位。
  - 同一台 Host 重新配对，原位替换记录，不会多出一条。
- Host 配对时把电脑名发给设备，以前发的是 `environment_id`（UUID）。桌面客户端也跟着显示电脑名。
- 切换 Host 时，旧 Host 的等待通知随总控数据一起清掉；Android 的常驻通知显示当前 Host 的名字。
- 不同时连多台：非当前 Host 的 Agent 等待不会通知。

**构建类型（2026-11-13，`docs/plans/2026-11-13-mobile-release-build-plan.md`）：**

- 验证用的自动化参数（iOS 的 `YTTT_DEBUG_*` 环境变量（原名 `YTTT_SPIKE_*`），Android 启动 Intent 的 `pair`、`qrImage`、`type`、`relayOnly`）只编进调试版：
  - Android 放在 `debug` 源码集，`release` 源码集里同名函数什么也不读；
  - iOS 在 `#if DEBUG` 里读。
  - `scripts/mobile/check-release.sh` 在 APK 或 `.app` 里找这些名字，找到就失败。
- Android 正式版用 R8 压缩，保留 JNA 和 UniFFI 生成的类；APK 从 40 MB 降到 22 MB。
- iOS 模拟器构建只要 arm64：Rust 核心只编了 arm64 的模拟器库，Release 默认还要 x86_64。
- 包名 `com.yttt.yai`，名字 `yttt`，版本 0.1.0（2026-11-13，`docs/plans/2026-11-13-mobile-app-identity-plan.md`）。
  - 图标由 `scripts/build-mobile-icons.py` 从桌面图标的 SVG 生成：iOS 一张不透明的 1024 图，Android 自适应图标（含主题图标用的单色层）和通知小图标。
  - 配对时告诉 Host 的设备名：Android 取系统设置里的设备名，没有就用型号；iOS 取 `UIDevice.current.name`。
  - 和 spike 版包名不同，是另一个 App：spike 版的配对和密钥不带过来。
- 正式版目前不签名：要你的 Apple 团队和 Android 签名密钥。

## 4. 终端

组件：iOS 用 SwiftTerm（MIT）；Android 用 ConnectBot termlib（Apache-2.0，Compose）。Termux 的组件 spike 后放弃，见 §8。

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
- 文件：`Project::ReadFile`（大文件 `ReadFileChunk`）读；`Project::SaveFile { mode: Check(fingerprint) }` 存，`Conflict` 时让用户选覆盖（`Force`）或重新加载。不使用文档 lease：`acquire_document` 要求文件已经 `OpenFile` 进工作区布局。工作区若显示该文件在电脑上有未保存草稿，打开时提示。 已实现：fingerprint 留在核心按 (项目, 路径) 记住，原生只拿文本；由工作区打开的项目无需手机再注册即可读写（集成测试确认）。
- SSH 项目的文件走 `RemoteFile`：`Project::ReadFile` 在 SSH 项目上返回 NotFound。
- 工作区：先 `WorkspaceRequest::List`，再 `Open` 其中标为 active 的那个；`Open` 一个未知 id 会得到空布局。不发 `SetActive`。
- `client_instance_id` 每次连接新生成（与桌面相同），不持久化。Host 把一个 ID 绑定到第一次使用它的认证会话（入口、凭据代数、session nonce、设备公钥），断开后保留 5 分钟（`CLIENT_SESSION_RETENTION`），期间拒绝其他会话使用这个 ID：持久化 ID 的 App 被杀后 5 分钟内重开会一直连不上。连 nonce 一起持久化也不行：Host 的请求日志按 `request_id` 回放，新进程从 1 重新编号会拿到旧响应。App 被杀后 lease 本来也不保留（断线后 60 s 转给别的 Client），重启只需重新附加。
- 手机上的输入会把对应 Agent 标为由人驱动，并清空编排器待投递的消息（`agent_control.rs`），与桌面输入相同。
- 首页数据（已实现，`docs/plans/2026-11-12-mobile-overview-plan.md`）：`yttt-mobile-core::overview` 从打开的工作区布局（`Opened.layout`，随 `WorkspaceChanged` 更新）、client-core 缓存的 Agent 快照（网络入口上 Host 不带 board）与资源目录构建 `Overview`，`watch_overview` 在变化后去抖 250 ms 推给原生；数数、分组、排序在原生。Agent 所在的 tab 与 pane 取自终端名 `{project}:{tab}:{pane}`（与桌面一致）：Host 自己启动的 pane，其 hook scope 的 tab 与 pane 都是整个终端名，不能直接用。

## 6. Android 通知

前台服务（类型 `specialUse`；`dataSync` 在 Android 15 每天限 6 小时）保持连接，只订阅 Agent 概要、所有终端不可见。Agent 进入 Waiting 时发本地通知，按 Agent generation 与状态去重，正在查看该 Agent 时不发，点通知打开对应项目、tab、pane。不接 FCM：无需推送服务器，自建 Relay 可用，没有 Google 服务的手机也能用。需引导用户关闭电池优化。

**已实现（2026-11-12，spike App，见 `docs/plans/2026-11-12-android-notifications-plan.md`）：**

- 连接从 Activity 移到进程级的 `Link`（Application 持有），Activity 与 `LinkService`（`specialUse` 前台服务）共用。概览经 Kotlin `StateFlow` 分发，Rust 侧每个会话仍只有一个监听者。Activity 进后台时 detach 终端，Host 只再推 Agent 状态。
- 去重用 `sessionId` + `stateStartedAtMs`（一次等待），代替 generation：`AgentSummary` 没有 generation，而重新进入等待时开始时间必然变化。失败不通知。
- 服务停止时撤掉它发过的所有通知，否则等待结束后通知还留在通知栏（模拟器上实测发现）。
- 模拟器验证：后台时 Agent 进入等待即收到高优先级通知（“Spike · Build / 等批准：Fix the failing build”），等待结束约 17 s 后撤销；点通知打开该 Agent 的终端；正在看某个终端时它不再通知，其他 Agent 照常；后台 5.5 分钟后仍能收到；点常驻通知的“停止”后服务退出、通知全部撤掉。
- App 在前台时，新的等待通知发到低重要性的渠道 `agent-waiting-quiet`：进通知栏，但不弹横幅挡住顶栏（首页本来就列出它）；在后台时仍用高重要性渠道弹横幅（2026-11-12 修复，模拟器验证）。
- 未验证：模拟器接着电源、没有进入 Doze；真机熄屏后的保活、厂商后台限制与电池优化豁免的效果需要真机测。

## 7. 未决问题

1. **终端控制权（已解决，2026-11-12）。** 原先 `TakeTerminalController` 会把所有 lease 移给接管方，也没有交还请求，与 `p2p-relay-architecture.md` §10.4 相矛盾。master 已改为按终端分配（`docs/plans/2026-11-12-terminal-control-per-session-design.md`）：`TakeTerminal { session_id }` 只接管一个终端，Host 记住交还给谁；`ReturnTerminals { session_ids }`（空列表 = 本设备单独接管的全部）交还。手机接管当前 pane、点“交还电脑”、进后台时交还全部（iOS 在 `beginBackgroundTask` 内，Android 在 `onStop`）；进程被杀来不及发时由 Host 的 60 s 宽限期兜底。主控不变，只观察的手机也不会在继任时被动成为主控。
2. **协议版本锁步（无需改协议）。** v22 起追加不升版本（`docs/wire-evolution.md`），旧端对不认识的请求回 `Unsupported` 而不断线，必须升版本时 Host 同时服务新旧版本。落后几个桌面版本的手机照样能连。移动端的约定：每个新请求都处理 `Unsupported`，隐藏或降级对应功能；例如旧 Host 不认识 `TakeTerminal` 时，`take_control` 返回 `HostTooOld`，App 提示升级电脑上的 yttt，不退回全部接管。
3. **iOS 上架限制。** `yttt-transport-iroh` 无条件启用 iroh 的 `fast-apple-datapath`，用到私有 API `sendmsg_x`/`recvmsg_x`，有被 App Store 拒审的风险，需要改为按 target 启用。portmapper 的 UPnP 在 iOS 上需要 multicast entitlement，移动端关掉。局域网直连需要 `NSLocalNetworkUsageDescription`。
4. **网络变化（已接，2026-11-12，见 `docs/plans/2026-11-12-mobile-link-plan.md`）。** `HostSession.network_changed()` 让 iroh 重读网卡（`IrohEndpoint::network_change`）并让 ClientCore 立即探测连接（`probe_now`，与桌面 `link_watch` 相同）。原生侧只在网络**变为可用**时调用（Android `onAvailable`/`onLinkPropertiesChanged`，iOS 路径 `.satisfied`），去抖 500 ms。断网时不调用：探测得不到回音会换掉本来能撑过短暂断网的 QUIC 连接，模拟器上实测因此更慢。Android 上 SELinux 不让 App 读 netlink 与 `/sys/class/net`，iroh 自己看不到网卡变化，只能靠原生回调与 `probe_now`；Wi-Fi 与蜂窝之间换 IP 的情形需要真机验证。正式 App 里这个入口应放在 `MobileCore` 上，转给所有 endpoint。
6. iOS 推送：APNs 只能由 App 发布者发送，需要官方推送网关，暂缓。
7. Relay 账号登录：现有 OAuth 回调只到 loopback，手机先用设备码流程。不登录账号时，配对得到的 relay 提示只对自建 relay 有效。

## 8. Spike 结论（2026-11-12）

三个问题的答案都是“可以”，Android 的终端组件换了一个。

**1. 编译与连接。** `yttt-mobile-core`（client-core + iroh + UniFFI）能编译到 iOS（真机与模拟器）和 Android arm64，iOS 模拟器与 Android 模拟器（NAT 之后）都经真实 P2P 配对并连上 Mac 上的 Host。需要的改动：

- iroh 的 `fast-apple-datapath`、`portmapper` 只在桌面启用（§7 问题 3 已落地）。
- iOS：`IPHONEOS_DEPLOYMENT_TARGET` 必须与 C 依赖一致（17.0），否则链接缺 `___chkstk_darwin`。
- Android：vendor 的 alacritty 用了 Android libc 没有的 `shm_open`。核心只需要它的样式位和模式位常量，改为本地常量加一致性测试，alacritty 与 terminal-core 不再进 App。
- 体积：Android `.so` strip 后 18.4 MB（gzip 约 7.1 MB）；iOS staticlib 80 MB（未 strip，App 里 dead-strip 后的体积未测）。依赖树里 ring 与 aws-lc、reqwest 0.12 与 0.13 并存，有裁剪空间。

**2. VT 重绘。** 正确且足够快。

- 正确性：Rust 往返与字节级测试 18 项；SwiftTerm 的 XCTest 用共享夹具（`mobile/fixtures/vt-fixtures.json`）逐格比较文字、宽字符、光标、颜色、样式，通过；两个模拟器上截图确认颜色、中文、emoji、实时刷新、接管后的输入与 PTY 改为手机尺寸。
- 性能：`seq 1 200000`（Host 上 0.75 s）在客户端只合成 49 帧、244 KB，drain p50 56 µs、p95 102 µs、max 323 µs（release，桌面）。iOS 模拟器 60 fps、drain p95 约 30 µs；Android 模拟器 60 fps、drain p95 83–190 µs（debug）。
- 按屏幕刷新拉取的方式成立：空闲时每帧只比较一次 sequence。

**3. Termux 组件不适合。** `TerminalSession` 是 `final`，其 `initializeEmulator` 必定经 JNI 启动本地子进程，`TerminalView` 只接受 `TerminalSession`，接远程数据源必须 fork；且 termux-app 为 GPLv3 only，例外条款只说其中用了 jackpal 的 Apache 代码，Termux 自己的修改是否 Apache 不明。改用 **ConnectBot termlib**：Apache-2.0、Compose 原生、为 SSH 这类远程数据源设计（`writeInput` 喂字节，`onKeyboardInput` 回调按键），有 `forcedSize` 固定行列，不需要 fork。

**对正式实现的修改与待办：**

1. `client_instance_id` 每次连接新生成（§5）。审查时建议的“按 Host 持久化”会让 App 重启后 5 分钟内连不上，spike 中实际撞到。
2. 控制权（§7 问题 1）与协议锁步（§7 问题 2）仍需另立协议计划。
3. Android 观察模式用 `forcedSize` 时字号缩到很小，需要横向滚动或双指缩放；iOS 用横向 ScrollView 包住 Host 尺寸的视图可行。
4. ~~接管后要等 resize 确认再放行输入~~（已做）：`take_control(session, cols, rows)` 在 Host 应用手机尺寸后才返回，两个模拟器上第一条命令已看到新尺寸（iOS 58×53、Android 63×64）。软键盘弹出导致的尺寸变化可用 termlib 的 `resizeSuspended` 合并（未做）。
5. UniFFI 导出避开 `close`、`message` 这类名字：Kotlin 里与 `AutoCloseable.close`、`Throwable.message` 冲突。
6. termlib 没有公开的屏幕读取 API，组件测试暂时只能截图；可向上游提或在 instrumented 测试里读无障碍语义。
7. ~~Android 上要做 `rustls-platform-verifier` 的 JNI 初始化~~（不需要）：`yttt-transport-iroh` 构建 iroh 时没开 `platform-verifier`，iroh 与 reqwest 都用内置的 webpki-roots，验证器链接进来但从不调用。Android 模拟器只经 iroh 公共 HTTPS relay（`aps1-1.relay.n0.iroh.link`）完成了配对、接管与输入。代价：不认用户自装或企业 CA，私有 CA 的自建 relay 在桌面和手机上都连不上；要支持时再开 `platform-verifier` 并做 JNI 初始化（坑见链路计划附注）。
8. 未验证：真机、蜂窝网络（Wi-Fi 与蜂窝之间换 IP）、vim / htop 等全屏程序、方向键与 Ctrl-C 等按键栏按键、Agent CLI。已验证：只经 relay 的连接（集成测试用进程内自建 relay 并断言路径始终为 relay、手机流量经过 relay；iOS 模拟器与 Android 模拟器各经本机自建 relay 与公共 HTTPS relay；默认模式下两者在局域网内都转为直连）；电脑取回控制权（集成测试）；前后台切换与断网恢复（模拟器）。
11. ~~经远处 relay 配对很慢~~（未复现）：第一次经 `aps1-1` relay 配对用了约一分钟，之后复测四次（本机 relay 与公共 HTTPS relay，iOS 与 Android 各一次）都在 2–6 s 内完成。那一次紧跟在一次 `InvitationClosed` 失败之后，原因未查明；真机上留意。
9. **本地 geometry epoch 只增不减。** 修复前 `drain` 用收到的 viewport 覆盖本地 geometry epoch：接管后的 resize 请求还在路上时，显示刷新把旧 viewport 再编码一次，epoch 退回旧值，之后的输入被 Host 当作过期而静默丢弃（输入是单向消息，手机收不到拒绝）。现在同一 session 内取较大值，集成测试在 `take_control` 期间每毫秒 drain 一次来覆盖。
10. **termlib 改变固定尺寸会清屏。** 从控制回到观察（`forcedSize` 改回 Host 尺寸）后画面变空，直到下一次输出。核心新增 `repaint(session)`，Android 在控制状态变化两帧后调用，下一次 drain 整屏重绘。SwiftTerm 不清屏，iOS 不需要。
