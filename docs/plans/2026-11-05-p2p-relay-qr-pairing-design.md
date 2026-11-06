# P2P + Relay 扫码连接与商业 Relay 设计

- 日期：2026-11-05
- 状态：设计提案（第 2 版，已按 reviewer 意见修订）；尚未实施。
- 文档工作树：`.worktrees/p2p-relay-design`，分支 `docs/p2p-relay-design`。
- 代码基线：`master@cc6fa2a`。
- 上位文档：[p2p-relay-architecture](../p2p-relay-architecture.md)（§7 传输、§9 身份配对）、[desktop-host-remote-access-design](2026-09-06-desktop-host-remote-access-design.md)（现有 TLS 端口入口）。
- 外部事实来源：iroh 1.0.x 与 `main` 分支源码、官方文档（见 §18）。每条 iroh 相关结论都标注了“已核实”或“待复核”；“待复核”的条目在 Spike 阶段于锁定版本上确认。

## 1. 目标与范围

用户在电脑 A 运行 yttt Host，在任意网络下的电脑 B 或手机上扫码一次后，即可连接 A 的项目、终端和 Agent，不再需要端口转发工具（替代当前依赖的 UU 端口映射）。连接优先 P2P 直连，失败时经 relay 回退。

relay 作为付费服务出售：按账号限制 relay 带宽和月流量，支持多节点部署；同一账号的设备只在同一个节点内连接。

**安全目标**：加密与认证端到端完成。relay 和控制面只负责传输与计费；**即使 relay 或控制面被完全攻破，攻击者也不能访问任何 Host**。

非目标：

- 浏览器客户端（浏览器只能走 relay，另行设计）。
- 多用户协作、不受信任用户的沙箱。
- 跨节点转发、全局地址发现（pkarr/DNS）。
- 一台 Client 经付费 relay 访问属于其他账号的 Host（D11）。
- 支付渠道、发票等计费周边。

## 2. 决策摘要

| # | 决策 | 理由 |
|---|---|---|
| D1 | 传输采用 iroh 1.0.x（QUIC + 打洞 + relay 回退），封装在新 crate `yttt-transport-iroh` | 上位文档已选定；不自研 NAT 穿透 |
| D2 | 设备身份直接使用 iroh EndpointId（Ed25519），Host 维护设备 ACL；不再使用 profile 级共享秘密 | 可单设备撤销；日常连接无可爆破的秘密。与上位文档 §9.1“DeviceId 与传输身份分离”不同，这里有意合并：实现更简单，代价是换设备密钥必须重新配对 |
| D3 | Host 身份只来自扫码配对，Client 固定保存，永不接受控制面修改 | 控制面被攻破也不能把 Client 指向假 Host |
| D4 | Host ACL 只能在本机修改；控制面登记设备仅为单向通知 | 控制面被攻破也不能往 Host 加设备 |
| D5 | 应用层握手中 Host 和 Client **双方**都用自己的 EndpointId 私钥签名 transcript | 纵深防御，防 iroh 实现缺陷或配置错误（不防私钥泄露） |
| D6 | 商业 relay 为自研二进制 `yttt-relay`，通过 `iroh-relay` 1.4 的公开嵌入接口实现，不 fork（见 §9.2） | 现成 relay 没有按账号限速、计量与按账号断开 |
| D7 | 每个账号固定分配一个节点；`RelayMap` 只含该节点 | 满足“只在同节点连接”，无需地址发现 |
| D8 | relay 准入用控制面签发、节点离线验签的长期 ticket（默认 30 天，只在准入时检查）；对已建立连接的实时控制走节点↔控制面的用量回报（当前速率、已移除设备、封停） | 鉴权不依赖每连接回调；iroh 1.4.1 的 relay 连接在整个生命周期内重连都沿用首个 token，短期 ticket 会把长时间运行的 endpoint 锁在门外（实施时核实，见 §9.3） |
| D9 | 计费只针对 relay 流量，以节点计数为唯一口径；P2P 直连不经过服务器，不计量 | 技术事实，产品定价须据此设计 |
| D10 | 现有 TLS 端口入口保留，作为独立入口和备用路径 | 不破坏已有用户；服务不可用时仍可用 |
| D11 | 一台 Client 安装实例只属于一个账号；用户在 Client 和 Host 上登录同一账号，不支持跨账号配对使用付费 relay | 用户控制的是自己的电脑，账号就是“我的设备集合”；同时消除了“一个 EndpointId 属于多个账号”带来的计费和准入歧义 |

## 3. 现状与需要改动的接缝

