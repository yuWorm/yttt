# P2P 后端实施计划

- 日期：2026-11-06
- 设计：[p2p-relay-qr-pairing-design](2026-11-05-p2p-relay-qr-pairing-design.md)
- 工作树：`.worktrees/p2p-host`，分支 `feat/p2p-host`（基于 `docs/p2p-relay-design@804c799`）。
- 范围：只做后端，不做 GUI。顺序为 Part A（Host 侧 P2P，设计 P1）→ Part B（`yttt-relay` + 控制面，设计 P3）。
- 本地测试：relay 与 PostgreSQL 都用 docker 跑在本机。
- 依赖版本：`iroh = "=1.4.1"`、`iroh-relay = "=1.4.1"`（实施时 crates.io 上的最新 1.x）。

## 0. 实施前核实的 iroh 1.4.1 事实

这些结论来自 `~/.cargo/registry/src/*/iroh-1.4.1` 与 `iroh-relay-1.4.1` 源码，取代设计文档中对应的“待复核”：

| 事实 | 位置 | 对计划的影响 |
|---|---|---|
| `EndpointHooks::after_handshake` 在 TLS 完成后拿到对端 EndpointId 与 ALPN，可拒绝 | `endpoint/hooks.rs` | Host 的 ACL 首道检查放在这里 |
| hook 中不得持有 `Connection`，要用 `weak_handle()` | 同上 | 连接登记表存 `WeakConnectionHandle` |
| `Connection::close`、`paths()`、`remote_id()`、`accept_bi()` 均为公开 API | `endpoint/connection.rs` | 撤销、路径显示、逐流检查可实现 |
| `Endpoint::insert_relay` / `remove_relay` 运行时可用 | `endpoint.rs` | ticket 续签与节点迁移不重建 endpoint |
| 核心库的地址查找只有 `dns`、`pkarr`、`memory`；**没有内置 mDNS** | `address_lookup/` | 直连提示用 `MemoryLookup`；mDNS 推迟 |
| relay 嵌入接口公开：`protos::handshake::serverside`、`RelayedStream::new`、`Clients::register`、`Clients::disconnect`、`streams::Bucket` | `iroh-relay/src/{protos,server}` | **`yttt-relay` 不需要 fork**：自建 HTTP/WS 服务，在流外包按账号的限速与计量，再注册进 `Clients` |
| 同一 EndpointId 新连接注册时，旧连接被置为 inactive 并收到 `SameEndpointIdConnected` | `server/clients.rs` | 续签重连不会两条同时转发 |

## 1. 线协议兼容策略

遵守 `docs/wire-evolution.md`：握手只追加字段，不升版本。

- `ClientHello` 追加 `endpoint_id: Option<EndpointKey>`；`HostChallenge` 追加 `endpoint_id: Option<EndpointKey>` 与 `signature: Option<Signature>`；`ClientAuthenticate` 追加 `signature: Option<Signature>`。全部 `#[serde(default)]`。
- `DeviceKey` 模式下 `proof` 字段填全零，验证只看 `signature`；transcript 标签为 `yttt-host-proof-device-v1` / `yttt-client-proof-device-v1`，并追加双方 EndpointId。`SharedSecret` 模式的 transcript 与 `-v3` 标签完全不变。
- 配对走独立 ALPN `yttt/pair/1`，消息类型 `PairMessage` 不进入现有握手枚举。
- `RemoteAccessRequest` / `RemoteAccessResponse` 只追加变体（仅本地管理入口可用，现有 `authorize_ingress` 已保证）。
- 每次追加后用 `YTTT_BLESS_WIRE_SHAPES=1 cargo test -p yttt-protocol --test wire_shapes` 重写样本，并在 `docs/wire-evolution.md` 记录。

## Part A：Host 侧 P2P 后端

每个任务结束时：`cargo fmt`、`cargo clippy -p <crate> --all-targets -- -D warnings`、相关测试通过后提交。

### A1 协议类型（`yttt-protocol`）

