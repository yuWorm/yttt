# Host 签名的 relay 位置：设计

- 日期：2026-11-13
- 分支：`docs/host-relay-placement`（worktree `.worktrees/host-relay-placement`）
- 基线：`master@1451de0`
- 计划：[`2026-11-13-host-relay-placement-plan.md`](2026-11-13-host-relay-placement-plan.md)
- 前提：目前只有一个用户，不考虑兼容旧数据和旧客户端，Host、控制面、桌面与手机一起升级。

## 1. 问题

Host 换节点以后，连接它的设备经常要过很久才跟上，有时根本跟不上。原因有五个：

1. **已连接的设备迟迟收不到通知。** 旧节点下一次上报（约 15 s）就会断开离开的设备，而 Client 只在 `keep_ticket` 里每 5 min（`NODE_CHECK`，`crates/yttt-control-client/src/lib.rs`）查一次节点，iroh 重试时也不报错。会话可能断开最多 5 min，没有任何提示。
2. **退路地址停在配对那一刻。** 手机的 `MobileSession::connect` 把 ticket 里的 `relay_url` 写进 `record.host` 的克隆，`PairedHostRecord` 本身从不更新（`crates/yttt-mobile-core/src/session.rs`）；桌面 `src/remote_host.rs` 也一样。控制面 5 s 内没有响应（`TICKET_WAIT`）时，就退回配对码里的旧地址。
3. **自建（`[open]`）模式没有更新渠道。** 没有账号时，配对码里的 `relay_url` 是 Client 唯一的信息来源；Host 一改 relay，就只能重新配对。
4. **控制面可以绕过 Host 移动它。** 有三条路径：控制台调 `switch_node` 时跳过密钥证明和冷却（`session.console`）；管理员的 `move_node` / `move_accounts`；`ticket_node` 发现套餐不再允许当前节点时，自动把 Host 移回默认节点。
5. **Client 没有缓存 ticket。** 只有 Host 把 ticket 存进 `StoredRelayAccount`，控制面一不可达，Client 就拿不到 relay。

## 2. 目标

- **节点只由 Host 决定。** 控制面只存放和转发 Host 签过名的位置记录，伪造不了，也不能拿旧记录回滚。
- **在线设备几秒内跟上**，离线设备下次连接时跟上，控制面短时间不可达也能用缓存连接。
- **只保留一条 relay 链路：控制面 + 节点。** 删除 `[open]` 模式、Host 手填 relay 地址，以及配对地址这条退路。需要自建，就部署开源的控制面。
- 保留不依赖控制面的两条路径：纯直连（局域网 / 打洞），以及 TLS 入口 + 隧道。

## 3. 不做

- 兼容旧数据和旧客户端（见前提）。数据库仍然要加迁移文件，因为线上的控制面已经有库；已有的 Host 升级后第一次取 ticket 时补签一条记录。
- 让没有 Host 所在账号的设备也能使用 relay，比如 Host 替已授权设备申请 ticket（见 §9）。
- 控制面安装脚本的"个人模式"预设。
- 接入 iroh 的 pkarr / DNS 地址发布。它也是"签名记录 + 第三方存放"，但会一并发布直连 IP，还要多一个服务；复用现有的 Ed25519 签名设施更简单。

## 4. 安全模型

| 角色 | 能做什么 | 做不到什么 |
|---|---|---|
| Host | 选择自己的节点，签名位置记录 | — |
| 控制面 | 存放和转发记录；发或不发 ticket（拒绝服务） | 伪造或回滚记录，把 Client 引到 Host 没签过的节点 |
| 节点 | 转发端到端加密的流量 | 读写内容、冒充设备 |
| Client | 只拨验签通过、且 `seq` 最新的记录里的 relay | — |

要说清楚的一点：relay 本来就只转发端到端加密的流量，被引到恶意 relay 也只会泄露元数据（IP、时序、流量大小）。这个设计主要换来的是**正确性和统一**：一套机制、一个"哪条记录更新"的判断（`seq`），并让"Host 是权威节点"成为代码里真正成立的约束。防元数据重定向是附带的收益。

**已知限制：节点地址由控制面定义。** 记录里同时有 `node_id` 和 `relay_url`。管理员修改某个节点的地址后，Host 下次取 ticket 时会发现 ticket 的 `relay_url` 和记录不一致，自动用同一个 `node_id` 重签一条（§6.4）。所以这里的保证是"**换不换节点由 Host 决定**"，而不是"节点地址不受控制面影响"。

## 5. 位置记录

放在 `yttt-relay-ticket`。控制面、`yttt-control-client` 和 `yttt-client-core` 已经依赖它；`yttt-protocol` 新增对它的依赖（这个 crate 只依赖 ciborium、ed25519-dalek、serde 等，很轻），因为 P2P 事件和配对码都要携带这条记录。

