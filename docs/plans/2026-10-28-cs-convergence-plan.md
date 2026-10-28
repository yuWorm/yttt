# yttt C/S 收敛演进计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划,不要把计划执行委托给 subagent。每项先写能复现缺失行为的测试,再做最小实现;只有验收条件实际通过后才能勾选。

**状态：** C1、C2、C3 完成；C4 设计已落盘（[`2026-10-28-c4-host-owned-layout-design.md`](./2026-10-28-c4-host-owned-layout-design.md)），待评审后实施。v22 在 C4 完成前不发布。

**依据:** [`2026-10-28-cs-convergence-analysis.md`](./2026-10-28-cs-convergence-analysis.md)(对照 herdr 的根因分析)、`docs/host-client-architecture.md`、`docs/wire-evolution.md`。

**目标:** 把 Host/Client 之间"多条连接 + 字节流同步 + 精确版本 + 布局两边对账"收敛为"稳定可演进协议 + 服务端维护基线 + 单连接 + 布局归 Host"。每一阶段独立可发布,完成后都能删掉一批补偿逻辑。

---

## 执行约束

- [ ] C1 → C2 → C3 → C4 顺序执行;C3 依赖 C2 的渲染槽,C4 动工前必须先有设计文档。
- [ ] 不改变 Host 的资源所有权模型:Host 仍是 terminal、SSH、项目后端和 Agent 状态的唯一权威。
- [ ] 不削弱现有认证:本机 IPC、TLS、SSH 入口的认证与权限检查保持不变。
- [ ] 所有队列保持硬上限;慢 Client 不阻塞 PTY reader、其他 Client 或 Host 资源循环。
- [ ] 每阶段结束时列出被删除的计数器/栅栏/通道;只加不删视为阶段未完成。
- [ ] 只有行为测试与真实冒烟实际通过后才能勾选。

---

# C1:协议分层与跨版本兼容

**问题:** Host 只接受完全相同的资源协议版本;launcher 只要 build 指纹不同就替换 Host,Host 忙则启动失败;Host 收到解不开的请求、Client 收到解不开的事件都会断线。结果是任何协议追加都要升版本,任何升版本都要求桌面、Host、远端 Server、CLI 同步升级,Independent Host 撑不过桌面升级。

**主要区域:** `crates/yttt-protocol`(codec、control、handshake)、`crates/yttt-transport/src/wire.rs`、`crates/yttt-host/src/lib.rs`、`src/host_launcher.rs`、`crates/yttt-server/src/lib.rs`、`crates/yttt-client-core/src/lib.rs`。

## C1-1:launcher 以协议协商结果决定 attach

- [x] 回归测试:不同 build、相同协议、正在运行项目的 Independent Host,`launch_or_attach` 直接 attach,不 spawn、不停止旧 Host。
- [x] `existing_host_action` 只看握手结果:协商成功 → `Attach`(或 lifetime 升级时 `ReplaceLifetime`);`VersionMismatch` → `ReplaceProtocol`。删除 `ExistingHostAction::ReplaceBuild` 与 build 指纹比较。
- [x] 删除无生产者的 `RejectReason::BuildMismatch` 及各处匹配分支。
- [x] `BuildIdentity` 只用于诊断与握手 transcript,不参与兼容判断。

**验收:** 旧 build 的忙碌 Host 不再阻止新桌面启动;协议不兼容时仍走"空闲才替换、忙碌报 blockers"的既有路径。

## C1-2:解不开的消息按类型降级,不断线

- [x] 协议层:对 control、terminal-interactive、state-event 帧,typed 解码失败但外层信封可识别时,返回 `ProtocolCodecError::Unrecognized`,区分 `Request { request_id }`、`Response { request_id }`、`Event`;外层信封也不认识时仍是 `Decode` 错误。
- [x] 新增 `FailureCode::Unsupported` 与 `HostResponse::unsupported(request_id)`。
- [x] 传输层接收策略:未知事件跳过;无法解析的响应转成该 `request_id` 的 `Unsupported` 失败;无法解析的请求以 typed 错误交给 Host。
- [x] Host 的 control 与 interactive 通道对无法解析的请求回复 `Unsupported`,连接继续服务后续请求。
- [x] 测试覆盖:未知请求得到 `Unsupported` 且同一连接的下一请求正常;未知事件被跳过、下一事件正常交付;未知响应变为对应请求的 `Unsupported`;外层信封未知仍断线。

**验收:** 新增请求或事件不再要求升级协议版本;旧端只失去该功能。

## C1-3:资源协议 v22 作为稳定基线

- [x] `RESOURCE_PROTOCOL_VERSION` 升到 22。v22 起,追加请求/响应/事件/可选字段不再升版本;删除、改名、改变已有字段语义才升版本,并由 Host 声明区间同时支持新旧版本。
- [x] lifecycle 协议保持 v3:它是新 launcher 停止旧 Host 的唯一通道,集合封闭,改动必须升版本并由 Host 同时支持旧版本。
- [x] 能力列表(`HostReady.capabilities`)推迟到第一个需要在发请求前做 UI 门控的功能出现时再加:字段带 `#[serde(default)]`,缺省为空集;在此之前 `Unsupported` 已足以降级,不提前加入没有消费者的字段。

## C1-4:冻结 wire 标签与字段

- [x] 测试从 serde 元数据提取资源协议与 lifecycle 协议所有 wire 类型的变体名和字段名,与 `crates/yttt-protocol/tests/fixtures/wire-shapes-v22.json` 精确比较。(探针反序列化器读取 derive 的静态名称表;`serde_json::Value`、内部标签、untagged、flatten 子树只经 `deserialize_any`,保持不透明。)
- [x] 失败信息说明:删除/改名 = 破坏兼容,需要升版本;追加 = 兼容,新增结构字段必须带 `#[serde(default)]`,然后用 `YTTT_BLESS_WIRE_SHAPES=1` 重写样本。
- [x] desktop-shell 协议不纳入(桌面与 CLI 同 build,按版本精确匹配)。

