# yttt 本地 Host 与远程能力后续实施清单

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。使用 `@superpowers:executing-plans` 在当前会话内按顺序执行本清单；不要把计划执行委托给 subagent。每项先写能够复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** Phase 1 本地 C/S Definition of Done 已于 2026-08-12 完成；公网远程、tray、移动端和协作能力尚未开始。

**目标：** 先补齐本地 Host 的正确性和可验证性，再依次实现桌面后台控制面、远程桌面、移动端和协作能力；后续阶段不得绕过同一个 Host 资源所有权模型。

**依据：** 原批准的《yttt 单二进制双进程本地 Host 实施计划》与 `docs/p2p-relay-architecture.md`。本文保留已完成的 Phase 1 验收记录，并继续跟踪尚未开始的 Phase 2–5；不重复 profile、Host role、local IPC、Host-owned PTY/SSH/project 和 semantic mirror 的基础设计。

---

## 执行约束

- [x] 严格按 Phase 1 → Phase 5 执行；Phase 1 全部通过前，不开始公网远程产品实现。
- [x] 保持一个发布 executable、desktop/Host 两个进程角色；除非后续实测证明 headless/tray RSS 或平台装载约束不可接受，不增加第二个 binary。
- [x] Host 始终是 terminal、SSH、项目后端和 Agent 运行状态的权威节点；desktop 只持有 presentation state 和有界 mirror。
- [x] 每个新 wire 行为都携带稳定资源 ID、epoch/sequence、typed failure、长度上限和取消/超时语义。
- [x] 每个 mutating request 都必须有幂等策略；含 password、passphrase 等 secret 的请求不得进入可重放 journal。
- [x] 所有 queue/channel 必须有硬上限和 overflow/resync 行为；禁止用无界队列隐藏 IPC backpressure。
- [x] 文档只能把已经通过行为测试、真实 smoke 和相应平台验证的能力标为已完成。

---

# Phase 1：补齐本地 C/S Definition of Done（已完成）

## P0-1：实现 desktop 重启后的真实 terminal re-attach

**主要区域：** `src/ui/terminal/pane.rs`、`src/ui/workbench/`、`crates/yttt-client-core/`、`crates/yttt-protocol/src/terminal.rs`

- [x] 增加真实进程回归：desktop/client detach 后，Host terminal 继续输出；新 client 使用相同 `TerminalSessionId/session_epoch` attach，并能继续输入。
- [x] GUI 启动 pane 时先用 catalog 和 durable binding 决定 `AttachTerminal` 或 `SpawnTerminal`，不得无条件 spawn。
- [x] 匹配的既有 session 走 attach/checkpoint；只有从未打开或确认允许重建的 placement 才能 spawn。
- [x] 同地址不同 spawn fingerprint 返回 typed `AddressConflict`，不得覆盖或终止旧 terminal。
- [x] attach 后恢复 lease、query palette、geometry 和最新 semantic checkpoint，再允许 input。
- [x] Host epoch 已变化或原 session 缺失时显示 `Lost`，不得静默创建新 shell 冒充恢复成功。

**验收：** 关闭最后窗口后 Host 和 local/SSH terminal 保持；重新启动 desktop 后，同 pane 显示关闭期间的输出，session ID/epoch 不变，cwd 和进程未重置。

## P0-2：实现 durable placement、catalog reconciliation 和 orphan recovery

**主要区域：** `src/config/layout_config.rs`、`src/ui/workbench/state/`、`src/ui/workbench/`、`crates/yttt-client-core/`

- [x] 持久化 `NeverOpened`、`OpenPending`、`Bound`、`ClosePending`、`Lost`、`Closed` 状态。
- [x] `Bound` 保存 host epoch、session ID、session epoch 和 spawn fingerprint。
- [x] `ClosePending` 保存原 request ID；desktop crash 后只重放原 terminate，不得重新 open。
- [x] startup reconciliation 先读取 Host catalog，再创建 pane attachment。
- [x] catalog 中没有 placement 的 session 作为 `RecoveredTerminal` 暴露，可 attach 或显式 terminate。
- [x] address/spec 冲突、stale epoch 和 missing resource 均显示 typed recovery error。
- [x] 增加 Host crash、terminate-ack 前 desktop crash、catalog orphan 和 stale binding 测试。

**验收：** 任意 desktop 退出点都不会造成重复 shell、隐藏 orphan 或 layout 与 Host catalog 静默分叉。

