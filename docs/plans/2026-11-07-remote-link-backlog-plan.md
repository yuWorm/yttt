# 远程链路积压、活动工作区与残留客户端计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划，不要把计划执行委托给 subagent。每项先写能复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** A–E 已完成（各节列出未做项与手工验证）；F 未开始。A3 的端到端限速测试未做，见该节。

**依据：** 2026-11-07 的现场诊断。Windows Client 经穿透工具（TLS）与 P2P（始终走 relay，从未直连）连接 macOS Host（34 个终端，多数为 omp agent）：

- 远程 Client 卡在“正在恢复工作区…”，没有错误。该连接 8 分钟内收到约 55 MB，实测 Host 持续以约 130 KB/s 发送，即穿透工具的吞吐上限；Client 没有附加任何终端。
- omp agent 的 `AgentSnapshot` 含完整 `board`（约 400 task、256 execution、256 message），单个 0.2–0.9 MB，该工作区 24 个合计约 4 MB；20 s 内重写 11 次，约 177 KB/s。
- Host 对每个会话用普通 `send_reliable` 逐条发送每次 Agent 快照（`forward_state_events`），不合并、不看 Client 是否在看；broadcast lag 时再发送全部快照。慢链路上可靠队列永远排不空，工作区响应排在队尾。
- 恢复工作区每步 `request_blocking_typed` 只等 15 s（`src/host_runtime.rs`），失败后 mode 仍为 `Loading`，界面只显示加载中。
- Host 上残留 4 个 `paired-host-client` 会话，均无附加，一直未结束：Windows 上失败启动的远程 Client 进程仍在发心跳。
- 远程链路心跳判定：静默 45 s，或“有数据但心跳未回复”180 s；往返时间已记录在 diagnostics，但没有显示。
- 远程 Client 按 `restore_on_startup` 顺序认领工作区，不知道 Host 本机当前使用的是哪个；本机多窗口会留下多个“启动时恢复”的工作区，远程可能落到空工作区（dashboard）。

**目标：** 慢链路上控制状态与工作区恢复不被 Agent 状态淹没；两端都能识别失联或不再读取的对端并释放它；远程进入后直接看到 Host 的活动工作区；连接状态（延迟、不稳定、重连）对用户可见。

---

## 执行约束

- [ ] A → F 顺序执行；每阶段单独提交，阶段内每项先有失败的测试。
- [ ] 不改变 Host 的资源所有权与认证模型。
- [ ] 所有队列保持硬上限；合并只替换尚未开始写的同键消息。
- [ ] 协议只做追加：新增字段带 `#[serde(default)]`；改动 wire 形状时重写 `wire-shapes-v22.json` 并说明兼容性。
- [ ] 每阶段结束时同步 `docs/host-client-architecture.md` 对应章节。
- [ ] 无法自动化的行为（真实 relay、熄屏、穿透工具）写明手工验证步骤。

---

# A：慢链路不再被 Agent 状态淹没

## A1：Agent 快照与 token 用量按键合并

- [x] 测试：一个不读取的会话在两个 Agent 与一个项目各更新 100 次后，读端收到的快照与用量有上限且以最新值结束（`state_event_coalescing_tests`；改动前收到全部 200 份快照）。
- [x] `forward_state_events` 的 `AgentSnapshot` 改用 `send_reliable_latest`，键为 agent 地址（project/tab/pane）加 generation；lag 后的全量补发同样走该键。Client 已按 (epoch, generation, sequence) 取最新，跳过中间序号安全。generation 进入键，使新一代 agent 的快照不越过排在旧一代之后的 `AgentStateCleared`。
- [x] `ProjectTokenUsage` 同样按项目合并。

**验收：** 慢 Client 的 Agent 积压有上限（每个 agent 一份），不随更新次数增长。

## A2：远程恢复工作区有期限、有错误、可重试