- `handshake.rs`：`EndpointKey([u8; 32])`、`Signature([u8; 64])`（Debug 脱敏）；按 §1 追加字段。
- 新模块 `p2p.rs`：
  - `DevicePreset { Full, Observe }` 与 `fn allows(Capability) -> bool`（设计 §6.2 的表）。
  - `PairedDevice { endpoint_id, label, preset, paired_at_millis, last_seen_millis }`。
  - `PairingCode { host, relay_url, direct_addrs, invitation_id, secret, environment_id, profile_id }`，编码为 `yttt-pair:1:<base32(cbor)>`，有界解析（≤1 KiB）。
  - `PairMessage { Hello { host_nonce_commit }, Request { invitation_id, device_label, client_nonce, mac }, Reveal { host_nonce }, Accepted { environment_id, profile_id, host_label }, Rejected { reason } }`。
  - SAS 派生函数：`sas(transcript) -> u32`（6 位）。
- `remote_access.rs` 追加：`P2pSettings { enabled, relay_url }`、`P2pStatus`；请求 `SetP2p`、`StartPairing { label_hint }`、`PairingStatus`、`ConfirmPairing { invitation_id, preset }`、`CancelPairing`、`ListDevices`、`RevokeDevice { endpoint_id, reset_tls }`；响应 `PairingInvitation { code, expires_millis }`、`Pairing(PairingState)`、`Devices(Vec<PairedDevice>)`。
- 测试：编码往返、超长与非法输入拒绝、`DevicePreset::allows` 全表、wire shape 样本。

### A2 设备认证（`yttt-transport`）

- `auth.rs`：`Credential { SharedSecret(AuthToken), DeviceKey(DeviceKeyCredential) }`，`DeviceKeyCredential { secret: [u8; 32], local: EndpointKey, peer: Option<EndpointKey> }`（Host 侧 peer 由入口填入，Client 侧为固定的 Host key）。`impl From<AuthToken> for Credential`。
- 新增 `client_handshake_with(&Credential)` / `server_handshake_with(&Credential)`；旧函数改为薄包装，现有调用方不变。
- 入口与凭据绑定：`IngressKind::P2pWork` 只接受 `DeviceKey`，其余只接受 `SharedSecret`（I3、设计测试 6）。
- 签名用 `ed25519-dalek`（已在 `Cargo.lock`），不让 `yttt-transport` 依赖 iroh。
- `AuthenticatedSession` 追加 `peer: Option<EndpointKey>`。
- 测试（设计 §16 第 4、5、6 条）：签名正确时双方成功；任一方 EndpointId、nonce、epoch、通道被替换时失败；Host 签名无效时 Client 不发送 `ClientAuthenticate`；凭据类型与入口不匹配被拒绝；现有 MAC 测试全部保持通过。

### A3 iroh 传输（新 crate `yttt-transport-iroh`）

- `Endpoint` 封装：`bind(secret, relay: Option<RelayUrl + token>, alpns, gate)`；无 relay 时使用 `presets::Minimal` 并清空 relay transport；有 relay 时 `RelayMode::custom([url])`。
- `PeerGate` trait：`fn admit(&self, peer: EndpointKey, alpn: &[u8]) -> bool`，由 `after_handshake` hook 调用。
- `IrohAcceptor`：`accept()` 产出 `IncomingStream { peer, alpn, stream: TransportStream }`；每条 `accept_bi` 前再次调用 `PeerGate`；按 peer 维护 `WeakConnectionHandle` 列表，`close_peer(peer)` 关闭全部连接。
- `IrohConnector: TransportConnector`：固定目标 key、ALPN、直连地址提示（`MemoryLookup`）；复用连接，`open_bi`；`link_profile() = REMOTE`；`path_kind()` 返回 `Direct | Relay | Unknown`。
- `relay` 模块：`set_relay_token(url, token)` 调用 `insert_relay`。
- 测试：两个本地 endpoint（无 relay，直连提示）完成 echo；gate 拒绝时连接失败；`close_peer` 后已有连接关闭、新流被拒；同一连接上多条流。

### A4 Host 准入重构（`yttt-host`）