| 层 | 现状 | 本设计的改动 |
|---|---|---|
| Transport 抽象 | `TransportListener::accept()` 只返回 `TransportStream`，不携带对端信息 | 新增对端元数据（已认证的 EndpointId、入口类型），供 Host 准入和认证使用 |
| TLS accept 路径 | `network_loop` 直接调用 `TlsListener::accept_tcp()`，未经 `TransportListener` | 抽出与传输无关的准入逻辑，TLS 与 P2P 两条 accept 路径共用 |
| 会话 | C3 之后一个会话一条连接，`ConnectionChannel::{Session, Lifecycle, DesktopOwner}` | 一条连接映射为一个 QUIC bi-stream；会话层协议本身不变 |
| 准入 | `NetworkAdmission` 以 Client 自报、未认证的 `ClientInstanceId` 为键；每次启用 TLS 入口新建一份，关闭时整体 `close()` | 改为 Host 级对象，以“已认证主体”为键（P2P 为 EndpointId），按入口关闭（§8.1） |
| 认证 | `server_handshake` 参数写死为 `&AuthToken`；`HostChallenge.proof` 与 `ClientAuthenticate.proof` 类型为 `AuthMac([u8; 32])`，transcript 标签 `-v3` | 握手升级到 v4：凭据枚举、proof 枚举、双方 EndpointId 进入 transcript（§8.2） |
| 授权 | `authorize_ingress` 只有入口级黑名单；`Capability` 枚举（`protocol/control.rs`）存在，但没有按设备的校验 | 新建按设备的 capability 校验（§6.2） |
| 连接信息 | `RemoteConnectionInfo`：证书 DER + 32B 长期秘密，≤8KiB，复制粘贴 | P2P 模式改为短期邀请二维码，不含长期秘密 |
| 握手限流 | `HandshakePenalties` 按 IP，loopback 豁免（`cc6fa2a`） | P2P 入口按 EndpointId 计 |

## 4. 威胁模型与安全不变量

### 4.1 信任边界

| 组件 | 定位 | 可以知道 | 不能做 |
|---|---|---|---|
| Host 本机 | 唯一信任根：Host 私钥、设备 ACL | 全部 | — |
| 已配对 Client | 持有自己的设备私钥 | 自己的会话 | 越过 capability 与控制 lease |
| 控制面 | 账号、计费、节点分配、ticket 签发 | 账号、设备 EndpointId、节点、用量 | 下发或修改 Host 身份；修改 ACL；下发配置、命令或更新 |
| relay 节点 | 转发密文 | IP、EndpointId、时间、包大小、流量 | 解密、篡改、冒充 |

### 4.2 不变量（必须由代码结构和测试保证）

- **I1 Host 身份固定**：Client 的 Host EndpointId 只在配对成功（收到 `PairAccepted`）时写入。之后任何来源（控制面、relay、对端消息）都不能修改。Host 密钥变化必须重新扫码，不提供“信任新身份”的按钮。
- **I2 ACL 仅本机可写**：增删设备只能经本地管理入口加本机用户确认。网络入口（TLS、SSH、P2P）上不存在能修改 ACL 的消息类型。Host 不从控制面读取设备列表。
- **I3 双层双向认证**：iroh 传输层验证双方 EndpointId；应用层 Host 与 Client 再分别用各自私钥签名 transcript。任一层、任一方失败即拒绝。
- **I4 未知设备零暴露**：P2P 入口在 iroh `after_handshake` 阶段（已知对端 EndpointId 与 ALPN）拒绝 ACL 之外的设备，应用握手不会开始；配对 ALPN 例外，见 §7。**每条新 stream** 在进入应用握手前重新查 ACL。
- **I5 ticket 与授权分离**：Host 不读取、不信任 relay ticket。ticket 只决定 relay 是否转发。
- **I6 更新渠道独立**：发布包用离线保管的密钥签名，Client 内置公钥验签；控制面不承载更新、远程配置或插件下发。
- **I7 relay URL 不可信**：relay URL 只是路由提示。被替换只会导致连不上，不影响身份验证。
- **I8 撤销即时且完整**：撤销设备时，在同一把锁内删除 ACL 条目并关闭该 EndpointId 的全部 QUIC 连接；此后的任何新连接或新 stream 都会在 I4 的检查点被拒绝。“检查 ACL”与“登记准入”在同一临界区完成，不存在撤销与准入的竞态。

### 4.3 被攻破时的影响

| 被攻破 | 攻击者能做 | 不能做 |
|---|---|---|
| relay 节点 | DoS；观察元数据 | 解密、注入、冒充 |
| 控制面（含 ticket 签名私钥） | DoS、拒绝续签；替换 relay URL；读取账号与设备列表；免费使用 relay | 修改 Client 固定的 Host 身份（I1）；往 ACL 加设备（I2） |
| 用户的控制面账号 | 消耗 relay 额度、查看设备列表 | 访问 Host |
| 已配对 Client 设备 | **访问 Host**（唯一真实入口） | 越过 capability |
| TLS 共享秘密泄露 | 经 TLS 入口访问 Host，不受设备 ACL 约束 | — |

已配对设备丢失的应对：设备私钥存 OS keychain；移动端启动需生物识别；Host 本机一键撤销并立即断开（I8）；新设备配对和远程接管时 Host 本机通知，并写审计日志（只记操作，不记终端内容）；本机“关闭远程访问”总开关**同时关闭 TLS 与 P2P 两个入口**。