```rust
pub struct HostPlacement {
    pub host: [u8; 32],      // Host 的 endpoint key，即 Client 配对时记下的信任锚
    pub seq: u64,            // 单调递增
    pub node_id: String,
    pub relay_url: String,
    pub issued_unix: u64,
}

pub struct SignedPlacement {
    pub placement: HostPlacement,
    pub signature: [u8; 64], // Ed25519，签 placement_bytes(&placement)
}
```

- 签名字节：`b"yttt host placement v1\0" ‖ host ‖ seq(BE) ‖ issued_unix(BE) ‖ u16 长度 ‖ node_id ‖ u16 长度 ‖ relay_url`。这个布局固定，不依赖 serde 编码细节。
- JSON 中（控制面 API）`host` 和 `signature` 用 hex 表示；CBOR 中（P2P 协议、配对码）用 `serde_bytes`。
- `SignedPlacement::verify(expected_host) -> Result<&HostPlacement>`：签名必须有效，`host` 必须等于期望的 Host，`node_id` 不超过 64 字节，`relay_url` 不超过 256 字节（与 `yttt_protocol::p2p::MAX_RELAY_URL_BYTES` 一致；常量移到 `yttt-relay-ticket`，由 protocol 重新导出）。
- **`seq` 的来源。** Host 在 profile root 下持久化 `relay-placement.json`，内容是当前记录；签新记录时取 `max(本地 seq, 控制面返回的 seq) + 1`。Host 的状态丢了，密钥也会一起丢，设备本来就要重新配对，所以 `seq` 重新开始没有问题。
- **"更新"的定义。** 验签通过，且 `seq` 大于已保存的 `seq`。`seq` 相同时内容必须完全一致，否则拒绝。

## 6. 流程

### 6.1 控制面存放

- `devices` 表增加 `placement TEXT`（`SignedPlacement` 的 JSON）和 `placement_seq BIGINT`。`devices.node_id` 仍是 ticket 和计费的依据，**只在接受新记录时**才改。
- `PUT /v1/devices/{host}/node`：请求体从 `{ node_id, proof }` 改为 `{ placement: SignedPlacement }`。控制面依次检查：
  1. 签名有效，且 `placement.host` 等于路径里的 Host；
  2. `seq` 大于已存的 `seq`；
  3. 节点在套餐内、处于启用状态、没有满载，`relay_url` 等于该节点当前地址；
  4. 冷却时间（同一 `node_id` 只是地址变化的重签不受冷却限制）；
  5. `max_nodes` 限制。

  全部通过后，存下记录，`node_id` 与旧值不同时执行 `move_devices(HostAndClients)`。
- **签名本身就证明 Host 持有密钥**，所以去掉 `ProofPurpose::SwitchNode` 和请求里的 `proof` 字段。
- **去掉控制面移动 Host 的所有路径**：
  - 删除 `switch_node` 的 `session.console` 分支，控制台也必须提交签名记录，实际上就是不能再移动 Host；
  - `max_nodes <= 1` 时不再把整个账号一起移走（见 §6.4）；
  - `move_node` / `move_accounts` 只改账号的默认节点，不再移动设备；
  - `ticket_node` 不再自动移动 Host。

### 6.2 取 ticket

`TicketGranted` 追加两个字段：

- `host_placement: Option<SignedPlacement>`：Client 请求时是它所连 Host 的记录，Host 请求时是它自己的记录。
- `placement_needed: Option<PlacementNeeded>`：只出现在 Host 的请求里，见 §6.4。

ticket 的节点一律取 `devices.node_id`。Host 还没有记录时（刚注册或刚升级），控制面**不发 ticket**，返回 `placement_needed { reason: Missing, node_id: 当前节点 }`。

### 6.3 Client 选用哪条记录

Client 在本地配对记录里存 `placement: Option<SignedPlacement>`，替代原来的 `relay_url`。每次拿到一条记录（来自 ticket 响应、会话内事件或配对码），都调用 `PairedHost::accept_placement(signed)`：验签通过且 `seq` 更大才替换，替换后由应用持久化。

拨号时：

1. 用记录里的 `relay_url` 构造 `PeerAddr`；
2. 用 ticket 时，ticket 的 `node_id` 必须等于记录里的 `node_id`，否则不用这张 ticket，记一条连接日志"控制面与 Host 的节点不一致"，只走直连；
3. 没有记录就不用 relay，只走直连。

**缓存。** Client 把 ticket（以及对应的 `node_id`）存进和账号会话同一个加密存储。控制面 5 s 内没有响应时，如果缓存的 ticket 还有效、并且节点与记录一致，就直接用它。

### 6.4 Host 换节点

**用户主动切换**（`SetRelayNode`，或 `yttt-server p2p node <id>`）：

