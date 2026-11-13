# Host 签名的 relay 位置：设计

- 日期：2026-11-13（同日按审查意见修订）
- 分支：`feat/host-relay-placement`（worktree `.worktrees/host-relay-placement`）
- 基线：`master@1451de0`
- 计划：[`2026-11-13-host-relay-placement-plan.md`](2026-11-13-host-relay-placement-plan.md)
- 前提：目前只有一个用户，不兼容旧数据和旧客户端，Host、控制面、桌面与手机一起升级。

## 1. 问题

Host 换节点以后，连接它的设备经常要过很久才跟上，有时根本跟不上。原因有五个：

1. **已连接的设备迟迟收不到通知。** 旧节点下一次上报（约 15 s）就会断开离开的设备，而 Client 只在 `keep_ticket` 里每 5 min（`NODE_CHECK`，`crates/yttt-control-client/src/lib.rs`）查一次节点，iroh 重试时也不报错。
2. **退路地址停在配对那一刻。** 手机的 `MobileSession::connect` 把 ticket 的 `relay_url` 写进 `record.host` 的克隆，`PairedHostRecord` 本身从不更新（`crates/yttt-mobile-core/src/session.rs`），桌面 `src/remote_host.rs` 也一样。控制面 5 s 内没有响应（`TICKET_WAIT`）时，就退回配对码里的旧地址。
3. **自建（`[open]`）模式没有更新渠道。** 配对码里的 `relay_url` 是唯一的信息来源。
4. **控制面可以绕过 Host 移动它。** 控制台调 `switch_node` 时跳过证明和冷却（`session.console`）；`move_node` / `move_accounts` 会移动设备；`ticket_node` 会自动把 Host 挪回默认节点。
5. **Client 没有缓存 ticket**，并且第一次没拿到 ticket 时根本不启动续期（`session.rs` 与 `remote_host.rs` 都只在拿到首张 ticket 后才 `spawn(keep_ticket)`）。

## 2. 目标

- **节点只由 Host 上的用户操作决定。** 控制面就算被攻破，也不能让 Client 连到 Host 没签过的节点；它最多只能让 relay 不可用。节点被停用、地址变更、套餐变化时，Host **不自动跟随**，而是停用 relay、只走直连，并提示用户选择。
- **在线设备几秒内跟上**，离线设备下次连接时跟上；控制面短时间不可达时，用缓存连接。
- **只保留一条 relay 链路：控制面 + 节点。** 删除 `[open]` 模式、Host 手填 relay 地址和配对地址退路。
- 保留不依赖控制面的两条路径：纯直连，以及 TLS 入口 + 隧道。

## 3. 不做

- 兼容旧数据（见前提）。数据库仍然要加迁移文件，因为线上已经有库。
- 让没有 Host 所在账号的设备也能使用 relay（见 §10）。
- 控制面安装脚本的"个人模式"。
- iroh pkarr / DNS 地址发布：它会一并发布直连 IP，还要多一个服务。

## 4. 安全模型

| 角色 | 能做什么 | 做不到什么 |
|---|---|---|
| Host 上的用户 | 选节点；确认节点地址变更 | — |
| Host 程序 | 在用户操作时签名位置记录 | 未经用户操作签名（只有登录那一刻例外，见 §6.4） |
| 控制面 | 存放和转发记录；发或不发 ticket；提示 Host 需要换节点 | 伪造或回滚记录；让 Client 连到未签名的节点或地址 |
| 节点 | 转发端到端加密的流量 | 读写内容 |
| Client | 只拨验签通过、且 `seq` 最新的记录里的 relay | — |

relay 本来就只转发端到端加密的流量，被引到恶意 relay 也只泄露元数据。这个设计保证的是：**设备走哪个节点、哪个地址，只由 Host 上的人决定。** 控制面对 relay 的权力只剩"不发 ticket"这一项。

**ticket 本身的安全。** ticket 绑定设备的 `endpoint_id` 和 `node_id`（`yttt-relay-ticket` 的 `Ticket`），节点准入时连接方必须持有对应的设备密钥。所以 ticket 泄露也用不了，可以和配对记录存在一起（§6.3）。

## 5. 位置记录