TLS 入口的共享秘密独立于设备 ACL。撤销设备时，如果 TLS 入口处于开启状态，界面提示“该设备若曾导入 TLS 连接信息，仍可经 TLS 入口连接”，并提供“同时重置 TLS 凭据”的选项。

### 4.4 防爆破与 DoS

- 日常连接没有共享秘密，靠 Ed25519 私钥证明身份。
- 配对是唯一攻击面，约束见 §7.3。
- 握手 DoS：iroh 入站连接最早只能在 TLS 握手完成后的 `after_handshake` 钩子拒绝（已核实），因此知道 Host EndpointId 的一方（包括 relay 运营方）可以迫使 Host 完成 TLS 握手。P2P 入口有独立的未认证握手上限，并按 EndpointId 退避；TLS 入口被打满不影响 P2P，反之亦然（§8.1）。

### 4.5 残余风险

- **元数据与按键时序**：relay 可见包的大小与时间，交互式终端存在按键时序分析的可能（参考 OpenSSH 9.5 的按键混淆）。第一期接受；后续可对输入流做填充或抖动。
- **可用性**：控制面或 relay 全部故障时，跨网远程中断。备用路径为：局域网内依靠保存的直连地址或本地发现（§5.4），以及 TLS 端口入口。
- **shell 等价**：`terminal.control` 等价于 Host 的 OS 账号权限，与本设计无关，靠 capability 和撤销管理。

## 5. 传输层：`yttt-transport-iroh`

### 5.1 依赖与 ALPN

- 锁定 iroh 1.0.x 精确版本（1.0.1 已发布，已核实），iroh 类型不泄漏出本 crate。`Cargo.lock` 中已有 quinn。
- ALPN：`yttt/work/1`（工作会话）、`yttt/pair/1`（配对，仅邀请有效期间接受）。
- D11 之下，一个 Client 进程只需一个 iroh Endpoint。

### 5.2 Host 侧

- 在 `after_handshake` 钩子中：ALPN 为 `yttt/work/1` 时检查对端 EndpointId 是否在 ACL；ALPN 为 `yttt/pair/1` 时检查是否存在有效邀请。不满足则拒绝连接。
- 对每条已接受的连接循环 `accept_bi()`。每条新 stream 在交给应用握手之前重新检查 ACL（I4），然后合并 `SendStream` 与 `RecvStream` 为 `TransportStream`，并附带对端 EndpointId 与 `IngressKind::P2pWork` 作为可信元数据。
- Host 按 EndpointId 记录活动的 QUIC 连接，用于撤销时 `Connection::close`（I8）。

### 5.3 Client 侧

- `IrohConnector` 持有 `Endpoint`、固定的 Host EndpointId、relay URL 提示、保存的 Host 直连地址提示。
- `connect()` 复用已建立的 QUIC 连接并 `open_bi()`；连接失效时重新拨号。`link_profile()` 返回 `LinkProfile::REMOTE`。

### 5.4 地址与发现

- `RelayMap` 只含账号分配的节点（D7）；自用或开发模式允许配置自建节点 URL；**不使用 n0 公共 relay 作为生产默认**。
- 配对成功时，Client 保存 Host 当时的直连地址作为提示；之后每次成功连接都会更新。relay 不可用时，Client 凭这些提示尝试直连，局域网内通常可以成功。
- 可选启用 iroh 的本地网络发现（mDNS 类机制）。**待复核**：iroh 1.0 中该功能的名称与启用方式。

### 5.5 路径显示

iroh 0.96 起支持 multipath，一条连接可以同时具有 relay 路径和直连路径。界面和诊断日志显示 `Connection::paths()` 中当前选中的路径（已核实 0.96/0.98 中的 API；**待复核** 1.0 中的名称）。

### 5.6 端点密钥

Host 和每个 Client 安装实例各自生成持久 Ed25519 密钥，按 profile 隔离（遵守上位文档 §16 的 profile 隔离要求），优先存 OS keychain；keychain 不可用时存当前用户专用权限的文件，不静默降级到共享配置。

## 6. 身份、设备 ACL 与 capability

### 6.1 ACL

存放在 Host private state 的 `remote-access/devices`，只经本地 IPC 修改：

```toml
schema_version = 1

[[device]]
endpoint_id = "<z32>"
label = "Windows 台式机"
preset = "full"            # full | observe
paired_at = "2026-11-05T12:00:00Z"
last_seen = "2026-11-05T12:30:00Z"
```

- 远程入口仍然拒绝 DesktopOwner、Host stop/restart、远程访问设置、凭据管理等管理操作（现有 `authorize_ingress` 保持不变）。
- 撤销：按 I8 执行，然后通知控制面（尽力而为，失败不影响本地撤销）。

### 6.2 capability（新建工作，不是沿用）

现有代码只有入口级黑名单，没有按设备的 capability 校验。本设计在每个 Request 的分发点增加按设备的校验，以现有 `Capability` 枚举（`crates/yttt-protocol/src/control.rs`）为准，第一期提供两档预设：

