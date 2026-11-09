# 借鉴同类产品的远程链路优化计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划，不要把计划执行委托给 subagent。每项先写能复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** 1.1 进行中；其余未开始。

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

- [ ] 1 → 5 顺序执行；每项单独提交，项内每步先有失败的测试。
- [ ] 协议只做追加：新增字段带 `#[serde(default)]`，新增请求在旧 Host 上回 `Unsupported`；改动 wire 形状时重写 `wire-shapes-v22.json` 并说明兼容性。
- [ ] 新旧两端任意组合都能工作：旧 Client 照旧收到全部 board；新 Client 连旧 Host 照旧收到全部 board。
- [ ] 每项结束时同步 `docs/host-client-architecture.md` 对应章节与 `CHANGELOG.md`。

---

# 1：Agent board 按订阅发送

现状：Host 把每个 Agent 的完整 board 推给每个会话（`forward_state_events` → `send_agent_snapshot`），`ReadAgentSnapshots` 的每页也带 board；而 Client 里只有指挥中心（`src/ui/workbench/command_center.rs`）读取 board 内容。补丁、合并、压缩只减少了每份的大小，没有减少不需要它的接收方。

指挥中心跨所有 Agent 汇总（待答问题数、执行数、任务数、默认选中项），所以订阅粒度取“本会话是否需要 board”，不按单个 Agent。概要字段（状态、`waiting_message`、任务、当前动作、进程退出）在 board 之外，照常发送。

## 1.1：会话级 board 订阅

- [ ] 协议：`ClientHello.accepts_agent_board_interest`；`Request::SetAgentBoardInterest { watch }`，回 `Applied`；`AgentSnapshotUpdate.board_omitted`。均带 `#[serde(default)]`，重写 wire 形状。
- [ ] Host：声明了该能力的会话从“不需要 board”开始；此时 Agent 快照事件与 `ReadAgentSnapshots` 回复都去掉 board 并标记 `board_omitted`，同时忘记该 Agent 已交付的补丁基准。收到 `watch: true` 后按 lag 补发路径重发全部当前快照（带 board，能补丁则补丁）。
- [ ] ClientCore：订阅意愿跨重连保留，每次连接在 `ReadAgentSnapshots` 之前上报；Host 回 `Unsupported` 时视为总会收到 board。收到 `board_omitted` 的快照时沿用本端为同一 Agent、同一 generation 持有的最后一份 board，并丢弃补丁基准；同序号的完整快照可替换 `board_omitted` 的那份。
- [ ] 桌面：指挥中心处于活动导航时订阅，离开时取消；同序号的完整快照能被桌面的 Agent 状态接受。
- [ ] 测试：未订阅的会话收到的 Agent 快照没有 board；订阅后收到完整 board，取消后又不带；重连后订阅意愿仍在；旧 Client（不声明能力）仍收到 board；ClientCore 沿用 board 与同序号替换的单元测试。

**验收：** 远程 Client 不打开指挥中心时，Agent 更新只传概要（约 1 KB/份），不再随 board 增长。

## 1.2：打开工作区时不下载 board

- [ ] 先测量：远程打开工作区时 `GetAgentState` 逐个读取每个 pane 保存的 Agent 状态（JSON，带完整 board，摘要校验），记录 24 个 Agent 时的字节数与耗时。
- [ ] 若可观：打开工作区只取概要（新请求或 `GetAgentState` 的追加参数，旧 Host 回退为读完整状态），完整 board 在订阅 board 时补齐。具体形状在测量后定。

---

# 2：状态类数据在写出时生成

现状：Agent 快照、`ProjectChanged`、通知仍先编码成消息进入可靠队列，靠按键合并限制积压；队列超限即断开连接，重连后全量重读。Cindy 的记录表明这是自放大循环。

- [ ] 每个会话维护状态脏标记（Agent、项目、token 用量、目录通知），写端空闲时读取当前状态、编码、发送；可靠队列只承载请求回复与必须按序到达的事件。
- [ ] 状态类数据不再触发 `QueueOverflow` 断连。
- [ ] 测试：不读取的会话在大量 Agent／项目更新后可靠队列不增长，恢复读取后收到的是最新状态；与 `AgentStateCleared` 等有序事件的相对顺序不变。

---

# 3：唤醒与网络变化时立即探测

现状：只有心跳（远程 2 s 间隔、6 s 不稳定、20 s 断开）；笔记本合盖再打开要等约 20 s 才重连。

- [ ] 桌面监听系统唤醒与网络变化（macOS、Windows），触发一次立即心跳；短时间无回应即走 `retry_now`。
- [ ] 测试：注入“唤醒”后，失效连接在数秒内被替换；健康连接不被打断。

---

# 4：断线时保留可读内容

现状：断开超过 1 s 即显示居中遮罩。

- [ ] 短时间断开只让内容变暗、禁止输入，并在状态栏提示；超过阈值再显示遮罩。
- [ ] 手工验证：断网数秒，窗口内容可读、输入被拒；恢复后提示断开时长。

---

# 5：同一 Host 的多条线路（视前几项效果决定）

- [ ] 把 TLS 隧道、P2P 直连、P2P 中转视为同一 Host 的有序线路；连着较差的线路时后台预检更优线路，可用则切换，失败线路冷却。
- [ ] 前提：会话层可续传（序号＋ACK，换连接后从断点继续），否则切换会丢消息。单独设计。

## 不做

- Orca 的应用层 ACK 窗口与链路估算：它弥补的是 WebSocket `bufferedAmount` 看不到中转积压；yttt 的 QUIC 流控会反压回发送端，终端也已是一格合并。
- Herdr 只走 SSH：不适合图形 Client 与 Windows 远程进入。