放在 `yttt-relay-ticket`（新文件 `src/placement.rs`）。控制面、`yttt-control-client`、`yttt-client-core` 已经依赖这个 crate；`yttt-protocol` 新增对它的依赖（它只依赖 ciborium、ed25519-dalek、serde 等，没有环）。

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPlacement {
    #[serde(with = "serde_bytes")]
    pub host: [u8; 32],      // Host 的 endpoint key，即 Client 配对时记下的信任锚
    pub seq: u64,            // 单调递增
    pub node_id: String,     // ≤ 64 字节
    pub relay_url: String,   // ≤ 256 字节
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPlacement {
    pub placement: HostPlacement,
    #[serde(with = "serde_bytes")]
    pub signature: [u8; 64], // Ed25519，签 placement_bytes(&placement)
}
```

- **签名字节**：`b"yttt host placement v1\0" ‖ host ‖ seq(BE u64) ‖ u16 长度 ‖ node_id ‖ u16 长度 ‖ relay_url`。布局固定，不依赖 serde。
- **编码**：只用 derive，不手写 serde。CBOR 中是字节串，JSON 中是数字数组，不美观但能用，而且 wire 样本工具能遍历到。
- **`verify(expected_host)`**：签名有效、`host` 相符、两个字符串的长度在上限内。
- **"更新"的定义**：验签通过，且 `seq` 更大。`seq` 相同时内容必须完全一致，否则视为异常并拒绝。
- **Host 的 `seq` 永不复用。** Host 在 profile root 下持久化 `relay-placement.json`，内容是 `{ next_seq, current: Option<SignedPlacement> }`。**签名之前**先把 `next_seq + 1` 写盘（fsync + rename），再用旧值签名。这样即使提交失败（节点满载、冷却中），这个 `seq` 也不会再用到另一条内容上。控制面返回的 `current` 如果比本地的更新，就采用它，`next_seq` 相应抬高。

## 6. 流程

### 6.1 控制面存放

- `devices` 表增加 `placement TEXT NULL`（`SignedPlacement` 的 JSON）和 `placement_seq BIGINT NOT NULL DEFAULT 0`。`devices.node_id` 仍是 ticket 和计费的依据，**只在接受一条新记录时**才会改变。
- **`PUT /v1/devices/{host}/node`**：请求体为 `{ placement: SignedPlacement }`（去掉 `node_id` 和 `proof`：签名本身就证明 Host 持有密钥）。控制面依次检查：
  1. 签名有效，`placement.host` 等于路径里的 Host，并且是本账号的 Host；
  2. `seq` 大于已存的 `seq`；
  3. 节点在套餐内、处于启用状态，`relay_url` 等于该节点当前的地址；
  4. 节点没有满载，除非它就是 Host 当前所在的节点（确认地址变更时）；
  5. 冷却 10 min，以下两种情况豁免：Host 当前节点不可用，或者 `node_id` 不变（地址变更）；
  6. 节点数量：`max_nodes <= 1` 时先把 `accounts.node_id` 改成目标节点，再跳过计数检查（否则默认节点和目标节点一起会被算成 2 个，永远通不过）；`max_nodes > 1` 时照旧用 `nodes_in_use` 计数。

  全部通过后，存下记录；`node_id` 变了才执行 `move_devices(HostAndClients)`。
- **去掉控制面移动 Host 的所有路径**：
  - 删除 `session.console` 分支；
  - `move_node` / `move_accounts` 只改账号默认节点和 `plan_nodes`，不再移动设备；
  - `ticket_node` 不再移动任何设备；
  - 注册时拒绝 Unknown 角色。

### 6.2 取 ticket

`POST /v1/tickets` 的响应改为：

```rust
enum TicketOutcome {
    Granted { ticket: TicketGranted, host_placement: Option<SignedPlacement> },
    PlacementNeeded(PlacementNeeded),
}
struct PlacementNeeded {
    reason: Missing | NodeUnavailable | AddressChanged,
    current: Option<SignedPlacement>,   // Host 已存的记录
    suggested: Option<NodeChoice>,      // 套餐内启用且未满的节点，优先账号默认节点
}
```

- **Host 请求**：
  - 没有记录 → `Missing`；
  - 记录里的节点停用、不在套餐里、或者超出 `max_nodes` → `NodeUnavailable`；
  - 节点仍可用，但地址和记录不一致 → `AddressChanged`；
  - 其余情况 → `Granted`，`host_placement` 为它自己的记录。

  只要是 `PlacementNeeded`，就不发 ticket。
- **Client 请求**：Host 有记录，并且记录里的节点可用 → `Granted`，ticket 发给 `devices.node_id`（等于记录里的节点），附 `host_placement`；否则返回 `PlacementNeeded`，原因与 Host 的相同，Client 只走直连。

### 6.3 Client 选用哪条记录

- 配对记录（`PairedHost`）增加 `placement: Option<SignedPlacement>`（替代 `relay_url`）和 `ticket: Option<RelayTicket>`。
- 每拿到一条记录（来自 ticket 响应或会话事件），都调用 `accept_placement(signed)`：验签通过且 `seq` 更大才替换，替换后通过回调让应用持久化。拿到新 ticket 时同样持久化。
- **拨号**：
  - `PeerAddr.relay_url` 取自记录；
  - ticket 只在 `ticket.node_id == placement.node_id` 时使用，不一致就记一条连接日志，只走直连；
  - 没有记录也只走直连。
- **控制面 5 s 内没有响应时**，如果缓存的 ticket 还 `usable()`，并且节点与记录一致，就直接用它。
- **只要登录了账号，就一直运行 `keep_ticket`**：没有首张 ticket 时立即开始重试，退避与现在相同。

### 6.4 Host 什么时候签名

签名只发生在下面两种用户操作里：

| 用户操作 | 签什么 |
|---|---|
| **登录 relay 账号**（应用浏览器登录、短码、`yttt-server p2p login`） | 登录结果里显示的节点：已有记录就沿用；收到 `Missing` 就签 `suggested`，界面上要显示这个节点 |
| **选择节点**（应用节点列表、`yttt-server p2p node <id>`），以及确认地址变更 | 用户选的节点 |

用户选择节点时的步骤（实现时按测试中的实际行为调整过顺序，见 §10）：

1. 预留 `seq`，签名，`PUT /node`，存盘；
2. 取新 ticket；
3. **关闭经 relay 的 P2P 连接**（`IrohEndpoint::close_relayed`）。此时旧节点还在，关闭帧送得到，这些设备立即重拨；
4. 对 Host 的 endpoint 调用 `set_relay`，**不重启 P2P**，并等到新节点成为 home relay（最多 10 s）；
5. 更新共享状态，通过 `RelayPlacementChanged` 推给仍在的会话（直连的会话不断，收到推送后更新地址）。

**Host 收到 `NodeUnavailable` 或 `AddressChanged` 时不签名。**
- Host 的 relay 关闭，只走直连；`RelayAccountStatus` 增加 `placement_needed: Option<PlacementNeeded>`。
- 桌面"远程访问"里显示提示，比如"节点 X 已停用，请选择节点"，或者"节点 X 的地址变为 Y，确认后继续使用"，并带一个按钮。
- `yttt-server p2p status` 显示同样的信息。
- 用户确认后走"选择节点"的步骤。

`max_nodes <= 1` 时，一台 Host 换节点会把账号默认节点一起改掉，其他 Host 下次取 ticket 时收到 `NodeUnavailable`，也会出现上面的提示。

### 6.5 会话内推送

- 新增事件 `ServerEvent::RelayPlacementChanged(SignedPlacement)`，只追加，不升协议版本（`docs/wire-evolution.md` 规则 1）。
- Host 的每个会话在握手完成后，先发一次当前记录；之后每次变化再发。用 `writer.send_reliable_latest("relay-placement", …)` 发送，只保留最新的一条。本地 GUI 会话也会收到，它会忽略。
- Client 收到事件后先 `accept_placement`。记录变新、并且节点或地址变了时：
  1. 取新 ticket；
  2. `endpoint.set_relay(target)`；
  3. `connector.set_peer_relay(url)`：只关闭不是直连的连接，直连的会话继续用。
- 经 relay 的会话收不到这条推送（Host 切换前已关闭它们），它们靠重拨前 hook 从控制面得知新位置（§6.6）。

### 6.6 连接层的改动（`yttt-transport-iroh`）

- `IrohConnector.peer` 改为放在互斥锁里，新增 `set_peer_relay(Option<String>)`：替换对端的 relay 地址，并关闭缓存的连接，下一个 `connect` 会用新地址重拨。
- 新增 `IrohConnector::with_before_redial(hook)`：缓存的连接已关闭、准备重拨之前，先 `await` 这个 hook，最多等 `TICKET_WAIT`。应用在 hook 里取 ticket、`accept_placement`，必要时调 `set_relay` 和 `set_peer_relay`。这样断线后会立刻重查，而不是等 `NODE_CHECK`。`ClientCore` 的重连逻辑不需要改。

### 6.7 配对

- `PairingCode` 去掉 `relay_url`，加 `control_url: Option<String>`（≤ 256 字节）。码里不放记录：经 relay 配对本来就要 ticket，ticket 响应里有记录，连上后的第一次推送也会送到。
- `pair(…, direct_only: bool)`：码里有 `control_url`、设备没登录、并且 `direct_only == false` 时，返回 `SignInRequired { control_url }`。用户选择"跳过，只用局域网"时，以 `direct_only = true` 重试。

## 7. 删除的部分

- `yttt-relay`：`OpenConfig`、`config.open`、`open_account_id`、`Node.open` 和 `admit` 里的 open 分支、`StartError::OpenWithControl`；改写 `NoTicketKeys` 的文案。删除 `packaging/relay-selfhost/`。
- `P2pSettings.relay_url`、`SetP2p.relay_url`、`yttt-server p2p enable --relay`、桌面的 relay 地址输入框。这些都是删除，按 wire 规则本该升版本；因为前提是所有组件一起升级，这里**有意破坏兼容**，在 `docs/wire-evolution.md` 里记一笔。
- `ProofPurpose::SwitchNode`、`PairedHost.relay_url`、`RelaySource::PairingHint`、手机端 `hint()`、桌面的配对地址退路。
- 控制台"设备"页的切换节点操作。

## 8. 测试

- **单元测试**：签名字节布局快照；验签各种失败；`seq` 回退、`seq` 相同而内容不同都被拒绝；`next_seq` 先写盘再签名；`accept_placement` 的各分支。
- **控制面**：`PUT /node` 的六项检查，其中 `max_nodes = 1` 的切换能成功；控制台会话无法移动 Host；`TicketOutcome` 的各分支；`move_node` 不再移动设备。`crates/yttt-control` 现有测试里依赖"控制面移动设备"的用例要改写，例如 `tests/control.rs` 中账号迁移会带走 Client 的那个。
- **集成测试**（T0 的 SQLite 夹具）：
  1. Host 切换节点，已连接的 Client 在 5 s 内经新节点恢复，现有会话不断；
  2. 控制面伪造的记录（另一把密钥签）和回滚的记录都被拒绝；
  3. 控制面不可达，Client 用缓存的 ticket 和记录连上；
  4. 停用节点后 Host 不自动切换，只走直连，并出现 `placement_needed`；用户选好节点后，Client 跟上；
  5. 首次连接时控制面超时，之后控制面恢复，`keep_ticket` 补上 relay。
- **真机**：iPhone（`scripts/mobile/run-ios-device.sh`）接自己的线上控制面，在 Host 上切换节点，手机不需要任何操作就能恢复。

## 9. 审查记录

2026-11-13 审查后的主要修订：

- 不自动跟随控制面的建议（原设计在 `NodeUnavailable` / `AddressChanged` 时自动重签，控制面因此能间接移动 Host）；
- `max_nodes <= 1` 时改为先改默认节点，再跳过计数检查；
- 新增 `TicketOutcome`，表达"不发 ticket"；
- `seq` 先写盘再签名，并能采用控制面返回的更新记录；
- ticket 缓存放进配对记录；始终运行 `keep_ticket`；
- 连接层新增 `set_peer_relay` 和重拨前 hook；
- 切换节点时用 `set_relay`，不重启 P2P；
- 配对码只带 `control_url`；
- 去掉 `issued_unix`；
- 按"删除最后一个使用者时再删除"重排计划，保证每一步都能编译。

## 10. 实现与设计的差异（2026-11-13）

- **切换顺序**：原设计是"先推送、再 `set_relay`"。集成测试中，经 relay 的会话在 Host 切换后静默失效，要等 20 s 心跳超时；手机若在 Host 完成 home relay 切换（iroh 约 3 s）之前重拨，新连接又会在切换完成时断开。现在的做法见 §6.4：Host 先关闭经 relay 的连接，再切换并等到新节点成为 home。
- **设备等 Host 站稳**：重拨前 hook 发现 Host 刚换了节点（记录里的地址变了）时，先等 `HOST_SETTLE`（5 s）再拨。集成测试里手机在切换后约 2–5 s 恢复。
- **桌面配对**：没登录时不弹出登录，而是先尝试直连配对；失败时错误信息说明要登录哪个控制面。
- **`MAX_RELAY_URL_BYTES`**：没有把常量统一到 `yttt-relay-ticket`，两处各自保持 256。
- **wire 兼容**：按 §7 有意删除 `P2pSettings.relay_url` 和 `SetP2p.relay_url`，没有升协议版本，已记入 `docs/wire-evolution.md`。

## 11. 以后可以做

- Host 替已授权设备申请 ticket，设备不用登录 Host 的账号就能使用 relay。
- 控制面安装脚本的"个人模式"。
