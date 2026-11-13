# Host 签名的 relay 位置：实施计划

- 日期：2026-11-13
- 设计：[`2026-11-13-host-relay-placement-design.md`](2026-11-13-host-relay-placement-design.md)
- 基线：`master@1451de0`
- 执行：按 `AGENTS.md` 在 worktree 里直接执行，不交给 subagent。实现分支建议为 `feat/host-relay-placement`（新建 worktree `.worktrees/host-relay-placement-impl`，本分支只放文档）。
- 前提：不兼容旧数据和旧客户端，所有组件一起升级。

## 顺序和依赖

```
T1 记录类型 ──┬─ T2 控制面 ──┐
              ├─ T3 协议 ────┼─ T4 Host ─┐
              │              └─ T5 Client 公共逻辑 ─┬─ T6 手机核心 ─ T8 iOS/Android
              │                                     └─ T7 桌面 Client
T0 测试夹具（SQLite 控制面）── 给 T2/T4/T6/T7 的集成测试用
T9 删除自建链路、T10 控制台、T11 文档 ── 最后做
```

每一项都先写失败的测试，再实现，最后跑该 crate 的测试和 `cargo clippy -p <crate> --all-targets`。每完成一项提交一次。

---

## T0 测试夹具：SQLite 控制面

目前控制面的集成测试（`crates/yttt-client-core/tests/paid_relay.rs`、`crates/yttt-mobile-core/tests/paid_relay.rs`）都要 `YTTT_TEST_DATABASE_URL` 指向一个 PostgreSQL，并且带 `#[ignore]`。删掉 `[open]` 之后，所有走 relay 的测试都需要控制面，所以先把夹具换成 SQLite。

1. 在 `crates/yttt-control` 加 `test-support` feature，导出 `test_support::spawn(node_ids: &[&str]) -> (Control, Vec<RelayNode>, String)`。内容就是现在 `paid_relay_nodes` 的逻辑，数据库改为临时目录下的 `sqlite://…/control.db`（`db.rs` 已有 `sqlite_opens_with_its_schema_and_reads_booleans` 证明可用）。同时导出 `account(control_url)` 和 `console(control_url)` 两个辅助函数。
   - 这个 feature 可选依赖 `yttt-relay`。两者目前互不依赖，不会成环。
2. 两个 `paid_relay.rs` 改用这个夹具，去掉 `#[ignore]`。设置 `YTTT_TEST_DATABASE_URL` 时仍然跑 PostgreSQL，保留双方言覆盖。
3. 验证：`cargo test -p yttt-client-core --test paid_relay`、`cargo test -p yttt-mobile-core --test paid_relay`，不依赖任何外部服务就能通过。

## T1 位置记录类型（`crates/yttt-relay-ticket`）

新文件 `src/placement.rs`，在 `lib.rs` 里导出。

1. 测试：
   - `placement_bytes` 的快照（固定输入 → 固定 hex），保证签名布局不会被无意改动；
   - 签名 → 验签往返；
   - 以下情况被拒绝：改动 `node_id` 后验签、`expected_host` 不同、`node_id` 超过 64 字节、`relay_url` 超过 256 字节；
   - JSON（hex）和 CBOR（bytes）两种编码各自往返。
2. 实现：`HostPlacement`、`SignedPlacement::sign(secret, placement)`、`verify(expected_host)`，以及 `newer_than(&self, other: Option<&SignedPlacement>) -> Result<bool, PlacementError>`：`seq` 更大返回 `true`；`seq` 相同且内容相同返回 `false`；`seq` 相同而内容不同返回错误。
3. 把 `MAX_RELAY_URL_BYTES = 256` 移到这里，`yttt-protocol::p2p` 改为重新导出。
4. 删除 `ProofPurpose::SwitchNode`（T2 不再使用）。

## T2 控制面（`crates/yttt-control`）

