# 终端控制权按终端分配：实施计划

设计：`docs/plans/2026-11-12-terminal-control-per-session-design.md`。分支 `feat/terminal-control-protocol`，worktree `.worktrees/terminal-control`，基于 master 9b8a7c7。每个任务先写失败的测试，再实现，测试通过后提交。

## 任务 1：协议追加两个请求

文件：`crates/yttt-protocol/src/control.rs`、`crates/yttt-protocol/tests/fixtures/wire-shapes-v22.json`、`docs/wire-evolution.md`。

1. `Request` 末尾追加 `TakeTerminal { session_id }` 与 `ReturnTerminals { session_ids: Vec<TerminalSessionId> }`，写文档注释（设计 §4.1）。
2. 更新 `Request` 上各个映射：
   - 请求名：`"TakeTerminal"`、`"ReturnTerminals"`；
   - 所需能力：两者都是 `Capability::TerminalInteractive`；
   - `session_id` 归属：`TakeTerminal` 归其 `session_id`，`ReturnTerminals` 与 `TakeTerminalController` 同组，不归属单个 session；
   - 其余穷举 `match` 按编译器提示补齐。
3. 在 `control.rs` 的单元测试里加断言：两个新请求的能力和名称。
4. `YTTT_BLESS_WIRE_SHAPES=1 cargo test -p yttt-protocol --test wire_shapes` 重写样本，检查 diff 只多出两个变体。
5. `docs/wire-evolution.md`“当前版本”追加一段“v22 内追加（未升版本）：单独接管终端”，写清新旧组合（设计 §4.3）。
6. `cargo test -p yttt-protocol`。

提交：`feat(protocol): take and return single terminals`。

## 任务 2：Host 运行时规则

文件：`crates/yttt-host/src/runtime.rs`、`crates/yttt-host/tests/terminal_runtime.rs`。

先在 `terminal_runtime.rs` 写测试（全部先失败）：

- `taking_one_terminal_leaves_the_others_and_the_controller_alone`：A 是主控，持有 a、b 两个终端；B `take_terminal(a)` → a 归 B，b 仍归 A，主控仍是 A，A 收到 a 的 `LeaseRevoked`，没有 `ControllerChanged`。
- `a_returned_terminal_goes_back_to_whoever_had_it`：接上例，B `return_terminals([])` → a 归 A，A 收到 `ControlGranted`。
- `a_terminal_returned_after_its_previous_holder_left_goes_to_the_controller`：C 是主控，A 持有 a（单独接管），B 从 A 接管 a，A `release_client` 后 B 交还 → 归 C。没有主控时放掉，B 收到 `LeaseReleased`。
- `implicit_hand_overs_skip_a_terminal_taken_on_its_own`：B 单独接管 a 之后，新 Client D 第一次写入 b → D 成为主控、拿到 b，a 仍归 B；主控 A 离开时继任者同样不拿 a。
- `taking_every_terminal_includes_one_taken_on_its_own`：A `take_terminal_control` → a 回到 A，再交还不改变什么。
- `a_return_goes_to_the_first_holder_after_several_takes`：A→B→C 连续接管 a，C 交还 → 归 A。
- 修改 `a_departed_controller_hands_control_to_the_latest_present_viewer`：两个 viewer 先请求写入（被拒也算），另加一个只观察的 newest viewer，断言它不会被选为继任者。

实现：

1. `LeaseState` 加 `taken: Option<TakenLease>`；`grant_lease_locked`、`transfer_lease_locked` 带上 `taken` 参数。
2. `pub fn take_terminal(&self, session_id, client_id) -> Result<TerminalLease, HostRuntimeError>`：终端不存在时报 `NotFound`；按设计 §3.1 计算 `taken`，不调用 `remember_writer`。
3. `pub fn return_terminals(&self, client_id, session_ids: &[TerminalSessionId])`：选出要交还的 lease，逐个计算归还目标（`returns_to` 在 `present` 或 `suspended` 中，否则主控 ≠ 请求方，否则放掉），用 transfer 或 yield 实现。
4. `hand_control_locked` 加参数 `include_taken: bool`：`TakeTerminalController` 传 `true`，并清掉 `taken`；`claim_locked` 首次请求和继任传 `false`。
5. `release_client`：先对离开者持有的单独接管 lease 走归还目标；其余 lease 照旧放掉；再清掉所有 `taken.returns_to == 离开者` 的记录；继任候选加上 `control.writers.contains(..)` 条件。
6. `cargo test -p yttt-host --test terminal_runtime`，再跑 `cargo test -p yttt-host --lib`（`a_lagged_connection_relearns_the_leases_it_holds` 等用到 lease 的测试）。