| 预设 | 允许 |
|---|---|
| `full` | 全部 capability，与 TLS 工作会话相同；可请求接管控制权 |
| `observe` | `ProjectRead`、`GitRead`、`WorkspaceRead`；只能观察终端，不能请求接管 |

实现时 `full` 改为包含 `SshConnect`、`RemoteCommandPrivileged`、`CredentialAnswer`：`TerminalInteractive` 已等价于 Host 账号的 shell，再禁止这三项不增加安全性，却会让 P2P 设备无法打开 SSH 项目。上位文档 §9.3 的点分名称（`terminal.observe` 等）只作为概念描述，代码以枚举为准。

TLS 入口沿用现有行为（等同 `full`），不受设备 ACL 约束（见 §4.3）。

## 7. 扫码配对

### 7.1 二维码内容

不使用 URL 形态，避免系统相机把含秘密的内容交给浏览器或记入历史：

```text
YTTT-PAIR:1:<BASE32(cbor{host, relay, direct_hint, inv, s, env, profile})>
```

全部使用大写字母、数字和 `:`、`-`，二维码可以使用更紧凑的字母数字模式；解析时不区分大小写。字段：Host EndpointId、relay URL、Host 当前直连地址提示、16B 邀请 ID、32B 邀请秘密、EnvironmentId、ProfileId。二维码只在应用内扫描；不注册为系统 URL handler（遵守现有设计中“秘密不进 argv 或 deep-link”的约束）。桌面 Client 没有摄像头时，在应用内粘贴同一字符串。为此允许 Host 用户主动复制文本（默认只显示二维码，文本需点开）；邀请结束时若剪贴板仍是该码则清空。码单次有效、5 分钟过期、需 Host 用户核对 6 位码，第二个持码者会使邀请作废，泄露的影响有上限（2026-11-06 修订，见 [p2p-app-integration](2026-11-06-p2p-app-integration.md) §2.4）。

### 7.2 消息流程

Client 用 `yttt/pair/1` 拨号 Host EndpointId 后，QUIC 握手已确认双方 EndpointId。配对流由 **Host 打开**：Host 先发言，而 QUIC 对端只有在打开方写入数据后才能看到新流。随后：

1. **Host → `PairHello { host_nonce_commit }`**：`commit = H(host_nonce)`。先承诺、后公开，防止 Host 一方（或冒充者）挑选 nonce 来凑 SAS。
2. **Client → `PairRequest { inv, device_label, client_nonce, mac }`**：`mac = HMAC(s, transcript)`，transcript 包含协议域与版本、`inv`、双方 EndpointId、EnvironmentId、ProfileId、`host_nonce_commit`、`client_nonce`。
3. **Host → `PairReveal { host_nonce }`**：Host 校验 `inv`、`mac`、有效期后公开 nonce。Client 校验 `H(host_nonce) == commit`。
4. 双方从完整 transcript 派生 6 位 SAS 码并显示。Client 端显示“请在 Host 上确认”。
5. Host 本机用户核对 SAS 一致，选择预设（`full` / `observe`），点确认。
6. **Host → `PairAccepted { environment_id, profile_id, host_label }`**：Host 先写入 ACL、作废邀请、关闭 `yttt/pair/1`，然后发送该消息。
7. Client 收到 `PairAccepted` 后才固定保存 Host EndpointId、EnvironmentId、ProfileId、relay URL 提示和直连地址提示（I1）。在此之前断开则不保存任何内容。
8. （商业模式）Client 与 Host 已登录同一账号（D11）。Client 向控制面登记自己的 EndpointId，登记需设备私钥签名（§10），随后申领 ticket。登记与 Host ACL 无关，失败只影响付费 relay 的使用。

### 7.3 配对约束

- 默认不接受 `yttt/pair/1`，只在邀请有效期间开放；同一时刻最多一个有效邀请。邀请单次有效，5 分钟过期，只存内存。
- **同一个邀请收到第二个 `mac` 有效的 `PairRequest` 时，立即作废邀请、中止所有进行中的配对，并在 Host 本机告警“配对二维码可能已泄露”**。这可以防止拍到二维码的攻击者抢先配对，让 Host 显示攻击者的 SAS。
- `mac` 校验失败 3 次即作废邀请。
- SAS 被拒绝、超时、Host 用户取消，都不能写入 ACL。
- 二维码被他人拍到：没有第 5 步的本机确认就无法完成配对。
- 后续支持短数字码时必须使用 PAKE（例如 SPAKE2），不能直接比较数字。

## 8. Host 入口与认证改动

### 8.1 入口与准入