1. 迁移：`migrations/{postgres,sqlite}/0002_host_placement.sql`，给 `devices` 加 `placement TEXT NULL`、`placement_seq BIGINT NOT NULL DEFAULT 0`。
2. API 类型（`yttt-relay-ticket/src/api.rs`）：
   - `SwitchNode` 改为 `{ placement: SignedPlacement }`，去掉 `node_id` 和 `proof`；
   - `TicketGranted` 追加 `host_placement: Option<SignedPlacement>`、`placement_needed: Option<PlacementNeeded>`，均为 `#[serde(default)]`；
   - 新增 `PlacementNeeded { reason: Missing | NodeUnavailable | AddressChanged, node_id, relay_url }`。
3. `placement::switch_node`，每项检查先写一个失败测试，用 T0 的 SQLite 夹具：
   - 删除 `session.console` 分支：控制台会话提交的请求同样必须带有效签名；控制台不持有 Host 的密钥，所以实际上无法移动 Host。测试：控制台会话 `PUT /node` 返回 403。
   - 依次检查：签名有效、`host` 与路径一致、`seq` 递增、节点在套餐内且启用且未满载、`relay_url` 等于节点当前地址、冷却时间（同 `node_id` 的重签不受冷却限制）、`max_nodes`。
   - 通过后写入 `placement` 和 `placement_seq`，`node_id` 变了才调用 `move_devices(HostAndClients)`。
   - `max_nodes <= 1` 时：只移动这台 Host 和它的 Client，同时把 `accounts.node_id` 改成新节点；其他 Host 由 T4 的 `NodeUnavailable` 自己跟过去。
4. `placement::ticket_node` 和 `api::issue_ticket`：
   - Host 请求：
     - 没有记录 → 不发 ticket，返回 `placement_needed { Missing, 当前 node }`；
     - 节点不再允许、或超出 `max_nodes` → 不移动，返回 `NodeUnavailable` 并附账号默认节点；
     - 节点地址和记录不一致 → `AddressChanged`；
     - 其余情况照常发 ticket，并附 `host_placement`（它自己的记录）。
   - Client 请求：按 `devices.node_id`（随 Host 变化）发 ticket，附 Host 的 `host_placement`。Host 还没有记录时只发 ticket、`host_placement` 为空，Client 侧会因此不用 relay（T5）。
   - 删除 `ticket_node` 里移动 Host 和 Unknown 设备的逻辑。Unknown 角色已经没有存量，注册时直接拒绝，返回 `a device says whether it is a Host or a Client`。
5. `nodes::move_accounts` / `placement::move_node`：只改 `accounts.node_id`（以及 `plan_nodes` 的开放），不再调用 `move_devices`。停用节点以后，其上的 Host 下次取 ticket 时收到 `NodeUnavailable`，自己切过去。
6. 验证：`cargo test -p yttt-control`、`cargo clippy -p yttt-control --all-targets`。

## T3 协议（`crates/yttt-protocol`）

1. `Cargo.toml` 增加对 `yttt-relay-ticket` 的依赖。
2. `ServerEvent` 末尾追加 `RelayPlacementChanged(SignedPlacement)`。
3. `PairingCode`：去掉 `relay_url`，增加 `control_url: Option<String>` 和 `placement: Option<SignedPlacement>`（`#[serde(default)]`）。`decode` 检查 `control_url` 的长度不超过 256，`placement` 只检查长度、不验签，验签在 T5 用 `code.host` 做。
4. 运行 `YTTT_BLESS_WIRE_SHAPES=1 cargo test -p yttt-protocol --test wire_shapes`，重写样本，并在 diff 里确认只有追加的部分和 `PairingCode` 的变化。
5. 在 `docs/wire-evolution.md` 的"当前版本"里追加一段：v22 内追加 `RelayPlacementChanged`，旧 Client 跳过。
6. 验证：`cargo test -p yttt-protocol`。

## T4 Host（`crates/yttt-host`、`crates/yttt-server`）