提交：`feat(host): take one terminal and give it back`。

## 任务 3：Host 请求分发与端到端测试

文件：`crates/yttt-host/src/lib.rs`、`crates/yttt-client-core/tests/host_roundtrip.rs`。

1. 先写 `host_roundtrip.rs` 测试 `a_client_takes_one_terminal_and_gives_it_back`：
   - owner 生成两个终端；observer 用 Observer 附加两个终端，然后 `TakeTerminal(a)` → 得到 Interactive lease；
   - owner 只在 a 上收到 `TerminalLeaseRevoked`，在 b 上仍能输入；observer 在 a 上用新 lease 输入成功，在 b 上输入得到 `PermissionDenied`；
   - observer `ReturnTerminals([])` → owner 收到 a 的 `TerminalControlGranted`，再在 a 上输入成功，observer 在 a 上输入被拒；
   - observer 再发一次 `ReturnTerminals([])` 仍回 `Applied`。
2. `process_control_request` 加两个分支：`TakeTerminal` → `runtime.take_terminal(..)` → `Response::TerminalLease`；`ReturnTerminals` → `runtime.return_terminals(..)` → `Response::Applied`。
3. 确认 `control_event` 会把 `ControlGranted`、`LeaseRevoked` 转发给对应连接，并更新该连接 attachment 的 mode（observer 是以 Observer 附加的，拿到 lease 后它的 attachment 要变成 Interactive，否则 `validate_terminal_mutation` 会拒绝输入）。如果拒绝，就在 `TakeTerminal` 分支里同步更新 attachment，并补一条单元测试。
4. `cargo test -p yttt-client-core --test host_roundtrip a_client_takes_one_terminal`，再跑 `cargo test -p yttt-host`。

提交：`feat(host): serve TakeTerminal and ReturnTerminals`。

## 任务 4：桌面 pane 只接管自己

文件：`src/ui/terminal/pane.rs`。

1. `take_terminal_control` 改发 `Request::TakeTerminal { session_id: self.session_id() }`。成功（`Response::TerminalLease`）后不需要额外处理，lease 由随后的 `TerminalControlGranted` 走 `accept_control_grant`。
2. 结果是 `ClientCoreError::Protocol(failure)` 且 `failure.code == FailureCode::Unsupported` 时，再发 `TakeTerminalController`（旧 Host）。
3. 更新文档注释（“Takes control of every terminal”）。
4. 若 `src/ui/terminal/pane/recovery_tests.rs` 有覆盖接管的测试，按新请求更新；没有则加一条：pane 的接管发出的是 `TakeTerminal`，且带本 pane 的 session id。

提交：`feat(desktop): take back only the pane you click`。

## 任务 5：桌面状态栏

文件：`src/ui/workbench/persistence.rs`、`src/ui/workbench/terminal_control.rs`、`src/ui/terminal/pane.rs`、`src/ui/i18n/{key,en,zh_cn}.rs`。

1. `observes_another_controller` 改为 `pub(crate)`。
2. `terminals_controlled_elsewhere()` 改为 `terminals_observed_elsewhere(&self, cx) -> usize`：统计 `self.terminal.terminal_panes` 中 `observes_another_controller` 为真的 pane 数。
3. 状态栏：N > 0 时显示“ · {count} 个终端由其他设备控制”，去掉接管按钮。`WorkspaceStatusTerminalsControlledElsewhere` 的中英文案改为带 `{count}` 的格式；`WorkspaceStatusTakeTerminalControl` 仍由 pane 使用，保留。
4. 删除不再使用的 `take_terminal_control`、`terminal_control_taking` 和 `TerminalControlState.taking`（workbench 侧）。`TerminalControlState` 仍用于跟踪主控，如果删掉按钮后它已无人读取，就整体删除 `terminal_control.rs`，连同 `start_terminal_control_listener` 的调用。
5. `cargo check`，以及 i18n 完整性测试（`cargo test -p yttt --lib i18n` 或项目里对应的测试名）。