- `IngressKind` 新增 `P2pWork`。
- 准入对象提升为 Host 级，TLS 和 P2P 两个入口共用，以“已认证主体”为键：P2P 为 EndpointId，TLS 为该入口的会话（TLS 没有设备身份，沿用现有行为）。远程 Client 名额 `MAX_REMOTE_CLIENTS` 与 `MAX_REMOTE_STREAMS` 由两个入口共享。
- **未认证握手的上限按入口分开**：TLS 入口经端口转发暴露时被打满，不影响已配对的 P2P 设备，反之亦然。
- **关闭某个入口只断开该入口的会话**，不再整体 `close()` 准入对象。
- P2P 入口的启用和关闭独立于 TLS 入口。设置项新增 `p2p_enabled`、`relay_url`（自建模式）；状态机沿用 `Disabled → Starting → Listening → Stopping`。
- 握手退避按 EndpointId 计；既有按 IP 的逻辑只对 TLS 入口生效。

### 8.2 应用层认证 v4（I3）

- `server_handshake` 与 `client_handshake` 的凭据参数改为枚举：`SharedSecret(AuthToken)`（TLS 入口）和 `DeviceKey`（P2P 入口，持有本端私钥与对端已固定的 EndpointId）。
- `HostChallenge.proof` 与 `ClientAuthenticate.proof` 改为枚举：`Mac([u8; 32])` 或 `Signature([u8; 64])`。
- `ClientHello` 与 `HostChallenge` 显式携带各自的 EndpointId；transcript 标签从 `-v3` 升为 `-v4`，在现有字段（协议域与版本、构建兼容性、环境与 profile、双方 nonce、Host epoch、通道、权限请求）基础上加入双方 EndpointId。
- `DeviceKey` 模式：Host 用 Host 私钥签名 transcript，Client 验签并比对固定的 Host EndpointId、EnvironmentId、ProfileId，任一不一致则不发送任何应用数据；Client 用设备私钥签名 transcript，Host 用 ACL 中的公钥验签，并检查签名公钥与传输层报告的对端 EndpointId 一致。
- `DeviceKey` 模式下 `credential_generation` 固定为 0 且不参与判断；设备的有效性只由 ACL 决定。
- 签名使用的就是传输层那把密钥。这一层防的是 iroh 实现缺陷或配置错误，不防私钥泄露。
- 入口与凭据类型严格绑定：P2P 入口只接受 `DeviceKey`，TLS 入口只接受 `SharedSecret`，不存在跨入口降级。

## 9. 商业 relay：`yttt-relay`

### 9.1 iroh-relay 现成能力

| 需求 | 现成能力 | 缺口 |
|---|---|---|
| 认证 | `AccessControl::on_connect(&ClientRequest) -> Access`，每条连接调用一次，可获得 endpoint_id、auth token（先读 `Authorization: Bearer`，再读 URL 参数）；另有 `on_disconnect`（已核实） | 只在建连时检查，之后不再复查 |
| 身份证明 | relay 握手要求 Client 签名 ServerChallenge 或 TLS 导出的密钥材料，证明持有 EndpointId 私钥（已核实） | 无 |
| 带宽 | `Limits.client_rx: ClientRateLimit { bytes_per_second, max_burst_bytes }`，全局一个配置（已核实） | 不能按账号设置 |
| 计量 | 只有整体 metrics | 没有按 Client 的字节计数 |
| 断开 | 未见公开的按 Client 断开接口（部分核实） | 需要自行添加 |
| 新连接速率 | `accept_conn_limit`、`accept_conn_burst` 注释写明尚未实现（已核实） | 在外层（防火墙或 L4）处理 |
| 转发点钩子 | 无（部分核实） | 跨账号检查需要改转发路径本身，推迟到 P4 |

### 9.2 嵌入方式（原“fork 范围”）

实现前核对 iroh-relay 1.4.1 源码后确认不需要 fork：`protos::handshake::serverside`、`RelayedStream::new`、`server::Clients::register` / `Clients::disconnect` 与 `server::streams::Bucket` 均为公开接口，供在自建 HTTP 服务上挂载 relay 协议的嵌入方使用。`yttt-relay` 自建 HTTP/WebSocket 服务，在帧层用按账号的 `Bucket` 限速并计数，设备被移除或账号封停时调用 `Clients::disconnect`。下面四项需求不变，只是实现方式从改源码变为包装：

第一期补丁：

1. **按账号限速**：Client 上行方向的限速包装由全局配置改为按 account_id 查表，同一账号在本节点上的所有连接共享一个令牌桶。
2. **计量**：在同一位置累加上行字节到账号计数器。
3. **实时控制**：节点每 15 秒回报各账号的上行字节与在线 EndpointId；控制面回复每个账号的状态（`Active{rate}` / `Throttled{rate}` / `Suspended`）与已不属于该账号的 EndpointId。节点据此换桶、断开被移除的设备并拒绝其移除前签发的 ticket（按 `issued_unix` 比较），或断开整个账号。已建立的连接不因 ticket 到期而断开。
4. **主动断开接口**：`disconnect_account(account_id)` 和按 EndpointId 断开，用于封停和额度用尽。

推迟到 P4：转发前检查目标 endpoint 是否与发送方同属一个账号。理由：ticket 已经绑定账号，其他租户无法免费使用 relay；Host 在 `after_handshake` 就会拒绝不在 ACL 中的设备，骚扰的代价有限；而该检查需要改动转发路径，fork 面较大。

