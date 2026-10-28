# C3 设计:每个 Client 一条连接

- 日期:2026-10-28
- 依据:[`2026-10-28-cs-convergence-plan.md`](./2026-10-28-cs-convergence-plan.md) C3、[`2026-10-28-cs-convergence-analysis.md`](./2026-10-28-cs-convergence-analysis.md) §4.3、C2 设计 [`2026-10-28-c2-terminal-sync-design.md`](./2026-10-28-c2-terminal-sync-design.md)
- 前置:C2 已落地(每 attachment 发送基线 + 只保留最新帧的渲染槽)

## 1. 现状

一个已认证 Client 会话由多条本机/TLS/SSH 连接组成,每条各做一次完整握手;握手校验 `channel` × `terminal_session_id` × `can_force_stop` 组合,并把两者写入 HMAC transcript(`crates/yttt-transport/src/auth.rs:281-310,437-446`):

| 连接 | 方向与内容 | Host 处理 | Client 处理 |
|---|---|---|---|
| Control | 请求/响应(`FrameKind::Control`) | `serve_control_connection`:一个排队请求串行执行(队列 64),`Ping`/`ListResources`/`ReadAgentSnapshots`/`SetTerminalVisibility` 内联(`crates/yttt-host/src/lib.rs:1064-1134`) | `connected_control_session` |
| TerminalInteractive | 输入、resize、scroll、心跳 | `serve_terminal_interactive_connection`(`lib.rs:1379-1481`) | `connected_interactive_session` |
| StateEvents | lease、退出、SSH、项目、Agent、catalog 失效(`FrameKind::StateEvent`) | `serve_state_event_connection`,直接 `await` socket 写(`lib.rs:1499-1765`) | state-event reader |
| TerminalData × N | 每个已附加终端一条,snapshot/delta(`ControlMessage::Event`) | `serve_terminal_data_connection` + 512 KiB 发送队列(`terminal_data.rs`) | 每终端 worker,带 generation |
| Lifecycle | launcher 的 status/stop | `serve_lifecycle_connection` | launcher |
| DesktopOwner | 桌面存活管道 | `serve_desktop_owner_connection` | desktop shell |

连接之间没有顺序关系,于是 Client 维护了一组只为"跨连接乱序"存在的栅栏:

- 建立三条资源连接时任一返回不同 `host_epoch` 就整体重试(`crates/yttt-client-core/src/lib.rs:961-1010`)。
- 每终端 data worker 的 `generation`、`TerminalWorkerFence`、`stop_terminal_data_channel_if_current`(`lib.rs:108,242,1674,1852`)。
- `TerminalExit` 经 state-events 先于最终帧到达时暂存于 `deferred_exits`,等 data 连接交付最终帧(`lib.rs:114-134`)。
- C2 的单终端 data 连接重建(诊断计数 `data_lane_resets`,`diagnostics.rs:44,76`):data 连接 gap 时只重建该连接。
- 0.3.16 的 `68b0dff`、`3ce53b7`(旧会话退出事件、启动响应与迟到事件)都属于这一类。

## 2. 设计

### 2.1 一条 Session 连接,不改帧头

- `ConnectionChannel` 资源侧只保留新变体 `Session`;`Control`、`TerminalInteractive`、`StateEvents`、`TerminalData` 删除。`Lifecycle` 与 `DesktopOwner` 不变:前者是 lifecycle 协议的封闭集合,后者只是存活管道。
- Session 连接上只走 `FrameKind::Control`,载荷是已有的 `ControlMessage { Request, Response, Event, TerminalInput }`。终端帧本来就是 `ControlMessage::Event(HostEvent{ Terminal(..) })`;状态事件改为同样的 `Event`;输入使用 `ControlMessage::TerminalInput`,为其追加带缺省的 `control: Option<ControlContext>` 字段(与 `TerminalInteractiveMessage::Input` 对齐)。
- **不在帧头加逻辑通道 id**(计划原文的写法)。每条消息已经自带路由键:`request_id`、`session_id`、事件类型。加通道 id 需要升 `FRAME_FORMAT_VERSION`,而帧头版本连握手帧都要先解码,无法协商;不加则帧格式保持 v1。
- 删除 `TerminalInteractiveMessage`、`FrameKind::TerminalInteractive`/`StateEvent` 的使用方。`FrameKind` 数值保留不复用。
- `ClientHello.terminal_session_id` 字段保留(wire 规则:不删字段)。握手组合校验收敛为 `(Session, None, _)` 与 `(Lifecycle | DesktopOwner, None, _)`。