提交：`feat(desktop): count terminals controlled on other devices`。

## 任务 6：验证

1. `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`。
2. `cargo test -p yttt-protocol -p yttt-host -p yttt-client-core`，以及桌面包里与 pane、workbench 相关的测试。
3. 手测（spike 分支的 `spike_host` 不在本分支，所以用开发版桌面 + `p2p_probe` 或第二个桌面 Client 都可以）：桌面 A 打开两个终端，第二个 Client 单独接管一个，A 上只有那个 pane 变只读，状态栏显示“1 个终端由其他设备控制”；A 点该 pane 的接管，只取回这一个。
4. 在本文末尾写执行记录与偏差。

## 之后（不在本分支）

- `spike/mobile-client`：`take_control(session)` 改用 `TakeTerminal`，加交还和进后台交还，`Unsupported` 时隐藏接管；更新移动端设计 §7 第 1、2 条（设计 §6）。

## 执行记录（2026-11-12）

| 任务 | 提交 | 结果 |
|---|---|---|
| 1 协议 | 4f33b4e | 两个请求追加到 `Request`，wire 样本只多出这两个变体；Host 暂回 `Unsupported`，在任务 3 换成实现。 |
| 2 运行时 | dfe6e21 | `terminal_runtime` 新增 7 个测试并修改继任测试，23 项通过。用变异验证过：去掉“隐式移交跳过单独接管”或“继任只选写过的 Client”，各有 2 项失败。 |
| 3 分发 | 80bf744 | `host_roundtrip::a_client_takes_one_terminal_and_gives_it_back` 通过；`yttt-host`、`yttt-client-core` 全部通过。 |
| 4 pane | 561e009 | 先加断言“pane 接管后主控仍是 laptop”，确认失败后实现；`recovery_tests` 5 项通过。 |
| 5 状态栏 | 2c2dae8 | 删除 `src/ui/workbench/terminal_control.rs`；pane 在只读状态变化时发 `ControlChanged`，测试断言发了两次（变只读、取回）。 |
| 6 验证 | 本提交 | fmt、CI clippy 包集通过（修了两处测试里的 `cloned_ref_to_slice_refs` 和一处 `items_after_test_module`）。CI 测试包集：整组运行在 `control_connection` 卡住（下述已知问题）被超时终止，此前 58 个目标 1449 项全部通过；之后补跑其余 8 个包（19 个目标 305 项）和 `yttt-host` 的其余目标，全部通过，`control_connection` 单独 5 项通过。 |

### 偏差

- **任务 3**：计划里写“需要时在 `ReturnTerminals` 分支同步把 attachment 改回 Observer”。实际不需要：`TerminalInput`、`ResizeTerminal` 除了看 attachment 还会调 `runtime.validate_lease`，交还后的输入立刻被拒；attachment 由随后的 lease 事件更新。写过这段代码，变异检查显示它没有作用，已删。`TakeTerminal` 则需要在应答时同步更新 attachment（与 `RequestTerminalControl` 同一映射），否则应答之后马上发出的输入可能早于事件被处理而遭拒。
- **任务 2**：`acquire_lease` 遇到别的 Client 单独接管的 lease 时返回 `LeaseConflict`，不再转移。设计 §5 已写明（重连后的隐式申请不推翻显式接管），计划里没单列。
- **任务 2**：`hand_control_locked` 遇到主控本来就持有、但带 `taken` 标记的 lease（交还到非主控 Client 后，它又成了主控）会清掉标记，否则下次隐式移交会错误地跳过它。
- **归还目标**：设计写“`returns_to` 仍是已知 Client（在线或挂起）”。实现上 `release_client` 会清掉指向离开者的 `returns_to`，所以只要还留着就有效，不再单独查在线状态。
- **手测**：没有启动桌面 GUI 手测。开发版桌面会连开发 profile 的 Host，避免动到你的环境；同样的流程由 `host_roundtrip`（真 Host、两个 Client）和 pane 的 GPUI 测试（真 Host、真 pane）覆盖。
- `control_connection::a_slow_agent_history_scan_does_not_hold_up_the_sessions_other_requests` 在整组运行时又卡住过一次，单独重跑 5 项 0.5 s 通过。已有的时序问题，与本分支无关。