补丁保持最小，每次升级 iroh 时 rebase。

**为什么只看上行**：同账号的设备都在同一节点，任何被转发的字节都必然先从某个 Client 上行进入 relay。因此只在上行方向限速，就同时限住了两个方向；只在上行方向计数，就是该账号消耗的 relay 流量。计数包含 relay 帧头，**以节点计数为唯一计费口径**。

### 9.3 ticket（D8）

- 由控制面用 Ed25519 签名，内容为 `{account_id, endpoint_id, node_id, rate_bps, burst, issued_unix, expires_unix}`，默认有效期 30 天（可配 1 小时到 365 天），只在准入时检查。
- Client 申领 ticket：使用账号登录会话，并附带设备私钥对 `(account_id, endpoint_id, nonce)` 的签名；控制面确认该 EndpointId 已登记在此账号下。
- 出示：Client 通过 `RelayConfig` 的 auth token 出示 ticket。节点在 `on_connect` 本地验签，检查 `endpoint_id == request.endpoint_id()`（relay 握手已证明私钥持有，因此 ticket 无法转给其他设备）、`node_id` 为本节点、未过期、账号未被封停。
- 续签：已核实 iroh 1.4.1 **不会**用新 token 重连。token 在 relay 连接 actor 启动时取一次，此后无限重试都沿用它；`insert_relay`、rebind、`network_change` 都不会替换，home relay 的 actor 也不会因空闲退出。因此 ticket 设计为长期凭据：设备在剩余不足 1/3 有效期时申领新 ticket 并写入 relay map（之后新建的 relay 连接使用它），旧 actor 若在旧 ticket 过期后断线重连会被拒，此时由 yttt 重建 endpoint。换节点（URL 不同）会新建 actor，不受此限。
- 节点内置控制面公钥，并支持两把公钥并存，用于轮换。
- 控制面短时故障不影响持有有效 ticket 的连接。

### 9.4 额度执行

- 节点每 15 秒把 `{account_id: Δbytes}` 上报给控制面，控制面在响应中下发各账号的额度状态（正常 / 降速 / 封停）。
- 超出月流量额度：默认把该账号令牌桶降到 32 KB/s（终端仍可勉强使用），可选断开。
- 封停：调用 `disconnect_account`，并拒绝新连接。
- 节点在本地缓存剩余额度。与控制面失联期间按本地值执行，误差上限为一个上报周期的流量；失联超过阈值后对新连接的处理策略见 §17。

### 9.5 客户端不可信

yttt 是开源项目，必须假设任何客户端都可能被修改。计费和限速的正确性只依赖服务端：

- fork 只在服务端 `yttt-relay` 中；客户端使用标准 iroh，不包含任何计费逻辑。
- 准入、限速、计量、到期断开全部在节点上执行；客户端上报的任何数字都不参与计费，界面上的用量只是从控制面读取后展示。
- ticket 绑定 `endpoint_id`，而 relay 握手要求证明私钥持有（已核实），因此 ticket 不能转给其他设备使用。
- 修改客户端、不续签而保持长连接：计费不受影响（用量按节点计数，速率按控制面最新回复）；被移除的设备与封停账号在下一次回报（≤15 秒）时被断开（§9.2 第 3 条）。
- 设备数上限与 EndpointId 归属由控制面在登记时检查。
- 修改客户端强制只走直连，不会造成计费损失（直连本来不计费）；把 relay 当通用隧道使用，同样计量封顶，由服务条款约束。
- 用户自建 relay 不付费是允许的使用方式。
- `yttt-relay` 与控制面开源与否不影响计费正确性和安全性，二者只依赖 ticket 签名私钥和发布签名私钥保密。

### 9.6 运维

- 每个节点使用独立域名和 TLS 证书（`LetsEncrypt` 或手动）；开启 QUIC 地址探测以支持打洞；健康检查用 `/healthz`；导出 Prometheus metrics。
- 节点前面最多放 L4 负载均衡，不放会中断长连接的 L7 代理。
- 单节点容量在 Spike 阶段压测确定（官方文档给出的约 6 万并发连接**待复核**）。

## 10. 控制面

职责仅限：

- 账号、套餐（`rate_bps`、月流量 GB、设备数）。
- 设备登记：设备用私钥签名 `(account_id, endpoint_id, nonce)`，证明持有该 EndpointId；**一个 EndpointId 只能属于一个账号**（D11）。由于要求私钥证明，他人无法把你的 EndpointId 抢先登记到自己的账号下。
- 节点分配与迁移。
- ticket 签发与续签（§9.3）。
- 用量汇总、额度状态下发、对接计费服务。

明确不做（对应 I1、I2、I6）：

- 不保存、不下发 Host EndpointId 作为信任依据（可以保存用于展示，但 Client 不使用它）。
- 不提供任何写 Host ACL 的接口。
- 不下发 Host 或 Client 的配置、命令、更新。