### 2.2 发送调度:一条写端,三类队列

两端各一个写任务独占 socket 写半边,读取三类待发项:

| 优先级 | 内容 | 有界方式 | 顺序 |
|---|---|---|---|
| 1 交互 | Client→Host:`TerminalInput`、resize、scroll、心跳 Ping;Host→Client:这些请求的响应与 Pong | Client 侧沿用命令队列 256(`lib.rs:38`),满则对调用方返回背压;Host 侧沿用每终端 writer 队列 1,024(`yttt-host/src/terminal.rs:43`) | 队列内 FIFO |
| 2 可靠 | 其余请求/响应、状态事件、终端生命周期事件(`TerminalExit`、lease、control) | Host 每连接 1,024 条 / 32 MiB;超出即断开该 Client,由其重连对账 | 全局 FIFO |
| 3 渲染 | 每终端一个渲染槽(C2 `PendingTerminalFrame`),只保留合并后的最新帧 | 每终端恒为 0 或 1 帧 | 槽间轮转 |

规则:

1. 写端优先发交互,其次可靠队列队首,最后在 16 ms tick 到点时轮转发送非空渲染槽。渲染帧只在写端空闲时取走,慢 Client 期间槽持续合并、基线不前进(C2 语义不变)。
2. **终端屏障**:可靠消息若涉及终端 X(`TerminalExit`、`TerminalAttached`/`TerminalSpawned` 响应、lease 变化),入队时记录 X;写端发送它之前先发出 X 的渲染槽(槽内帧作为普通帧在它前面发送)。由此最终帧必定先于退出事件,`deferred_exits` 删除。
3. **大消息分块**:编码后超过 256 KiB 的可靠消息拆为 `ControlMessage::Chunk { stream, last, bytes }` 序列(新变体,v22 追加)。写端在块之间可以插入交互与渲染帧,但不插入其他可靠消息,因此可靠顺序不变。接收端按 `stream` 拼接,`last` 时解码内层 `ControlMessage`;同时在途最多 4 个 stream,拼接总量上限 `MAX_FRAME_BYTES`(32 MiB),超出视为协议违约断开。两个方向使用同一实现(Client 发布 16 MiB workspace manifest 时,输入不再排在其后)。
4. 帧序号仍校验:同一连接内 delta 出现 gap 只可能是实现错误,Client 断开整条连接并重连,不再单独修补某个终端。

### 2.3 Host 读端与执行

Host 读任务按消息类型分发,保持现有并发语义,只是共用一条连接:

- 交互请求(输入/resize/scroll)交给每连接的交互 worker,按到达顺序执行,不等待排队中的慢请求。
- `recovery_request`(Ping、ListResources、ReadAgentSnapshots、SetTerminalVisibility、RequestCheckpoint)内联执行。
- 其余请求进入现有 64 项队列,一次执行一个,请求日志与 mutation gate 不变。
- 订阅不再由"打开 data 连接"表达:`SpawnTerminal`/`AttachTerminal` 成功即为该连接建立终端 X 的流状态(`SentBaseline` + 渲染槽 + 可见性),首帧为当前 snapshot;`DetachTerminal`、终止、退出确认与连接关闭释放该状态。
- 状态事件源(runtime、SSH、项目、Agent、token、catalog 失效、profile control)的订阅保持不变,只是产出写入可靠队列而不是直接 `await` socket。广播 lag 的处理照旧(重发快照/全量)。