## P0-3：把显式关闭改为 Host-confirmed transaction

**主要区域：** `src/ui/workbench/document_lifecycle.rs`、`src/ui/workbench/action_handlers.rs`、`src/ui/workbench/mod.rs`、`crates/yttt-client-core/`、`crates/yttt-host/`

- [x] `TerminateMany` 返回每个 session 的 typed result，不使用单一 aggregate success。
- [x] pane/tab/project close 在 dirty Save/Discard 确认完成后才写入 `ClosePending` 并发送 terminate。
- [x] 用户选择 Cancel 时不得发送任何 Host mutation。
- [x] UI 等待 Host ack 后才删除 layout；失败项恢复 `Bound`、保留 pane 并显示错误。
- [x] pending 期间禁用重复 close；request retry 使用同一 request ID。
- [x] process-exit auto-close 不重复 terminate；view drop/client disconnect 只 detach。
- [x] 覆盖 pane、tab、project、window close 以及部分批量失败。

**验收：** Host 未确认 terminate 的资源始终在 UI/layout 中可见，任何失败都不会产生仍在运行但不可管理的 terminal。

## P1-1：完成多客户端 attachment 和独立 presentation state

**主要区域：** `crates/yttt-protocol/src/terminal.rs`、`crates/yttt-host/src/terminal.rs`、`crates/yttt-client-core/`

- [x] `AttachTerminal` 显式区分 `Observer` 与 `Interactive`；observer 不抢占已有 input/resize lease。
- [x] 多个 observer 与一个 interactive owner 可同时 attach 并持续接收 frame。
- [x] scroll viewport、search generation、selection cache、focus 和 unseen-output 归 client attachment，不修改其他 client 的 presentation state。
- [x] Host canonical geometry 只接受当前 lease holder 的 resize；其他客户端 letterbox/local scale。
- [x] 增加基于 stable line ID 的 `ReadViewport`、`Search`、返回 bottom/checkpoint 和 typed stale 行为。
- [x] lease grant/transfer 后先同步 query palette 和 geometry，再开放 input。
- [x] 增加两个不同窗口尺寸客户端和一个慢 observer 的集成测试。

**验收：** 两个 desktop 可同时观察同一 terminal；一方滚动、搜索、选择或 focus 不改变另一方，且同一时刻只有一方能 input/resize。

## P1-2：补齐 epoch 校验、request journal 和数据通道

**主要区域：** `crates/yttt-protocol/`、`crates/yttt-transport-local/`、`crates/yttt-host/`、`crates/yttt-client-core/`

- [x] input、resize、scroll、query palette、clipboard reply 带 host/session/lease/geometry epoch 和 client sequence。
- [x] Host 拒绝 stale lease、stale session、stale geometry、重复 input sequence 和旧 external event。
- [x] 为每 client 实现有界 request journal；重复 spawn/terminate/file mutation/ack 返回原结果，不重复副作用。
- [x] secret-bearing 请求不进入 journal；断线后由 UI 明确重新提交。
- [x] control connection 只承载 RPC/catalog/lifecycle；每个 terminal attachment 使用独立 data connection。
- [x] 每 attachment live output 上限为 512 KiB；overflow 只清理该 attachment 的 delta，并发送唯一 `ResyncRequired`。
- [x] sequence gap、geometry/scrollback epoch 变化立即停止增量 merge 并请求 checkpoint。
- [x] 增加 arbitrary frame-boundary disconnect、retry、malformed/oversized frame 和 slow-client 测试。

**验收：** request retry 不重复任何可观察副作用；一个 slow client 不阻塞 PTY/parser、其他 client、host-key 或项目 RPC。

## P1-3：完成 exited-session 保留和 Host lifecycle policy

**主要区域：** `crates/yttt-protocol/src/control.rs`、`crates/yttt-host/src/lib.rs`、`crates/yttt-host/src/runtime.rs`、desktop Host supervisor

