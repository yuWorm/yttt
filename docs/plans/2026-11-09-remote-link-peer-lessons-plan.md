# 借鉴同类产品的远程链路优化计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划，不要把计划执行委托给 subagent。每项先写能复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** 1、3、4 完成（手工验证待做）；2、5 未开始。**顺序调整（2026-11-09）：** 1 完成后各类状态消息都已按键合并、Agent 未订阅时只有概要，2 的收益变小，先做 3；2 等日志中仍出现队列溢出再做。

**依据：** 2026-11-09 对 Herdr、T3 Code、Orca、Cindy 远程链路实现的阅读（源码与设计文档），对照 yttt 在 `2026-11-07-remote-link-backlog-plan.md` 完成后的状态。

| | 借鉴点 | 出处 |
|---|---|---|
| Herdr | 渲染槽容量为 1；客户端声明 `surface_interest`，只收看得见的画面；连接健康 1 分钟才重置退避；断线时保留变暗的缓存画面、禁止输入 | `src/server/client_transport.rs`、`docs/.../connecting-machines.mdx` |
| T3 Code | shell 快照与 thread 详情分离，详情只在页面挂载时订阅，缓存与续传游标保留 5 分钟；传输健康与数据新鲜度分开；回到前台／网络变化时先探测已有会话；有序线路列表、预检更优线路、失败线路冷却 | `docs/internals/connection-runtime.md`、`remote.md` |
| Orca | 每终端 ACK 窗口；溢出时丢输出、补发终端快照而不断连；不可见终端暂停输出；回到前台时立即探测、清除放弃重连的上限 | `src/shared/terminal/stream/terminal-multiplex-flow-control.ts`、`mobile/issue-5049-unresponsive-session-findings.md` |
| Cindy | 故障半径：恢复动作的半径不超过故障；“重连 → 全量重放 → 再被踢”是自放大循环；直连优先、中继兜底，可靠层按序号去重，先建后拆 | `docs/dev-rules/remote-and-mobile-adaptation.md`、`docs/self-hosted-relay-plan.md` |

yttt 已对齐的部分不再列入：终端语义快照＋增量、每终端一格合并（`terminal_data.rs`）；`SetTerminalVisibility`；连上即断时退避不清零；P2P 中转＋直连与路径诊断。

**目标：** 慢链路上 Host 只发送 Client 正在使用的状态；状态落后不再导致断连；睡眠唤醒后数秒内恢复；断线不遮挡仍可读的内容。

---

## 执行约束

- [ ] 1 → 5 顺序执行（3 已提前到 2 之前，见状态）；每项单独提交，项内每步先有失败的测试。
- [ ] 协议只做追加：新增字段带 `#[serde(default)]`，新增请求在旧 Host 上回 `Unsupported`；改动 wire 形状时重写 `wire-shapes-v22.json` 并说明兼容性。
- [ ] 新旧两端任意组合都能工作：旧 Client 照旧收到全部 board；新 Client 连旧 Host 照旧收到全部 board。
- [ ] 每项结束时同步 `docs/host-client-architecture.md` 对应章节与 `CHANGELOG.md`。

---

# 1：Agent board 按订阅发送

现状：Host 把每个 Agent 的完整 board 推给每个会话（`forward_state_events` → `send_agent_snapshot`），`ReadAgentSnapshots` 的每页也带 board；而 Client 里只有指挥中心（`src/ui/workbench/command_center.rs`）读取 board 内容。补丁、合并、压缩只减少了每份的大小，没有减少不需要它的接收方。

指挥中心跨所有 Agent 汇总（待答问题数、执行数、任务数、默认选中项），所以订阅粒度取“本会话是否需要 board”，不按单个 Agent。概要字段（状态、`waiting_message`、任务、当前动作、进程退出）在 board 之外，照常发送。

## 1.1：会话级 board 订阅

- [ ] 协议：`ClientHello.accepts_agent_board_interest`；`Request::SetAgentBoardInterest { watch }`，回 `Applied`；`AgentSnapshotUpdate.board_omitted`。均带 `#[serde(default)]`，重写 wire 形状。
- [x] Host：网络入口上声明了该能力的会话从“不需要 board”开始；此时 Agent 快照事件与 `ReadAgentSnapshots` 回复都去掉 board 并标记 `board_omitted`，同时忘记该 Agent 已交付的补丁基准。本机 IPC 一律带 board（`yttt agents list` 等本机工具不必订阅）。**偏离：** 订阅后不由 Host 重发全部快照，而由 Client 重读只持有概要的 Agent（游标不含它们）：Host 不知道 Client 在之前的连接里收到的是哪些概要，且连接开始时 Host 重发会与第一页重复。
- [x] ClientCore：订阅意愿跨重连保留，每次连接在 `ReadAgentSnapshots` 之前上报；Host 回 `Unsupported` 时视为总会收到 board。收到 `board_omitted` 的快照时沿用本端为同一 Agent、同一 generation 持有的最后一份 board，并丢弃补丁基准；同序号的完整快照可替换 `board_omitted` 的那份。
- [x] 桌面：指挥中心处于活动导航时订阅，离开时取消；同序号的完整快照能被桌面的 Agent 状态接受。**偏离：** 订阅的接线没有单独的界面测试（需要 GPUI 夹具），由 ClientCore 与端到端测试覆盖其下各层。
- [x] 测试：`a_remote_client_receives_agent_boards_only_while_it_watches`（TLS 入口：分页与事件都不带 board，订阅后补齐，取消后保留最后一份）；`a_session_asks_for_agent_boards_before_reading_snapshots_and_when_asked`；`a_snapshot_without_its_board_keeps_the_board_held`、`a_snapshot_without_its_board_drops_the_patch_base`；`a_host_snapshot_with_its_board_replaces_the_same_one_without`；`only_network_sessions_to_asking_clients_leave_boards_out`。去掉各自的实现后均失败。
- [ ] 手工验证：Windows 远程窗口不打开指挥中心时，Host 诊断的该会话发送字节随 Agent 更新明显减少；打开指挥中心后看板完整。