Host 会话表:每个 `client_instance_id` 至多一条 Session 连接。新连接以同一认证会话到达时替换旧连接(旧连接被关闭),不同认证会话仍被拒绝;`SessionBinding.connections` 计数与 `control_connected` 合并为"当前连接"。闲置会话保留 5 分钟、最多 16 个、总计 64 的上限不变。

### 2.4 Client

- `establish_session_lanes` 变为一次握手;`SessionLanes`、三个 reader、跨连接 epoch 检查删除。
- 终端 data worker、`TerminalDataChannel { generation, desired }`、`TerminalWorkerFence`、`next_worker_generation`、`deferred_exits` 删除;`data_lane_resets` 诊断计数随之删除,gap 计入整连接重连次数。终端帧在读任务里直接合并进 mirror。
- 心跳沿用 5 s 间隔、15 s 超时,走交互优先级。
- 重连只有一个状态机:断开时 mirror 保留为缓存;重连后依次 `ListResources`、对仍存在的终端 `AttachTerminal`、重新上报可见集合。Host epoch 变化的处理不变(清空 mirror 与 Agent 缓存)。
- `ClientCore` 对外 API(`request`、`terminal_updates`、`terminal_snapshot`、`set_visible_terminals`、诊断)保持不变,桌面与远程 Client 不需要改调用方式。

### 2.5 远程与平台

- TLS 与 SSH work 入口走同一 `serve_connection`,ingress 权限检查不变(`auth.rs:117-146`)。SSH 转发从 3+N 条通道降为 1 条。
- Windows named pipe 每条连接是一个 pipe 实例,多路复用发生在实例之上,不依赖平台能力;实例数随之下降。

### 2.6 兼容与版本

v22 尚未发布(只在 `CHANGELOG.md` Unreleased 中)。两个选择:

| 方案 | 做法 | 代价 |
|---|---|---|
| A(建议) | C3 并入 v22 基线一起发布;重写 `wire-shapes-v22.json`,删除旧 channel 变体 | 无兼容代码;v22 的 "最后一次同步升级" 承诺仍成立 |
| B | 升 v23;Host 同时服务 v22 多连接与 v23 单连接直到 v22 下线 | 保留全部旧连接代码,C3 的删除目标推迟一个版本 |

已发布的 ≤0.3.18(v21)与新 Host 本来就因 v22 协商失败,按现有 `ReplaceProtocol` 流程在 Host 空闲时替换,方案 A 不新增不兼容面。

## 3. 删除清单(完成标志)

| 侧 | 删除 |
|---|---|
| 协议 | `ConnectionChannel::{Control, TerminalInteractive, StateEvents, TerminalData}`;`TerminalInteractiveMessage`;`send/receive_terminal_interactive`、`send/receive_state_event` |
| 传输 | `auth.rs` 中针对四个资源 channel 与 `terminal_session_id` 的组合分支 |
| Host | `serve_terminal_interactive_connection`、`serve_state_event_connection`、`serve_terminal_data_connection` 作为独立连接入口;`TerminalDataWriter` 512 KiB 每终端发送队列;`SessionBinding.connections`/`control_connected` |
| Client | `SessionLanes`、跨连接 epoch 检查、interactive/state-event reader、每终端 data worker 与 generation 栅栏、`deferred_exits`、`data_lane_resets` 计数 |

新增只有:写端调度器(两端共用,放在 `yttt-transport`)、`ControlMessage::Chunk` 与拼接器、`ControlMessage::TerminalInput.control` 字段。

## 4. 验收