- [x] exited terminal 保留 final checkpoint/status，直到匹配 epoch/final sequence 的 ack 或 10 分钟无 client TTL。
- [x] exited-unacked resource 进入 Host blocker 集合；TTL 后才允许 idle countdown。
- [x] 实现 `StopIfIdle`：原子检查完整 blocker，Busy 时零资源被停止并返回 typed blocker 列表。
- [x] 实现 `DrainAndStop`：拒绝新资源，等待 running/exited-unacked blocker 自然清空，不立即 `terminate_all`。
- [x] 实现经认证的 `ForceStop`，并要求 UI 二次确认。
- [x] 无 client 且无 blocker 后启动 30 秒 idle exit；新 client/resource 到来取消 countdown。
- [x] 版本/build 不兼容且 Host busy 时保留旧 Host；idle 时通过 `StopIfIdle` 安全升级。
- [x] 增加 Host crash、child 在 desktop 离线时退出、版本不兼容和 stop/drain blocker 测试。

**验收：** 普通 stop/update 不会误杀 terminal；Host crash 明确产生 `Lost`，而不是自动重建旧 session。

## P1-4：把 Agent reconnect snapshot 真正放入 Host

**主要区域：** `crates/yttt-agent-core/`、`crates/yttt-agent-runtime/`、`crates/yttt-host/`、`crates/yttt-protocol/src/agent.rs`、`src/runtime/agent_manager.rs`

- [x] Host hook ingress 将请求转换为 framework-neutral `AgentEventKind`。
- [x] Host 按 terminal/resource epoch 和 sequence 维护有界 `AgentReducer` state，保留最新 snapshot，不保留无界 event log。
- [x] 新 client attach/reconnect 时请求最新 `AgentSnapshot`，并从已 ack sequence 后继续。
- [x] broadcast lag 不得静默丢失状态；触发 snapshot resync。
- [x] desktop `AgentManager` 只负责 UI/persistence，不再作为 hook listener 或运行状态权威。
- [x] 删除 PID polling 作为 Agent 生命周期权威；process running/exited 来自 Host terminal state。
- [x] Host 拒绝 client 伪造 `YTTT_AGENT_HOOK_*`，每 terminal generation 使用 scoped token。
- [x] 增加 desktop 离线期间产生 hook、重开后状态连续且不重复应用的真实进程测试。

**验收：** desktop 不在线时 Agent 状态仍持续更新；重开后 UI 直接得到连续 snapshot，无需依赖本地 PID 推断。

## P1-5：增加安全 diagnostics 和无订阅优化

**主要区域：** `crates/yttt-host/`、`crates/yttt-client-core/`、terminal performance instrumentation

- [x] 输出 Host epoch、session/client/attachment 数量、RSS、thread count 和 idle CPU。
- [x] 记录每个 queue 的 current/high-water、drop/resync、run-end backlog。
- [x] 记录 parser、semantic encode、IPC write/read、client merge 和 input-to-PTY 分段延迟。
- [x] 日志 profile-scoped 且轮转；不得记录 input、env value、password、clipboard 或文件内容。
- [x] Host 无 subscriber 时继续 drain/parse PTY，但不 capture/encode semantic frame。
- [x] 多 subscriber 共享不可变 encoded frame，避免每 client 重复 encode 和无谓复制。
- [x] 为 tests 提供内存 diagnostics sink 和 fake clock。

**验收：** 无客户端输出 workload 的 semantic encode 计数为零；所有 queue 都能证明有上限且测试结束 backlog 为零。

## P1-6：完成性能、资源和三平台验收

**主要区域：** `scripts/run-terminal-perf.sh`、`scripts/summarize-terminal-perf.py`、`tests/performance_metrics.rs`、三平台 packaging/CI

- [x] perf runner 支持 `--backend direct|host|both`，两条路径使用相同 workload、窗口、warmup、duration 和 release build。
- [x] 同机至少 5 runs，保存 raw JSON/trace，并报告 median/p95/p99。
- [x] 覆盖 damage、full、scroll、11 MiB burst、interactive echo。
- [x] 覆盖 Host-only 0 pane、1/4/16 local pane、1 SSH pane。
- [x] 覆盖 1 client、2 client、1 live + 1 intentionally slow client。
- [x] 覆盖 desktop open、desktop exited、desktop reopened。
- [x] 每个 interactive run 至少采集 600 个 input-to-PTY 和 600 个 echo-to-paint 样本。
- [x] 强制原计划门槛：吞吐退化 ≤10%，input-to-PTY p95 ≤0.5 ms，echo-to-paint 相对 baseline 增量 ≤3 ms，约 60 FPS。
- [x] 强制 final sentinel 1 秒内 merge/paint、2 秒观察窗结束全部 backlog 为零。
- [x] 强制 Host-only idle CPU <0.5%，combined one-pane idle RSS 增量 ≤15 MiB。
- [x] macOS 验证 bundle 内同一 executable 的 Host survive/reopen attach。
- [x] Windows 验证 named-pipe DACL、GUI subsystem、单 executable installer 和 busy-Host upgrade abort。
- [x] Linux 验证无 DISPLAY/Wayland/DBus 的 Host role、单 executable tar 和 runtime-dir 权限。