ticket 签名私钥与发布签名私钥分开存放，属于不同的基础设施。

## 11. 多节点与迁移（D7）

- 账号创建时，按用户选择的地区或延迟分配节点。Host 和该账号下所有 Client 的 `RelayMap` 只包含这一个节点。
- 不做跨节点转发，不做全局地址发现。
- 节点故障或缩容时，控制面把账号迁到新节点；客户端在下次续签 ticket 时拿到新 URL，用 `RelayMap::insert`/`remove` 在运行时替换节点，**不重建 endpoint**（重建会断开已有的直连）。Client 侧固定的 Host 身份不受影响（I7）。
- 迁移期间，同一账号可能短暂分布在两个节点上，表现为暂时只能直连或连不上，续签后恢复。

## 12. 界面

**Host：设置 → 远程访问此电脑**

```text
P2P 远程访问                          [开启]
账号                                  user@example / 套餐 / 本月 relay 用量 3.2/50 GB
节点                                  tokyo-1
配对新设备                            [显示二维码]
已配对设备                            Windows 台式机  完整  直连   在线  [撤销]
                                      iPhone          观察  relay  离线  [撤销]
TCP/TLS 端口（高级）                  （现有界面，保持不变）
关闭全部远程访问                      [关闭 TLS 与 P2P 入口]
```

**Client：连接已有 yttt**

- 使用付费 relay 前需登录与 Host 相同的账号（D11）。
- 新增“扫码 / 粘贴配对码”。配对成功后保存到连接记录，之后一键连接。
- 标题栏显示 Host 设备标签、profile、工作区，以及当前选中的路径（直连 / relay）。
- 错误分类显示：Host 身份不一致、设备已被撤销、Client 与 Host 账号不一致、relay 额度用尽、节点不可达、Host 离线、版本不兼容。不能自动改走其他路径，也不能自动重新配对。

## 13. 计费语义（D9）

- 收费单位：relay 带宽上限 + 每月 relay 流量额度 + 设备数。
- 直连流量不经过服务器，不计量、不限制，可以作为产品卖点。
- 不为了计量而强制走 relay。
- 计量以节点计数为唯一口径（含 relay 帧头）；界面显示的用量只代表 relay 流量。

## 14. 失败与降级

| 情况 | 行为 |
|---|---|
| 控制面不可达 | 节点按已验签的 ticket 和最后一次收到的状态继续服务，未上报的字节留到下次；新连接在 ticket 有效期内照常准入；直连不受影响 |
| 节点不可达 | 已建立的直连不受影响；新连接凭保存的直连地址或本地发现尝试直连，失败时提示“节点不可达” |
| 额度用尽 | 降速或断开，界面明确提示；直连不受影响 |
| 账号被封停 | 节点断开该账号所有 relay 连接；直连不受影响 |
| Client 与 Host 账号不一致 | 无法申领可用的 ticket，提示“账号不一致”；直连仍可用 |
| Host EndpointId 不一致 | 立即失败，不发送应用数据，提示需要重新扫码 |
| 设备被撤销 | Host 关闭该设备全部连接，Client 显示“已被撤销”，不自动重新配对 |
| UDP 被封 | 只走 relay（HTTPS 443） |
| Windows 防火墙 | 首次绑定 UDP 时可能弹出授权提示；在开启 P2P 时提前说明 |

## 15. 分期

1. **Spike（1–2 天）**：macOS Host 与 Windows Client 之间，用 iroh 1.0.x 加一台自建 relay 跑 echo 和真实 session。测量直连成功率、relay 延迟、Wi-Fi 切换后恢复情况、单节点容量；确认全部“待复核”条目。
2. **P1 自用 P2P**：
   - `yttt-transport-iroh`（`after_handshake` 过滤、逐 stream 查 ACL、按 EndpointId 关闭连接、直连地址提示）。
   - Transport 对端元数据、Host 级准入重构、按入口分开的握手上限。
   - v4 握手（双向签名、proof 枚举、EndpointId 入 transcript）。
   - 设备 ACL 与按设备的 capability 校验。
   - 配对协议（commit-reveal、SAS、`PairAccepted`、并发作废）与粘贴配对码。
   - relay 使用自建 URL，不接控制面。TLS 入口保持不变。
3. **P2 扫码与设备管理**：二维码渲染与应用内扫码、设备列表与撤销、路径显示、审计日志与本机通知、全局关闭开关。
4. **P3 商业 relay**：`yttt-relay`（ticket 验签、按账号限速与计量、按控制面回复换速、断开移除设备与封停账号）、控制面（账号、带私钥证明的设备登记、ticket、用量）、Client 与 Host 账号登录。
5. **P4**：多地区节点与迁移、跨账号转发检查、移动端、PAKE 短码、按键时序混淆。

## 16. 验收与测试

安全不变量（自动化测试）：