- [x] `request_blocking_typed` 的等待改为 `ClientCore::request_timeout`（随链路区分，已有单元测试）加 5 s 余量，不再另维护一张写死的表；远程 `Workspace` 请求因此等 120 s 而非 15 s。**偏离：** 未单独为桌面层写测试——缺陷来自重复的期限表，修复是删除它。
- [x] 恢复失败后仍保持加载并自动重试，加载页显示上次失败的原因与“将自动重试”。
- [ ] 手工验证：远程 Client 在 Host 不可达的工作区请求超时后，加载页出现失败原因。

## A3：慢链路回归测试

- [x] 测试夹具：Client 经限速 128 KB/s 的连接读取；项目内连续写 6,000 个文件（模拟构建）期间请求持续得到回复、Client 不被断开（`a_build_in_a_project_does_not_disconnect_a_slow_client`，去掉 `ProjectChanged` 合并即以 `NotConnected` 失败）。

---

# B：双向探测与残留客户端

## B0：P2P 断开的会话不再残留

- [x] 复现：Host 上 4 个 `paired-host-client` 会话一小时未结束、`clients` 计数为 5，而 Windows 只剩 2 个 yttt 进程。原因在 Host：QUIC 连接在对端消失约 30 s 后结束（早于会话的 60 s 空闲期限），`serve_work_connection` 随即 `sessions.shutdown()` 中止会话任务；会话收尾写在 `.await` 之后（计数、`suspend_client` 解除附加、lease 宽限与释放、`dropped` 日志），全部被跳过。停用或重启 P2P 时 `accept_loop` 的 `connections.shutdown()` 同理。
- [x] 测试：设备的 endpoint 突然关闭后，Host 诊断的 `clients` 回到只剩管理员；P2P 被关闭后同样（`p2p_pairing.rs`，两项在修复前都失败）。
- [x] 连接结束与 P2P 停止时等待会话自行结束，不再中止。

## B1：Host 识别不再读取的 Client

- [x] 已有机制，未改动：写端每次写入在 30 s 内没有进展即以 `write failed or timed out` 关闭（`send_frame`）。对端不读时，TCP 缓冲与 QUIC 流控都会让写入停滞，因此不读的 Client 在约 30 s 内被关闭；仍在读但读得慢的 Client 由 A1 限制积压。

## B2：远程 Client 不残留、失联可操作

- [x] 复现结论：Windows 上的残留是 Host 一侧的幽灵会话（B0），不是远程 Client 进程。
- [x] 断开超过 1 s 时远程窗口以居中遮罩显示 Host、第几次重连、已断开时长与原因，提供“立即重连”“关闭窗口”；停止重试（如设备被撤销）时只提供关闭。（由 E 提前到此处。）
- [x] 测试：卡在失效路径上的重连在 `retry_now` 后 2 s 内恢复（`transport_contract.rs`，未接线前超时失败）；Host 停止后远程 runtime 报告断开、原因与重连次数，连接正常时不报告（`host_runtime` 测试）。
- [ ] 手工验证：远程窗口断网后出现遮罩，计时增长；“立即重连”在网络恢复后立即连上；“关闭窗口”结束远程进程，Host 在约 30 s 内记录该会话 `dropped`。
- [ ] 恢复工作区长时间失败时提供关闭窗口。

---

# C：活动工作区与本机单窗口

## C1：Host 记录活动工作区

- [x] 测试：活动工作区持久化、重启后仍在，未知工作区 `NotFound`，工作区删除后清除（`the_active_workspace_survives_a_restart_and_leaves_with_its_workspace`）；`SessionCoordinator` 先认领活动工作区（`a_restoring_window_claims_the_hosts_active_workspace_first`）。
- [x] 协议追加 `WorkspaceRequest::SetActive`、`WorkspaceResponse::ActiveSet` 与 `WorkspaceSummary::active`（`#[serde(default)]`），已重写 `wire-shapes-v22.json`，只有新增。
- [x] 本机窗口在显示工作区与被激活时上报；远程窗口不上报，进入后不跟随 Host 切换。**偏离：** 上报的接线没有单独测试，需要本机 Host 的 GPUI 夹具。