**验收：** summarizer 对缺失 metric、低样本、非有限值、catch-up 超时、queue saturation 或非零 backlog 必须非零退出；三平台 smoke 均有可追溯结果。

## P1-7：完成 clean cutover 和文档纠偏

- [x] root desktop 没有 direct PTY、SSH terminal handle、UI-owned hook listener 或 Host 失败后的本地 fallback。
- [x] 删除残留迁移开关、deprecated alias、双实现和依赖 `Drop` 的正常 kill。
- [x] 增加 architecture contract tests：production terminal 必经 Host、window close 不 terminate、explicit close 必经 transaction。
- [x] 跑 `cargo fmt --check`、`cargo test --workspace`、严格 clippy 和三平台 packaging/process smoke。
- [x] 完成 local shell、SSH/SFTP/Git/host-key、Agent reconnect、双 client、慢 client、desktop reopen 和 Host crash 产品 smoke。
- [x] 修正架构文档中的“本地 C/S 完整实现”状态；只有本 Phase 全部验收通过后才能恢复该表述。

---

# Phase 2：桌面后台控制面（已完成）

## P2-1：完整 tray/menu-bar 生命周期

- [x] desktop shell 使用 `QuitMode::Explicit`；关闭最后窗口后 desktop 可无窗口驻留。
- [x] 引入独立 tray adapter；tray 归 desktop shell，Host 保持纯 headless，不依赖 AppKit/Win32/GTK event loop。
- [x] 支持 Open/New Window、Host 状态、active terminal/client/job 计数、Open Logs。
- [x] 支持 Start Host、`StopIfIdle`、Restart Host、Quit Desktop、Quit All；所有动作只调用 Host protocol/lifecycle supervisor，不直接 kill PID。
- [x] tray crash 或 desktop force-exit 后 Host 和资源继续运行。
- [x] Linux 无 tray/AppIndicator 环境提供 Settings/CLI fallback，不把 tray 作为唯一控制入口。
- [x] 测量 Host-only、Host + no-window GPUI/tray、reopened window 的资源占用和重开延迟；macOS release physical footprint 分别为 4.55 MiB、≤59.91 MiB 和 111.10 MiB，重开窗口 171.2 ms，当前不引入第三个轻量 tray 进程。

## P2-2：登录启动和平台后台注册

- [x] 首次启用远程访问时明确询问用户，不在安装时静默注册。
- [x] macOS 13+ 使用 `SMAppService`，验证签名、更新和 Host role 参数传递。
- [x] Windows 使用 per-user startup registration，不创建 machine-wide/Session 0 service。
- [x] Linux 优先 systemd user service，无 systemd 时使用 XDG autostart。
- [x] 注册项携带稳定 profile 和 Host role，不携带 secret。
- [x] 真实 macOS 注册 smoke 使用唯一临时 bundle/LaunchAgent identity 和隔离 profile，验证注册、更新、Host 启动、注销及资源清理；普通自动化测试只使用 fake backend，`scripts/run-login-startup-smoke.sh` 强制 disposable 环境 opt-in。

---

# Phase 3：远程桌面与公网传输

## P3-1：冻结远程 wire 与 transport abstraction

- [ ] 将 local transport 与资源协议解耦；同一 Host contract 可运行在 local IPC 和远程 QUIC 上。
- [ ] wire schema 显式版本化并支持 capability negotiation、unknown-field/message 跳过和升级兼容。
- [ ] 所有 Host 绝对路径转换为 platform-neutral opaque resource/path 表示。
- [ ] 为 terminal、project、SSH、Agent、lifecycle 分配独立 stream/priority，避免大文件阻塞控制面。
- [ ] 定义断线 resume token、checkpoint、event journal window 和 full resync 边界。

## P3-2：Iroh/P2P、Relay 和端到端加密