1. **持久化**：在 `p2p.rs` 加 `load_placement(root)` 和 `save_placement(root, &SignedPlacement)`，文件 `relay-placement.json`，写入方式与 `save_relay_account` 相同（临时文件 + rename）。
2. **共享状态**：Host context 里加 `placement: watch::Sender<Option<SignedPlacement>>`。启动时从文件载入；`remote_access.rs` 签名成功后 `send_replace`。
3. **会话推送**（`crates/yttt-host/src/lib.rs` 的会话循环，就在 `context.workspaces.subscribe` 附近）：每个会话 `subscribe` 这个 watch，握手完成后立刻发送当前值，之后每次变化都用 `writer.send_reliable_latest("relay-placement", …)` 发送 `ServerEvent::RelayPlacementChanged`。本地 GUI 会话也会收到，它会忽略这个事件。
4. **签名入口**（`remote_access.rs`），统一放在一个函数里：`sign_and_switch(node_id, relay_url) -> Result<SignedPlacement>`。它依次：取 `seq = max(本地, 控制面返回的) + 1` → 签名 → 调 `switch_node` → 存文件 → `send_replace`。
   - `RemoteAccessRequest::SetRelayNode`：先查 `list_nodes` 拿到目标节点的 `relay_url`，调 `sign_and_switch`，等 1 s（或者等所有会话的 writer 把事件发出去，以先到者为准），再 `restart_p2p`。
   - `keep_relay_ticket` 收到 `placement_needed`：按 `reason` 调 `sign_and_switch`，然后重新取 ticket，并在 `RelayAccountStatus` 里加 `notice: Option<String>`，例如"节点 X 已停用，已切到 Y"，供界面显示。节点变化时照常触发重启（沿用现有的 `restart_after_unix` 机制）。
   - `yttt-control-client`：`keep_ticket` 遇到 `placement_needed` 时交给回调处理，签好名后继续循环。改法是把 `renewed` 回调扩展为一个 `TicketEvents` trait：`renewed`、`failed`、`placement_needed`。
5. **配对码**（`remote_access.rs` 约 704 行）：`control_url` 取自已登录的账号，`placement` 取自当前记录。没有账号时两项都为 `None`，只能直连配对。
6. **测试**（`crates/yttt-client-core/tests/paid_relay.rs`，改写现有的 `a_host_moves_to_another_node_and_its_device_follows`）：
   - Host 切换节点 → 已连接的 Client 在 5 s 内收到 `RelayPlacementChanged`，`seq` 加一，`node_id` 是新节点；
   - 管理员停用 Host 所在节点 → Host 自动切到默认节点，`status.notice` 不为空；
   - 新 Host 第一次登录 → 收到 `Missing` 后自动签名，然后拿到 ticket。
7. 验证：`cargo test -p yttt-host`、`cargo test -p yttt-client-core --test paid_relay`、`cargo test -p yttt-server`。

## T5 Client 公共逻辑（`crates/yttt-client-core/src/p2p.rs`）

1. `PairedHost`：用 `placement: Option<SignedPlacement>` 替换 `relay_url`。
2. `accept_placement(&mut self, signed) -> Result<bool, PlacementError>`：用 `self.host` 验签，`newer_than` 判断，替换时返回 `true`。
3. `peer_addr()`：`relay_url` 取自 `placement`；没有记录就是 `None`，只走直连。
4. `pair()`：配对成功后，对码里的 `placement` 调 `accept_placement`。验签失败时配对照样完成，只是不保存记录，并记日志。
5. 新函数 `ticket_for_placement(ticket, placement) -> Option<RelayTarget>`：两者的 `node_id` 一致才返回 target，否则返回 `None` 并给出原因字符串，用于连接日志。
6. 单元测试覆盖 2、3、5 的所有分支。
7. 验证：`cargo test -p yttt-client-core --lib`。