**验收：** 远程 Client 不打开指挥中心时，Agent 更新只传概要（约 1 KB/份），不再随 board 增长。

## 1.2：打开工作区时不下载 board

- [x] 测量（2026-11-09，本机 Host 的活动工作区）：22 份保存的 Agent 状态共 4,080,940 字节 JSON，最大一份 1,072,406 字节，board 占 99.9%，其余字段约 370 字节；远程打开时逐个 `GetAgentState` 串行读取。压缩后约 1.3 MB，130 KB/s 链路约 10 s，超额 relay（16 KB/s）一分多钟。
- [x] `Opened` 追加 `agent_summaries`（每个引用对应 pane 当前状态、去掉 board；Host 从内存中的布局生成，不读盘）。远程窗口在摘要覆盖全部引用时只用摘要恢复，跳过全部 `GetAgentState`；指挥中心首次打开时后台读取完整状态并把 board 补进 pane。本机窗口与旧 Host 照旧读取完整状态。**偏离：** 没有给 `GetAgentState` 加参数——Host 内存中已有各 pane 的状态，随 `Opened` 一并带回还省掉每个 Agent 一次往返。
- [x] 测试：`an_opened_workspace_summarizes_each_agent_state_without_its_board`（含 Host 重启后；去掉摘要即失败）；`summaries_stand_for_the_states_only_when_they_cover_every_one`、`a_summary_for_a_missing_pane_fails_the_restore`、`a_stored_board_fills_an_empty_one_and_trails_a_live_one`。**偏离：** 桌面的恢复与补读接线没有界面测试（需要 GPUI 夹具）。
- [ ] 手工验证：Windows 远程窗口进入工作区明显更快；打开指挥中心后，已结束的 Agent 也显示完整看板。

---

# 2：状态类数据在写出时生成

现状：Agent 快照、`ProjectChanged`、通知仍先编码成消息进入可靠队列，靠按键合并限制积压；队列超限即断开连接，重连后全量重读。Cindy 的记录表明这是自放大循环。

- [ ] 每个会话维护状态脏标记（Agent、项目、token 用量、目录通知），写端空闲时读取当前状态、编码、发送；可靠队列只承载请求回复与必须按序到达的事件。
- [ ] 状态类数据不再触发 `QueueOverflow` 断连。
- [ ] 测试：不读取的会话在大量 Agent／项目更新后可靠队列不增长，恢复读取后收到的是最新状态；与 `AgentStateCleared` 等有序事件的相对顺序不变。

---

# 3：唤醒与网络变化时立即探测

现状：只有心跳（远程 2 s 间隔、6 s 不稳定、20 s 断开）；笔记本合盖再打开要等约 20 s 才重连。

- [x] `ClientCore::probe_now`：已连接时立即发心跳，`PROBE_TIMEOUT`（5 s）内无任何入站消息即断开重连；未连接时立即重试。本机 IPC 不探测。
- [x] 远程窗口（`src/link_watch.rs`）：唤醒以墙上时钟与 2 s 定时器的差距判断（> 8 s 视为睡眠），网络变化用 `netwatch` 的 `is_major_change`；触发原因写入 `client.log`。
- [x] 测试：`a_probed_session_that_hears_nothing_is_replaced_within_the_probe_timeout`（去掉探测即以 20 s 的心跳超时失败）、`a_probed_session_that_answers_stays_up`、`only_a_tick_far_longer_than_its_timer_counts_as_a_sleep`。**偏离：** 系统唤醒与网络事件本身无法在测试中注入，只测判断与探测。
- [ ] 手工验证：Windows 远程窗口合盖数分钟再打开，数秒内恢复（`client.log` 有“the system woke”一行）；切换 Wi-Fi 同样。

---

# 4：断线时保留可读内容

现状：断开超过 1 s 即显示居中遮罩。

- [x] 断开 1–15 s：内容变暗（遮罩层拦截输入），顶部显示“连接中断，正在恢复”、重连次数、时长与“立即重连”；超过 15 s 或已停止重试：原居中对话框。**偏离：** 提示放在顶部浮层而非状态栏——状态栏位置随用户配置，遮罩本来就要覆盖它。
- [x] 测试：`a_short_outage_dims_the_window_before_a_dialog_covers_it`。界面本身无 GPUI 测试。
- [ ] 手工验证：断网数秒，窗口内容可读、输入被拒；超过 15 s 出现对话框；恢复后提示断开时长。

---

# 5：同一 Host 的多条线路（视前几项效果决定）

- [ ] 把 TLS 隧道、P2P 直连、P2P 中转视为同一 Host 的有序线路；连着较差的线路时后台预检更优线路，可用则切换，失败线路冷却。
- [ ] 前提：会话层可续传（序号＋ACK，换连接后从断点继续），否则切换会丢消息。单独设计。

## 不做

- Orca 的应用层 ACK 窗口与链路估算：它弥补的是 WebSocket `bufferedAmount` 看不到中转积压；yttt 的 QUIC 流控会反压回发送端，终端也已是一格合并。
- Herdr 只走 SSH：不适合图形 Client 与 Windows 远程进入。