- [ ] 接入 Iroh/QUIC，优先 P2P 直连并支持 NAT traversal。
- [ ] 直连失败时自动回退 dedicated/self-hosted Relay。
- [ ] Relay 只转发端到端加密字节，不持有项目明文、terminal 明文或长期工作区状态。
- [ ] 本地客户端保持 local IPC fast path，不绕公网 Relay。
- [ ] 覆盖连接切换、网络抖动、Relay failover、限流和大输出 backpressure。

## P3-3：设备身份、配对和 capability ACL

- [ ] 生成并安全保存 device identity；定义设备撤销和密钥轮换。
- [ ] 实现显式跨设备 pairing，防止未授权 discovery 自动获得 Host 能力。
- [ ] capability ACL 至少区分 observe terminal、control terminal、read files、write files、Git、Host lifecycle。
- [ ] 每个 remote client 有稳定 identity、审计字段、lease 和撤销语义。
- [ ] pairing/ACL 状态与普通 profile sync 分离并加密保存。

## P3-4：远程桌面端到端能力

- [ ] 远程 PC 可以列出 workspace/project、attach terminal observe/control。
- [ ] 支持文件树、bounded/streaming read、CAS save、mutation 和 watcher journal resync。
- [ ] 支持 Git status/diff/mutation，所有结果绑定 project/resource epoch。
- [ ] 支持 SSH-backed project；remote client 不获得 Host SSH handle 或 credential。
- [ ] 支持断线恢复、Host/desktop 升级兼容和 typed capability denial。
- [ ] 增加 direct P2P 与 Relay 路径的 terminal 延迟、文件吞吐、RSS 和慢客户端矩阵。

---

# Phase 4：移动端与可选浏览器客户端

- [ ] 移动端完成设备配对、workspace 和文件树浏览。
- [ ] 实现文件只读和增量刷新。
- [ ] 实现 terminal observe，断线/后台后从 checkpoint 恢复。
- [ ] 实现显式 terminal control lease 转移；移动端后台时自动释放控制权。
- [ ] 实现单 writer 文档编辑和 CAS conflict UI。
- [ ] 实现 `ContinueHere`、handoff 和 per-client presence。
- [ ] 实现 APNs/FCM 唤醒；后台任务按随时被系统终止设计。
- [ ] 可选浏览器客户端先走 Relay/WebTransport 类兼容路径；浏览器 P2P 直连不作为首版目标。

---

# Phase 5：协作、文档权威和离线能力

- [ ] 将 unsaved document draft 迁为 Host-owned resource，带 revision、single-writer lease 和恢复 snapshot。
- [ ] 实现 PC/移动端之间显式 writer handoff，dirty draft 未处理时不得静默抢占。
- [ ] 增加多用户 presence/cursor 和资源级审计。
- [ ] 只有明确产品需求后再选择 CRDT 或 OT；先定义冲突、不变量和存储成本基准。
- [ ] 实现离线编辑、重连合并和不可自动合并时的显式冲突 UI。
- [ ] 评估独立 Git worktree、项目级 sandbox 和云端工作区副本；每项单独设计和批准，不与基础远程访问捆绑。

---

## 非默认后续项：需要单独证据或产品批准

以下不计入当前默认交付清单，不能为了“完整”而顺带实现：

- Host/机器重启后从磁盘恢复原 PTY 进程。
- 第二个 `yttt-host` executable 或纯 headless 安装包。
- terminal cell-level 二进制压缩。
- Relay 保存工作区副本。
- 不受信任多租户的项目级 shell sandbox。
- 多人同时编辑和离线 CRDT 自动合并。

触发条件分别是：平台装载或 RSS 数据证明单 binary 不可接受；row-level semantic delta 的带宽/CPU 基准失败；或产品明确批准协作、云副本、沙箱和持久 PTY 的安全模型。

---

## 最终完成条件

- [x] Phase 1 的行为、恢复、安全、资源和三平台验证全部通过，本地 C/S 已标记为完成。
- [x] Phase 2 完成后，用户可在无窗口状态安全管理 Host，且 tray/autostart 不成为 Host 存活的技术依赖。
- [ ] Phase 3 完成后，远程 PC 可经 P2P/Relay 安全使用同一 Host resource contract。
- [ ] Phase 4 完成后，移动端在频繁断线和后台限制下仍能可靠恢复。
- [ ] Phase 5 仅在协作/离线产品需求获批且一致性模型验证后标记完成。