## T6 手机核心（`crates/yttt-mobile-core`）

1. **记录持久化**：`MobileSession` 持有 `Arc<Mutex<PairedHostRecord>>`。每次 `accept_placement` 返回 `true` 时，通过 FFI 回调 `on_record_changed(bytes)` 交给应用保存。改动 `ffi.rs` 和 UniFFI 接口：`connect` 增加一个 `RecordSink` 回调对象。
2. **`connect`**：
   - 最多等 `TICKET_WAIT` 取 ticket，并对 `host_placement` 调 `accept_placement`；
   - 超时就用缓存的 ticket（见下一项）；
   - 最后用 `ticket_for_placement` 决定用不用 relay。
3. **ticket 缓存**：`Account`（`account.rs`）按 Host 缓存最近的 `RelayTicket`，随 `SignedIn` 一起序列化，由应用存进 Keychain / Keystore。`ticket()` 成功时更新缓存；`cached_ticket(host)` 只返回仍然 `usable()` 的那张。
4. **事件**：会话收到 `RelayPlacementChanged` → 调 `accept_placement`；返回 `true` 且 `relay_url` 变了 → 重新取 ticket，`set_relay`，用新的 `PeerAddr` 重建 connector，然后走现有的重连路径（`MobileSession` 里 `connect_with` 的那套逻辑）。
5. **断线立即重查**：现有重连路径在第一次重拨之前先调一次 `account_ticket`，不等 `NODE_CHECK`。
6. **删除**：`RelaySource::PairingHint`、`hint()`。`RelaySource` 只剩 `Account` 和 `None` 两个值，可以考虑把 `None` 改名为 `DirectOnly`。
7. **配对**：`pair()` 在码里有 `control_url`、设备却没登录时返回 `MobileError::SignInRequired { control_url }`，应用据此引导登录。
8. **测试**：
   - `tests/session.rs` 里依赖 `[open]` 节点的用例改用 T0 夹具；
   - `tests/paid_relay.rs` 新增三个用例：①手机连着时 Host 切换节点，手机在 5 s 内经新节点恢复，并且 `on_record_changed` 被调用；②控制面不可达（关掉 `Control`），手机用缓存的 ticket 连上；③伪造的记录（用另一把密钥签）被忽略。
9. 验证：`cargo test -p yttt-mobile-core`。

## T7 桌面 Client（`src/remote_host.rs`、`src/paired_hosts.rs`、`src/relay_account.rs`、`src/ui/app/paired_host.rs`）

1. `paired_hosts::save_records` 已经存在。`remote_host.rs` 在 `accept_placement` 返回 `true` 时通知 UI 线程保存，复用 `paired_host.rs:328` 附近的保存路径。
2. 用 `ticket_for_placement` 替换 `remote_host.rs` 约 520 行的"配对地址退路"。
3. ticket 缓存放进 `relay_account` 的会话存储，和手机做法相同。
4. 处理 `RelayPlacementChanged` 事件，并在断线时立即重查，和 T6 第 4、5 项相同。
5. 扫码配对：码里有 `control_url`、但还没登录时，弹出登录（复用现有的浏览器登录）。
6. 验证：`cargo test -p yttt remote_host paired_hosts`；然后手动测一遍：本机起两个 profile，一个当 Host、一个当 Client，在 Host 上切换节点，看 Client 能不能恢复。

## T8 iOS / Android

1. 实现 `RecordSink`：iOS 写回存放 `PairedHostRecord` 的地方（`AppModel` 管理的 Host 列表），Android 同理，都用原子写。
2. `SignedIn` 的序列化里多了 ticket 缓存，存储方式不变（Keychain / Keystore）。
3. 配对页处理 `SignInRequired`：显示"这台电脑使用 `<control_url>`，先登录"，跳到现有的登录页，登录后自动继续配对。
4. 连接日志（设置 → 连接日志）里显示 `ticket_for_placement` 给出的拒绝原因。
5. 验证：在 iPhone 上用 `scripts/mobile/run-ios-device.sh`，接自己的线上控制面——配对，然后在 Host 上切换节点，手机不需要任何操作就能恢复；Android 在模拟器上走一遍配对和登录。

