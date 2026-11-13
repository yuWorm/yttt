# Host 签名的 relay 位置：实施计划

- 日期：2026-11-13（同日按审查意见修订）
- 设计：[`2026-11-13-host-relay-placement-design.md`](2026-11-13-host-relay-placement-design.md)
- 分支：`feat/host-relay-placement`（worktree `.worktrees/host-relay-placement`）
- 执行：按 `AGENTS.md` 直接在这个 worktree 里执行，不交给 subagent。
- 规则：
  - 每项先写失败的测试，再实现；最后跑该 crate 的测试和 `cargo clippy -p <crate> --all-targets`，然后提交。
  - **每次提交都必须能编译整个 workspace**：一个字段或变体，在删掉它最后一个使用者的那一项里才删除。

## 顺序

| 项 | 内容 | 依赖 |
|---|---|---|
| T0 | SQLite 测试夹具（**已完成**，`aaadc00`） | — |
| T1 | 位置记录类型 | — |
| T2 | 控制面 + API + control-client + Host 签名 | T1 |
| T3 | 协议事件 + Host 推送 | T2 |
| T4 | 连接层：`set_peer_relay` 和重拨前 hook | — |
| T5 | Client 公共逻辑 + 手机核心 + 桌面 Client | T3, T4 |
| T6 | 配对码改为 `control_url` | T5 |
| T7 | iOS / Android | T5, T6 |
| T8 | 删除自建链路和旧字段 | T5–T7 |
| T9 | 控制台 | T2 |
| T10 | 文档 | 全部 |

---

## T0 SQLite 测试夹具（已完成）

- `yttt-control` 新增 `test-support` feature 和 `test_support::TestRelay`：在临时 SQLite 文件上启动控制面和会上报的节点；设置 `YTTT_TEST_DATABASE_URL` 时改用 PostgreSQL。
- `crates/yttt-client-core/tests/paid_relay.rs`（3 个测试）和 `crates/yttt-mobile-core/tests/paid_relay.rs`（1 个测试）改用这个夹具，去掉 `#[ignore]`，全部通过。

## T1 位置记录类型（`crates/yttt-relay-ticket/src/placement.rs`）

只新增，不删除任何东西。

1. 测试：
   - `placement_bytes` 快照（固定输入 → 固定 hex）；
   - 签名 → 验签往返；
   - 以下情况验签失败：改动 `node_id`、`expected_host` 不同、`node_id` 超过 64 字节、`relay_url` 超过 256 字节；
   - CBOR 和 JSON 两种编码各自往返；
   - `newer_than`：`seq` 更大 → `true`；`seq` 相同且内容相同 → `false`；`seq` 相同而内容不同 → 错误。
2. 实现：`HostPlacement`、`SignedPlacement::{sign, verify, newer_than}`，只用 derive 加 `serde_bytes`。
3. 新增常量 `MAX_PLACEMENT_RELAY_URL_BYTES = 256`。`yttt-protocol` 的 `MAX_RELAY_URL_BYTES` 先保留，T8 再统一。

## T2 控制面、API、control-client、Host 签名

这几部分接口互相咬合，一起改才能编译。

**API 类型**（`yttt-relay-ticket/src/api.rs`）：
- `SwitchNode { placement: SignedPlacement }`；
- `TicketOutcome`、`PlacementNeeded`、`PlacementReason`（见设计 §6.2）。
- 删除 `ProofPurpose::SwitchNode`，它最后的使用者 `placement.rs` 的 `switch_node` 也在这一项里改掉。

**控制面**（`crates/yttt-control`）：
1. 迁移 `migrations/{postgres,sqlite}/0002_host_placement.sql`：`devices.placement TEXT NULL`、`devices.placement_seq BIGINT NOT NULL DEFAULT 0`。
2. `placement::switch_node`：设计 §6.1 的六项检查，每项一个测试，写在 `tests/control.rs`（已经默认用 SQLite），包括：控制台会话返回 403；`max_nodes = 1` 的切换成功，并且账号默认节点跟着变化。
3. `api::issue_ticket` 和 `placement::ticket_node`：返回 `TicketOutcome`，不再调用 `move_devices`；Host 和 Client 的分支按设计 §6.2 实现。
4. `move_node` / `move_accounts`：只改 `accounts.node_id` 和 `plan_nodes`。`register`：拒绝 Unknown 角色，`(_, None)` 分支只接受 Host。
5. 改写现有测试：
   - `a_client_relays_through_its_hosts_node_and_follows_it_when_the_account_moves` → Host 签名切换后 Client 跟随，账号迁移不再移动设备；
   - `hosts_spread_over_as_many_nodes_as_the_plan_allows_and_return_when_it_shrinks` → 套餐缩小后得到 `NodeUnavailable`，设备不动；
   - `a_full_node_takes_no_new_hosts…`、`a_disabled_node_drops_its_devices…` → 按新语义核对。
   - 测试辅助函数 `switch(...)`（`tests/control.rs` 约 2134 行）改为签名记录。