1. 签一条新记录 `seq+1`，调 `PUT /node`，成功后写入 `relay-placement.json`；
2. 通过 `ServerEvent::RelayPlacementChanged` 推给所有已连接的会话，最多等 1 s 让事件发出去；
3. 重启 P2P，用新节点的 ticket 绑定 endpoint。

**被动跟随**（由 Host 的 `keep_relay_ticket` 处理 `placement_needed`）：

| `reason` | 场景 | Host 的处理 |
|---|---|---|
| `Missing` | 还没有记录 | 为控制面给出的 `node_id`（当前节点）签名 |
| `NodeUnavailable` | 节点停用、套餐不再允许、超出 `max_nodes` | 为控制面给出的账号默认节点签名 |
| `AddressChanged` | ticket 的 `relay_url` 和记录不一致 | 同一 `node_id` 重签 |

被动跟随同样走"推送事件 → 重启"，并在 Host 状态里留一条可见的提示，比如"节点 X 已停用，已切到 Y"。不签名，Host 就拿不到 relay，所以这些情况都由 Host 自动处理。它仍然是 Host 签的记录，控制面只是"建议"。

`max_nodes <= 1` 的账号：一台 Host 换节点时，控制面同时把账号默认节点改成新节点；其他 Host 下次取 ticket 时收到 `NodeUnavailable` 并自己跟过去。

### 6.5 会话内推送

- 新增事件 `ServerEvent::RelayPlacementChanged(SignedPlacement)`，只追加，不升协议版本（`docs/wire-evolution.md` 规则 1）。
- Host 在每个 P2P work 会话握手完成后，先发一次当前记录。这样直连进来、控制面又不可达的 Client 也能拿到最新记录。换节点时再发一次。
- Client 收到事件后先 `accept_placement`。记录确实变新、并且 `relay_url` 变了时：
  1. 重新取 ticket；
  2. `endpoint.set_relay(新 target)`；
  3. 用新的 `PeerAddr` 重建 connector，走已有的重连路径。

### 6.6 断线时立即重查

连接失败、会话断开、或者 relay 拒绝认证时，Client 立刻取一次 ticket（同时拿到最新记录），不等 `NODE_CHECK`。`NODE_CHECK` 只作为兜底，保持 5 min。

### 6.7 配对

`PairingCode` 去掉 `relay_url`，新增：

- `control_url: Option<String>`：Host 登录的控制面；
- `placement: Option<SignedPlacement>`：Host 的当前记录。

手机或桌面扫码时：

- 码里有 `control_url`、但设备还没登录这个控制面：先引导登录，再配对；
- 用户跳过登录：只尝试直连。

`PAIRING_CODE_PREFIX` 不变。配对码只有 5 分钟有效，不需要兼容旧码。

## 7. 删除的部分

- `yttt-relay` 的 `[open]` 配置、`OpenConfig`、`open_account_id`，`StartError::OpenWithControl`，以及 `StartError::NoTicketKeys` 文案里关于 `[open]` 的说明；`packaging/relay-selfhost/`。
- `P2pSettings.relay_url`、`RemoteAccessRequest::SetP2p` 的 `relay_url` 字段、`yttt-server p2p enable --relay`、桌面"远程访问"里的 relay 地址输入框。不登录账号时 P2P 只走直连。
- `PairedHost.relay_url`、`RelaySource::PairingHint`、手机端 `hint()`，以及桌面 `remote_host.rs` 的配对地址退路。
- 控制台"设备"页的切换节点操作（节点只读显示）。

## 8. 测试

- **单元测试**：签名字节布局固定（快照测试）；验签失败、换了 Host、`seq` 回退、`seq` 相同而内容不同都被拒绝；`accept_placement` 的各种分支。
- **控制面**：`PUT /node` 的五项检查；控制台会话无法移动 Host；`ticket_node` 不再移动设备；`placement_needed` 的三种 `reason`。
- **集成测试**：控制面测试改用 SQLite 临时库，不再需要 `YTTT_TEST_DATABASE_URL`，也就不必 `#[ignore]`。原来依赖 `[open]` 节点的测试改成接控制面。关键用例：
  1. Host 换节点，已连接的手机在 5 s 内收到事件，并经新节点重新连上；
  2. 控制面伪造的记录、回滚的记录都被 Client 拒绝；
  3. 控制面不可达时，Client 用缓存的 ticket 和记录连上；
  4. 节点停用，Host 自动切到默认节点，Client 跟上。
- **真机**：iPhone（`scripts/mobile/run-ios-device.sh`）和自己的线上控制面，在 Host 上切换节点，手机不需要任何操作就能恢复。

## 9. 以后可以做

- Host 替已授权设备申请 ticket：设备不需要登录 Host 的账号，就能把电脑分享给别人。
- 控制面安装脚本的"个人模式"：关闭注册、只有一个管理员、不限额度的套餐。