## T9 删除自建链路

1. `crates/yttt-relay`：删除 `OpenConfig`、`config.open`、`open_account_id`、`Node.open` 以及 `admit` 里的 open 分支、`StartError::OpenWithControl`，改写 `NoTicketKeys` 的文案。`tests/endpoints.rs` 里的 open 用例改为有 ticket key 的节点，用测试密钥签 ticket，不需要控制面。
2. 删除 `packaging/relay-selfhost/`；`packaging/relay-dev/relay.toml` 里如果有 `[open]` 也一并删掉。
3. `yttt-protocol::remote_access`：删除 `P2pSettings.relay_url` 和 `SetP2p.relay_url`；重写 wire 样本。
4. `yttt-host/src/remote_access.rs`：`start_p2p` 没有账号时用 `RelaySetup::Off`（只走直连）；删除 `SetP2p` 里的 URL 校验。
5. `yttt-server/src/p2p_cli.rs`：删除 `enable --relay`，更新 `P2P_USAGE` 及其测试。
6. `src/ui/workbench/remote_access_p2p.rs`：删除 relay 地址输入框；没登录账号时显示"登录 relay 账号后可在外网访问，目前只能直连"。
7. `crates/yttt-mobile-core/examples/spike_host.rs`、`crates/yttt-client-core/examples/p2p_probe.rs`：删除 `--relay` 参数，或者改为接控制面。
8. 收尾检查：`rg -n "relay_url" crates src mobile` 的结果只剩记录、ticket 和节点相关的地方；`rg -n "\[open\]|OpenConfig|PairingHint|relay-selfhost"` 没有结果。
9. 验证：`cargo test --workspace`、`cargo clippy --workspace --all-targets`。

## T10 控制台（`web/control-console`）

1. `src/pages/Devices.tsx`：删除切换节点的操作，节点改为只读显示，旁边加说明"节点由 Host 自己选择"。
2. `src/pages/admin/Nodes.tsx` 和 `AccountDetail.tsx` 里的"迁移"：文案改为"把这些账号的默认节点改为 …；Host 下次核对时自己切换（最长约 5 分钟）"。管理员要立刻生效，就停用旧节点。
3. 验证：`npm run build`（项目没有前端测试脚本）；在 `packaging/relay-dev` 的 compose 环境里手动点一遍。

## T11 文档

- `docs/relay-operations.md`：重写"节点归属"和"切换的传播"两条（约 177–182 行），删除"控制台'设备'页"切换和管理员迁移会移动设备的说法。
- `docs/p2p-relay-architecture.md` §1.2：relay 只剩控制面模式；补充签名位置记录。
- `docs/p2p-testing.md` §4：本机测试 relay 改为 `packaging/relay-dev` 的 compose（控制面 + 节点）。
- `CHANGELOG.md`：写明不兼容——自建 relay 需要部署控制面，旧配对要重新配对。

---

## 完成标准

- [ ] `cargo test --workspace` 通过，不需要外部 PostgreSQL，也没有新增 `#[ignore]`。
- [ ] Host 切换节点后，已连接的手机和桌面 Client 在 5 s 内经新节点恢复，不需要用户操作。
- [ ] 控制台和管理员都无法让 Host 换节点；停用节点后，Host 自己切换并留下提示。
- [ ] Client 只使用验签通过、且 `seq` 最新的记录里的 relay；伪造和回滚的记录都被拒绝（测试覆盖）。
- [ ] 控制面不可达时，Client 用缓存的 ticket 连上。
- [ ] 代码和打包里不再有 `[open]` 模式和配对地址退路。
- [ ] 真机：iPhone 和线上控制面验证通过。