**control-client**（`crates/yttt-control-client/src/lib.rs`）：
6. `ControlSession::ticket` 返回 `TicketOutcome`，`RelayTicket` 保持不变，记录另外返回。`switch_node(secret, placement)` 改为提交签名记录。
7. `keep_ticket`：改用回调 trait `TicketEvents { renewed(&RelayTicket, Option<&SignedPlacement>), placement_needed(&PlacementNeeded), failed(&ControlError) }`。收到 `PlacementNeeded` 时，按退避间隔继续轮询，等用户处理完。
   - 允许从"没有 ticket"开始：传入 `Option<RelayTicket>`，为 `None` 时立即请求。

**Host**（`crates/yttt-host/src/remote_access.rs`、`p2p.rs`，以及 `crates/yttt-server/src/p2p_cli.rs`）：
8. `p2p.rs`：`PlacementStore { next_seq, current }` 存在 `relay-placement.json`。`reserve_seq()` 先写盘再返回；`adopt(signed)` 用自己的密钥验签，记录更新时采用它并抬高 `next_seq`。
9. Host context 里加 `placement: watch::Sender<Option<SignedPlacement>>`，启动时从文件载入。
10. `sign_and_switch(node: &NodeChoice)`：`reserve_seq` → 签名 → `switch_node` → `adopt` → `send_replace`。
11. 登录账号（`SetRelayAccount`）后先取一次 ticket：
    - 收到 `Missing` 就对 `suggested` 调 `sign_and_switch`；
    - 登录响应里返回所在节点，让界面显示。
12. `SetRelayNode`：`sign_and_switch` → 取 ticket → `endpoint.set_relay`。**删除**这里的 `restart_p2p`。
13. `keep_relay_ticket`：`placement_needed` 只写入 `RelayAccountStatus.placement_needed`，不签名；Host 只走直连（`endpoint.set_relay` 为 `Pending`，或者清掉 relay）。`renewed` 时调用 `adopt`，并清掉 `placement_needed`。
14. `RelayAccountStatus` 追加 `placement_needed: Option<PlacementNeeded>`（`#[serde(default)]`），运行 `YTTT_BLESS_WIRE_SHAPES=1 cargo test -p yttt-protocol --test wire_shapes` 重写样本。这样 `yttt-protocol` 在这一项里就要依赖 `yttt-relay-ticket`。
15. `yttt-server p2p status` 显示 `placement_needed`；`p2p node <id>` 走 `SetRelayNode`。
16. 手机端和桌面对 `ControlSession::ticket` 新返回值的适配：这一步只把 `PlacementNeeded` 当成"拿不到 ticket"，完整逻辑在 T5。

验证：`cargo test -p yttt-relay-ticket -p yttt-control -p yttt-control-client -p yttt-host -p yttt-server`，以及 `cargo test -p yttt-client-core --test paid_relay`。其中 `a_host_moves_to_another_node_and_its_device_follows` 改为通过 `SetRelayNode` 切换。

## T3 协议事件和 Host 推送

1. `yttt-protocol`：`ServerEvent` 末尾追加 `RelayPlacementChanged(SignedPlacement)`，重写 wire 样本；`docs/wire-evolution.md` 记一笔：v22 内追加，旧 Client 跳过。
2. `crates/yttt-host/src/lib.rs` 的会话循环（`context.workspaces.subscribe` 附近）：订阅 `placement` watch，握手后立刻发送当前值，之后每次变化都用 `send_reliable_latest("relay-placement", …)` 发送。
3. 测试（`crates/yttt-client-core/tests/paid_relay.rs`）：Client 连上后收到当前记录；Host `SetRelayNode` 后，Client 在 5 s 内收到 `seq + 1` 的记录，并且现有会话没有断开。

## T4 连接层（`crates/yttt-transport-iroh/src/lib.rs`）