- `NetworkAdmission` 改为 Host 级单例，键为 `Principal { Tls(ClientInstanceId), Device(EndpointKey) }`，每条会话记录所属 `IngressKind`。
- `close_ingress(kind)` 只取消该入口的会话；`cancel(principal)`；名额 `MAX_REMOTE_CLIENTS`、`MAX_REMOTE_STREAMS` 两入口共享。
- 未认证握手信号量按入口分开（TLS 一个、P2P 一个，各 `MAX_PENDING_HANDSHAKES`）。
- `admit` 与 ACL 检查在同一把锁内完成：`admit_device(peer, acl)`（I8）。
- `ConnectionContext` 追加 `credential: Arc<Credential>` 与 `device_preset: Option<DevicePreset>`；`serve_connection` 改用 `server_handshake_with`。
- 测试：关闭 TLS 不影响 P2P 会话（设计 §16 第 9 条）；名额共享；撤销与准入并发压力测试（第 2 条）。

### A5 设备 ACL 与 Host 密钥（`yttt-host`）

- `remote-access/p2p-key`（32 字节，用户专用权限文件，与 `credentials.json` 相同的读写校验）。
- `remote-access/devices.json`：`schema_version` + `Vec<PairedDevice>`，原子写入。
- `DeviceRegistry`：`contains`、`preset`、`insert`、`revoke`、`touch(last_seen)`；所有修改只在 Manager 内经本地命令进行（I2）。
- 测试：持久化往返、非法权限拒绝、撤销后 `contains` 为假。

### A6 P2P 入口与配对服务（`yttt-host`）

- `Manager` 增加 `p2p: Option<P2pGeneration>`：endpoint、accept 任务、配对状态。`SetP2p` 启停，状态机与 TLS 一致，设置持久化到 `DeviceSettingsStore`（追加字段带缺省）。
- accept 循环：`yttt/work/1` 流 → 握手信号量 → `serve_connection`（`IngressKind::P2pWork`、`Credential::DeviceKey`、peer）；`yttt/pair/1` 流 → 配对状态机。
- 配对状态机（设计 §7.2–7.3）：`Idle → Invited → AwaitingConfirm{sas, peer, label} → Done`；第二个有效请求作废并记告警；`mac` 失败 3 次作废；5 分钟过期；`ConfirmPairing` 写 ACL 后才发 `Accepted`。
- `RevokeDevice`：锁内删 ACL → `close_peer` → `admission.cancel` → 可选重置 TLS 凭据。
- 按设备 capability：`authorize_ingress` 之后检查 `device_preset.allows(request.required_capability())`；`Observe` 不能 `TakeTerminalController`。
- 测试（设计 §16 第 1、2、8、10 条）：未配对 peer 被拒；完整配对流程；commit 与 reveal 不符、SAS 拒绝、取消、过期、重放、并发第二请求；Observe 权限。

### A7 Client 侧（`yttt-client-core`）

- 连接记录类型 `PairedHostRecord { host, environment_id, profile_id, relay_url, direct_addrs, device_secret_ref }`（存储位置由调用方决定，本任务只定义类型与序列化）。
- `pair(code, device_secret, label, on_sas) -> PairedHostRecord`：执行 Client 侧配对，收到 `Accepted` 才返回（I1）。
- 现有连接路径接受 `Credential`；新增 `p2p_connector(record, endpoint)`。
- 测试：端到端（同进程 Host + Client，无 relay）完成配对、会话握手、发一个 `Request` 并收到响应；Host 身份不一致时不发送应用数据（第 4 条）。

### A8 docker relay 与真实路径测试

- `packaging/relay-dev/docker-compose.yml`：`n0computer/iroh-relay:v1.4.1 --dev`（HTTP，仅本机）。
- 被 `#[ignore]` 的集成测试：读取 `YTTT_TEST_RELAY_URL`，禁用直连提示，强制经 relay 完成 A7 的端到端流程。
- `docs/` 中补充本地运行说明。

## Part B：商业 relay 与控制面

### B1 共享 ticket 类型（新 crate `yttt-relay-ticket`）