## C1-5:文档

- [x] `docs/wire-evolution.md` 改写兼容规则与 v22 基线;`docs/host-client-architecture.md` 同步版本号与 launcher 行为;`CHANGELOG.md` Unreleased 记录用户可见变化(最后一次要求同步升级)。

**C1 删除项:** `ExistingHostAction::ReplaceBuild`、`host_build_requires_replacement`、`RejectReason::BuildMismatch`、"每次追加都升版本"的流程。

---

# C2:服务端维护基线,只推可见终端

**动工前:** 写设计文档,回答下列问题,并用 perf-metrics 记录当前高输出终端与多 Client 场景的基线数据。

- Host 为每个"Client × 终端"保存的基线是什么:语义 viewport(行 + 光标 + 模式)还是行哈希?内存上限与淘汰策略?
- diff 粒度:行级还是单元格 span?滚动是否单独编码?
- 可见性如何上报:每窗口一份可见 session 集合,带 revision;Host 对不可见终端只发生命周期/标题/Agent 事件。
- 发送失败(队列满)时基线不推进,下一次发全量;全量帧上限与分块策略沿用 `MAX_FRAME_BYTES`。
- 迁移:新旧同步模型能否在 v22 内以新请求/新事件追加(按 C1 规则),旧 Client 继续走 checkpoint 路径直到下线。

**完成标志:** Client 侧删除 `SequenceGap` 处理与 checkpoint 请求通路;不可见终端不再产生帧流量;慢 Client 测试证明基线不推进且恢复后一次全量追平。

**C2 删除项:** Host 每终端原始 PTY 回放环(最多 8 MiB)及其订阅容量切换;Host 发送 `ResyncRequired`;ClientCore 的 checkpoint 补快照请求、`SequenceGap`→checkpoint 恢复、catalog 刷新触发的 checkpoint、scroll/viewport 响应写入 mirror;checkpoint resync 诊断计数(由 `data_lane_resets` 取代)。设计与验收数据见 [`2026-10-28-c2-terminal-sync-design.md`](./2026-10-28-c2-terminal-sync-design.md)。

# C3:单连接 + 逻辑通道

**前置:** C2 完成(渲染槽 = 每终端只保留最新一帧)。

- 帧头加逻辑通道 id(帧格式版本升级,握手协商);每个 Client 一条连接、一次认证。
- Host 发送侧:有序可靠队列(响应、状态事件、终端生命周期事件)+ 每终端渲染槽 + 输入优先。
- 慢请求并发执行,大响应分块。
- **完成标志:** 删除 terminal-interactive、state-events、terminal-data 独立连接及其各自的 epoch 校验与重连状态机;0.3.16 引入的乱序栅栏由队列顺序取代并删除。

**C3 删除项:** `ConnectionChannel::{Control, TerminalInteractive, StateEvents, TerminalData}`、`TerminalInteractiveMessage` 及其收发函数;Host 的 interactive/state-event/terminal-data 独立连接入口、每终端 512 KiB `TerminalDataWriter`、`SessionBinding.connections`/`control_connected`;ClientCore 的 `SessionLanes`、跨连接 epoch 重试、每终端 data worker 与 generation 栅栏、`deferred_exits`、`data_lane_resets`。实际做法与计划原文的差异(不加帧头通道 id、慢请求并发未做)及验收数据见 [`2026-10-28-c3-single-connection-design.md`](./2026-10-28-c3-single-connection-design.md)。

# C4:布局归 Host

**前置:** 单独设计文档并评审。

- Host 持有 workspace 树与 pane→session 绑定;Client 发操作意图(split/close/move/focus),Host 按序执行并广播布局 revision。
- Client 只保留 focus、选区、滚动位置、主题与编辑器 UI 状态。
- 编辑器草稿按单文档 revision 存 Host(乐观并发),与 workspace CAS 解耦。
- **完成标志:** 删除 profile 级单写入者、控制权交接(Preparing/Fencing、5 秒强制确认)、ControlLost 冻结与设备恢复副本中只为布局对账存在的部分。

---

## 进度记录

| 日期 | 阶段 | 结果 |
|---|---|---|
| 2026-10-28 | 分析与计划 | 落盘本计划与分析文档 |
| 2026-10-28 | C1 | 协议/传输/Host/launcher 落地;protocol、transport、transport-local、host、client-core、server 套件与 `process_roles`、`desktop_tray_lifecycle` 通过;真实二进制冒烟:不同 build 的 `--start-host` 复用同一 Host pid |
| 2026-10-28 | C2 | 发送基线/渲染槽/可见性上报落地;workspace 测试除 `desktop_tray_lifecycle` 外全部通过(该套件在 `cargo test` 下 Host 8 s 未就绪,改造前 worktree 同样失败,直接执行测试二进制通过);流量探针:observer 全部隐藏时输出期间 0 字节,重新可见一次 50 KB;真实桌面冒烟:隐藏的高输出终端 PTY 吞吐约 2.3 倍,桌面 CPU 4–5% → 1–2% |
| 2026-10-28 | C3 | 单 Session 连接 + 三级写端调度 + 大消息分块落地;workspace 测试除 `gpui-component` 的 `drag_inside_dialog_still_selects_its_text`(vendor,单独重跑通过)与 `desktop_tray_lifecycle`(同 C2,直接执行测试二进制通过)外全部通过;探针帧率、字节/帧、输入延迟与 C2 持平;真实桌面冒烟:每 Client 一条 Session 连接,退出桌面后 Host 约 300 ms 退出 |