1. `IrohConnector.peer` 改为 `parking_lot::Mutex<PeerAddr>`。新增 `set_peer_relay(Option<String>)`：替换地址；缓存的连接如果还开着，就关闭它。用 `try_lock` 关闭，避免在 hook 里死锁。
2. `with_before_redial(Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>)`：`connection()` 发现缓存为空或已关闭、准备拨号之前，先 `await` 这个 hook。第一次拨号不调用 hook。
3. 测试：两个 relay 节点（不需要控制面，用测试密钥签 ticket），对端换节点后调用 `set_peer_relay`，下一次 `connect` 能连上；hook 在每次重拨前恰好调用一次。
4. 验证：`cargo test -p yttt-transport-iroh`。

## T5 Client：公共逻辑、手机核心、桌面

**公共逻辑**（`crates/yttt-client-core/src/p2p.rs`）：
1. `PairedHost` 追加 `placement: Option<SignedPlacement>` 和 `ticket: Option<RelayTicket>`（`#[serde(default)]`），`relay_url` 先保留，T8 删除。
2. `accept_placement(&mut self, signed) -> Result<bool, PlacementError>`。
3. `peer_addr()`：有记录时用记录里的地址，没有就用 `None`，不再读 `relay_url`。
4. `relay_target(&self, ticket: Option<&RelayTicket>, now) -> Result<RelayTarget, &'static str>`：从传入的 ticket 或缓存的 ticket 中选出与记录节点一致、并且仍然可用的那张，否则给出原因。
5. 一个公共的 `RelayFollower`，手机和桌面都用它，避免写两遍。它持有 account、endpoint、connector、`Arc<Mutex<PairedHost>>` 和 `on_record_changed` 回调，提供：
   - `refresh()`：取 ticket → `accept_placement` → 必要时 `set_relay` 和 `set_peer_relay` → 持久化；
   - `on_event(signed)`：处理会话里的 `RelayPlacementChanged`；
   - `before_redial_hook()`：返回给 T4 用的 hook，内部调用 `refresh()`，最多等 `TICKET_WAIT`；
   - `spawn_keep_ticket()`：只要登录了账号就启动。
6. 单元测试覆盖 2、3、4。

**手机核心**（`crates/yttt-mobile-core`）：
7. `MobileSession::connect` 和 `pair` 改用 `RelayFollower`；删除对 `hint()` 的使用（函数本身 T8 删除）。
8. FFI：`connect` 增加 `RecordSink` 回调接口，参数为 `on_record_changed(bytes)`；`ffi.rs` 中 `follow_events` 收到 `RelayPlacementChanged` 时交给 follower。
9. `RelaySource`：值改为 `Account | DirectOnly`。删除 `PairingHint` 会改变 UniFFI 接口，需要同步改 `AppModel.swift`、`Account.swift`、`Link.kt`、`AppNav.kt` 中用到它的地方，并重新生成 bindings（按 `scripts/mobile/` 里的现有脚本）。
10. 测试（`tests/paid_relay.rs`）：
    - 手机连着时 Host 切换节点，手机在 5 s 内经新节点恢复，`RecordSink` 被调用；
    - 关掉 `Control` 后重连，手机用缓存的 ticket 连上；
    - 伪造的记录被忽略；
    - 首次连接时控制面超时，之后控制面恢复，relay 被补上。
    - `tests/session.rs` 里依赖 `[open]` 节点的用例改用 `TestRelay`。

**桌面 Client**（`src/remote_host.rs`、`src/paired_hosts.rs`、`src/ui/app/paired_host.rs`）：
11. 改用 `RelayFollower`：`on_record_changed` 回到 UI 线程，经 `paired_hosts::save_records` 保存；删除约 520 行的配对地址退路；`keep_ticket` 改由 follower 启动。
12. 事件订阅里处理 `RelayPlacementChanged`。

验证：`cargo test -p yttt-client-core -p yttt-mobile-core`、`cargo test -p yttt remote_host paired_hosts`、`cargo clippy --workspace --all-targets`。然后手动测一遍：本机起两个 profile，一个当 Host、一个当 Client，Host 上切换节点，看 Client 能否恢复。

## T6 配对码

1. `PairingCode`：`relay_url` 换成 `control_url: Option<String>`，`decode` 检查长度不超过 256，并为这些限制加测试。`PairingCode` 不在 wire 样本里。
2. Host 生成配对码（`remote_access.rs` 约 704 行）：`control_url` 取自已登录的账号。
3. `yttt_client_core::p2p::pair` 和 `yttt_mobile_core::session::pair` 增加 `direct_only: bool`，满足设计 §6.7 的条件时返回 `SignInRequired { control_url }`。
4. 桌面扫码配对：收到 `SignInRequired` 时弹出现有的浏览器登录；用户选"只用局域网"时以 `direct_only = true` 重试。
5. 验证：`cargo test -p yttt-protocol -p yttt-client-core -p yttt-mobile-core`。