## C2：单窗口

- [x] 测试：已有窗口时再次“打开窗口”不新开（`opening_a_window_again_reuses_the_work_window`）；从历史打开在原位置替换并只剩一个窗口，被替换的工作区退出启动集合（`a_workspace_from_history_replaces_the_windows_workspace`）。
- [x] 本机启动与远程启动都只开一个窗口；`OpenWindow` 在已有窗口中打开请求的项目；托盘去掉“新建窗口”。
- [ ] 手工验证：Mac 上从历史切换工作区，窗口位置不变、旧工作区出现在历史；Windows 远程进入直接显示 Mac 当前工作区。

---

# D：进入远程后关闭连接列表

- [x] 测试：表单发出 `RemoteClientLaunched` 后远程服务窗口关闭（`the_connections_window_closes_once_a_remote_client_starts`，去掉订阅即失败）。
- [x] TLS 连接码与 P2P 配对表单在远程 Client 进程启动成功后发出该事件；SSH 项目选择器启动成功后同样关闭（SSH 服务器原本已关闭）。
- [ ] 手工验证：从远程服务窗口连接后，该窗口自动关闭，只剩远程 Client 的连接窗口与工作窗口。

---

# E：连接状态可见

- [x] 测试：心跳测出延迟；Host 回复停滞时标记不稳定且会话不断开，恢复后取消标记（`heartbeats_measure_latency_and_flag_a_stalled_link`）；中位数取最近 5 次、一次慢回复不改变显示（诊断单元测试）。
- [x] `LinkProfile` 增加 `heartbeat_interval`（本机 5 s，远程 2 s）与 `unstable_after`（6 s）；远程 `liveness_timeout` 由 45 s 收紧到 20 s（静默期限：仍在送达数据的慢链路不受影响）。
- [x] 远程窗口状态栏在 Host 名或 P2P 路径后显示延迟（≥ 300 ms 警告色）或“连接不稳定”；TLS 窗口此前不显示链路。
- [x] 断开遮罩（B2）显示次数、时长与原因；遮罩消失时提示“已重新连接”与断开时长。
- [x] 重连不重复拉取 Agent 状态：Host 不再在新连接上推全量 Agent 快照，只由 Client 的 `ReadAgentSnapshots` 按游标读取（`fix/link-storm`）。
- [ ] 手工验证：Windows 远程窗口状态栏显示延迟；拔网线数秒内显示“连接不稳定”，约 20 s 后出现断开遮罩；恢复网络后遮罩消失并提示断开时长。

---

# F：后续

- [x] Agent 快照瘦身：每个 Agent 首份状态之后以 `AgentSnapshotPatch` 发送 board 补丁（`a_connected_client_rebuilds_a_growing_agent_board`、`agent_board_patches_rebuild_the_newest_board_on_a_slow_reader`）。
- [x] P2P／QUIC 发送缓冲：流窗口 256 KiB、连接发送窗口 512 KiB（原默认 1.25 MB / 10 MB），会话分块 32 KiB（`a_writer_stops_soon_after_the_peer_stops_reading`）。
- [x] 断连风暴（2026-11-09 诊断）：`ProjectChanged` 按项目合并、可靠消息连续 64 KiB 后让渲染、连上即断时退避不清零、溢出原因细分（`fix/link-storm`）。
- [x] TLS／TCP 发送缓冲：Linux 设 `TCP_NOTSENT_LOWAT` 64 KiB；macOS、Windows 无安全接口，未设；穿透工具自身的缓冲无法控制。
- [x] 远程链路压缩大消息：`ClientHello.accepts_compression` + `ControlMessage::Compressed`（DEFLATE），仅 Host→Client、仅网络入口。
- [x] 诊断：`host-diagnostics.jsonl` schema 7 的 `session_queues`。
- [ ] `ProjectChanged` 只发给打开了该项目的会话（合并后收益小，未做）。
- [ ] P2P 始终 relay 的原因排查（两端网络、UDP 打洞）。