1. P2P 入口收到 ACL 之外的 EndpointId，在 `after_handshake` 阶段拒绝，应用握手不开始（I4）。
2. 设备被撤销后：该设备已建立的 QUIC 连接被关闭；如果在关闭前开出新 stream，该 stream 在应用握手前被拒绝；新连接被拒绝（I8）。并发执行撤销与准入的压力测试中，不出现撤销后仍被准入的会话。
3. 网络入口的协议枚举中不存在能修改 ACL 的消息（I2），用编译期或协议枚举测试保证。
4. Client 收到的 Host EndpointId、EnvironmentId 或 ProfileId 与固定值不一致，或 Host 签名无效时，不发送任何应用数据（I1、I3）。
5. 应用层签名中任一字段（nonce、任一方 EndpointId、Host epoch、通道）被替换时，双方各自认证失败（I3）。
6. P2P 入口出示 `SharedSecret` 凭据、TLS 入口出示 `DeviceKey` 凭据，均被拒绝（无跨入口降级）。
7. 传输中篡改或注入任意字节，连接失败，不产生应用层可见的错误数据。
8. 配对：邀请过期、重复使用、`mac` 错误、commit 与 reveal 不符、SAS 被拒绝、本机取消，都不能写入 ACL；同一邀请收到第二个有效请求时，邀请作废并产生告警；Client 在收到 `PairAccepted` 前断开，不保存任何 Host 身份。
9. TLS 入口的未认证握手打满时，已配对设备仍能通过 P2P 连接；关闭 TLS 入口不影响 P2P 会话，反之亦然。
10. `observe` 预设的设备无法请求接管，也无法执行任何 Mutate 类请求。
11. relay：endpoint_id 与 ticket 不符、过期、节点不符的 ticket 被拒绝；已建立的连接在 ticket 到期后继续，但不能用过期 ticket 重连；移除的设备被断开，且只有移除后签发的 ticket 能再准入；封停断开该账号全部连接；Host 的认证路径不读取 ticket（I5）。
12. 修改过的客户端：出示其他设备的 ticket、不出示 ticket、超速发送、设备被移除后保持长连接，节点分别拒绝、拒绝、限速、在下次回报时断开；计量结果只来自节点计数。
13. 控制面：没有私钥签名的设备登记被拒绝；同一 EndpointId 无法登记到第二个账号。

功能与性能：

- 直连与 relay 路径下终端输入回显延迟、断线恢复，与现有 TLS 路径对比。
- 按账号限速：同一账号多连接共享令牌桶。
- 计量：节点计数与节点侧抓包统计的字节数一致。
- 额度降速与封停的切换时间不超过一个上报周期。
- 节点迁移：账号迁移后，客户端在一个续签周期内恢复连接，已有直连不中断。
- relay 不可用时，局域网内凭直连地址提示或本地发现可以连接。

## 17. 开放问题

1. `RelayMap::insert` 换入带新 token 的 `RelayConfig` 后，iroh 是否自动重连 relay。
2. relay 对同一 EndpointId 的新连接是否会顶掉旧连接（影响续签与迁移时的行为）。
3. iroh 1.0 中本地网络发现与 `paths()` 的 API 名称。
4. 单节点实际容量。
5. 节点与控制面失联超过多久后，对新连接按什么策略处理（全部拒绝、按本地缓存放行、降速放行）。
6. 账号体系是否需要支持一个账号下多个 Host，以及它们是否共享额度。

## 18. 参考

项目内：

- [p2p-relay-architecture](../p2p-relay-architecture.md)
- [desktop-host-remote-access-design](2026-09-06-desktop-host-remote-access-design.md)
- `crates/yttt-transport/src/stream.rs`、`crates/yttt-transport/src/auth.rs`
- `crates/yttt-transport-tls/src/lib.rs`
- `crates/yttt-host/src/remote_access.rs`、`crates/yttt-host/src/lib.rs`（`authorize_ingress`）
- `crates/yttt-protocol/src/remote_access.rs`、`crates/yttt-protocol/src/handshake.rs`、`crates/yttt-protocol/src/control.rs`（`Capability`）

外部：

- [iroh-relay server.rs（库 API）](https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/server.rs)
- [iroh-relay main.rs（二进制配置）](https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/main.rs)
- [iroh-relay handshake.rs（relay 身份证明）](https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/protos/handshake.rs)
- [iroh-relay relay_map.rs](https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/relay_map.rs)
- [iroh 1.0.1 endpoint hooks](https://docs.rs/crate/iroh/1.0.1/source/src/endpoint/hooks.rs)
- [iroh 0.96 multipath](https://iroh-computer-mcnwp3q4m-number-0.vercel.app/blog/iroh-0-96-0-the-multipaths-to-1-0)
- [Authenticated relays](https://iroh.computer/blog/authenticated-relays)
- [Self-hosted relays](https://docs.iroh.computer/iroh-services/relays/self-hosted.md)
- [Dedicated infrastructure](https://docs.iroh.computer/deployment/dedicated-infrastructure)
- [Relays concept](https://docs.iroh.computer/concepts/relays)