- 行为测试(memory transport + 真实 Host):
  - 退出:不经 Client 暂存,最终帧总在 `TerminalExit` 之前;现有退出重连测试保持通过。
  - 分块:Host 回 16 MiB 响应期间,另一终端的输入回显与渲染帧继续到达,往返延迟不超过无大响应时的 2 倍。
  - 慢 Client:停止读取期间 Host 内存有界;可靠队列超限只断开该 Client;恢复后无 gap。
  - 重连:断开后一次重连恢复全部 mirror、可见性与 catalog。
  - 交互不被慢请求阻塞:一个 5 s 的项目请求执行期间,输入与 resize 正常响应。
  - 每 `client_instance_id` 至多一条 Session 连接;替换与拒绝路径。
- 探针:`host_terminal_sync_traffic_probe` 与 `host_terminal_performance_probe` 与 C2 结果对比,流量与输入延迟不劣化。
- 桌面冒烟:与 C2 相同的 `yttt ctl` 驱动场景;Host 侧每 Client 只有一条连接(`lsof` 计数)。
- 文档:`host-client-architecture.md` 通道与背压章节、`wire-evolution.md`、`CHANGELOG.md`、计划进度。

### 结果(2026-10-28)

删除清单全部落地:上表符号在 `crates/`、`src/`、`tests/` 中均无残留;`FrameKind` 只剩 Handshake/Control/Lifecycle/DesktopShell(5、6 保留不复用)。

覆盖验收的测试:

| 验收项 | 测试 |
|---|---|
| 最终帧先于退出 | `host_roundtrip::one_session_connection_carries_requests_output_and_ordered_exit`(自然退出 `exit 7`,连续 8 次通过) |
| 分块期间交互穿插 | `yttt-transport` session 测试 `interactive_messages_pass_between_the_chunks_of_a_large_message`、`interactive_then_reliable_then_render_once_the_writer_is_free`;拼接器拒绝嵌套块与超量 stream |
| 可靠队列超限 | `reliable_overflow_closes_the_connection` |
| 慢 Client / 慢请求 | `slow_observer_does_not_block_the_owner_or_change_canonical_geometry`、`blocked_project_request_does_not_delay_terminal_input`、`many_terminals_keep_catalog_and_interactive_lanes_responsive` |
| 每会话一条连接 | `control_connection::a_session_connection_replaces_its_predecessor_and_refuses_another_session` |

探针(release,与 C3 动工前的 `d51facb` 交替运行各 4 次;8 终端输出时长随 `date +%s` 对齐在 2–3 s 间浮动,故按每毫秒归一):

| 指标 | C2 | C3 |
|---|---|---|
| 终端帧/ms(每 Client) | 0.490–0.500 | 0.455–0.497 |
| 字节/帧 | 1,979–2,479 | 1,963–2,158 |
| Host 语义捕获/ms | 1.36–1.38 | 1.26–1.34 |
| 输入 ping p50 / p95(µs) | 18 / 27–32 | 14–19 / 26–35 |
| 8 MiB 输出吞吐(MiB/s) | 26.6–27.2 | 25.3–27.1 |
| reconnects / attachment drops | 0 / 0 | 0 / 0 |

帧率受 16 ms cadence 约束(8 终端 ≈ 0.5 帧/ms),两者持平;C3 的 Client 字节计数包含整条连接的全部消息,仍不高于 C2 只计 data 连接的数值。

桌面冒烟(debug,隔离 `YTTT_PROFILE_ROOT`,`yttt ctl` 驱动):3 个终端(一个输出 200,000 行)时 Host 只持有监听 socket 与 2 条已接受连接(Session + DesktopOwner);`panes send`/`panes read` 回显往返 83–87 ms(含两次 CLI 进程启动);高输出终端读到 `BURST-DONE`;Host diagnostics `lagged_events` 全为 0、日志无 error/panic;结束桌面进程后 DesktopOwned Host 约 300 ms 内退出。

## 5. 非目标

- 不改变请求并发模型(仍是一个排队请求 + 内联恢复请求 + 交互 worker)。是否让只读慢请求并发,等 C3 后用 perf 数据决定。
- 不合并 Lifecycle / DesktopOwner 连接。
- 不改帧头格式与 `FRAME_FORMAT_VERSION`。