## T7 iOS / Android

1. `RecordSink` 写回 Host 列表（iOS `HostList.swift` / `AppModel.swift`，Android `HostList.kt`），保持原有的原子写。
2. 配对页处理 `SignInRequired`：显示"这台电脑使用 `<control_url>`，先登录"，提供［登录］和［只用局域网］两个按钮。
3. `RelaySource` 的改名；连接日志里显示 `relay_target` 给出的拒绝原因。
4. 验证：
   - iPhone 上用 `scripts/mobile/run-ios-device.sh`，接线上控制面——配对，然后在 Host 上切换节点，手机自动恢复；再停用节点，Host 出现提示、手机走直连；
   - Android 模拟器上走一遍配对、登录和切换。

## T8 删除自建链路和旧字段

1. `crates/yttt-relay`：删除 open 模式（设计 §7）。`tests/endpoints.rs` 的 open 用例改为用测试密钥签 ticket 的节点。删除 `packaging/relay-selfhost/`，以及 `packaging/relay-dev/relay.toml` 里的 `[open]`。
2. 删除 `P2pSettings.relay_url`、`SetP2p.relay_url`，重写 wire 样本，并在 `wire-evolution.md` 里写明这是有意的破坏。Host `start_p2p` 没有账号时只走直连。删除 `p2p_cli.rs` 的 `--relay` 并更新 `P2P_USAGE` 的测试。删除 `src/ui/workbench/remote_access_p2p.rs` 里的 relay 地址输入框，改为提示"登录 relay 账号后可在外网访问，目前只能直连"。
3. 删除 `PairedHost.relay_url`、`hint()`、`RelaySource::PairingHint`（如果 T5 还留着）。统一 `MAX_RELAY_URL_BYTES`，由 protocol 重新导出 relay-ticket 里的常量。
4. 示例程序：`crates/yttt-mobile-core/examples/spike_host.rs` 和 `crates/yttt-client-core/examples/p2p_probe.rs` 去掉 `--relay` 参数，或者改为接控制面。
5. 收尾检查：`rg -n "OpenConfig|open_account_id|PairingHint|relay-selfhost|SwitchNode\b"` 没有结果；`rg -n "relay_url" crates src` 只剩记录、ticket 和节点相关的地方。
6. 验证：`cargo test --workspace`、`cargo clippy --workspace --all-targets`。

## T9 控制台（`web/control-console`）

1. `src/pages/Devices.tsx`：删除切换节点的操作，节点只读显示，并说明"节点由 Host 选择"；如果 Host 有 `placement_needed`，也显示出来。
2. `src/pages/admin/Nodes.tsx` 和 `AccountDetail.tsx` 的"迁移"：文案改为"把默认节点改为 …（只影响以后登录的 Host）；要让现有 Host 离开某个节点，就停用它，Host 上会提示用户选择新节点"。
3. 验证：`npm run build`（项目没有前端测试脚本），并在 `packaging/relay-dev` 的 compose 环境里手动点一遍。

## T10 文档

- `docs/relay-operations.md`：重写"节点归属""切换的传播""节点停用"三条，删除控制台和管理员会移动设备的说法。
- `docs/p2p-relay-architecture.md` §1.2：relay 只剩控制面模式；补充签名位置记录。
- `docs/p2p-testing.md` §4：本机测试 relay 改为 `packaging/relay-dev` 的 compose。
- `CHANGELOG.md`：写明不兼容——自建 relay 需要部署控制面，旧配对要重新配对，停用节点不再自动迁移 Host。

---

## 完成标准

- [ ] `cargo test --workspace` 通过，不需要外部 PostgreSQL，也没有新增 `#[ignore]`。
- [ ] Host 切换节点后，已连接的手机和桌面 Client 在 5 s 内经新节点恢复，现有会话不断。
- [ ] 控制面（包括控制台和管理员）无法让 Client 连到 Host 没签过的节点或地址；节点停用或地址变更时，Host 不自动签名，只提示用户。
- [ ] 伪造和回滚的记录都被拒绝（测试覆盖）。
- [ ] 控制面不可达时，Client 用缓存连上；控制面恢复后，relay 自动补上。
- [ ] 代码和打包里不再有 `[open]` 模式和配对地址退路。
- [ ] 真机：iPhone 和线上控制面验证通过。