- `Ticket { account_id, endpoint_id, node_id, rate_bps, burst, exp_unix }`，CBOR + Ed25519 签名，base64url 编码为 bearer token。
- `verify(token, keys: &[VerifyingKey], node_id, now) -> Result<Ticket>`；支持两把公钥并存。
- 设备登记与 ticket 申领的签名载荷：`device_proof(account_id, endpoint_id, nonce)`。
- 测试：篡改、过期、错误节点、错误公钥、轮换。

### B2 `yttt-relay`（新 crate，二进制）

- 自建 HTTP（hyper）与 WebSocket 升级，复用 iroh-relay 的握手与帧：`handshake::serverside` → 校验 ticket（`endpoint_id` 一致、节点、未过期、账号未封停）→ `authorize_if` → 包装流 → `Clients::register`。具体包装层次在任务开始时按 `http_server.rs` 的升级路径确认；若某个必需类型未公开，退回最小 fork，并在本文件记录原因。
- `AccountMeter`：每账号一个 `Bucket`（同账号所有连接共享）与上行字节计数；降速时替换为低速桶。
- 到期断开：按 `(endpoint_id, connection_id)` 记录 `exp`，定时器到期调用 `Clients::disconnect`。
- `disconnect_account`。
- 用量上报：每 15 秒 `POST /v1/nodes/{node}/usage`，响应中取额度状态；失联时按本地缓存执行。
- QUIC 地址探测：用 `iroh_relay::server::Server` 仅启用 `quic`（需要 TLS；本地开发可关闭）。
- 配置：TOML（监听地址、节点 ID、控制面 URL、节点令牌、控制面公钥列表、TLS）。
- 测试：ticket 拒绝矩阵；两条连接共享账号桶；计数与实际转发字节一致；到期断开；封停断开（设计 §16 第 11、12 条）。

### B3 控制面（新 crate `yttt-control`，二进制）

- axum + sqlx（PostgreSQL），`migrations/` 目录，启动时迁移。
- 表：`accounts`、`plans`、`devices(endpoint_id UNIQUE)`、`nodes`、`usage_months`、`sessions`。
- API：
  - `POST /v1/dev/accounts`（仅开发模式，创建账号与密码）。
  - `POST /v1/sessions`（登录，argon2 校验，返回不透明会话 token）。
  - `POST /v1/devices`（会话 + 设备签名，检查设备数与唯一性）、`DELETE /v1/devices/{endpoint_id}`。
  - `POST /v1/tickets`（会话 + 设备签名 → ticket 与节点 URL）。
  - `POST /v1/nodes/{node}/usage`（节点令牌；累加用量，返回各账号额度状态）。
  - `GET /v1/accounts/me`（套餐、节点、本月用量）。
- 明确不提供：Host 身份下发、任何 ACL 写接口（I1、I2）。
- 测试：sqlx 测试库（docker PostgreSQL）；登记私钥证明、重复登记、设备数上限、ticket 签发与 relay 验证互通、用量累加与额度状态（设计 §16 第 13 条）。

### B4 docker-compose 与端到端

- `packaging/relay-dev/docker-compose.full.yml`：postgres、yttt-control、yttt-relay（本仓库 Dockerfile 构建）。
- 被 `#[ignore]` 的端到端测试：创建账号 → Host 与 Client 登记设备 → 申领 ticket → 经 `yttt-relay` 完成配对与会话 → 检查用量增长 → 设为封停后连接断开。

### B5 Host 与 Client 接入控制面（后端 API，不含 GUI）

- `yttt-client-core` 新增 `ControlClient`：登录、登记设备、申领与定时续签 ticket，续签后 `set_relay_token`。
- Host 在 `SetP2p` 中接受控制面会话（本地管理入口提供），用于自身的 ticket。

## 风险与回退

- iroh-relay 嵌入接口若缺少必需的公开类型（B2），退回最小 fork（vendored 到 `vendor/iroh-relay`），并记录差异。
- 若 `after_handshake` 拒绝导致对端看到的错误不可区分，Client 侧以握手超时和关闭码映射错误类型。
- Part A 完成后先请用户在 macOS 与 Windows 之间经 docker relay 实测，再进入 Part B。
